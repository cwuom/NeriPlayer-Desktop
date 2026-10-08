// 三方合并算法：对齐 Android GitHubSyncManager.performThreeWayMerge
use std::collections::{BTreeMap, HashMap, HashSet};

use super::models::*;

const MAX_SYNC_LOG: usize = 100;
#[cfg(test)]
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

pub fn three_way_merge(
    local: &SyncData,
    remote: &SyncData,
    last_sync_time: i64,
    base_snapshot: &HashMap<String, HashSet<String>>,
) -> SyncData {
    let mut local = local.normalized_for_sync();
    let mut remote = remote.normalized_for_sync();
    // 先收集各候选的歌词版本，展示元数据的选择不能丢掉独立歌词修改
    for data in [&mut local, &mut remote] {
        if let Err(error) = super::archive::capture_legacy_lyrics(data).and_then(|_|converge_lyrics(data,false)) {
            log::warn!(target:"sync","invalid lyric metadata will be rejected before publication: {error}");
        }
    }
    let playlist_deletions = merge_playlist_song_deletions(
        &local.playlist_song_deletions,
        &remote.playlist_song_deletions,
    );
    let playlists = merge_playlists(
        &local.playlists,
        &remote.playlists,
        last_sync_time,
        base_snapshot,
        &playlist_deletions,
    );
    let playlist_deletions = prune_playlist_song_deletions(&playlist_deletions, &playlists);
    let favorites = merge_favorite_playlists(&local.favorite_playlists, &remote.favorite_playlists);
    let recent_deletions = merge_recent_play_deletions(
        &local.recent_play_deletions,
        &remote.recent_play_deletions,
    );
    let recent = merge_recent_plays(&local.recent_plays, &remote.recent_plays, &recent_deletions);
    let recent_deletions = prune_recent_play_deletions(&recent_deletions, &recent);
    let playback_stats_cleared_at = local
        .playback_stats_cleared_at
        .max(remote.playback_stats_cleared_at)
        .max(0);

    let stat_buckets = merge_stat_buckets(
        &local.playback_stat_buckets,
        &remote.playback_stat_buckets,
        playback_stats_cleared_at,
    );
    let playback_stats = merge_playback_stats(
        &local.playback_stats,
        &remote.playback_stats,
        playback_stats_cleared_at,
    );
    // 全量分桶抬升聚合值，历史数据由归档预算保护，不能靠裁剪绕过预算
    let playback_stats = lift_stats_to_bucket_totals(&playback_stats, &stat_buckets);

    let mut result = SyncData {
        version: "2.0".into(),
        device_id: local.device_id.clone(),
        device_name: local.device_name.clone(),
        last_modified: chrono::Utc::now().timestamp_millis(),
        playlists,
        favorite_playlists: favorites,
        recent_plays: recent,
        sync_log: merge_sync_log(&local.sync_log, &remote.sync_log),
        recent_play_deletions: recent_deletions,
        playback_stats,
        playback_stats_cleared_at,
        playback_stat_buckets: stat_buckets,
        playlist_song_deletions: playlist_deletions,
        extensions: merge_extensions(&local.extensions, &remote.extensions),
    };
    // 歌词版本独立于歌单重排和歌曲展示元数据
    let _ = converge_lyrics(&mut result, false);
    result
}

pub fn converge_lyrics(data: &mut SyncData, validate_references: bool) -> crate::error::AppResult<()> {
    let mut overrides: HashMap<String, SyncSong> = HashMap::new();
    if let Some(values) = data.extensions.get("lyricOverrides").and_then(serde_json::Value::as_array) {
        for value in values {
            let song: SyncSong = serde_json::from_value(value.clone()).map_err(|error| crate::error::AppError::Other(format!("Invalid lyric override: {error}")))?;
            select_lyric_override(&mut overrides, normalize_lyric_state(&song));
        }
    }
    if validate_references {
        for song in data.playlists.iter().flat_map(|playlist|playlist.songs.iter()).chain(data.favorite_playlists.iter().flat_map(|playlist|playlist.songs.iter())).chain(data.recent_plays.iter().map(|play|&play.song)) {
            if song.lyric_sync_edited.is_some() && (song.lyric_sync_revision>0 || song.lyric_sync_edited==Some(true)) && overrides.get(&song.identity().stable_key()).is_none_or(|item|item.lyric_sync_revision<song.lyric_sync_revision) {
                return Err(crate::error::AppError::Other("Sync lyric reference has no committed override".into()));
            }
        }
    }
    for song in data.playlists.iter().flat_map(|playlist|playlist.songs.iter()).chain(data.favorite_playlists.iter().flat_map(|playlist|playlist.songs.iter())).chain(data.recent_plays.iter().map(|play|&play.song)) {
        select_lyric_override(&mut overrides, normalize_lyric_state(song));
    }
    for song in data.playlists.iter_mut().flat_map(|playlist|playlist.songs.iter_mut()).chain(data.favorite_playlists.iter_mut().flat_map(|playlist|playlist.songs.iter_mut())).chain(data.recent_plays.iter_mut().map(|play|&mut play.song)) {
        let mut normalized=normalize_lyric_state(song);
        if let Some(latest)=overrides.get(&song.identity().stable_key()) {copy_lyric_state(&mut normalized,latest);}
        *song=normalized;
    }
    let mut values: Vec<_> = overrides.into_iter().collect(); values.sort_by(|left,right|left.0.cmp(&right.0));
    let values=values.into_iter().map(|(_,song)|serde_json::to_value(lyric_override_record(&song))).collect::<Result<Vec<_>,_>>().map_err(|error|crate::error::AppError::Other(format!("Serialize lyric overrides: {error}")))?;
    if !values.is_empty() || data.extensions.contains_key("lyricOverrides") {data.extensions.insert("lyricOverrides".into(),serde_json::Value::Array(values));}
    Ok(())
}

/// 把一首歌当前的歌词状态并入覆盖记录，修订号更高才替换（对齐 Android SyncLyricOverrideStore）
pub fn record_lyric_override(
    extensions: &mut serde_json::Map<String, serde_json::Value>,
    song: &SyncSong,
) -> crate::error::AppResult<()> {
    let mut data = SyncData {
        extensions: std::mem::take(extensions),
        playlists: vec![SyncPlaylist {
            id: String::new(),
            name: String::new(),
            songs: vec![song.clone()],
            created_at: 0,
            modified_at: 0,
            is_deleted: false,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        }],
        ..Default::default()
    };
    let result = converge_lyrics(&mut data, false);
    *extensions = data.extensions;
    result
}

/// 记一次歌单打开（对齐 Android PlaylistUsageRepository.recordOpen）
///
/// 只增加本机的计数分片（epoch 0），其它设备的分片原样保留，同步时按分片相加；
/// 歌单已经没有歌曲时移除记录。返回扩展段是否有变化
pub(crate) fn record_playlist_open(
    extensions: &mut serde_json::Map<String, serde_json::Value>,
    open: &crate::library::playlist_usage::PlaylistUsageOpen,
    device_id: &str,
    now: i64,
) -> bool {
    let mut stats = extensions
        .get("playlistUsageStats")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let index = stats.iter().position(|entry| entry["playlistKey"].as_str() == Some(open.key.as_str()));
    if open.track_count <= 0 {
        let Some(index) = index else { return false };
        stats.remove(index);
        extensions.insert("playlistUsageStats".into(), serde_json::Value::Array(stats));
        return true;
    }

    // 在其它设备被移除过的歌单要带上已见过的删除令牌，否则合并时会被当成删除前的旧记录丢掉
    let observed = normalize_sync_causal_tokens(
        &extensions
            .get("playlistUsageDeletions")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|deletion| deletion["playlistKey"].as_str() == Some(open.key.as_str()))
            .flat_map(usage_deletion_tokens)
            .collect::<Vec<_>>(),
    );
    let previous = index.map(|index| &stats[index]).filter(|entry| {
        let tokens = json_tokens(entry, "observedDeletionTokens");
        observed.iter().all(|token| tokens.contains(token))
    });
    let mut entry = match previous {
        Some(previous) => {
            let mut entry = previous.clone();
            if let Some(cover) = &open.cover_url {
                entry["coverUrl"] = cover.clone().into();
            }
            if let Some(subtitle) = &open.subtitle {
                entry["subtitle"] = subtitle.clone().into();
            }
            let tokens = [json_tokens(previous, "observedDeletionTokens"), observed].concat();
            entry["observedDeletionTokens"] = serde_json::json!(normalize_sync_causal_tokens(&tokens));
            entry
        }
        None => serde_json::json!({
            "playlistKey": open.key,
            "source": open.source,
            "id": open.id,
            "coverUrl": open.cover_url,
            "subtitle": open.subtitle,
            "firstOpenedAt": now,
            "lastOpenedAt": now,
            "openCount": 0,
            "counterBaseOpenCount": 0,
            "counterShards": [],
            "observedDeletionTokens": observed,
        }),
    };
    entry["name"] = open.name.clone().into();
    entry["trackCount"] = open.track_count.min(i64::from(i32::MAX)).into();
    entry["fid"] = open.fid.into();
    entry["mid"] = open.mid.into();
    entry["browseId"] = serde_json::json!(open.browse_id);
    entry["playlistId"] = serde_json::json!(open.playlist_id);
    entry["subtype"] = serde_json::json!(open.subtype);

    count_on_device_shard(&mut entry, ("openCount", "counterBaseOpenCount"), device_id, now, i64::from(i32::MAX));
    entry["firstOpenedAt"] = min_positive(json_i64(&entry, "firstOpenedAt"), now).into();
    entry["lastOpenedAt"] = json_i64(&entry, "lastOpenedAt").max(now).into();

    match index {
        Some(index) => stats[index] = entry,
        None => stats.push(entry),
    }
    extensions.insert("playlistUsageStats".into(), serde_json::Value::Array(stats));
    true
}

/// 记一次本地歌单播放（对齐 Android LocalPlaylistPlaybackStatsRepository.recordPlayNow）：
/// 歌单总数和当天分桶各在本机分片上加一
pub(crate) fn record_local_playlist_play(
    extensions: &mut serde_json::Map<String, serde_json::Value>,
    playlist_id: i64,
    day_start_at: i64,
    device_id: &str,
    played_at: i64,
) -> bool {
    if playlist_id == 0 {
        return false;
    }
    let record = |section: &str, matches: &dyn Fn(&serde_json::Value) -> bool, empty: serde_json::Value, count_key: &str| {
        let mut items = extensions.get(section).and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
        let index = items.iter().position(matches);
        let mut item = index.map_or(empty, |index| items[index].clone());
        count_on_device_shard(&mut item, (count_key, "counterBasePlayCount"), device_id, played_at, i64::MAX);
        item["firstPlayedAt"] = min_positive(json_i64(&item, "firstPlayedAt"), played_at).into();
        item["lastPlayedAt"] = json_i64(&item, "lastPlayedAt").max(played_at).into();
        match index {
            Some(index) => items[index] = item,
            None => items.push(item),
        }
        (section.to_string(), serde_json::Value::Array(items))
    };
    let stats = record(
        "localPlaylistPlaybackStats",
        &|stat| json_i64(stat, "playlistId") == playlist_id,
        serde_json::json!({"playlistId": playlist_id, "totalPlayCount": 0, "lastPlayedAt": 0, "firstPlayedAt": 0, "counterBasePlayCount": 0, "counterShards": []}),
        "totalPlayCount",
    );
    let buckets = record(
        "localPlaylistPlaybackBuckets",
        &|bucket| json_i64(bucket, "playlistId") == playlist_id && json_i64(bucket, "dayStartAt") == day_start_at,
        serde_json::json!({"dayStartAt": day_start_at, "playlistId": playlist_id, "playCount": 0, "lastPlayedAt": 0, "firstPlayedAt": 0, "counterBasePlayCount": 0, "counterShards": []}),
        "playCount",
    );
    for (section, items) in [stats, buckets] {
        extensions.insert(section, items);
    }
    true
}

/// 本机分片（epoch 0）加一，总数取原值与「基数 + 各分片之和」的较大者，其它设备的分片原样保留
/// （对齐 Android UsageEntry.recordOpen / updateLocalPlaylistCounter）
fn count_on_device_shard(
    record: &mut serde_json::Value,
    (count_key, base_key): (&str, &str),
    device_id: &str,
    at: i64,
    count_limit: i64,
) {
    let shards: Vec<SyncPlaybackCounterShard> =
        serde_json::from_value(record["counterShards"].clone()).unwrap_or_default();
    let mut shards = merge_counter_shards(&shards, &[]);
    let shard_total = |shards: &[SyncPlaybackCounterShard]| {
        shards.iter().fold(0_i64, |total, shard| total.saturating_add(i64::from(shard.play_count.max(0))))
    };
    let count = json_i64(record, count_key).max(0);
    let base = if shards.is_empty() {
        count
    } else {
        json_i64(record, base_key).max(0).max(count.saturating_sub(shard_total(&shards)))
    };
    match shards.iter_mut().find(|shard| shard.device_id == device_id && shard.epoch_started_at == 0) {
        Some(shard) => {
            shard.play_count = shard.play_count.saturating_add(1);
            shard.first_played_at = min_positive(shard.first_played_at, at);
            shard.last_played_at = shard.last_played_at.max(at);
        }
        None => shards.push(SyncPlaybackCounterShard {
            device_id: device_id.to_string(),
            play_count: 1,
            first_played_at: at,
            last_played_at: at,
            ..Default::default()
        }),
    }
    let shards = merge_counter_shards(&shards, &[]);
    record[count_key] = count.max(base.saturating_add(shard_total(&shards))).min(count_limit).into();
    record[base_key] = base.into();
    record["counterShards"] = serde_json::json!(shards);
}

fn normalize_lyric_state(song: &SyncSong) -> SyncSong {
    let mut song=song.clone();
    let has_text=[song.matched_lyric.as_ref(),song.matched_translated_lyric.as_ref(),song.matched_romanized_lyric.as_ref(),song.original_lyric.as_ref(),song.original_translated_lyric.as_ref(),song.original_romanized_lyric.as_ref()].iter().any(|value|value.is_some());
    let channel=song.channel_id.as_deref().unwrap_or_default().trim();
    let bilibili=if channel.is_empty(){song.identity().album.to_ascii_lowercase().starts_with("bilibili")}else{channel.eq_ignore_ascii_case("bilibili")};
    if has_text && (song.lyric_sync_edited.is_none() || (song.lyric_sync_edited==Some(false) && song.lyric_sync_revision<=0 && bilibili)) {
        song.lyric_sync_edited=Some(true); song.lyric_sync_revision=1;
        if song.matched_lyric.is_none() {song.matched_lyric=song.original_lyric.clone();}
        if song.matched_translated_lyric.is_none() {song.matched_translated_lyric=song.original_translated_lyric.clone();}
        if song.matched_romanized_lyric.is_none() {song.matched_romanized_lyric=song.original_romanized_lyric.clone();}
    } else if song.lyric_sync_edited==Some(true) {song.lyric_sync_revision=song.lyric_sync_revision.max(1);}
    else if song.lyric_sync_edited==Some(false) || song.lyric_sync_revision!=0 {
        if song.lyric_sync_edited.is_none(){song.lyric_sync_revision=0;}else{song.lyric_sync_revision=song.lyric_sync_revision.max(0);}
        song.lyric_sync_edited=Some(false);
        song.matched_lyric=None; song.matched_translated_lyric=None; song.matched_romanized_lyric=None;
        song.original_lyric=None; song.original_translated_lyric=None; song.original_romanized_lyric=None;
    }
    song
}

fn lyric_payload_key(song:&SyncSong)->Vec<u16> {
    let key:String=[song.matched_lyric.as_ref(),song.matched_translated_lyric.as_ref(),song.matched_romanized_lyric.as_ref(),song.original_lyric.as_ref(),song.original_translated_lyric.as_ref(),song.original_romanized_lyric.as_ref(),song.matched_lyric_source.as_ref(),song.matched_song_id.as_ref()].iter().map(|value|match value {Some(value)=>format!("{}:{value}",value.encode_utf16().count()),None=>"-1:".into()}).collect();
    key.encode_utf16().collect()
}

