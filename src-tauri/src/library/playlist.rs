// 播放列表管理（JSON 持久化）
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{atomic::{AtomicU64, Ordering}, Mutex, MutexGuard, OnceLock};
use crate::state::TrackInfo;
use crate::error::{AppError, AppResult};
use crate::sync::models::{
    normalize_sync_causal_tokens,
    SyncPlaylistSongDeletion,
    SyncSong,
};

const MAX_SAFE_PLAYLIST_ID: i64 = (1_i64 << 53) - 1;

static PLAYLIST_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PLAYLIST_EPOCH: AtomicU64 = AtomicU64::new(0);

/// 串行化歌单读改写与同步应用，避免并发命令互相覆盖
pub(crate) fn lock_io() -> MutexGuard<'static, ()> {
    PLAYLIST_IO_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 返回进程内歌单写入版本，用于检测同步网络窗口中的本地编辑
pub(crate) fn io_epoch() -> u64 {
    PLAYLIST_EPOCH.load(Ordering::Acquire)
}

pub(crate) fn mark_io_changed() {
    PLAYLIST_EPOCH.fetch_add(1, Ordering::AcqRel);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub tracks: Vec<TrackInfo>,
    pub modified_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PlaylistStore {
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub deleted_playlist_ids: Vec<i64>,
    #[serde(default)]
    pub playlist_song_deletions: Vec<SyncPlaylistSongDeletion>,
    next_id: i64,
}

impl PlaylistStore {
    /// 严格加载：文件损坏时隔离现场并返回错误，绝不返回空库
    ///
    /// 旧实现解析失败静默返回空库，后续任何 save 都会用空库覆盖原文件
    /// （断电/磁盘满即歌单全灭）。现在解析失败先把原文件移到
    /// `.corrupt-<ts>` 保留现场，再向上层报错——上层命令报错给前端，
    /// 不得在失败态上继续 save
    pub fn load_strict(path: &PathBuf) -> AppResult<Self> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(AppError::Other(format!(
                    "read playlists store failed: {error}"
                )));
            }
        };
        match serde_json::from_str(&content) {
            Ok(store) => Ok(store),
            Err(error) => {
                let quarantined = crate::fsutil::quarantine_corrupt_file(path);
                log::error!(
                    target: "playlist-io",
                    "playlists.json 解析失败, 现场已隔离到 {:?}: {}",
                    quarantined,
                    error,
                );
                Err(AppError::Other(format!(
                    "playlists store is corrupt (original preserved as {:?}): {error}",
                    quarantined,
                )))
            }
        }
    }

    pub fn save(&self, path: &PathBuf) -> AppResult<()> {
        let _guard = lock_io();
        self.save_locked(path)
    }

    /// 在已持有 `lock_io` 时写入，避免同步应用检查 epoch 后再次获取同一把锁
    pub(crate) fn save_locked(&self, path: &PathBuf) -> AppResult<()> {
        let json = serde_json::to_string_pretty(self)?;
        // 原子写：temp + fsync + rename，杜绝半截 JSON
        crate::fsutil::atomic_write(path, json)?;
        mark_io_changed();
        Ok(())
    }

    pub fn create(&mut self, name: String) -> &Playlist {
        // 53 位随机 ID 可被 JavaScript 精确表示，并避免多台设备从同一序号开始
        let id = loop {
            let bytes = uuid::Uuid::new_v4().into_bytes();
            let candidate = (u64::from_be_bytes(bytes[..8].try_into().expect("UUID prefix"))
                & MAX_SAFE_PLAYLIST_ID as u64) as i64;
            if candidate > 0 && self.playlists.iter().all(|playlist| playlist.id != candidate) {
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
        self.deleted_playlist_ids.retain(|deleted_id| *deleted_id != id);
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
        deletion.removed_membership_tokens = normalize_sync_causal_tokens(
            &deletion.removed_membership_tokens,
        );
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
                existing.identity() == identity
                    && !existing.removed_membership_tokens.is_empty()
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
        let max = self.playlists.iter().map(|p| p.id).filter(|&id| id > 0).max().unwrap_or(0);
        if self.next_id <= max {
            self.next_id = max + 1;
        }
        if self.next_id < 1 { self.next_id = 1; }
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

    /// 损坏的 playlists.json 必须报错并隔离现场，绝不返回空库（E1 回归）
    #[test]
    fn corrupt_store_is_quarantined_and_never_silently_emptied() {
        let dir = std::env::temp_dir().join(format!("neri-playlist-e1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("playlists.json");
        std::fs::write(&path, b"{ this is not json").unwrap();

        let result = PlaylistStore::load_strict(&path);
        assert!(result.is_err(), "corrupt store must surface an error");
        // 现场已被移走保留，原路径不再存在
        assert!(!path.exists(), "corrupt file must be moved aside");
        let quarantined = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(quarantined, "a .corrupt-* file must be preserved");

        // 隔离后视为全新库，可正常创建
        let store = PlaylistStore::load_strict(&path).unwrap();
        assert!(store.playlists.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn causal_deletion_snapshots_union_all_observed_tokens() {
        let mut store = PlaylistStore::default();
        store.record_playlist_song_deletion(deletion(
            100,
            "a",
            vec![token("phone", 1)],
        ));
        store.record_playlist_song_deletion(deletion(
            200,
            "b",
            vec![token("desktop", 1)],
        ));

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
        store.record_playlist_song_deletion(deletion(
            200,
            "phone",
            vec![token("phone", 1)],
        ));
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

    #[test]
    fn save_advances_io_epoch() {
        let before = io_epoch();
        let dir = std::env::temp_dir().join(format!("neri-playlist-epoch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("playlists.json");
        PlaylistStore::default().save(&path).unwrap();
        assert!(io_epoch() > before);
        let _ = std::fs::remove_dir_all(dir);
    }
}
