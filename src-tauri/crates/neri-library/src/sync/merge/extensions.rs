//! 扩展段合并：歌单使用计数、本地歌单播放统计与元数据记录

use super::*;

/// 记一次歌单打开（对齐 Android PlaylistUsageRepository.recordOpen）
///
/// 只增加本机的计数分片（epoch 0），其它设备的分片原样保留，同步时按分片相加；
/// 歌单已经没有歌曲时移除记录。返回扩展段是否有变化
pub fn record_playlist_open(
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
    let index = stats
        .iter()
        .position(|entry| entry["playlistKey"].as_str() == Some(open.key.as_str()));
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
            entry["observedDeletionTokens"] =
                serde_json::json!(normalize_sync_causal_tokens(&tokens));
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

    count_on_device_shard(
        &mut entry,
        ("openCount", "counterBaseOpenCount"),
        device_id,
        now,
        i64::from(i32::MAX),
    );
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
pub fn record_local_playlist_play(
    extensions: &mut serde_json::Map<String, serde_json::Value>,
    playlist_id: i64,
    day_start_at: i64,
    device_id: &str,
    played_at: i64,
) -> bool {
    if playlist_id == 0 {
        return false;
    }
    let record = |section: &str,
                  matches: &dyn Fn(&serde_json::Value) -> bool,
                  empty: serde_json::Value,
                  count_key: &str| {
        let mut items = extensions
            .get(section)
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let index = items.iter().position(matches);
        let mut item = index.map_or(empty, |index| items[index].clone());
        count_on_device_shard(
            &mut item,
            (count_key, "counterBasePlayCount"),
            device_id,
            played_at,
            i64::MAX,
        );
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
        &|bucket| {
            json_i64(bucket, "playlistId") == playlist_id
                && json_i64(bucket, "dayStartAt") == day_start_at
        },
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
        shards.iter().fold(0_i64, |total, shard| {
            total.saturating_add(i64::from(shard.play_count.max(0)))
        })
    };
    let count = json_i64(record, count_key).max(0);
    let base = if shards.is_empty() {
        count
    } else {
        json_i64(record, base_key)
            .max(0)
            .max(count.saturating_sub(shard_total(&shards)))
    };
    match shards
        .iter_mut()
        .find(|shard| shard.device_id == device_id && shard.epoch_started_at == 0)
    {
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
    record[count_key] = count
        .max(base.saturating_add(shard_total(&shards)))
        .min(count_limit)
        .into();
    record[base_key] = base.into();
    record["counterShards"] = serde_json::json!(shards);
}

pub(super) fn merge_extensions(
    local: &serde_json::Map<String, serde_json::Value>,
    remote: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut result = remote.clone();
    for (key, value) in local {
        if let Some(items) = value.as_array() {
            let mut merged = items.clone();
            merged.extend(
                remote
                    .get(key)
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            merged.sort_by_key(serde_json::Value::to_string);
            merged.dedup();
            result.insert(key.clone(), serde_json::Value::Array(merged));
        } else {
            result.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    for (section, keys, timestamp) in [
        ("playlistUsageDeletions", &["playlistKey"][..], "deletedAt"),
        ("playlistUsageStats", &["playlistKey"][..], "lastOpenedAt"),
        (
            "localPlaylistPlaybackStats",
            &["playlistId"][..],
            "lastPlayedAt",
        ),
        (
            "localPlaylistPlaybackBuckets",
            &["playlistId", "dayStartAt"][..],
            "lastPlayedAt",
        ),
    ] {
        let mut groups: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let Some(items) = result
            .get(section)
            .and_then(serde_json::Value::as_array)
            .cloned()
        else {
            continue;
        };
        for item in items {
            if matches!(
                section,
                "localPlaylistPlaybackStats" | "localPlaylistPlaybackBuckets"
            ) && (json_i64(&item, "playlistId") == 0
                || (section == "localPlaylistPlaybackBuckets" && json_i64(&item, "dayStartAt") < 0))
            {
                continue;
            }
            let key = keys
                .iter()
                .map(|key| item[*key].to_string())
                .collect::<Vec<_>>()
                .join("|");
            if section == "playlistUsageStats" && !usage_observes_deletions(&item, &result) {
                continue;
            }
            groups
                .entry(key)
                .and_modify(|previous| {
                    *previous = merge_metadata_record(section, previous, &item, timestamp)
                })
                .or_insert_with(|| merge_metadata_record(section, &item, &item, timestamp));
        }
        result.insert(
            section.into(),
            serde_json::Value::Array(groups.into_values().collect()),
        );
    }
    if result.contains_key("biliVideoSkipRules") {
        if let Ok(rules) = crate::library::bili_video_skip::rules_from_extensions(&result) {
            result.insert("biliVideoSkipRules".into(), serde_json::json!(rules));
        }
    }
    lift_local_playlist_stats(&mut result);
    result
}

fn lift_local_playlist_stats(extensions: &mut serde_json::Map<String, serde_json::Value>) {
    let mut totals: BTreeMap<i64, (i64, i64, i64)> = BTreeMap::new();
    for bucket in extensions
        .get("localPlaylistPlaybackBuckets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let value = totals.entry(json_i64(bucket, "playlistId")).or_default();
        value.0 = value.0.saturating_add(json_i64(bucket, "playCount").max(0));
        value.1 = min_positive(value.1, json_i64(bucket, "firstPlayedAt"));
        value.2 = value.2.max(json_i64(bucket, "lastPlayedAt"));
    }
    if totals.is_empty() {
        return;
    }
    let mut stats = extensions
        .get("localPlaylistPlaybackStats")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for stat in &mut stats {
        if let Some((count, first, last)) = totals.remove(&json_i64(stat, "playlistId")) {
            stat["totalPlayCount"] = json_i64(stat, "totalPlayCount").max(count).into();
            stat["firstPlayedAt"] = min_positive(json_i64(stat, "firstPlayedAt"), first).into();
            stat["lastPlayedAt"] = json_i64(stat, "lastPlayedAt").max(last).into();
        }
    }
    stats.extend(totals.into_iter().map(|(id,(count,first,last))|serde_json::json!({"playlistId":id,"totalPlayCount":count,"firstPlayedAt":first,"lastPlayedAt":last,"counterBasePlayCount":0,"counterShards":[]})));
    stats.sort_by_key(|stat| json_i64(stat, "playlistId"));
    extensions.insert(
        "localPlaylistPlaybackStats".into(),
        serde_json::Value::Array(stats),
    );
}

pub(super) fn json_i64(value: &serde_json::Value, key: &str) -> i64 {
    value[key]
        .as_i64()
        .or_else(|| value[key].as_str()?.parse().ok())
        .unwrap_or(0)
}
fn json_tokens(value: &serde_json::Value, key: &str) -> Vec<SyncCausalToken> {
    serde_json::from_value(value[key].clone())
        .map(|tokens: Vec<SyncCausalToken>| normalize_sync_causal_tokens(&tokens))
        .unwrap_or_default()
}

fn usage_deletion_tokens(value: &serde_json::Value) -> Vec<SyncCausalToken> {
    let tokens = json_tokens(value, "deletionTokens");
    if !tokens.is_empty() || json_i64(value, "deletedAt") <= 0 {
        return tokens;
    }
    let encoded = value["playlistKey"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .encode_utf16()
        .map(|code| format!("{code:04x}"))
        .collect::<String>();
    vec![SyncCausalToken {
        device_id: format!("usage-legacy:{encoded}"),
        counter: json_i64(value, "deletedAt").max(1),
    }]
}

fn usage_observes_deletions(
    value: &serde_json::Value,
    extensions: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    let observed = json_tokens(value, "observedDeletionTokens");
    extensions
        .get("playlistUsageDeletions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|deletion| deletion["playlistKey"] == value["playlistKey"])
        .all(|deletion| {
            usage_deletion_tokens(deletion)
                .iter()
                .all(|token| observed.contains(token))
        })
}

fn merge_metadata_record(
    section: &str,
    left: &serde_json::Value,
    right: &serde_json::Value,
    timestamp: &str,
) -> serde_json::Value {
    let newer = if (json_i64(left, timestamp), left.to_string())
        >= (json_i64(right, timestamp), right.to_string())
    {
        left
    } else {
        right
    };
    let mut result = newer.clone();
    if section == "playlistUsageDeletions" {
        let tokens = normalize_sync_causal_tokens(
            &[usage_deletion_tokens(left), usage_deletion_tokens(right)].concat(),
        );
        result["deletionTokens"] = serde_json::json!(tokens);
        result["deletedAt"] = json_i64(left, "deletedAt")
            .max(json_i64(right, "deletedAt"))
            .max(0)
            .into();
        return result;
    }
    let (count, base, first, last) = if section == "playlistUsageStats" {
        (
            "openCount",
            "counterBaseOpenCount",
            "firstOpenedAt",
            "lastOpenedAt",
        )
    } else if section == "localPlaylistPlaybackStats" {
        (
            "totalPlayCount",
            "counterBasePlayCount",
            "firstPlayedAt",
            "lastPlayedAt",
        )
    } else {
        (
            "playCount",
            "counterBasePlayCount",
            "firstPlayedAt",
            "lastPlayedAt",
        )
    };
    let left_shards: Vec<SyncPlaybackCounterShard> =
        serde_json::from_value(left["counterShards"].clone()).unwrap_or_default();
    let right_shards: Vec<SyncPlaybackCounterShard> =
        serde_json::from_value(right["counterShards"].clone()).unwrap_or_default();
    let shards = merge_counter_shards(&left_shards, &right_shards);
    let shard_count = shards.iter().fold(0_i64, |total, shard| {
        total.saturating_add(i64::from(shard.play_count.max(0)))
    });
    let effective_base = |value: &serde_json::Value, empty: bool| {
        json_i64(value, base).max(0).max(if empty {
            json_i64(value, count).max(0)
        } else {
            json_i64(value, count).saturating_sub(shard_count).max(0)
        })
    };
    let base_count = if shards.is_empty() {
        0
    } else {
        effective_base(left, left_shards.is_empty())
            .max(effective_base(right, right_shards.is_empty()))
    };
    let total = json_i64(left, count)
        .max(json_i64(right, count))
        .max(base_count.saturating_add(shard_count))
        .max(0);
    result[count] = if section == "playlistUsageStats" {
        total.min(i64::from(i32::MAX))
    } else {
        total
    }
    .into();
    result[base] = base_count.into();
    result[first] = min_positive(json_i64(left, first), json_i64(right, first)).into();
    result[last] = json_i64(left, last).max(json_i64(right, last)).into();
    if section != "playlistUsageStats" {
        result[first] = shards
            .iter()
            .fold(json_i64(&result, first), |value, shard| {
                min_positive(value, shard.first_played_at)
            })
            .into();
        result[last] = shards
            .iter()
            .fold(json_i64(&result, last), |value, shard| {
                value.max(shard.last_played_at)
            })
            .into();
    }
    result["counterShards"] = serde_json::json!(shards);
    if section == "playlistUsageStats" {
        result["observedDeletionTokens"] = serde_json::json!(normalize_sync_causal_tokens(
            &[
                json_tokens(left, "observedDeletionTokens"),
                json_tokens(right, "observedDeletionTokens")
            ]
            .concat()
        ));
    }
    result
}