fn lyric_override_record(song:&SyncSong)->SyncSong {
    let mut record=SyncSong {id:song.id.clone(),album:song.album.clone(),media_uri:song.media_uri.clone(),channel_id:song.channel_id.clone(),audio_id:song.audio_id.clone(),sub_audio_id:song.sub_audio_id.clone(),..Default::default()};
    copy_lyric_state(&mut record,song);record
}

fn select_lyric_override(overrides:&mut HashMap<String,SyncSong>,song:SyncSong) {
    if song.lyric_sync_revision<=0 {return;}
    let key=song.identity().stable_key();
    let incoming=(song.lyric_sync_revision,song.lyric_sync_edited==Some(false),lyric_payload_key(&song));
    let replace=overrides.get(&key).is_none_or(|previous|incoming>(previous.lyric_sync_revision,previous.lyric_sync_edited==Some(false),lyric_payload_key(previous)));
    if replace {overrides.insert(key,song);}
}

fn copy_lyric_state(target:&mut SyncSong,source:&SyncSong) {
    target.matched_lyric=source.matched_lyric.clone(); target.matched_translated_lyric=source.matched_translated_lyric.clone(); target.matched_romanized_lyric=source.matched_romanized_lyric.clone();
    target.original_lyric=source.original_lyric.clone(); target.original_translated_lyric=source.original_translated_lyric.clone(); target.original_romanized_lyric=source.original_romanized_lyric.clone();
    target.matched_lyric_source=source.matched_lyric_source.clone(); target.matched_song_id=source.matched_song_id.clone(); target.lyric_sync_revision=source.lyric_sync_revision; target.lyric_sync_edited=source.lyric_sync_edited;
}

fn merge_extensions(local:&serde_json::Map<String,serde_json::Value>,remote:&serde_json::Map<String,serde_json::Value>)->serde_json::Map<String,serde_json::Value> {
    let mut result=remote.clone();
    for (key,value) in local {
        if let Some(items)=value.as_array() {
            let mut merged=items.clone(); merged.extend(remote.get(key).and_then(serde_json::Value::as_array).into_iter().flatten().cloned());
            merged.sort_by_key(serde_json::Value::to_string); merged.dedup();
            result.insert(key.clone(),serde_json::Value::Array(merged));
        } else {result.entry(key.clone()).or_insert_with(||value.clone());}
    }
    for (section,keys,timestamp) in [
        ("playlistUsageDeletions",&["playlistKey"][..],"deletedAt"),
        ("playlistUsageStats",&["playlistKey"][..],"lastOpenedAt"),
        ("localPlaylistPlaybackStats",&["playlistId"][..],"lastPlayedAt"),
        ("localPlaylistPlaybackBuckets",&["playlistId","dayStartAt"][..],"lastPlayedAt"),
        ("biliVideoSkipRules",&["bvid","cid"][..],"modifiedAt"),
    ] {
        let mut groups:BTreeMap<String,serde_json::Value>=BTreeMap::new();
        let Some(items)=result.get(section).and_then(serde_json::Value::as_array).cloned() else {continue;};
        for item in items {
            if matches!(section,"localPlaylistPlaybackStats"|"localPlaylistPlaybackBuckets") && (json_i64(&item,"playlistId")==0 || (section=="localPlaylistPlaybackBuckets" && json_i64(&item,"dayStartAt")<0)) {continue;}
            let key=keys.iter().map(|key|item[*key].to_string()).collect::<Vec<_>>().join("|");
            if section=="playlistUsageStats" && !usage_observes_deletions(&item,&result) {continue;}
            groups.entry(key).and_modify(|previous|*previous=merge_metadata_record(section,previous,&item,timestamp)).or_insert_with(||merge_metadata_record(section,&item,&item,timestamp));
        }
        result.insert(section.into(),serde_json::Value::Array(groups.into_values().collect()));
    }
    lift_local_playlist_stats(&mut result);
    result
}

fn lift_local_playlist_stats(extensions:&mut serde_json::Map<String,serde_json::Value>) {
    let mut totals:BTreeMap<i64,(i64,i64,i64)>=BTreeMap::new();
    for bucket in extensions.get("localPlaylistPlaybackBuckets").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let value=totals.entry(json_i64(bucket,"playlistId")).or_default();
        value.0=value.0.saturating_add(json_i64(bucket,"playCount").max(0));
        value.1=min_positive(value.1,json_i64(bucket,"firstPlayedAt"));
        value.2=value.2.max(json_i64(bucket,"lastPlayedAt"));
    }
    if totals.is_empty() {return;}
    let mut stats=extensions.get("localPlaylistPlaybackStats").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
    for stat in &mut stats {
        if let Some((count,first,last))=totals.remove(&json_i64(stat,"playlistId")) {
            stat["totalPlayCount"]=json_i64(stat,"totalPlayCount").max(count).into();
            stat["firstPlayedAt"]=min_positive(json_i64(stat,"firstPlayedAt"),first).into();
            stat["lastPlayedAt"]=json_i64(stat,"lastPlayedAt").max(last).into();
        }
    }
    stats.extend(totals.into_iter().map(|(id,(count,first,last))|serde_json::json!({"playlistId":id,"totalPlayCount":count,"firstPlayedAt":first,"lastPlayedAt":last,"counterBasePlayCount":0,"counterShards":[]})));
    stats.sort_by_key(|stat|json_i64(stat,"playlistId"));
    extensions.insert("localPlaylistPlaybackStats".into(),serde_json::Value::Array(stats));
}

fn json_i64(value:&serde_json::Value,key:&str)->i64 {value[key].as_i64().or_else(||value[key].as_str()?.parse().ok()).unwrap_or(0)}
fn json_tokens(value:&serde_json::Value,key:&str)->Vec<SyncCausalToken> {serde_json::from_value(value[key].clone()).map(|tokens:Vec<SyncCausalToken>|normalize_sync_causal_tokens(&tokens)).unwrap_or_default()}

fn usage_deletion_tokens(value:&serde_json::Value)->Vec<SyncCausalToken> {
    let tokens=json_tokens(value,"deletionTokens");
    if !tokens.is_empty() || json_i64(value,"deletedAt")<=0 {return tokens;}
    let encoded=value["playlistKey"].as_str().unwrap_or_default().trim().encode_utf16().map(|code|format!("{code:04x}")).collect::<String>();
    vec![SyncCausalToken{device_id:format!("usage-legacy:{encoded}"),counter:json_i64(value,"deletedAt").max(1)}]
}

fn usage_observes_deletions(value:&serde_json::Value,extensions:&serde_json::Map<String,serde_json::Value>)->bool {
    let observed=json_tokens(value,"observedDeletionTokens");
    extensions.get("playlistUsageDeletions").and_then(serde_json::Value::as_array).into_iter().flatten().filter(|deletion|deletion["playlistKey"]==value["playlistKey"]).all(|deletion|usage_deletion_tokens(deletion).iter().all(|token|observed.contains(token)))
}

fn merge_metadata_record(section:&str,left:&serde_json::Value,right:&serde_json::Value,timestamp:&str)->serde_json::Value {
    let newer=if (json_i64(left,timestamp),left.to_string())>=(json_i64(right,timestamp),right.to_string()){left}else{right};
    let mut result=newer.clone();
    if section=="playlistUsageDeletions" {
        let tokens=normalize_sync_causal_tokens(&[usage_deletion_tokens(left),usage_deletion_tokens(right)].concat());
        result["deletionTokens"]=serde_json::json!(tokens); result["deletedAt"]=json_i64(left,"deletedAt").max(json_i64(right,"deletedAt")).max(0).into(); return result;
    }
    if section=="biliVideoSkipRules" {
        if json_i64(left,timestamp)!=json_i64(right,timestamp) {return result;}
        let left_deleted=left["isDeleted"].as_bool().unwrap_or(false); let right_deleted=right["isDeleted"].as_bool().unwrap_or(false);
        if left_deleted && right_deleted {result["intervals"]=serde_json::json!([]); return result;}
        if left_deleted {return right.clone();}
        if right_deleted {return left.clone();}
        let mut intervals:Vec<(i64,i64)>=left["intervals"].as_array().into_iter().flatten().chain(right["intervals"].as_array().into_iter().flatten()).map(|value|(json_i64(value,"startMs").max(0),json_i64(value,"endMs").max(0))).filter(|(start,end)|end>start).collect(); intervals.sort_unstable();
        let mut merged:Vec<(i64,i64)>=Vec::new(); for (start,end) in intervals {if let Some(previous)=merged.last_mut().filter(|previous|start<=previous.1){previous.1=previous.1.max(end);}else{merged.push((start,end));}}
        result["intervals"]=serde_json::json!(merged.into_iter().map(|(start,end)|serde_json::json!({"startMs":start,"endMs":end})).collect::<Vec<_>>()); return result;
    }
    let (count,base,first,last)=if section=="playlistUsageStats" {("openCount","counterBaseOpenCount","firstOpenedAt","lastOpenedAt")}else if section=="localPlaylistPlaybackStats" {("totalPlayCount","counterBasePlayCount","firstPlayedAt","lastPlayedAt")}else{("playCount","counterBasePlayCount","firstPlayedAt","lastPlayedAt")};
    let left_shards:Vec<SyncPlaybackCounterShard>=serde_json::from_value(left["counterShards"].clone()).unwrap_or_default();
    let right_shards:Vec<SyncPlaybackCounterShard>=serde_json::from_value(right["counterShards"].clone()).unwrap_or_default();
    let shards=merge_counter_shards(&left_shards,&right_shards); let shard_count=shards.iter().fold(0_i64,|total,shard|total.saturating_add(i64::from(shard.play_count.max(0))));
    let effective_base=|value:&serde_json::Value,empty:bool|json_i64(value,base).max(0).max(if empty{json_i64(value,count).max(0)}else{json_i64(value,count).saturating_sub(shard_count).max(0)});
    let base_count=if shards.is_empty(){0}else{effective_base(left,left_shards.is_empty()).max(effective_base(right,right_shards.is_empty()))};
    let total=json_i64(left,count).max(json_i64(right,count)).max(base_count.saturating_add(shard_count)).max(0);
    result[count]=if section=="playlistUsageStats"{total.min(i64::from(i32::MAX))}else{total}.into(); result[base]=base_count.into();
    result[first]=min_positive(json_i64(left,first),json_i64(right,first)).into(); result[last]=json_i64(left,last).max(json_i64(right,last)).into();
    if section!="playlistUsageStats" {result[first]=shards.iter().fold(json_i64(&result,first),|value,shard|min_positive(value,shard.first_played_at)).into();result[last]=shards.iter().fold(json_i64(&result,last),|value,shard|value.max(shard.last_played_at)).into();}
    result["counterShards"]=serde_json::json!(shards);
    if section=="playlistUsageStats" {result["observedDeletionTokens"]=serde_json::json!(normalize_sync_causal_tokens(&[json_tokens(left,"observedDeletionTokens"),json_tokens(right,"observedDeletionTokens")].concat()));}
    result
}

