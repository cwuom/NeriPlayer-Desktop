// 播放列表管理（JSON 持久化）
use crate::error::{AppError, AppResult};
use crate::state::TrackInfo;
use crate::sync::models::{normalize_sync_causal_tokens, SyncPlaylistSongDeletion, SyncSong};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

const MAX_SAFE_PLAYLIST_ID: i64 = (1_i64 << 53) - 1;
static PLAYLIST_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn acquire_playlist_io_lock() -> AppResult<MutexGuard<'static, ()>> {
    PLAYLIST_IO_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Playlist storage lock poisoned".into()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub tracks: Vec<TrackInfo>,
    pub modified_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistStore {
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub deleted_playlist_ids: Vec<i64>,
    #[serde(default)]
    pub playlist_song_deletions: Vec<SyncPlaylistSongDeletion>,
    next_id: i64,
}

impl PlaylistStore {
    pub fn load(path: &PathBuf) -> Self {
        Self::load_strict(path).unwrap_or_default()
    }

    pub fn load_strict(path: &PathBuf) -> AppResult<Self> {
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(serde_json::from_str(&content)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, path: &PathBuf) -> AppResult<()> {
        let json = serde_json::to_string_pretty(self)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;

        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(json.as_bytes())?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn create(&mut self, name: String) -> &Playlist {
        // 53 位随机 ID 可被 JavaScript 精确表示，并避免多台设备从同一序号开始
        let id = loop {
            let bytes = uuid::Uuid::new_v4().into_bytes();
            let candidate = (u64::from_be_bytes(bytes[..8].try_into().expect("UUID prefix"))
                & MAX_SAFE_PLAYLIST_ID as u64) as i64;
            if candidate > 0
                && self
                    .playlists
                    .iter()
                    .all(|playlist| playlist.id != candidate)
            {
                break candidate;
            }
        };
        self.next_id = self.next_id.max(id.saturating_add(1));
        self.playlists.push(Playlist {
            id,
            name,
            tracks: Vec::new(),
            modified_at: chrono::Utc::now().timestamp_millis() as u64,
        });
        self.deleted_playlist_ids
            .retain(|deleted_id| *deleted_id != id);
        self.playlists.last().unwrap()
    }

    pub fn delete(&mut self, id: i64) -> bool {
        let len = self.playlists.len();
        self.playlists.retain(|p| p.id != id);
        let deleted = self.playlists.len() < len;
        if deleted && !self.deleted_playlist_ids.contains(&id) {
            self.deleted_playlist_ids.push(id);
        }
        deleted
    }

    pub fn record_playlist_song_deletion(&mut self, deletion: SyncPlaylistSongDeletion) {
        let mut deletion = deletion;
        deletion.removed_membership_tokens =
            normalize_sync_causal_tokens(&deletion.removed_membership_tokens);
        let identity = deletion.identity();

        if deletion.removed_membership_tokens.is_empty() {
            if let Some(existing) = self.playlist_song_deletions.iter_mut().find(|existing| {
                existing.identity() == identity && existing.removed_membership_tokens.is_empty()
            }) {
                if deletion_snapshot_cmp(&deletion, existing).is_ge() {
                    *existing = deletion;
                }
            } else {
                self.playlist_song_deletions.push(deletion);
            }
            return;
        }

        let mut causal_snapshots: Vec<SyncPlaylistSongDeletion> = self
            .playlist_song_deletions
            .iter()
            .filter(|existing| {
                existing.identity() == identity && !existing.removed_membership_tokens.is_empty()
            })
            .cloned()
            .collect();
        causal_snapshots.push(deletion);
        let mut merged = causal_snapshots
            .iter()
            .max_by(|left, right| deletion_snapshot_cmp(left, right))
            .cloned()
            .expect("causal deletion snapshot must exist");
        merged.removed_membership_tokens = normalize_sync_causal_tokens(
            &causal_snapshots
                .iter()
                .flat_map(|snapshot| snapshot.removed_membership_tokens.iter().cloned())
                .collect::<Vec<_>>(),
        );
        self.playlist_song_deletions.retain(|existing| {
            existing.identity() != identity || existing.removed_membership_tokens.is_empty()
        });
        self.playlist_song_deletions.push(merged);
    }

    pub fn clear_playlist_song_deletion(&mut self, playlist_id: &str, song: &SyncSong) {
        self.playlist_song_deletions.retain(|deletion| {
            !deletion.matches_song(playlist_id, song)
                || !deletion.removed_membership_tokens.is_empty()
        });
    }

    /// 确保 next_id 大于所有正数歌单 ID
    pub fn fix_next_id(&mut self) {
        let max = self
            .playlists
            .iter()
            .map(|p| p.id)
            .filter(|&id| id > 0)
            .max()
            .unwrap_or(0);
        if self.next_id <= max {
            self.next_id = max + 1;
        }
        if self.next_id < 1 {
            self.next_id = 1;
        }
    }
}

fn deletion_snapshot_cmp(
    left: &SyncPlaylistSongDeletion,
    right: &SyncPlaylistSongDeletion,
) -> std::cmp::Ordering {
    left.deleted_at
        .cmp(&right.deleted_at)
        .then_with(|| left.device_id.cmp(&right.device_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::models::SyncCausalToken;

    #[test]
    fn new_playlist_ids_are_cross_device_safe_and_javascript_exact() {
        let mut store = PlaylistStore::default();
        let first = store.create("First".into()).id;
        let second = store.create("Second".into()).id;

        assert!(first > 0);
        assert!(first <= MAX_SAFE_PLAYLIST_ID);
        assert!(second > 0);
        assert!(second <= MAX_SAFE_PLAYLIST_ID);
        assert_ne!(second, first);
    }

    #[test]
    fn strict_load_distinguishes_missing_and_malformed_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("playlists.json");

        assert!(PlaylistStore::load_strict(&path)
            .unwrap()
            .playlists
            .is_empty());

        std::fs::write(&path, "{malformed").unwrap();
        assert!(PlaylistStore::load_strict(&path).is_err());
        assert!(PlaylistStore::load(&path).playlists.is_empty());
    }

    #[test]
    fn save_atomically_publishes_a_strictly_loadable_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("playlists.json");
        let mut store = PlaylistStore::default();
        store.create("Saved".into());

        store.save(&path).unwrap();

        let loaded = PlaylistStore::load_strict(&path).unwrap();
        assert_eq!(loaded.playlists.len(), 1);
        assert_eq!(loaded.playlists[0].name, "Saved");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn causal_deletion_snapshots_union_all_observed_tokens() {
        let mut store = PlaylistStore::default();
        store.record_playlist_song_deletion(deletion(100, "a", vec![token("phone", 1)]));
        store.record_playlist_song_deletion(deletion(200, "b", vec![token("desktop", 1)]));

        let merged = &store.playlist_song_deletions[0];
        assert_eq!(merged.deleted_at, 200);
        assert_eq!(merged.device_id, "b");
        assert_eq!(
            merged.removed_membership_tokens,
            vec![token("desktop", 1), token("phone", 1)]
        );
    }

    #[test]
    fn readd_clears_only_legacy_deletion_snapshot() {
        let mut store = PlaylistStore::default();
        store.record_playlist_song_deletion(deletion(100, "legacy", Vec::new()));
        store.record_playlist_song_deletion(deletion(200, "phone", vec![token("phone", 1)]));
        let song = SyncSong {
            id: "42".into(),
            album: "netease".into(),
            ..Default::default()
        };

        store.clear_playlist_song_deletion("1", &song);

        assert_eq!(store.playlist_song_deletions.len(), 1);
        assert_eq!(
            store.playlist_song_deletions[0].removed_membership_tokens,
            vec![token("phone", 1)]
        );
    }

    fn deletion(
        deleted_at: i64,
        device_id: &str,
        removed_membership_tokens: Vec<SyncCausalToken>,
    ) -> SyncPlaylistSongDeletion {
        SyncPlaylistSongDeletion {
            playlist_id: "1".into(),
            song_id: "42".into(),
            album: "netease".into(),
            deleted_at,
            device_id: device_id.into(),
            removed_membership_tokens,
            ..Default::default()
        }
    }

    fn token(device_id: &str, counter: i64) -> SyncCausalToken {
        SyncCausalToken {
            device_id: device_id.into(),
            counter,
        }
    }
}
