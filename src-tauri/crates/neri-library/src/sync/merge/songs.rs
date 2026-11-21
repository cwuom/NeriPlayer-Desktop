//! 歌单内歌曲合并：身份匹配、去重、载荷选择与元数据补全

use super::*;

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
pub(super) fn merge_songs(
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

pub(super) fn deduplicate_songs(songs: &[SyncSong]) -> Vec<SyncSong> {
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
    pub(super) fn new(resolve_payload_deterministically: bool) -> Self {
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
                        .then_with(|| {
                            canonical_payload_key(left).cmp(&canonical_payload_key(right))
                        })
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
            let Some(absorbed) = self.slots[index].take() else {
                continue;
            };
            for alias in absorbed.aliases {
                if !aliases.iter().any(|known| same_song_payload(known, &alias)) {
                    aliases.push(alias);
                }
            }
        }
        if !aliases.iter().any(|known| same_song_payload(known, &other)) {
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
    fill_missing_option(&mut target.matched_translated_lyric, candidates, |song| {
        song.matched_translated_lyric.as_deref()
    });
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
    fill_missing_option(&mut target.original_translated_lyric, candidates, |song| {
        song.original_translated_lyric.as_deref()
    });
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

fn fill_missing_option<F>(target: &mut Option<String>, candidates: &[SyncSong], selector: F)
where
    F: for<'a> Fn(&'a SyncSong) -> Option<&'a str>,
{
    if target
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
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
    song.identity_keys()
        .iter()
        .any(|key| base_songs.contains(key))
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