/// 聚合统计不得小于同曲目日分桶之和
///
/// Android 的「总」读聚合值, 「日/周/月/年」读日分桶求和; 两者用了不同的合并代数
/// (聚合取 max, 分桶按天取 max 后再求和), 于是会出现「年 > 总」。
/// 这里只做单调抬升: 结果只增不减, 因此与对端的 max 合并天然收敛, 不会产生回声。
pub(super) fn lift_stats_to_bucket_totals(
    stats: &[SyncTrackStat],
    buckets: &[SyncPlaybackStatBucket],
) -> Vec<SyncTrackStat> {
    if buckets.is_empty() {
        return stats.to_vec();
    }
    let mut totals: HashMap<&str, (i64, i64)> = HashMap::new();
    for bucket in buckets {
        let entry = totals.entry(bucket.identity_key.as_str()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(bucket.total_listen_ms.max(0));
        entry.1 = entry.1.saturating_add(i64::from(bucket.play_count.max(0)));
    }

    stats
        .iter()
        .map(|stat| {
            let Some((listen_ms, play_count)) = totals.get(stat.identity_key.as_str()) else {
                return stat.clone();
            };
            let mut lifted = stat.clone();
            lifted.total_listen_ms = lifted.total_listen_ms.max(*listen_ms);
            lifted.play_count = lifted
                .play_count
                .max(i32::try_from(*play_count).unwrap_or(i32::MAX));
            lifted
        })
        .collect()
}

fn merge_playlists(
    local: &[SyncPlaylist],
    remote: &[SyncPlaylist],
    last_sync_time: i64,
    base_snapshot: &HashMap<String, HashSet<String>>,
    deletions: &[SyncPlaylistSongDeletion],
) -> Vec<SyncPlaylist> {
    let local_map: HashMap<&str, &SyncPlaylist> = local
        .iter()
        .map(|playlist| (playlist.id.as_str(), playlist))
        .collect();
    let remote_map: HashMap<&str, &SyncPlaylist> = remote
        .iter()
        .map(|playlist| (playlist.id.as_str(), playlist))
        .collect();
    let all_ids: HashSet<&str> = local_map
        .keys()
        .chain(remote_map.keys())
        .copied()
        .collect();
    let mut merged = BTreeMap::new();

    for id in all_ids {
        let result = match (local_map.get(id).copied(), remote_map.get(id).copied()) {
            (Some(local_playlist), None) => apply_deletions_to_playlist(local_playlist, deletions),
            (None, Some(remote_playlist)) => apply_deletions_to_playlist(remote_playlist, deletions),
            (Some(local_playlist), Some(remote_playlist)) => {
                if should_keep_playlist_deleted(local_playlist, remote_playlist) {
                    merge_deleted_playlist(local_playlist, remote_playlist)
                } else if local_playlist.is_deleted || remote_playlist.is_deleted {
                    // 另一端在删除之后又改过（或 Android 撤销了删除），活着的那份赢
                    let active = if local_playlist.is_deleted { remote_playlist } else { local_playlist };
                    apply_deletions_to_playlist(active, deletions)
                } else {
                    merge_single_playlist(
                        local_playlist,
                        remote_playlist,
                        last_sync_time,
                        base_snapshot.get(id).cloned().unwrap_or_default(),
                        deletions,
                    )
                }
            }
            (None, None) => continue,
        };
        merged.insert(id.to_string(), result);
    }

    order_merged_playlists(&merged, local, remote, last_sync_time)
        .into_iter()
        .map(|playlist| playlist.normalized_for_display_order())
        .collect()
}

fn apply_deletions_to_playlist(
    playlist: &SyncPlaylist,
    deletions: &[SyncPlaylistSongDeletion],
) -> SyncPlaylist {
    if playlist.is_deleted {
        return playlist.clone();
    }
    let mut merged = playlist.clone();
    merged.songs = apply_playlist_song_deletions(&playlist.id, &playlist.songs, deletions);
    merged
}

/// 对齐 Android SyncPlaylistDeletionPolicy.shouldKeepPlaylistDeleted：
/// 一端删除、另一端还在时，只有删除不早于那份歌单的最后修改才保留墓碑
fn should_keep_playlist_deleted(left: &SyncPlaylist, right: &SyncPlaylist) -> bool {
    match (left.is_deleted, right.is_deleted) {
        (false, false) => false,
        (true, true) => true,
        _ => {
            let (deleted, active) = if left.is_deleted { (left, right) } else { (right, left) };
            deleted.modified_at >= active.modified_at
        }
    }
}

fn merge_deleted_playlist(local: &SyncPlaylist, remote: &SyncPlaylist) -> SyncPlaylist {
    SyncPlaylist {
        id: local.id.clone(),
        name: if !local.name.trim().is_empty() {
            local.name.clone()
        } else {
            remote.name.clone()
        },
        songs: Vec::new(),
        created_at: min_positive(local.created_at, remote.created_at),
        modified_at: local.modified_at.max(remote.modified_at),
        is_deleted: true,
        song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
    }
}

fn merge_single_playlist(
    local: &SyncPlaylist,
    remote: &SyncPlaylist,
    last_sync_time: i64,
    base_songs: HashSet<String>,
    deletions: &[SyncPlaylistSongDeletion],
) -> SyncPlaylist {
    let local_changed = local.modified_at > last_sync_time;
    let remote_changed = remote.modified_at > last_sync_time;
    // 仅远端单侧改名时取远端；双方都改（或都未改但名字不同）固定 local-wins，
    // 对齐 Android GitHubSyncManager 的 PLAYLIST_RENAMED_BOTH_SIDES 裁决。
    // 按 modified_at 较新者裁决会让快钟设备永远赢且双端结果不同，名字乒乓。
    let name = if local.name == remote.name {
        local.name.clone()
    } else if remote_changed && !local_changed {
        remote.name.clone()
    } else {
        local.name.clone()
    };

    SyncPlaylist {
        id: local.id.clone(),
        name,
        songs: merge_songs(
            &local.songs,
            &remote.songs,
            local.modified_at,
            remote.modified_at,
            local_changed,
            remote_changed,
            last_sync_time,
            local.id == "-1001" || remote.id == "-1001",
            &base_songs,
            &local.id,
            deletions,
        ),
        created_at: min_positive(local.created_at, remote.created_at),
        modified_at: local.modified_at.max(remote.modified_at),
        is_deleted: false,
        song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
    }
}

fn order_merged_playlists(
    merged: &BTreeMap<String, SyncPlaylist>,
    local: &[SyncPlaylist],
    remote: &[SyncPlaylist],
    last_sync_time: i64,
) -> Vec<SyncPlaylist> {
    let local_changed = local.iter().any(|playlist| playlist.modified_at > last_sync_time);
    let remote_changed = remote.iter().any(|playlist| playlist.modified_at > last_sync_time);
    let (primary, secondary) = if remote_changed && !local_changed {
        (remote, local)
    } else {
        (local, remote)
    };
    let mut ordered_ids = Vec::new();
    let mut seen = HashSet::new();
    // 与 Android orderMergedPlaylists 的 filterNot(isDeleted) 对齐：
    // 已删歌单不参与来源顺序，统一由下方兜底循环按 key 序补在末尾，
    // 否则双端排序结果不同会被误判为数据变更而回声上传
    for playlist in primary.iter().chain(secondary.iter()) {
        if playlist.is_deleted {
            continue;
        }
        if merged.contains_key(&playlist.id) && seen.insert(playlist.id.clone()) {
            ordered_ids.push(playlist.id.clone());
        }
    }
    for id in merged.keys() {
        if seen.insert(id.clone()) {
            ordered_ids.push(id.clone());
        }
    }
    ordered_ids
        .into_iter()
        .filter_map(|id| merged.get(&id).cloned())
        .collect()
}

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
fn merge_songs(
    local: &[SyncSong],
    remote: &[SyncSong],
    local_modified_at: i64,
    remote_modified_at: i64,
    local_changed: bool,
    remote_changed: bool,
    last_sync_time: i64,
    is_favorites: bool,
    base_songs: &HashSet<String>,
    playlist_id: &str,
    deletions: &[SyncPlaylistSongDeletion],
) -> Vec<SyncSong> {
    let local_is_empty = local.is_empty();
    let remote_is_empty = remote.is_empty();
    let local_has_membership_tokens = has_membership_tokens(local);
    let remote_has_membership_tokens = has_membership_tokens(remote);
    let prefer_remote_favorites =
        is_favorites && local_is_empty && !remote_is_empty && last_sync_time <= 0;

    let mut merged = if prefer_remote_favorites {
        deduplicate_songs(remote)
    } else if local_is_empty && remote_is_empty {
        Vec::new()
    } else if local_is_empty {
        if remote_has_membership_tokens {
            deduplicate_songs(remote)
        } else if local_changed && local_modified_at >= remote_modified_at {
            Vec::new()
        } else {
            deduplicate_songs(remote)
        }
    } else if remote_is_empty {
        if local_has_membership_tokens {
            deduplicate_songs(local)
        } else if remote_changed && remote_modified_at > local_modified_at {
            Vec::new()
        } else {
            deduplicate_songs(local)
        }
    } else if remote_changed && !local_changed {
        merge_membership_tokens_into_primary(remote, local)
    } else if local_changed && !remote_changed {
        merge_membership_tokens_into_primary(local, remote)
    } else if local_changed && local_modified_at > remote_modified_at {
        merge_concurrent_changes(local, remote)
    } else if local_changed && remote_modified_at > local_modified_at {
        merge_concurrent_changes(remote, local)
    } else {
        merge_songs_with_deterministic_payload(local, remote)
    };

    if !base_songs.is_empty() {
        merged.retain(|song| {
            if !song.sync_membership_tokens.is_empty() {
                return true;
            }
            let local_has = contains_matching_song(local, song);
            let remote_has = contains_matching_song(remote, song);
            if local_has == remote_has {
                return true;
            }
            !song_matches_base(song, base_songs)
        });
    }

    apply_playlist_song_deletions(playlist_id, &merged, deletions)
}

fn deduplicate_songs(songs: &[SyncSong]) -> Vec<SyncSong> {
    let mut accumulator = SongMergeAccumulator::new(false);
    for song in songs {
        accumulator.add_if_absent(song);
    }
    accumulator.into_songs()
}

fn merge_songs_with_deterministic_payload(
    local: &[SyncSong],
    remote: &[SyncSong],
) -> Vec<SyncSong> {
    let mut accumulator = SongMergeAccumulator::new(true);
    for song in local.iter().chain(remote) {
        accumulator.add_if_absent(song);
    }
    accumulator.into_songs()
}

fn merge_membership_tokens_into_primary(
    primary: &[SyncSong],
    secondary: &[SyncSong],
) -> Vec<SyncSong> {
    let mut accumulator = SongMergeAccumulator::new(false);
    for song in primary {
        accumulator.add_if_absent(song);
    }
    for song in secondary {
        accumulator.merge_matching_membership_tokens(song);
    }
    accumulator.into_songs()
}

fn merge_concurrent_changes(primary: &[SyncSong], secondary: &[SyncSong]) -> Vec<SyncSong> {
    let mut accumulator = SongMergeAccumulator::new(false);
    for song in primary.iter().chain(secondary) {
        accumulator.add_if_absent(song);
    }
    accumulator.into_songs()
}

fn has_membership_tokens(songs: &[SyncSong]) -> bool {
    songs
        .iter()
        .any(|song| !song.sync_membership_tokens.is_empty())
}

#[derive(Debug, Clone)]
struct SongMergeEntry {
    song: SyncSong,
    aliases: Vec<SyncSong>,
}

/// add_if_absent 的候选召回键
///
/// songs_match 的每一层正匹配都必然与对方共享其中一种键
/// （token 相交 / identity 相等或 identity_keys 相交 / channel 三元组相等 /
/// 兜底层的相同 id），因此按键召回候选后再用 songs_match 精确复核，
/// 结果与全量线性扫描完全一致，但把万曲合并从 O(n²) 降到近 O(n)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SongIndexKey {
    Token(SyncCausalToken),
    Identity(String),
    Channel(String),
    Id(String),
}

fn song_index_keys(song: &SyncSong) -> Vec<SongIndexKey> {
    let mut keys: Vec<SongIndexKey> = song
        .sync_membership_tokens
        .iter()
        .cloned()
        .map(SongIndexKey::Token)
        .collect();
    keys.extend(song.identity_keys().into_iter().map(SongIndexKey::Identity));
    if let Some(channel_key) = channel_audio_key(song) {
        keys.push(SongIndexKey::Channel(channel_key));
    }
    if !song.id.is_empty() && song.id != "0" {
        keys.push(SongIndexKey::Id(song.id.clone()));
    }
    keys
}

struct SongMergeAccumulator {
    /// 槽位表：槽位号即插入序，被吸收的槽位置 None，输出按槽位号升序，
    /// 与旧实现「Vec 移除元素、剩余保持相对顺序」的输出顺序一致
    slots: Vec<Option<SongMergeEntry>>,
    /// 别名键 -> 槽位号倒排索引；查询时过滤掉已被吸收（None）的槽位
    index: HashMap<SongIndexKey, Vec<usize>>,
    resolve_payload_deterministically: bool,
}

impl SongMergeAccumulator {
    fn new(resolve_payload_deterministically: bool) -> Self {
        Self {
            slots: Vec::new(),
            index: HashMap::new(),
            resolve_payload_deterministically,
        }
    }

    fn add_if_absent(&mut self, song: &SyncSong) {
        let normalized = song.normalized_for_sync();
        let matching_indices = self.matching_indices(&normalized);
        if matching_indices.is_empty() {
            let slot = self.slots.len();
            self.index_alias_keys(slot, std::slice::from_ref(&normalized));
            self.slots.push(Some(SongMergeEntry {
                aliases: vec![normalized.clone()],
                song: normalized,
            }));
            return;
        }
        self.merge_matching_components(&matching_indices, normalized);
    }

    fn merge_matching_membership_tokens(&mut self, song: &SyncSong) {
        let normalized = song.normalized_for_sync();
        let matching_indices = self.matching_indices(&normalized);
        if !matching_indices.is_empty() {
            self.merge_matching_components(&matching_indices, normalized);
        }
    }

    /// 把一组别名的召回键指向槽位；重复指向无害（查询时去重）
    fn index_alias_keys(&mut self, slot: usize, aliases: &[SyncSong]) {
        for alias in aliases {
            for key in song_index_keys(alias) {
                let ids = self.index.entry(key).or_default();
                if !ids.contains(&slot) {
                    ids.push(slot);
                }
            }
        }
    }

    fn matching_indices(&self, song: &SyncSong) -> Vec<usize> {
        let mut candidates: Vec<usize> = song_index_keys(song)
            .iter()
            .filter_map(|key| self.index.get(key))
            .flatten()
            .copied()
            .filter(|slot| self.slots[*slot].is_some())
            .collect();
        // 升序即插入序，保证 primary 选取与旧实现（首个匹配位置）一致
        candidates.sort_unstable();
        candidates.dedup();
        candidates
            .into_iter()
            .filter(|slot| {
                self.slots[*slot]
                    .as_ref()
                    .is_some_and(|entry| entry.aliases.iter().any(|alias| songs_match(alias, song)))
            })
            .collect()
    }

    fn merge_matching_components(&mut self, matching_indices: &[usize], other: SyncSong) {
        let primary_index = matching_indices[0];
        let mut payload_candidates: Vec<SyncSong> = matching_indices
            .iter()
            .filter_map(|index| self.slots[*index].as_ref().map(|entry| entry.song.clone()))
            .collect();
        payload_candidates.push(other.clone());

        let primary_song = self.slots[primary_index]
            .as_ref()
            .map(|entry| entry.song.clone())
            .unwrap_or_else(|| other.clone());
        let selected = if self.resolve_payload_deterministically {
            payload_candidates
                .iter()
                .max_by(|left, right| {
                    left.sync_metadata_version
                        .cmp(&right.sync_metadata_version)
                        .then_with(|| canonical_payload_key(left).cmp(&canonical_payload_key(right)))
                })
                .cloned()
                .unwrap_or_else(|| primary_song.clone())
        } else {
            primary_song
        };
        let mut resolved = resolve_selected_sync_payload(&selected, &payload_candidates);
        resolved.added_at = resolve_primary_added_at(selected.added_at, &payload_candidates);
        resolved.sync_metadata_version = CURRENT_SYNC_METADATA_VERSION;
        resolved.sync_membership_tokens = normalize_sync_causal_tokens(
            &payload_candidates
                .iter()
                .flat_map(|song| song.sync_membership_tokens.iter().cloned())
                .collect::<Vec<_>>(),
        );

        let mut aliases = self.slots[primary_index]
            .as_ref()
            .map(|entry| entry.aliases.clone())
            .unwrap_or_default();
        for index in matching_indices.iter().copied().skip(1) {
            let Some(absorbed) = self.slots[index].take() else { continue };
            for alias in absorbed.aliases {
                if !aliases.iter().any(|known| same_song_payload(known, &alias)) {
                    aliases.push(alias);
                }
            }
        }
        if !aliases
            .iter()
            .any(|known| same_song_payload(known, &other))
        {
            aliases.push(other);
        }

        // 被吸收槽位的别名键必须重指到主槽位，后续歌曲才能继续命中该合并分量
        self.index_alias_keys(primary_index, &aliases);
        self.slots[primary_index] = Some(SongMergeEntry {
            song: resolved,
            aliases,
        });
    }

    fn into_songs(self) -> Vec<SyncSong> {
        self.slots
            .into_iter()
            .flatten()
            .map(|entry| entry.song)
            .collect()
    }
}

fn songs_match(left: &SyncSong, right: &SyncSong) -> bool {
    if left
        .sync_membership_tokens
        .iter()
        .any(|token| right.sync_membership_tokens.contains(token))
    {
        return true;
    }
    if left.identity() == right.identity() {
        return true;
    }
    if left
        .identity_keys()
        .iter()
        .any(|key| right.identity_keys().iter().any(|other| key == other))
    {
        return true;
    }
    if channel_audio_key(left).is_some_and(|key| Some(key) == channel_audio_key(right)) {
        return true;
    }
    let same_id = !left.id.is_empty() && left.id != "0" && left.id == right.id;
    same_id
        && normalize_text(&left.name) == normalize_text(&right.name)
        && normalize_text(&left.artist) == normalize_text(&right.artist)
        && source_hints_compatible(left, right)
}

fn source_hints_compatible(left: &SyncSong, right: &SyncSong) -> bool {
    match (source_hint(left), source_hint(right)) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn normalize_text(value: &str) -> String {
    value.trim().to_lowercase()
}

fn channel_audio_key(song: &SyncSong) -> Option<String> {
    let channel = song
        .channel_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_lowercase();
    let audio = song
        .audio_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let sub_audio = song.sub_audio_id.as_deref().unwrap_or_default().trim();
    Some(format!("{}|{}|{}", channel, audio, sub_audio))
}

fn source_hint(song: &SyncSong) -> Option<String> {
    if let Some(channel) = song
        .channel_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(channel.to_lowercase());
    }
    let album = song.album.trim().to_lowercase();
    if album.starts_with("netease") {
        Some("netease".into())
    } else if album.starts_with("bilibili") {
        Some("bilibili".into())
    } else if song.media_uri.to_lowercase().contains("youtube") {
        Some("youtube".into())
    } else {
        None
    }
}

fn canonical_payload_key(song: &SyncSong) -> String {
    let values = [
        song.id.clone(),
        song.name.clone(),
        song.artist.clone(),
        song.album.clone(),
        song.album_id.clone(),
        song.duration_ms.to_string(),
        song.cover_url.clone(),
        song.media_uri.clone(),
        song.added_at.to_string(),
        song.matched_lyric.clone().unwrap_or_default(),
        song.matched_translated_lyric.clone().unwrap_or_default(),
        song.matched_lyric_source.clone().unwrap_or_default(),
        song.matched_song_id.clone().unwrap_or_default(),
        song.user_lyric_offset_ms.to_string(),
        song.custom_cover_url.clone().unwrap_or_default(),
        song.custom_name.clone().unwrap_or_default(),
        song.custom_artist.clone().unwrap_or_default(),
        song.original_name.clone().unwrap_or_default(),
        song.original_artist.clone().unwrap_or_default(),
        song.original_cover_url.clone().unwrap_or_default(),
        song.original_lyric.clone().unwrap_or_default(),
        song.original_translated_lyric.clone().unwrap_or_default(),
        song.channel_id.clone().unwrap_or_default(),
        song.audio_id.clone().unwrap_or_default(),
        song.sub_audio_id.clone().unwrap_or_default(),
        song.playlist_context_id.clone().unwrap_or_default(),
        song.sync_metadata_version.to_string(),
    ];
    values
        .iter()
        .map(|value| format!("{}:{}", value.encode_utf16().count(), value))
        .collect::<Vec<_>>()
        .concat()
}

fn resolve_selected_sync_payload(selected: &SyncSong, candidates: &[SyncSong]) -> SyncSong {
    if selected.sync_metadata_version >= CURRENT_SYNC_METADATA_VERSION {
        return selected.clone();
    }

    if let Some(current) = candidates
        .iter()
        .filter(|song| song.sync_metadata_version >= CURRENT_SYNC_METADATA_VERSION)
        .max_by_key(|song| canonical_payload_key(song))
    {
        return current.clone();
    }

    let mut resolved = selected.clone();
    fill_missing_sync_metadata(&mut resolved, candidates);
    resolved
}

fn fill_missing_sync_metadata(target: &mut SyncSong, candidates: &[SyncSong]) {
    if target.id.is_empty() || target.id == "0" {
        target.id = first_non_empty(candidates, |song| {
            (song.id != "0").then_some(song.id.as_str())
        })
        .unwrap_or_default()
        .to_string();
    }
    if target.name.is_empty() {
        target.name = first_non_empty(candidates, |song| Some(song.name.as_str()))
            .unwrap_or_default()
            .to_string();
    }
    if target.artist.is_empty() {
        target.artist = first_non_empty(candidates, |song| Some(song.artist.as_str()))
            .unwrap_or_default()
            .to_string();
    }
    if target.album.is_empty() {
        target.album = first_non_empty(candidates, |song| Some(song.album.as_str()))
            .unwrap_or_default()
            .to_string();
    }
    if target.album_id.is_empty() {
        target.album_id = first_non_empty(candidates, |song| Some(song.album_id.as_str()))
            .unwrap_or_default()
            .to_string();
    }
    if target.duration_ms <= 0 {
        target.duration_ms = candidates
            .iter()
            .map(|song| song.duration_ms)
            .find(|duration| *duration > 0)
            .unwrap_or_default();
    }
    if target.cover_url.is_empty() {
        target.cover_url = first_non_empty(candidates, |song| Some(song.cover_url.as_str()))
            .unwrap_or_default()
            .to_string();
    }
    if target.media_uri.is_empty() {
        target.media_uri = first_non_empty(candidates, |song| Some(song.media_uri.as_str()))
            .unwrap_or_default()
            .to_string();
    }

    fill_missing_option(&mut target.matched_lyric, candidates, |song| {
        song.matched_lyric.as_deref()
    });
    fill_missing_option(
        &mut target.matched_translated_lyric,
        candidates,
        |song| song.matched_translated_lyric.as_deref(),
    );
    fill_missing_option(&mut target.matched_lyric_source, candidates, |song| {
        song.matched_lyric_source.as_deref()
    });
    fill_missing_option(&mut target.matched_song_id, candidates, |song| {
        song.matched_song_id.as_deref()
    });
    fill_missing_option(&mut target.custom_cover_url, candidates, |song| {
        song.custom_cover_url.as_deref()
    });
    fill_missing_option(&mut target.custom_name, candidates, |song| {
        song.custom_name.as_deref()
    });
    fill_missing_option(&mut target.custom_artist, candidates, |song| {
        song.custom_artist.as_deref()
    });
    fill_missing_option(&mut target.original_name, candidates, |song| {
        song.original_name.as_deref()
    });
    fill_missing_option(&mut target.original_artist, candidates, |song| {
        song.original_artist.as_deref()
    });
    fill_missing_option(&mut target.original_cover_url, candidates, |song| {
        song.original_cover_url.as_deref()
    });
    fill_missing_option(&mut target.original_lyric, candidates, |song| {
        song.original_lyric.as_deref()
    });
    fill_missing_option(
        &mut target.original_translated_lyric,
        candidates,
        |song| song.original_translated_lyric.as_deref(),
    );
    fill_missing_option(&mut target.channel_id, candidates, |song| {
        song.channel_id.as_deref()
    });
    fill_missing_option(&mut target.audio_id, candidates, |song| {
        song.audio_id.as_deref()
    });
    fill_missing_option(&mut target.sub_audio_id, candidates, |song| {
        song.sub_audio_id.as_deref()
    });
    fill_missing_option(&mut target.playlist_context_id, candidates, |song| {
        song.playlist_context_id.as_deref()
    });
    if target.user_lyric_offset_ms == 0 {
        target.user_lyric_offset_ms = candidates
            .iter()
            .map(|song| song.user_lyric_offset_ms)
            .find(|offset| *offset != 0)
            .unwrap_or_default();
    }
}

fn fill_missing_option<F>(
    target: &mut Option<String>,
    candidates: &[SyncSong],
    selector: F,
) where
    F: for<'a> Fn(&'a SyncSong) -> Option<&'a str>,
{
    if target.as_deref().is_some_and(|value| !value.trim().is_empty()) {
        return;
    }
    *target = first_non_empty(candidates, selector).map(String::from);
}

fn first_non_empty<'a, F>(candidates: &'a [SyncSong], selector: F) -> Option<&'a str>
where
    F: Fn(&'a SyncSong) -> Option<&'a str>,
{
    candidates
        .iter()
        .filter_map(selector)
        .find(|value| !value.trim().is_empty())
}

fn same_song_payload(left: &SyncSong, right: &SyncSong) -> bool {
    canonical_payload_key(left) == canonical_payload_key(right)
        && normalize_sync_causal_tokens(&left.sync_membership_tokens)
            == normalize_sync_causal_tokens(&right.sync_membership_tokens)
}

fn contains_matching_song(songs: &[SyncSong], target: &SyncSong) -> bool {
    songs.iter().any(|song| songs_match(song, target))
}

fn song_matches_base(song: &SyncSong, base_songs: &HashSet<String>) -> bool {
    song.identity_keys().iter().any(|key| base_songs.contains(key))
}

fn resolve_primary_added_at(selected: i64, candidates: &[SyncSong]) -> i64 {
    if selected > 0 {
        return selected;
    }
    candidates
        .iter()
        .map(|song| song.added_at)
        .max()
        .unwrap_or_default()
        .max(0)
}

fn apply_playlist_song_deletions(
    playlist_id: &str,
    songs: &[SyncSong],
    deletions: &[SyncPlaylistSongDeletion],
) -> Vec<SyncSong> {
    let relevant: Vec<&SyncPlaylistSongDeletion> = deletions
        .iter()
        .filter(|deletion| deletion.playlist_id == playlist_id)
        .collect();
    if relevant.is_empty() {
        return songs.to_vec();
    }
    let causal_tokens: HashSet<SyncCausalToken> = relevant
        .iter()
        .flat_map(|deletion| deletion.removed_membership_tokens.iter().cloned())
        .collect();
    songs
        .iter()
        .filter_map(|song| {
            let identity_deletions: Vec<&SyncPlaylistSongDeletion> = relevant
                .iter()
                .copied()
                .filter(|deletion| deletion.matches_song(playlist_id, song))
                .collect();
            let song_tokens = normalize_sync_causal_tokens(&song.sync_membership_tokens);
            if song_tokens.is_empty() {
                let latest = identity_deletions
                    .iter()
                    .max_by(|left, right| deletion_cmp(left, right));
                return match latest {
                    Some(deletion) if effective_added_at(song) <= deletion.deleted_at => None,
                    _ => Some(song.clone()),
                };
            }
            let remaining: Vec<SyncCausalToken> = song_tokens
                .into_iter()
                .filter(|token| !causal_tokens.contains(token))
                .collect();
            if remaining.is_empty() {
                None
            } else {
                let mut surviving = song.clone();
                surviving.sync_membership_tokens = remaining;
                Some(surviving)
            }
        })
        .collect()
}

fn effective_added_at(song: &SyncSong) -> i64 {
    let timestamp = song.legacy_added_at.unwrap_or(song.added_at);
    if timestamp > 0 {
        timestamp
    } else {
        i64::MIN
    }
}

fn merge_favorite_playlists(
    local: &[SyncFavoritePlaylist],
    remote: &[SyncFavoritePlaylist],
) -> Vec<SyncFavoritePlaylist> {
    let mut groups: BTreeMap<String, Vec<SyncFavoritePlaylist>> = BTreeMap::new();
    for favorite in local.iter().chain(remote.iter()) {
        groups
            .entry(favorite.group_key())
            .or_default()
            .push(favorite.normalized_for_sync());
    }
    let mut result: Vec<SyncFavoritePlaylist> = groups
        .into_values()
        .map(|snapshots| snapshots.into_iter().reduce(|left, right| merge_single_favorite(&left, &right)).unwrap())
        .collect();
    result.sort_by(|left, right| {
        right
            .sort_order
            .cmp(&left.sort_order)
            .then_with(|| right.modified_at.cmp(&left.modified_at))
            .then_with(|| right.added_time.cmp(&left.added_time))
            .then_with(|| left.group_key().cmp(&right.group_key()))
    });
    result
}

fn merge_single_favorite(
    left: &SyncFavoritePlaylist,
    right: &SyncFavoritePlaylist,
) -> SyncFavoritePlaylist {
    let left = left.normalized_for_sync();
    let right = right.normalized_for_sync();
    let newer = if right.modified_at > left.modified_at {
        &right
    } else {
        &left
    };
    let older = if std::ptr::eq(newer, &left) {
        &right
    } else {
        &left
    };
    if left.is_deleted != right.is_deleted {
        if left.modified_at == right.modified_at {
            let mut result = if left.is_deleted {
                left.clone()
            } else {
                right.clone()
            };
            result.songs.clear();
            result.track_count = 0;
            result.added_time = left.added_time.max(right.added_time);
            result.modified_at = left.modified_at.max(right.modified_at);
            result.sort_order = left.sort_order.max(right.sort_order);
            return result;
        }
        if newer.is_deleted {
            let mut result = newer.clone();
            result.songs.clear();
            result.track_count = 0;
            result.sort_order = left.sort_order.max(right.sort_order);
            return result;
        }
        let mut result = newer.clone();
        result.songs = deduplicate_songs(&[left.songs.clone(), right.songs.clone()].concat());
        result.track_count = left
            .track_count
            .max(right.track_count)
            .max(result.songs.len() as i32);
        if result.sort_order == 0 {
            result.sort_order = older.sort_order;
        }
        return result;
    }
    if newer.is_deleted {
        let mut result = newer.clone();
        result.songs.clear();
        result.track_count = 0;
        result.added_time = left.added_time.max(right.added_time);
        result.sort_order = left.sort_order.max(right.sort_order);
        return result;
    }
    let mut result = newer.clone();
    result.cover_url = if !newer.cover_url.is_empty() {
        newer.cover_url.clone()
    } else {
        older.cover_url.clone()
    };
    result.songs = deduplicate_songs(&[left.songs.clone(), right.songs.clone()].concat());
    result.track_count = left
        .track_count
        .max(right.track_count)
        .max(result.songs.len() as i32);
    result.added_time = left.added_time.max(right.added_time);
    result.modified_at = left.modified_at.max(right.modified_at);
    if result.sort_order == 0 {
        result.sort_order = older.sort_order;
    }
    result.is_deleted = false;
    result
}

pub(super) fn merge_recent_plays(
    local: &[SyncRecentPlay],
    remote: &[SyncRecentPlay],
    deletions: &[SyncRecentPlayDeletion],
) -> Vec<SyncRecentPlay> {
    let mut all: Vec<SyncRecentPlay> = local.iter().chain(remote.iter()).cloned().collect();
    all.sort_by(|left, right| {
        right
            .played_at
            .cmp(&left.played_at)
            .then_with(|| right.resume_position_ms.cmp(&left.resume_position_ms))
            .then_with(|| right.device_id.cmp(&left.device_id))
            .then_with(|| recent_song_key(left).cmp(&recent_song_key(right)))
    });
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for mut recent in all {
        recent.song = recent.song.normalized_for_sync();
        let key = recent_song_key(&recent);
        if !seen.insert(key.clone()) || recent_is_deleted(&recent.song, recent.played_at, deletions) {
            continue;
        }
        result.push(recent);
    }
    result
}

fn recent_song_key(recent: &SyncRecentPlay) -> String {
    recent.song.identity().stable_key()
}

fn recent_is_deleted(song: &SyncSong, played_at: i64, deletions: &[SyncRecentPlayDeletion]) -> bool {
    deletions.iter().any(|deletion| {
        let matches = deletion
            .identity_keys()
            .iter()
            .any(|key| song.identity_keys().iter().any(|song_key| song_key == key));
        matches && deletion.deleted_at >= played_at
    })
}

pub(super) fn merge_recent_play_deletions(
    local: &[SyncRecentPlayDeletion],
    remote: &[SyncRecentPlayDeletion],
) -> Vec<SyncRecentPlayDeletion> {
    let mut groups: BTreeMap<String, Vec<SyncRecentPlayDeletion>> = BTreeMap::new();
    for deletion in local.iter().chain(remote.iter()) {
        groups
            .entry(deletion.identity().stable_key())
            .or_default()
            .push(deletion.clone());
    }
    let mut result: Vec<SyncRecentPlayDeletion> = groups
        .into_values()
        .filter_map(|snapshots| snapshots.into_iter().max_by(recent_deletion_cmp))
        .collect();
    result.sort_by(|left, right| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| right.device_id.cmp(&left.device_id))
            .then_with(|| left.identity().stable_key().cmp(&right.identity().stable_key()))
    });
    result
}

fn recent_deletion_cmp(left: &SyncRecentPlayDeletion, right: &SyncRecentPlayDeletion) -> std::cmp::Ordering {
    left.deleted_at
        .cmp(&right.deleted_at)
        .then_with(|| left.device_id.cmp(&right.device_id))
}

fn prune_recent_play_deletions(
    deletions: &[SyncRecentPlayDeletion],
    recent: &[SyncRecentPlay],
) -> Vec<SyncRecentPlayDeletion> {
    let mut result: Vec<SyncRecentPlayDeletion> = deletions
        .iter()
        .filter(|deletion| {
            recent.iter().all(|played| {
                let matches = deletion
                    .identity_keys()
                    .iter()
                    .any(|key| played.song.identity_keys().iter().any(|song_key| song_key == key));
                !matches || played.played_at <= deletion.deleted_at
            })
        })
        .cloned()
        .collect();
    result.sort_by(|left, right| right.deleted_at.cmp(&left.deleted_at).then_with(|| right.device_id.cmp(&left.device_id)));
    result
}

fn merge_sync_log(local: &[SyncLogEntry], remote: &[SyncLogEntry]) -> Vec<SyncLogEntry> {
    let mut unique = BTreeMap::new();
    for entry in local.iter().chain(remote.iter()) {
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            entry.timestamp,
            entry.device_id,
            entry.action,
            entry.playlist_id.as_deref().unwrap_or_default(),
            entry.song_id.as_deref().unwrap_or_default(),
            entry.details.as_deref().unwrap_or_default()
        );
        unique.insert(key, entry.clone());
    }
    let mut result: Vec<SyncLogEntry> = unique.into_values().collect();
    result.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| right.device_id.cmp(&left.device_id))
            .then_with(|| left.action.cmp(&right.action))
    });
    result.truncate(MAX_SYNC_LOG);
    result
}

fn merge_playlist_song_deletions(
    local: &[SyncPlaylistSongDeletion],
    remote: &[SyncPlaylistSongDeletion],
) -> Vec<SyncPlaylistSongDeletion> {
    let mut groups: BTreeMap<String, Vec<SyncPlaylistSongDeletion>> = BTreeMap::new();
    for deletion in local.iter().chain(remote.iter()) {
        let mut normalized = deletion.clone();
        normalized.removed_membership_tokens = normalize_sync_causal_tokens(
            &deletion.removed_membership_tokens,
        );
        groups
            .entry(normalized.identity())
            .or_default()
            .push(normalized);
    }
    let mut result = Vec::new();
    for snapshots in groups.into_values() {
        let legacy = snapshots
            .iter()
            .filter(|deletion| deletion.removed_membership_tokens.is_empty())
            .max_by(|left, right| deletion_cmp(left, right))
            .cloned();
        let causal_snapshots: Vec<&SyncPlaylistSongDeletion> = snapshots
            .iter()
            .filter(|deletion| !deletion.removed_membership_tokens.is_empty())
            .collect();
        let causal = causal_snapshots.iter().max_by(|left, right| deletion_cmp(left, right)).map(|deletion| {
            let mut merged = (*deletion).clone();
            let tokens: Vec<SyncCausalToken> = causal_snapshots
                .iter()
                .flat_map(|snapshot| snapshot.removed_membership_tokens.iter().cloned())
                .collect();
            merged.removed_membership_tokens = normalize_sync_causal_tokens(&tokens);
            merged
        });
        if let Some(legacy) = legacy { result.push(legacy); }
        if let Some(causal) = causal { result.push(causal); }
    }
    result.sort_by(deletion_order_cmp);
    result
}

fn deletion_cmp(left: &SyncPlaylistSongDeletion, right: &SyncPlaylistSongDeletion) -> std::cmp::Ordering {
    left.deleted_at
        .cmp(&right.deleted_at)
        .then_with(|| left.device_id.cmp(&right.device_id))
}

fn deletion_order_cmp(left: &SyncPlaylistSongDeletion, right: &SyncPlaylistSongDeletion) -> std::cmp::Ordering {
    right
        .deleted_at
        .cmp(&left.deleted_at)
        .then_with(|| right.device_id.cmp(&left.device_id))
        .then_with(|| left.identity().cmp(&right.identity()))
        .then_with(|| left.removed_membership_tokens.is_empty().cmp(&right.removed_membership_tokens.is_empty()))
}

fn prune_playlist_song_deletions(
    deletions: &[SyncPlaylistSongDeletion],
    playlists: &[SyncPlaylist],
) -> Vec<SyncPlaylistSongDeletion> {
    let mut result: Vec<SyncPlaylistSongDeletion> = deletions
        .iter()
        .filter(|deletion| {
            if !deletion.removed_membership_tokens.is_empty() {
                return true;
            }
            // 仅当活跃歌带非空 membership token（identity 已由 causal token 接管、
            // 确实是新版本重新添加）且 added_at 晚于删除时刻才裁 legacy 墓碑。
            // legacy 迁移合成的 addedAt（无 token）不可据此判定重新添加，否则
            // 误裁墓碑会让已删歌在所有端复活（对齐 Android SyncPlaylistDeletionPolicy P1-1）
            playlists.iter().all(|playlist| {
                playlist.id != deletion.playlist_id
                    || playlist.is_deleted
                    || playlist.songs.iter().all(|song| {
                        !deletion.matches_song(&playlist.id, song)
                            || normalize_sync_causal_tokens(&song.sync_membership_tokens).is_empty()
                            || effective_added_at(song) <= deletion.deleted_at
                    })
            })
        })
        .cloned()
        .collect();
    result.sort_by(deletion_order_cmp);
    result
}

#[derive(Default)]
struct MergedCounters {
    total_listen_ms: i64,
    play_count: i32,
    first_played_at: i64,
    last_played_at: i64,
    base_listen_ms: i64,
    base_play_count: i32,
    shards: Vec<SyncPlaybackCounterShard>,
}

fn merge_counter_shards(
    local: &[SyncPlaybackCounterShard],
    remote: &[SyncPlaybackCounterShard],
) -> Vec<SyncPlaybackCounterShard> {
    let mut grouped: BTreeMap<(String, i64), SyncPlaybackCounterShard> = BTreeMap::new();
    for raw in local.iter().chain(remote.iter()) {
        if raw.device_id.trim().is_empty() {
            continue;
        }
        let mut shard = raw.clone();
        shard.epoch_started_at = shard.epoch_started_at.max(0);
        shard.total_listen_ms = shard.total_listen_ms.max(0);
        shard.play_count = shard.play_count.max(0);
        shard.last_played_at = shard.last_played_at.max(0);
        shard.first_played_at = if shard.first_played_at <= 0 || shard.first_played_at > shard.last_played_at {
            shard.last_played_at
        } else {
            shard.first_played_at
        };
        let key = (shard.device_id.clone(), shard.epoch_started_at);
        grouped
            .entry(key)
            .and_modify(|existing| {
                existing.total_listen_ms = existing.total_listen_ms.max(shard.total_listen_ms);
                existing.play_count = existing.play_count.max(shard.play_count);
                existing.first_played_at = min_positive(existing.first_played_at, shard.first_played_at);
                existing.last_played_at = existing.last_played_at.max(shard.last_played_at);
            })
            .or_insert(shard);
    }
    grouped.into_values().collect()
}

fn retained_counter_shards(
    shards: &[SyncPlaybackCounterShard],
    first: i64,
    last: i64,
    cleared_at: i64,
) -> Option<Vec<SyncPlaybackCounterShard>> {
    let mut normalized = merge_counter_shards(shards, &[]);
    if cleared_at <= 0 {
        return Some(normalized);
    }
    if last < cleared_at {
        return None;
    }
    if normalized.is_empty() {
        return (cleared_at..=last).contains(&first).then_some(normalized);
    }
    // 旧 epoch 无法拆出清除后的增量，最后一次播放更新不能让旧累计值复活
    normalized.retain(|shard| {
        shard.epoch_started_at >= cleared_at && shard.first_played_at >= cleared_at
    });
    (!normalized.is_empty()).then_some(normalized)
}

fn counters_after_clear(shards: &[SyncPlaybackCounterShard]) -> MergedCounters {
    MergedCounters {
        total_listen_ms: shards.iter().fold(0_i64, |total, shard| {
            total.saturating_add(shard.total_listen_ms)
        }),
        play_count: shards
            .iter()
            .fold(0_i32, |total, shard| total.saturating_add(shard.play_count)),
        first_played_at: shards
            .iter()
            .map(|shard| shard.first_played_at)
            .min()
            .unwrap_or(0),
        last_played_at: shards
            .iter()
            .map(|shard| shard.last_played_at)
            .max()
            .unwrap_or(0),
        ..Default::default()
    }
}

fn normalize_stat_after_clear(stat: &SyncTrackStat, cleared_at: i64) -> Option<SyncTrackStat> {
    let shards = retained_counter_shards(
        &stat.counter_shards,
        stat.first_played_at,
        stat.last_played_at,
        cleared_at,
    )?;
    let mut normalized = stat.clone();
    normalized.total_listen_ms = normalized.total_listen_ms.max(0);
    normalized.play_count = normalized.play_count.max(0);
    normalized.counter_shards = shards;
    if cleared_at > 0 && !normalized.counter_shards.is_empty() {
        let totals = counters_after_clear(&normalized.counter_shards);
        normalized.total_listen_ms = totals.total_listen_ms;
        normalized.play_count = totals.play_count;
        normalized.first_played_at = totals.first_played_at;
        normalized.last_played_at = totals.last_played_at;
        normalized.counter_base_listen_ms = 0;
        normalized.counter_base_play_count = 0;
    }
    Some(normalized)
}

fn normalize_bucket_after_clear(
    bucket: &SyncPlaybackStatBucket,
    cleared_at: i64,
) -> Option<SyncPlaybackStatBucket> {
    let shards = retained_counter_shards(
        &bucket.counter_shards,
        bucket.first_played_at,
        bucket.last_played_at,
        cleared_at,
    )?;
    let mut normalized = bucket.clone();
    normalized.total_listen_ms = normalized.total_listen_ms.max(0);
    normalized.play_count = normalized.play_count.max(0);
    normalized.counter_shards = shards;
    if cleared_at > 0 && !normalized.counter_shards.is_empty() {
        let totals = counters_after_clear(&normalized.counter_shards);
        normalized.total_listen_ms = totals.total_listen_ms;
        normalized.play_count = totals.play_count;
        normalized.first_played_at = totals.first_played_at;
        normalized.last_played_at = totals.last_played_at;
        normalized.counter_base_listen_ms = 0;
        normalized.counter_base_play_count = 0;
    }
    Some(normalized)
}

/// 参与计数合并的一侧快照
///
/// 聚合统计与日分桶字段完全同构，用同一个快照类型描述，避免把 14 个
/// 平铺参数在调用点排错顺序 —— 这里一旦错位是静默的数据损坏。
struct CounterSide<'a> {
    total_listen_ms: i64,
    play_count: i32,
    first_played_at: i64,
    last_played_at: i64,
    base_listen_ms: i64,
    base_play_count: i32,
    shards: &'a [SyncPlaybackCounterShard],
}

impl CounterSide<'_> {
    /// 分片机制上线前的历史存量：没有分片且 base 为 0 时，总量本身就是 base
    fn effective_base_listen_ms(&self) -> i64 {
        if self.shards.is_empty() && self.base_listen_ms == 0 {
            self.total_listen_ms.max(0)
        } else {
            self.base_listen_ms.max(0)
        }
    }

    fn effective_base_play_count(&self) -> i32 {
        if self.shards.is_empty() && self.base_play_count == 0 {
            self.play_count.max(0)
        } else {
            self.base_play_count.max(0)
        }
    }
}

fn merge_counter_values(local: CounterSide<'_>, remote: CounterSide<'_>) -> MergedCounters {
    let shards = merge_counter_shards(local.shards, remote.shards);
    // 分片为空时 base 显式归零（对齐 Android SyncPlaybackStatsMergePolicy.mergeCounters）：
    // 此时把 total 折算进 base 只会让本端与 Android 的 counter_base 永远不同，
    // has_data_changed 每轮都判「有变化」，双端互相"纠正"形成永不收敛的回声上传；
    // 总量本身仍由下方的 max 兜底，归零不丢任何数据
    if shards.is_empty() {
        return MergedCounters {
            total_listen_ms: local.total_listen_ms.max(remote.total_listen_ms).max(0),
            play_count: local.play_count.max(remote.play_count).max(0),
            first_played_at: min_positive(local.first_played_at, remote.first_played_at),
            last_played_at: local.last_played_at.max(remote.last_played_at),
            base_listen_ms: 0,
            base_play_count: 0,
            shards,
        };
    }
    let base_total = local
        .effective_base_listen_ms()
        .max(remote.effective_base_listen_ms());
    let base_count = local
        .effective_base_play_count()
        .max(remote.effective_base_play_count());
    let sharded_total = shards.iter().fold(base_total, |total, shard| total.saturating_add(shard.total_listen_ms));
    let sharded_count = shards.iter().fold(base_count, |total, shard| total.saturating_add(shard.play_count));
    MergedCounters {
        total_listen_ms: sharded_total
            .max(local.total_listen_ms)
            .max(remote.total_listen_ms)
            .max(0),
        play_count: sharded_count
            .max(local.play_count)
            .max(remote.play_count)
            .max(0),
        first_played_at: min_positive(
            min_positive(local.first_played_at, remote.first_played_at),
            shards.iter().map(|shard| shard.first_played_at).fold(0, min_positive),
        ),
        last_played_at: local
            .last_played_at
            .max(remote.last_played_at)
            .max(shards.iter().map(|shard| shard.last_played_at).max().unwrap_or(0)),
        base_listen_ms: base_total,
        base_play_count: base_count,
        shards,
    }
}

pub(super) fn merge_playback_stats(
    local: &[SyncTrackStat],
    remote: &[SyncTrackStat],
    cleared_at: i64,
) -> Vec<SyncTrackStat> {
    let mut grouped: BTreeMap<String, SyncTrackStat> = BTreeMap::new();
    for stat in local.iter().chain(remote.iter()) {
        let Some(stat) = normalize_stat_after_clear(stat, cleared_at) else { continue };
        if let Some(existing) = grouped.remove(&stat.identity_key) {
            let newer = if stat.last_played_at >= existing.last_played_at { &stat } else { &existing };
            let counters = merge_counter_values(
                CounterSide {
                    total_listen_ms: existing.total_listen_ms,
                    play_count: existing.play_count,
                    first_played_at: existing.first_played_at,
                    last_played_at: existing.last_played_at,
                    base_listen_ms: existing.counter_base_listen_ms,
                    base_play_count: existing.counter_base_play_count,
                    shards: &existing.counter_shards,
                },
                CounterSide {
                    total_listen_ms: stat.total_listen_ms,
                    play_count: stat.play_count,
                    first_played_at: stat.first_played_at,
                    last_played_at: stat.last_played_at,
                    base_listen_ms: stat.counter_base_listen_ms,
                    base_play_count: stat.counter_base_play_count,
                    shards: &stat.counter_shards,
                },
            );
            let mut merged = newer.clone();
            merged.total_listen_ms = counters.total_listen_ms;
            merged.play_count = counters.play_count;
            merged.first_played_at = counters.first_played_at;
            merged.last_played_at = counters.last_played_at;
            merged.counter_base_listen_ms = counters.base_listen_ms;
            merged.counter_base_play_count = counters.base_play_count;
            merged.counter_shards = counters.shards;
            grouped.insert(stat.identity_key.clone(), merged);
        } else {
            grouped.insert(stat.identity_key.clone(), stat);
        }
    }
    grouped.into_values().collect()
}

pub(super) fn merge_stat_buckets(
    local: &[SyncPlaybackStatBucket],
    remote: &[SyncPlaybackStatBucket],
    cleared_at: i64,
) -> Vec<SyncPlaybackStatBucket> {
    let mut grouped: BTreeMap<(i64, String), SyncPlaybackStatBucket> = BTreeMap::new();
    for bucket in local.iter().chain(remote.iter()) {
        let Some(bucket) = normalize_bucket_after_clear(bucket, cleared_at) else { continue };
        let key = (bucket.day_start_at, bucket.identity_key.clone());
        if let Some(existing) = grouped.remove(&key) {
            let newer = if bucket.last_played_at >= existing.last_played_at { &bucket } else { &existing };
            let counters = merge_counter_values(
                CounterSide {
                    total_listen_ms: existing.total_listen_ms,
                    play_count: existing.play_count,
                    first_played_at: existing.first_played_at,
                    last_played_at: existing.last_played_at,
                    base_listen_ms: existing.counter_base_listen_ms,
                    base_play_count: existing.counter_base_play_count,
                    shards: &existing.counter_shards,
                },
                CounterSide {
                    total_listen_ms: bucket.total_listen_ms,
                    play_count: bucket.play_count,
                    first_played_at: bucket.first_played_at,
                    last_played_at: bucket.last_played_at,
                    base_listen_ms: bucket.counter_base_listen_ms,
                    base_play_count: bucket.counter_base_play_count,
                    shards: &bucket.counter_shards,
                },
            );
            let mut merged = newer.clone();
            merged.total_listen_ms = counters.total_listen_ms;
            merged.play_count = counters.play_count;
            merged.first_played_at = counters.first_played_at;
            merged.last_played_at = counters.last_played_at;
            merged.counter_base_listen_ms = counters.base_listen_ms;
            merged.counter_base_play_count = counters.base_play_count;
            merged.counter_shards = counters.shards;
            grouped.insert(key, merged);
        } else {
            grouped.insert(key, bucket);
        }
    }
    grouped.into_values().collect()
}

/// 按稳定键排序后逐条比较
///
/// 直接 zip 比较会把「仅顺序不同」误判为有变更, 触发无意义上传;
/// 对端合并后顺序又可能变回去, 形成永不收敛的回声。
fn pairs_by_key<'a, T, K, F>(left: &'a [T], right: &'a [T], key: F) -> Option<Vec<(&'a T, &'a T)>>
where
    K: Ord,
    F: Fn(&T) -> K,
{
    if left.len() != right.len() {
        return None;
    }
    let mut left: Vec<&T> = left.iter().collect();
    let mut right: Vec<&T> = right.iter().collect();
    left.sort_by_key(|item| key(item));
    right.sort_by_key(|item| key(item));
    Some(left.into_iter().zip(right).collect())
}

pub fn has_data_changed(remote: &SyncData, merged: &SyncData) -> bool {
    let remote = remote.normalized_for_sync();
    let merged = merged.normalized_for_sync();
    match pairs_by_key(&remote.playlists, &merged.playlists, |playlist| {
        playlist.id.clone()
    }) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| !same_playlist(left, right)) {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(
        &remote.favorite_playlists,
        &merged.favorite_playlists,
        SyncFavoritePlaylist::group_key,
    ) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| !same_favorite(left, right)) {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(&remote.recent_plays, &merged.recent_plays, |play| {
        (play.song.identity().stable_key(), play.played_at)
    }) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| {
                left.song_id != right.song_id
                    || left.played_at != right.played_at
                    || left.device_id != right.device_id
                    || left.resume_position_ms != right.resume_position_ms
                    || !same_song(&left.song, &right.song)
            }) {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(
        &remote.recent_play_deletions,
        &merged.recent_play_deletions,
        |deletion| deletion.identity().stable_key(),
    ) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| {
                left.identity() != right.identity()
                    || left.deleted_at != right.deleted_at
                    || left.device_id != right.device_id
            }) {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(
        &remote.playlist_song_deletions,
        &merged.playlist_song_deletions,
        |deletion| {
            (
                deletion.identity(),
                deletion.removed_membership_tokens.is_empty(),
            )
        },
    ) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| {
                left.identity() != right.identity()
                    || left.deleted_at != right.deleted_at
                    || left.device_id != right.device_id
                    || normalize_sync_causal_tokens(&left.removed_membership_tokens)
                        != normalize_sync_causal_tokens(&right.removed_membership_tokens)
            }) {
                return true;
            }
        }
        None => return true,
    }
    if remote.playback_stats_cleared_at != merged.playback_stats_cleared_at
        || !same_stats(&remote.playback_stats, &merged.playback_stats)
        || !same_buckets(&remote.playback_stat_buckets, &merged.playback_stat_buckets)
    {
        return true;
    }
    if remote.extensions != merged.extensions { return true; }
    false
}

fn same_playlist(left: &SyncPlaylist, right: &SyncPlaylist) -> bool {
    // 对齐 Android SyncDataChangeDetector: 仅比较内容, 不比较 created_at/modified_at
    // 时间戳单调增长, 若纳入比较会把"仅时间戳变化"误判为改动, 导致同步回流/回声上传
    left.id == right.id
        && left.name == right.name
        && left.is_deleted == right.is_deleted
        && left.song_order_version == right.song_order_version
        && left.songs.len() == right.songs.len()
        && left.songs.iter().zip(&right.songs).all(|(a, b)| same_song(a, b))
}

fn same_favorite(left: &SyncFavoritePlaylist, right: &SyncFavoritePlaylist) -> bool {
    // 对齐 Android SyncDataChangeDetector.favorite:
    // 比较 isDeleted/modifiedAt/sortOrder/trackCount/song identity+metadata
    // 不比较 name/cover/source 展示字段与 addedTime, 避免仅展示刷新触发回声上传
    left.id == right.id
        && left.source == right.source
        && left.modified_at == right.modified_at
        && left.is_deleted == right.is_deleted
        && left.sort_order == right.sort_order
        && left.track_count == right.track_count
        && left.songs.len() == right.songs.len()
        && left.songs.iter().zip(&right.songs).all(|(a, b)| same_song(a, b))
}

fn same_song(left: &SyncSong, right: &SyncSong) -> bool {
    left.identity_keys() == right.identity_keys()
        && left.name == right.name
        && left.artist == right.artist
        && left.album == right.album
        && left.album_id == right.album_id
        && left.duration_ms == right.duration_ms
        && left.cover_url == right.cover_url
        && left.media_uri == right.media_uri
        && left.added_at == right.added_at
        && left.matched_lyric == right.matched_lyric
        && left.matched_translated_lyric == right.matched_translated_lyric
        && left.matched_lyric_source == right.matched_lyric_source
        && left.matched_song_id == right.matched_song_id
        && left.user_lyric_offset_ms == right.user_lyric_offset_ms
        && left.custom_cover_url == right.custom_cover_url
        && left.custom_name == right.custom_name
        && left.custom_artist == right.custom_artist
        && left.original_name == right.original_name
        && left.original_artist == right.original_artist
        && left.original_cover_url == right.original_cover_url
        && left.original_lyric == right.original_lyric
        && left.original_translated_lyric == right.original_translated_lyric
        && left.matched_romanized_lyric == right.matched_romanized_lyric
        && left.original_romanized_lyric == right.original_romanized_lyric
        && left.lyric_sync_revision == right.lyric_sync_revision
        && left.lyric_sync_edited == right.lyric_sync_edited
        && left.channel_id == right.channel_id
        && left.audio_id == right.audio_id
        && left.sub_audio_id == right.sub_audio_id
        && left.playlist_context_id == right.playlist_context_id
        && left.sync_metadata_version == right.sync_metadata_version
        && normalize_sync_causal_tokens(&left.sync_membership_tokens)
            == normalize_sync_causal_tokens(&right.sync_membership_tokens)
}

fn same_stats(left: &[SyncTrackStat], right: &[SyncTrackStat]) -> bool {
    if left.len() != right.len() { return false; }
    let mut left = left.iter().collect::<Vec<_>>();
    let mut right = right.iter().collect::<Vec<_>>();
    left.sort_by(|a, b| a.identity_key.cmp(&b.identity_key));
    right.sort_by(|a, b| a.identity_key.cmp(&b.identity_key));
    left.iter().zip(right).all(|(a, b)| {
        a.identity_key == b.identity_key
            && a.name == b.name
            && a.artist == b.artist
            && a.album == b.album
            && a.total_listen_ms == b.total_listen_ms
            && a.play_count == b.play_count
            && a.last_played_at == b.last_played_at
            && a.first_played_at == b.first_played_at
            && a.cover_url == b.cover_url
            && a.duration_ms == b.duration_ms
            && a.media_uri == b.media_uri
            && a.id == b.id
            && a.album_id == b.album_id
            && a.counter_base_listen_ms == b.counter_base_listen_ms
            && a.counter_base_play_count == b.counter_base_play_count
            && merge_counter_shards(&a.counter_shards, &[]) == merge_counter_shards(&b.counter_shards, &[])
    })
}

fn same_buckets(left: &[SyncPlaybackStatBucket], right: &[SyncPlaybackStatBucket]) -> bool {
    if left.len() != right.len() { return false; }
    let mut left = left.iter().collect::<Vec<_>>();
    let mut right = right.iter().collect::<Vec<_>>();
    left.sort_by(|a, b| a.day_start_at.cmp(&b.day_start_at).then_with(|| a.identity_key.cmp(&b.identity_key)));
    right.sort_by(|a, b| a.day_start_at.cmp(&b.day_start_at).then_with(|| a.identity_key.cmp(&b.identity_key)));
    left.iter().zip(right).all(|(a, b)| {
        a.day_start_at == b.day_start_at
            && a.identity_key == b.identity_key
            && a.name == b.name
            && a.artist == b.artist
            && a.album == b.album
            && a.total_listen_ms == b.total_listen_ms
            && a.play_count == b.play_count
            && a.last_played_at == b.last_played_at
            && a.first_played_at == b.first_played_at
            && a.cover_url == b.cover_url
            && a.duration_ms == b.duration_ms
            && a.media_uri == b.media_uri
            && a.id == b.id
            && a.album_id == b.album_id
            && a.counter_base_listen_ms == b.counter_base_listen_ms
            && a.counter_base_play_count == b.counter_base_play_count
            && merge_counter_shards(&a.counter_shards, &[]) == merge_counter_shards(&b.counter_shards, &[])
    })
}

fn min_positive(left: i64, right: i64) -> i64 {
    match (left > 0, right > 0) {
        (false, false) => 0,
        (false, true) => right,
        (true, false) => left,
        (true, true) => left.min(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_open(track_count: i64) -> crate::library::playlist_usage::PlaylistUsageOpen {
        crate::library::playlist_usage::PlaylistUsageOpen {
            key: "netease:7".into(),
            source: "netease".into(),
            id: 7,
            subtype: None,
            name: "Mix".into(),
            cover_url: Some("https://example.test/cover.jpg".into()),
            track_count,
            fid: 0,
            mid: 0,
            browse_id: None,
            playlist_id: None,
            subtitle: None,
        }
    }

    #[test]
    fn playlist_opens_count_this_device_and_add_up_with_other_devices() {
        let mut extensions = serde_json::Map::new();
        assert!(record_playlist_open(&mut extensions, &usage_open(10), "desktop", 100));
        assert!(record_playlist_open(&mut extensions, &usage_open(11), "desktop", 200));
        let entry = extensions["playlistUsageStats"][0].clone();
        assert_eq!(
            [json_i64(&entry, "openCount"), json_i64(&entry, "firstOpenedAt"), json_i64(&entry, "lastOpenedAt"), json_i64(&entry, "trackCount")],
            [2, 100, 200, 11]
        );
        assert_eq!(entry["counterShards"][0]["deviceId"], "desktop");
        assert_eq!(entry["counterShards"][0]["playCount"], 2);
        crate::sync::archive::prepare(&SyncData { extensions: extensions.clone(), ..Default::default() }, None)
            .expect("the record must satisfy the strict archive schema");

        let mut android = entry.clone();
        android["counterShards"] = serde_json::json!([{"deviceId": "android", "epochStartedAt": 0, "totalListenMs": 0, "playCount": 3, "firstPlayedAt": 50, "lastPlayedAt": 60}]);
        android["openCount"] = 3.into();
        android["lastOpenedAt"] = 60.into();
        let remote = serde_json::Map::from_iter([("playlistUsageStats".to_string(), serde_json::json!([android]))]);
        let merged = merge_extensions(&extensions, &remote);
        assert_eq!(json_i64(&merged["playlistUsageStats"][0], "openCount"), 5, "opens on both devices add up");

        assert!(record_playlist_open(&mut extensions, &usage_open(0), "desktop", 300), "an emptied playlist is removed");
        assert!(extensions["playlistUsageStats"].as_array().unwrap().is_empty());
        assert!(!record_playlist_open(&mut extensions, &usage_open(0), "desktop", 400));
    }

    #[test]
    fn local_playlist_plays_count_the_total_and_the_day_on_this_device() {
        let day = 86_400_000;
        let mut extensions = serde_json::Map::new();
        assert!(record_local_playlist_play(&mut extensions, 7, day, "desktop", day + 100));
        assert!(record_local_playlist_play(&mut extensions, 7, day, "desktop", day + 200));
        assert!(record_local_playlist_play(&mut extensions, 7, 2 * day, "desktop", 2 * day + 100));
        assert!(!record_local_playlist_play(&mut extensions, 0, day, "desktop", day));
        let stat = extensions["localPlaylistPlaybackStats"][0].clone();
        assert_eq!(
            [json_i64(&stat, "totalPlayCount"), json_i64(&stat, "firstPlayedAt"), json_i64(&stat, "lastPlayedAt")],
            [3, day + 100, 2 * day + 100]
        );
        let buckets: Vec<(i64, i64)> = extensions["localPlaylistPlaybackBuckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|bucket| (json_i64(bucket, "dayStartAt"), json_i64(bucket, "playCount")))
            .collect();
        assert_eq!(buckets, [(day, 2), (2 * day, 1)]);
        crate::sync::archive::prepare(&SyncData { extensions: extensions.clone(), ..Default::default() }, None)
            .expect("the records must satisfy the strict archive schema");

        let mut android = stat.clone();
        android["counterShards"] = serde_json::json!([{"deviceId": "android", "epochStartedAt": 0, "totalListenMs": 0, "playCount": 4, "firstPlayedAt": 10, "lastPlayedAt": 20}]);
        android["totalPlayCount"] = 4.into();
        let remote = serde_json::Map::from_iter([("localPlaylistPlaybackStats".to_string(), serde_json::json!([android]))]);
        let merged = merge_extensions(&extensions, &remote);
        assert_eq!(json_i64(&merged["localPlaylistPlaybackStats"][0], "totalPlayCount"), 7, "plays on both devices add up");
    }

    #[test]
    fn reopening_a_playlist_removed_elsewhere_starts_a_record_that_survives_the_merge() {
        let mut extensions = serde_json::Map::from_iter([
            (
                "playlistUsageDeletions".to_string(),
                serde_json::json!([{"playlistKey": "netease:7", "deletionTokens": [{"deviceId": "android", "counter": 4}], "deletedAt": 90}]),
            ),
            (
                "playlistUsageStats".to_string(),
                serde_json::json!([{"playlistKey": "netease:7", "source": "netease", "id": 7, "name": "Old", "trackCount": 5, "firstOpenedAt": 10, "lastOpenedAt": 20, "openCount": 9, "counterBaseOpenCount": 9, "counterShards": [], "fid": 0, "mid": 0, "observedDeletionTokens": []}]),
            ),
        ]);
        assert!(record_playlist_open(&mut extensions, &usage_open(10), "desktop", 100));
        let entry = &extensions["playlistUsageStats"][0];
        assert_eq!(json_i64(entry, "openCount"), 1, "counts from before the removal are dropped");
        assert_eq!(entry["observedDeletionTokens"], serde_json::json!([{"deviceId": "android", "counter": 4}]));
        let merged = merge_extensions(&extensions, &serde_json::Map::new());
        assert_eq!(merged["playlistUsageStats"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn lyric_versions_reset_and_utf16_ties_match_android_independent_of_input_order() {
        let unknown=SyncSong{id:"1".into(),lyric_sync_revision:5,..Default::default()};
        assert_eq!(normalize_lyric_state(&unknown).lyric_sync_revision,0);
        let edit=SyncSong{id:"1".into(),lyric_sync_edited:Some(true),lyric_sync_revision:7,matched_lyric:Some("edit".into()),..Default::default()};
        let reset=SyncSong{id:"1".into(),lyric_sync_edited:Some(false),lyric_sync_revision:7,..Default::default()};
        let supplementary=SyncSong{matched_lyric:Some("\u{10000}".into()),..edit.clone()};
        let bmp=SyncSong{matched_lyric:Some("\u{e000}\u{e000}".into()),..edit.clone()};
        for (first,second) in [(supplementary.clone(),bmp.clone()),(bmp.clone(),supplementary.clone())] {
            let mut registry=HashMap::new();select_lyric_override(&mut registry,first);select_lyric_override(&mut registry,second);
            assert_eq!(registry.values().next().unwrap().matched_lyric,bmp.matched_lyric);
        }
        for songs in [vec![edit.clone(),reset.clone()],vec![reset,edit]] {
            let mut registry=HashMap::new();for song in songs {select_lyric_override(&mut registry,normalize_lyric_state(&song));}
            assert_eq!(registry.values().next().unwrap().lyric_sync_edited,Some(false));
            assert!(registry.values().next().unwrap().matched_lyric.is_none());
        }
    }

    #[test]
    fn latest_lyric_survives_older_selected_display_metadata_and_registry_stays_minimal() {
        let mut local=sync_data(vec![playlist(vec![SyncSong{id:"1".into(),name:"desktop title".into(),added_at:20,sync_metadata_version:1,lyric_sync_edited:Some(true),lyric_sync_revision:2,matched_lyric:Some("old".into()),..Default::default()}])]);
        let mut remote=local.clone();local.playlists[0].modified_at=30;remote.playlists[0].modified_at=10;
        remote.playlists[0].songs[0].lyric_sync_revision=3;remote.playlists[0].songs[0].matched_lyric=Some("new\n".into());
        let merged=three_way_merge(&local,&remote,0,&HashMap::new());
        assert_eq!(merged.playlists[0].songs[0].matched_lyric.as_deref(),Some("new\n"));
        assert_eq!(merged.extensions["lyricOverrides"][0]["name"],"");
        assert_eq!(merged.extensions["lyricOverrides"][0]["addedAt"],0);
    }

    #[test]
    fn usage_deletion_proofs_are_checked_per_candidate_before_union() {
        let deletion=serde_json::json!({"playlistKey":"key","deletionTokens":[{"deviceId":"a","counter":1},{"deviceId":"b","counter":1}],"deletedAt":10});
        let left=serde_json::json!({"playlistKey":"key","openCount":1,"observedDeletionTokens":[{"deviceId":"a","counter":1}]});
        let right=serde_json::json!({"playlistKey":"key","openCount":2,"observedDeletionTokens":[{"deviceId":"b","counter":1}]});
        let local=serde_json::Map::from_iter([("playlistUsageStats".into(),serde_json::json!([left])),("playlistUsageDeletions".into(),serde_json::json!([deletion]))]);
        let remote=serde_json::Map::from_iter([("playlistUsageStats".into(),serde_json::json!([right]))]);
        assert!(merge_extensions(&local,&remote)["playlistUsageStats"].as_array().unwrap().is_empty());
    }

    #[test]
    fn local_playlist_aggregate_is_lifted_to_all_day_buckets_without_losing_shards() {
        let local=serde_json::Map::from_iter([("localPlaylistPlaybackStats".into(),serde_json::json!([{"playlistId":1,"totalPlayCount":1,"firstPlayedAt":30,"lastPlayedAt":40,"counterShards":[{"deviceId":"phone","epochStartedAt":1,"playCount":1,"firstPlayedAt":30,"lastPlayedAt":40}]}]))]);
        let remote=serde_json::Map::from_iter([("localPlaylistPlaybackBuckets".into(),serde_json::json!([{"playlistId":1,"dayStartAt":1,"playCount":3,"firstPlayedAt":10,"lastPlayedAt":20},{"playlistId":1,"dayStartAt":2,"playCount":4,"firstPlayedAt":50,"lastPlayedAt":60},{"playlistId":2,"dayStartAt":1,"playCount":5,"firstPlayedAt":2,"lastPlayedAt":4}]))]);
        for merged in [merge_extensions(&local,&remote),merge_extensions(&remote,&local)] {
            assert_eq!(merged["localPlaylistPlaybackStats"][0]["totalPlayCount"],7);
            assert_eq!(merged["localPlaylistPlaybackStats"][0]["firstPlayedAt"],10);
            assert_eq!(merged["localPlaylistPlaybackStats"][0]["lastPlayedAt"],60);
            assert_eq!(merged["localPlaylistPlaybackStats"][0]["counterShards"][0]["deviceId"],"phone");
            assert_eq!(merged["localPlaylistPlaybackStats"][1]["playlistId"],2);
            assert_eq!(merged["localPlaylistPlaybackStats"][1]["totalPlayCount"],5);
        }
    }

    #[test]
    fn remote_phone_reorder_keeps_remote_added_at_values_and_order() {
        let mut local = playlist(vec![song("1", 300), song("2", 200), song("3", 100)]);
        local.modified_at = 100;
        let mut remote = playlist(vec![song("3", 900), song("1", 899), song("2", 898)]);
        remote.modified_at = 300;

        let merged = three_way_merge(
            &sync_data(vec![local]),
            &sync_data(vec![remote]),
            200,
            &HashMap::new(),
        );

        assert_eq!(
            merged.playlists[0]
                .songs
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["3", "1", "2"]
        );
        assert_eq!(
            merged.playlists[0]
                .songs
                .iter()
                .map(|song| song.added_at)
                .collect::<Vec<_>>(),
            vec![900, 899, 898]
        );
    }

    #[test]
    fn local_desktop_reorder_keeps_local_added_at_values_and_order() {
        let mut local = playlist(vec![song("2", 900), song("3", 899), song("1", 898)]);
        local.modified_at = 300;
        let mut remote = playlist(vec![song("1", 300), song("2", 200), song("3", 100)]);
        remote.modified_at = 100;

        let merged = three_way_merge(
            &sync_data(vec![local]),
            &sync_data(vec![remote]),
            200,
            &HashMap::new(),
        );

        assert_eq!(
            merged.playlists[0]
                .songs
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "3", "1"]
        );
        assert_eq!(
            merged.playlists[0]
                .songs
                .iter()
                .map(|song| song.added_at)
                .collect::<Vec<_>>(),
            vec![900, 899, 898]
        );
    }

    #[test]
    fn missing_created_at_does_not_replace_valid_timestamp() {
        let mut local = playlist(Vec::new());
        local.created_at = 100;
        let mut remote = local.clone();
        remote.created_at = 0;

        let merged = three_way_merge(
            &sync_data(vec![local]),
            &sync_data(vec![remote]),
            0,
            &HashMap::new(),
        );

        assert_eq!(merged.playlists[0].created_at, 100);
    }

    #[test]
    fn merge_preserves_remote_added_at_when_desktop_local_lacks_it() {
        let local_song = song("42", 0);
        let remote_song = song("42", 500);
        let base_key = remote_song.identity().stable_key();
        let base_snapshot = HashMap::from([("1".to_string(), HashSet::from([base_key]))]);
        let merged = three_way_merge(
            &sync_data(vec![playlist(vec![local_song])]),
            &sync_data(vec![playlist(vec![remote_song])]),
            100,
            &base_snapshot,
        );
        assert_eq!(merged.playlists[0].songs[0].added_at, 500);
    }

    #[test]
    fn local_deleted_song_is_not_resurrected_from_remote() {
        let deleted_song = song("42", 500);
        let base_snapshot = HashMap::from([(
            "1".to_string(),
            HashSet::from([deleted_song.identity().stable_key()]),
        )]);
        let merged = three_way_merge(
            &sync_data(vec![playlist(Vec::new())]),
            &sync_data(vec![playlist(vec![deleted_song])]),
            100,
            &base_snapshot,
        );
        assert!(merged.playlists[0].songs.is_empty());
    }

    #[test]
    fn remote_deleted_song_is_not_resurrected_from_local() {
        let deleted_song = song("42", 500);
        let base_snapshot = HashMap::from([(
            "1".to_string(),
            HashSet::from([deleted_song.identity().stable_key()]),
        )]);
        let merged = three_way_merge(
            &sync_data(vec![playlist(vec![deleted_song])]),
            &sync_data(vec![playlist(Vec::new())]),
            100,
            &base_snapshot,
        );
        assert!(merged.playlists[0].songs.is_empty());
    }

    #[test]
    fn playlist_deletion_tombstone_is_preserved() {
        let active = playlist(Vec::new());
        let mut deleted = active.clone();
        deleted.is_deleted = true;
        deleted.modified_at = 200;
        let merged = three_way_merge(
            &sync_data(vec![active]),
            &sync_data(vec![deleted]),
            100,
            &HashMap::new(),
        );
        assert!(merged.playlists[0].is_deleted);
        assert!(merged.playlists[0].songs.is_empty());
    }

    fn edited_lyrics(revision: i64, text: &str) -> SyncSong {
        SyncSong {
            lyric_sync_edited: Some(true),
            lyric_sync_revision: revision,
            matched_lyric: Some(text.into()),
            ..song("1", 10)
        }
    }

    #[test]
    fn recorded_lyric_overrides_keep_the_highest_revision() {
        let mut extensions = serde_json::Map::new();
        record_lyric_override(&mut extensions, &edited_lyrics(10, "first")).unwrap();
        record_lyric_override(&mut extensions, &edited_lyrics(5, "older")).unwrap();
        assert_eq!(extensions["lyricOverrides"][0]["matchedLyric"], "first");
        record_lyric_override(&mut extensions, &edited_lyrics(20, "newer")).unwrap();
        assert_eq!(extensions["lyricOverrides"].as_array().unwrap().len(), 1);
        assert_eq!(extensions["lyricOverrides"][0]["matchedLyric"], "newer");
    }

    #[test]
    fn a_desktop_lyric_edit_after_the_phone_edit_wins_the_merge() {
        let phone = edited_lyrics(5, "phone");
        let desk = edited_lyrics(1_800_000_000_000, "desk");
        for (local, remote) in [(&desk, &phone), (&phone, &desk)] {
            let merged = three_way_merge(
                &sync_data(vec![playlist(vec![local.clone()])]),
                &sync_data(vec![playlist(vec![remote.clone()])]),
                0,
                &HashMap::new(),
            );
            assert_eq!(merged.playlists[0].songs[0].matched_lyric.as_deref(), Some("desk"));
        }
    }

    #[test]
    fn a_playlist_edited_after_its_deletion_survives_the_merge() {
        let mut tombstone = playlist(Vec::new());
        tombstone.is_deleted = true;
        tombstone.modified_at = 100;
        let mut edited = playlist(vec![song("42", 150)]);
        edited.modified_at = 200;
        for (local, remote) in [(&tombstone, &edited), (&edited, &tombstone)] {
            let merged = three_way_merge(
                &sync_data(vec![local.clone()]),
                &sync_data(vec![remote.clone()]),
                50,
                &HashMap::new(),
            );
            assert!(!merged.playlists[0].is_deleted, "the newer live copy wins on either side");
            assert_eq!(merged.playlists[0].songs.len(), 1);
        }
        edited.modified_at = 100;
        let merged = three_way_merge(&sync_data(vec![tombstone]), &sync_data(vec![edited]), 50, &HashMap::new());
        assert!(merged.playlists[0].is_deleted, "a deletion at the same time as the last edit is kept");
    }

    #[test]
    fn playlist_song_deletion_is_applied() {
        let target = song("42", 100);
        let deletion = SyncPlaylistSongDeletion {
            playlist_id: "1".into(),
            song_id: target.id.clone(),
            album: target.album.clone(),
            deleted_at: 200,
            device_id: "desktop".into(),
            ..Default::default()
        };
        let mut local = sync_data(vec![playlist(vec![target])]);
        local.playlist_song_deletions = vec![deletion];
        let merged = three_way_merge(&local, &SyncData::default(), 0, &HashMap::new());
        assert!(merged.playlists[0].songs.is_empty());
    }

    #[test]
    fn legacy_added_at_prevents_display_order_migration_from_reviving_deleted_song() {
        let mut target = song("42", 900);
        target.legacy_added_at = Some(100);
        let deletion = SyncPlaylistSongDeletion {
            playlist_id: "1".into(),
            song_id: target.id.clone(),
            album: target.album.clone(),
            deleted_at: 200,
            device_id: "android".into(),
            ..Default::default()
        };

        let remaining = apply_playlist_song_deletions("1", &[target], &[deletion]);

        assert!(remaining.is_empty());
    }

    #[test]
    fn local_primary_does_not_clear_remote_lyrics() {
        let mut local = song("42", 500);
        local.matched_lyric = Some(String::new());
        local.sync_metadata_version = LEGACY_SYNC_METADATA_VERSION;
        let mut remote = song("42", 500);
        remote.matched_lyric = Some("[00:01.00]cloud lyric".into());
        remote.matched_translated_lyric = Some("[00:01.00]translation".into());
        remote.original_lyric = Some("original cloud lyric".into());
        remote.sync_metadata_version = LEGACY_SYNC_METADATA_VERSION;

        let merged = merge_songs(
            &[local],
            &[remote],
            300,
            200,
            true,
            true,
            100,
            false,
            &HashSet::new(),
            "1",
            &[],
        );

        assert_eq!(
            merged[0].matched_lyric.as_deref(),
            Some("[00:01.00]cloud lyric")
        );
        assert_eq!(
            merged[0].matched_translated_lyric.as_deref(),
            Some("[00:01.00]translation")
        );
        assert_eq!(
            merged[0].original_lyric.as_deref(),
            Some("original cloud lyric")
        );
        assert_eq!(
            merged[0].sync_metadata_version,
            CURRENT_SYNC_METADATA_VERSION
        );
    }

    #[test]
    fn legacy_primary_order_keeps_current_rich_metadata() {
        let mut current = song("42", 100);
        current.matched_lyric = Some("[00:01.00]lyric".into());
        current.custom_name = Some("Custom title".into());
        current.original_name = Some("Original".into());
        let mut legacy_primary = current.clone();
        legacy_primary.name = "Custom title".into();
        legacy_primary.added_at = 900;
        legacy_primary.matched_lyric = None;
        legacy_primary.custom_name = None;
        legacy_primary.original_name = None;
        legacy_primary.sync_metadata_version = LEGACY_SYNC_METADATA_VERSION;

        let merged = merge_songs(
            &[current],
            &[legacy_primary],
            100,
            300,
            false,
            true,
            200,
            false,
            &HashSet::new(),
            "1",
            &[],
        );

        assert_eq!(merged[0].added_at, 900);
        assert_eq!(merged[0].name, "Song");
        assert_eq!(merged[0].matched_lyric.as_deref(), Some("[00:01.00]lyric"));
        assert_eq!(merged[0].custom_name.as_deref(), Some("Custom title"));
        assert_eq!(merged[0].sync_metadata_version, CURRENT_SYNC_METADATA_VERSION);
    }

    #[test]
    fn current_primary_can_intentionally_clear_metadata() {
        let mut local = song("42", 100);
        local.matched_lyric = Some("[00:01.00]old lyric".into());
        local.custom_name = Some("Old custom title".into());
        let mut remote = local.clone();
        remote.added_at = 200;
        remote.matched_lyric = None;
        remote.custom_name = None;

        let merged = merge_songs(
            &[local],
            &[remote],
            100,
            300,
            false,
            true,
            200,
            false,
            &HashSet::new(),
            "1",
            &[],
        );

        assert_eq!(merged[0].added_at, 200);
        assert!(merged[0].matched_lyric.is_none());
        assert!(merged[0].custom_name.is_none());
    }

    #[test]
    fn remote_only_change_uses_remote_membership_and_payload() {
        let local_token = SyncCausalToken {
            device_id: "desktop".into(),
            counter: 1,
        };
        let remote_token = SyncCausalToken {
            device_id: "android".into(),
            counter: 1,
        };
        let mut local_match = song("42", 100);
        local_match.name = "Local metadata".into();
        local_match.sync_membership_tokens = vec![local_token.clone()];
        let mut remote_match = local_match.clone();
        remote_match.name = "Remote metadata".into();
        remote_match.sync_membership_tokens = vec![remote_token.clone()];
        let local_only = song("99", 100);

        let merged = merge_songs(
            &[local_match, local_only],
            &[remote_match],
            100,
            200,
            false,
            true,
            150,
            false,
            &HashSet::new(),
            "1",
            &[],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "Remote metadata");
        assert_eq!(
            merged[0].sync_membership_tokens,
            vec![remote_token, local_token]
        );
    }

    #[test]
    fn bridge_aliases_collapse_the_full_membership_component() {
        let token_a = SyncCausalToken {
            device_id: "a".into(),
            counter: 1,
        };
        let token_b = SyncCausalToken {
            device_id: "b".into(),
            counter: 1,
        };
        let mut first = song("1", 100);
        first.sync_membership_tokens = vec![token_a.clone()];
        let mut second = song("2", 100);
        second.sync_membership_tokens = vec![token_b.clone()];
        let mut bridge = first.clone();
        bridge.sync_membership_tokens = vec![token_b.clone()];

        let merged = deduplicate_songs(&[first, second, bridge]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sync_membership_tokens, vec![token_a, token_b]);
    }

    #[test]
    fn counter_shards_keep_multiple_epochs_in_stable_order() {
        let left = SyncPlaybackCounterShard { device_id: "d".into(), epoch_started_at: 1, play_count: 1, ..Default::default() };
        let right = SyncPlaybackCounterShard { device_id: "d".into(), epoch_started_at: 2, play_count: 2, ..Default::default() };
        let merged = merge_counter_shards(&[left], &[right]);
        assert_eq!(merged.iter().map(|shard| shard.epoch_started_at).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn data_change_detection_includes_song_metadata_and_deletions() {
        let remote = sync_data(vec![playlist(vec![song("42", 10_000)])]);
        let mut merged = remote.clone();
        merged.playlists[0].songs[0].name = "Changed".into();
        assert!(has_data_changed(&remote, &merged));
    }

    #[test]
    fn playlist_timestamp_only_change_is_not_data_change() {
        let remote = sync_data(vec![playlist(vec![song("42", 10_000)])]);
        let mut merged = remote.clone();
        merged.playlists[0].created_at = 999;
        merged.playlists[0].modified_at = 1_000_000;
        assert!(!has_data_changed(&remote, &merged));
    }

    #[test]
    fn favorite_display_only_fields_are_not_data_change() {
        let remote = SyncData {
            favorite_playlists: vec![SyncFavoritePlaylist {
                id: "7".into(),
                name: "Fav".into(),
                cover_url: "https://a".into(),
                track_count: 1,
                source: "netease".into(),
                songs: vec![song("42", 10)],
                added_time: 1,
                modified_at: 2,
                is_deleted: false,
                sort_order: 3,
                browse_id: Some("B".into()),
                playlist_id: Some("P".into()),
                subtitle: Some("old".into()),
            }],
            ..Default::default()
        };
        let mut merged = remote.clone();
        // 仅展示字段变化不应触发回声上传
        merged.favorite_playlists[0].name = "Fav Renamed".into();
        merged.favorite_playlists[0].cover_url = "https://b".into();
        merged.favorite_playlists[0].added_time = 99;
        merged.favorite_playlists[0].browse_id = Some("B2".into());
        merged.favorite_playlists[0].subtitle = Some("new".into());
        assert!(!has_data_changed(&remote, &merged));

        // 内容字段变化仍应检测
        merged.favorite_playlists[0].sort_order = 4;
        assert!(has_data_changed(&remote, &merged));
    }

    #[test]
    fn favorite_deletion_ties_win_in_both_merge_directions() {
        let deleted = SyncFavoritePlaylist {
            id: "7".into(),
            name: "Deleted favorite".into(),
            cover_url: String::new(),
            source: "netease".into(),
            songs: vec![song("1", 10)],
            track_count: 1,
            added_time: 40,
            modified_at: 100,
            is_deleted: true,
            sort_order: 9,
            browse_id: None,
            playlist_id: None,
            subtitle: None,
        };
        let active = SyncFavoritePlaylist {
            name: "Active favorite".into(),
            songs: vec![song("2", 10)],
            added_time: 80,
            is_deleted: false,
            sort_order: 4,
            ..deleted.clone()
        };

        for (left, right) in [(&active, &deleted), (&deleted, &active)] {
            let merged =
                merge_favorite_playlists(std::slice::from_ref(left), std::slice::from_ref(right));
            assert_eq!(merged.len(), 1);
            let favorite = &merged[0];
            assert!(favorite.is_deleted);
            assert_eq!(favorite.name, "Deleted favorite");
            assert!(favorite.songs.is_empty());
            assert_eq!(favorite.track_count, 0);
            assert_eq!(favorite.modified_at, 100);
            assert_eq!(favorite.added_time, 80);
            assert_eq!(favorite.sort_order, 9);
        }
    }

    #[test]
    fn favorite_restore_after_deletion_is_retained() {
        let deleted = SyncFavoritePlaylist {
            id: "7".into(),
            name: String::new(),
            cover_url: String::new(),
            source: "netease".into(),
            songs: Vec::new(),
            track_count: 0,
            added_time: 0,
            modified_at: 100,
            is_deleted: true,
            sort_order: 9,
            browse_id: None,
            playlist_id: None,
            subtitle: None,
        };
        let restored = SyncFavoritePlaylist {
            modified_at: 110,
            is_deleted: false,
            songs: vec![song("2", 110)],
            sort_order: 4,
            ..deleted.clone()
        };
        for (left, right) in [(&restored, &deleted), (&deleted, &restored)] {
            let merged =
                merge_favorite_playlists(std::slice::from_ref(left), std::slice::from_ref(right));
            assert_eq!(merged.len(), 1);
            assert!(!merged[0].is_deleted);
            assert_eq!(merged[0].songs.len(), 1);
            assert_eq!(merged[0].songs[0].id, "2");
            assert_eq!(merged[0].track_count, 1);
            assert_eq!(merged[0].sort_order, 4);
        }
    }

    #[test]
    fn playback_clear_rejects_old_epoch_despite_a_new_last_play() {
        let shard = SyncPlaybackCounterShard {
            device_id: "android".into(),
            epoch_started_at: 0,
            first_played_at: 50,
            last_played_at: 110,
            total_listen_ms: 1_000,
            play_count: 10,
        };
        let stat = SyncTrackStat {
            identity_key: "k".into(),
            first_played_at: 50,
            last_played_at: 110,
            total_listen_ms: 1_000,
            play_count: 10,
            counter_shards: vec![shard],
            ..Default::default()
        };
        let bucket = bucket_from_clear_stat(&stat);

        assert!(merge_playback_stats(std::slice::from_ref(&stat), &[], 100).is_empty());
        assert!(merge_playback_stats(&[], std::slice::from_ref(&stat), 100).is_empty());
        assert!(merge_stat_buckets(std::slice::from_ref(&bucket), &[], 100).is_empty());
        assert!(merge_stat_buckets(&[], std::slice::from_ref(&bucket), 100).is_empty());
    }

    #[test]
    fn playback_clear_recomputes_totals_from_only_current_epoch_shards() {
        let shards = vec![
            SyncPlaybackCounterShard {
                device_id: "old".into(),
                epoch_started_at: 0,
                first_played_at: 50,
                last_played_at: 150,
                total_listen_ms: 1_000,
                play_count: 10,
            },
            SyncPlaybackCounterShard {
                device_id: "cross-clear".into(),
                epoch_started_at: 100,
                first_played_at: 90,
                last_played_at: 145,
                total_listen_ms: 20,
                play_count: 2,
            },
            SyncPlaybackCounterShard {
                device_id: "current-a".into(),
                epoch_started_at: 100,
                first_played_at: 100,
                last_played_at: 130,
                total_listen_ms: 300,
                play_count: 3,
            },
            SyncPlaybackCounterShard {
                device_id: "current-b".into(),
                epoch_started_at: 110,
                first_played_at: 115,
                last_played_at: 140,
                total_listen_ms: 400,
                play_count: 4,
            },
        ];
        let stat = SyncTrackStat {
            identity_key: "k".into(),
            first_played_at: 50,
            last_played_at: 150,
            total_listen_ms: 2_620,
            play_count: 28,
            counter_base_listen_ms: 900,
            counter_base_play_count: 9,
            counter_shards: shards,
            ..Default::default()
        };
        let bucket = bucket_from_clear_stat(&stat);
        let merged = merge_playback_stats(
            std::slice::from_ref(&stat),
            std::slice::from_ref(&stat),
            100,
        );
        let buckets = merge_stat_buckets(
            std::slice::from_ref(&bucket),
            std::slice::from_ref(&bucket),
            100,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(buckets.len(), 1);
        for (total, count, first, last, base_total, base_count, retained) in [
            (
                merged[0].total_listen_ms,
                merged[0].play_count,
                merged[0].first_played_at,
                merged[0].last_played_at,
                merged[0].counter_base_listen_ms,
                merged[0].counter_base_play_count,
                &merged[0].counter_shards,
            ),
            (
                buckets[0].total_listen_ms,
                buckets[0].play_count,
                buckets[0].first_played_at,
                buckets[0].last_played_at,
                buckets[0].counter_base_listen_ms,
                buckets[0].counter_base_play_count,
                &buckets[0].counter_shards,
            ),
        ] {
            assert_eq!((total, count, first, last), (700, 7, 100, 140));
            assert_eq!((base_total, base_count), (0, 0));
            assert_eq!(retained.len(), 2);
            assert_eq!(retained[0].device_id, "current-a");
            assert_eq!(retained[1].device_id, "current-b");
        }
        let again = merge_playback_stats(&merged, &merged, 100);
        let buckets_again = merge_stat_buckets(&buckets, &buckets, 100);
        assert_eq!((again[0].total_listen_ms, again[0].play_count), (700, 7));
        assert_eq!(again[0].counter_shards.len(), 2);
        assert_eq!(
            (
                buckets_again[0].total_listen_ms,
                buckets_again[0].play_count
            ),
            (700, 7)
        );
        assert_eq!(buckets_again[0].counter_shards.len(), 2);
    }

    #[test]
    fn playback_clear_accepts_legacy_counters_only_when_all_plays_follow_clear() {
        for (first, last, retained) in [
            (50, 110, false),
            (100, 110, true),
            (110, 110, true),
            (100, 99, false),
        ] {
            let stat = SyncTrackStat {
                identity_key: "legacy".into(),
                first_played_at: first,
                last_played_at: last,
                total_listen_ms: 600,
                play_count: 6,
                ..Default::default()
            };
            let bucket = bucket_from_clear_stat(&stat);
            let merged = merge_playback_stats(std::slice::from_ref(&stat), &[], 100);
            let buckets = merge_stat_buckets(std::slice::from_ref(&bucket), &[], 100);
            assert_eq!(!merged.is_empty(), retained, "first={first}, last={last}");
            assert_eq!(!buckets.is_empty(), retained, "first={first}, last={last}");
            if retained {
                assert_eq!((merged[0].total_listen_ms, merged[0].play_count), (600, 6));
                assert_eq!(
                    (buckets[0].total_listen_ms, buckets[0].play_count),
                    (600, 6)
                );
            }
        }
    }

    fn bucket_from_clear_stat(stat: &SyncTrackStat) -> SyncPlaybackStatBucket {
        SyncPlaybackStatBucket {
            day_start_at: 0,
            identity_key: stat.identity_key.clone(),
            total_listen_ms: stat.total_listen_ms,
            play_count: stat.play_count,
            first_played_at: stat.first_played_at,
            last_played_at: stat.last_played_at,
            counter_base_listen_ms: stat.counter_base_listen_ms,
            counter_base_play_count: stat.counter_base_play_count,
            counter_shards: stat.counter_shards.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn playlist_reordering_alone_is_not_a_data_change() {
        let first = playlist(vec![song("1", 10)]);
        let second = SyncPlaylist { id: "2".into(), ..playlist(vec![song("2", 20)]) };
        let remote = sync_data(vec![first.clone(), second.clone()]);
        let reordered = sync_data(vec![second, first]);

        assert!(!has_data_changed(&remote, &reordered));
    }

    #[test]
    fn aggregate_stats_never_fall_below_daily_bucket_totals() {
        let stat = SyncTrackStat {
            identity_key: "k".into(),
            total_listen_ms: 900,
            play_count: 876,
            last_played_at: 100,
            ..Default::default()
        };
        let buckets = vec![
            SyncPlaybackStatBucket {
                day_start_at: 0,
                identity_key: "k".into(),
                total_listen_ms: 600,
                play_count: 500,
                last_played_at: 50,
                ..Default::default()
            },
            SyncPlaybackStatBucket {
                day_start_at: MILLIS_PER_DAY,
                identity_key: "k".into(),
                total_listen_ms: 700,
                play_count: 381,
                last_played_at: 100,
                ..Default::default()
            },
        ];

        let lifted = lift_stats_to_bucket_totals(&[stat], &buckets);

        assert_eq!(lifted[0].play_count, 881);
        assert_eq!(lifted[0].total_listen_ms, 1_300);
        // 单调抬升必须幂等, 否则与对端的 max 合并会来回震荡
        let again = lift_stats_to_bucket_totals(&lifted, &buckets);
        assert_eq!(again[0].play_count, 881);
        assert_eq!(again[0].total_listen_ms, 1_300);
    }

    #[test]
    fn archive_merge_preserves_old_buckets_and_all_stats() {
        let mut local=sync_data(vec![]);
        local.playback_stats=(0..2010).map(|index|SyncTrackStat{identity_key:format!("key{index}"),last_played_at:index, ..Default::default()}).collect();
        local.playback_stat_buckets=vec![stat_bucket(MILLIS_PER_DAY,"old"),stat_bucket(1000*MILLIS_PER_DAY,"new")];
        let merged=three_way_merge(&local,&SyncData::default(),0,&HashMap::new());
        assert_eq!(merged.playback_stats.len(),2010);
        assert_eq!(merged.playback_stat_buckets.len(),2);
        let again=three_way_merge(&merged,&merged,0,&HashMap::new());
        assert_eq!(again.playback_stats.len(),2010);
        assert_eq!(again.playback_stat_buckets.len(),2);
    }

    /// Y2 回归：legacy 迁移合成的 addedAt（无 membership token）不得据此裁墓碑，
    /// 否则已删歌会在所有端复活（对齐 Android SyncPlaylistDeletionPolicy P1-1）
    #[test]
    fn legacy_tombstone_survives_active_song_without_membership_tokens() {
        let active = song("42", 300);
        assert!(active.sync_membership_tokens.is_empty());
        let deletion = SyncPlaylistSongDeletion {
            playlist_id: "1".into(),
            song_id: "42".into(),
            album: "netease".into(),
            deleted_at: 200,
            device_id: "phone".into(),
            ..Default::default()
        };

        let pruned = prune_playlist_song_deletions(&[deletion], &[playlist(vec![active])]);

        assert_eq!(pruned.len(), 1, "无 token 的活跃歌不构成\"重新添加\"证据");
    }

    #[test]
    fn legacy_tombstone_is_pruned_when_readded_song_carries_membership_token() {
        let mut readded = song("42", 300);
        readded.sync_membership_tokens = vec![SyncCausalToken {
            device_id: "desktop".into(),
            counter: 7,
        }];
        let deletion = SyncPlaylistSongDeletion {
            playlist_id: "1".into(),
            song_id: "42".into(),
            album: "netease".into(),
            deleted_at: 200,
            device_id: "phone".into(),
            ..Default::default()
        };

        let pruned = prune_playlist_song_deletions(&[deletion], &[playlist(vec![readded])]);

        assert!(pruned.is_empty(), "带 token 且 added_at 更晚才算真正重新添加");
    }

    #[test]
    fn counter_shard_totals_saturate_before_overflow() {
        for (listen, count) in [(i64::MAX, 1), (1, i32::MAX)] {
            let local = SyncTrackStat {
                identity_key: "k".into(),
                total_listen_ms: listen,
                play_count: count,
                first_played_at: 100,
                last_played_at: 100,
                counter_shards: vec![SyncPlaybackCounterShard {
                    device_id: "desktop".into(),
                    total_listen_ms: listen,
                    play_count: count,
                    first_played_at: 100,
                    last_played_at: 100,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let mut remote = local.clone();
            remote.counter_shards[0].device_id = "android".into();
            let stats = merge_playback_stats(std::slice::from_ref(&local), std::slice::from_ref(&remote), 0);
            let buckets = merge_stat_buckets(&[bucket_from_clear_stat(&local)], &[bucket_from_clear_stat(&remote)], 0);
            let expected_listen = listen.saturating_add(listen);
            let expected_count = count.saturating_add(count);
            assert_eq!(stats[0].total_listen_ms, expected_listen);
            assert_eq!(stats[0].play_count, expected_count);
            assert_eq!(buckets[0].total_listen_ms, expected_listen);
            assert_eq!(buckets[0].play_count, expected_count);
            assert_eq!(stats[0].counter_shards.len(), 2);
            let again = merge_playback_stats(&stats, &stats, 0);
            assert_eq!(again[0].total_listen_ms, expected_listen);
            assert_eq!(again[0].play_count, expected_count);
        }
    }

    /// Y3 回归：合并后分片为空时 counter_base 必须归零（对齐 Android），
    /// 否则 base=total 与对端的 0 每轮互相"纠正"，回声上传永不收敛
    #[test]
    fn counter_base_is_zeroed_when_merged_shards_are_empty() {
        let local = SyncTrackStat {
            identity_key: "k".into(),
            total_listen_ms: 900,
            play_count: 9,
            last_played_at: 100,
            ..Default::default()
        };
        let mut remote = local.clone();
        remote.total_listen_ms = 700;
        remote.play_count = 7;
        remote.last_played_at = 90;

        let merged = merge_playback_stats(&[local], &[remote], 0);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].total_listen_ms, 900);
        assert_eq!(merged[0].play_count, 9);
        assert_eq!(merged[0].counter_base_listen_ms, 0);
        assert_eq!(merged[0].counter_base_play_count, 0);
    }

    /// Y6 回归：双方都改名固定 local-wins（对齐 Android），不再按 modified_at 裁决
    #[test]
    fn playlist_rename_conflict_prefers_local_when_both_sides_changed() {
        let mut local = playlist(Vec::new());
        local.name = "Local name".into();
        local.modified_at = 300;
        let mut remote = playlist(Vec::new());
        remote.name = "Remote name".into();
        // 远端时间戳更晚（快钟设备）也不能赢
        remote.modified_at = 400;

        let merged = three_way_merge(
            &sync_data(vec![local]),
            &sync_data(vec![remote]),
            200,
            &HashMap::new(),
        );

        assert_eq!(merged.playlists[0].name, "Local name");
    }

    /// Y7 回归：已删歌单不参与来源顺序，墓碑统一补在末尾（对齐 Android filterNot）
    #[test]
    fn deleted_playlists_are_appended_after_active_ones_in_merge_order() {
        let mut tombstone = playlist(Vec::new());
        tombstone.id = "2".into();
        tombstone.is_deleted = true;
        tombstone.modified_at = 300;
        let active = playlist(Vec::new());
        let local = sync_data(vec![tombstone.clone(), active.clone()]);
        let remote = sync_data(vec![tombstone, active]);

        let merged = three_way_merge(&local, &remote, 200, &HashMap::new());

        assert_eq!(
            merged
                .playlists
                .iter()
                .map(|playlist| (playlist.id.as_str(), playlist.is_deleted))
                .collect::<Vec<_>>(),
            vec![("1", false), ("2", true)],
        );
    }

    fn stat_bucket(day_start_at: i64, identity_key: &str) -> SyncPlaybackStatBucket {
        SyncPlaybackStatBucket {
            day_start_at,
            identity_key: identity_key.into(),
            play_count: 1,
            last_played_at: day_start_at,
            ..Default::default()
        }
    }

    fn sync_data(playlists: Vec<SyncPlaylist>) -> SyncData {
        SyncData { playlists, ..Default::default() }
    }

    fn playlist(songs: Vec<SyncSong>) -> SyncPlaylist {
        SyncPlaylist {
            id: "1".into(),
            name: "Playlist".into(),
            songs,
            created_at: 1,
            modified_at: 20,
            is_deleted: false,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        }
    }

    fn song(id: &str, added_at: i64) -> SyncSong {
        SyncSong {
            id: id.into(),
            name: "Song".into(),
            album: "netease".into(),
            channel_id: Some("netease".into()),
            audio_id: Some(id.into()),
            added_at,
            sync_metadata_version: CURRENT_SYNC_METADATA_VERSION,
            ..Default::default()
        }
    }
}
