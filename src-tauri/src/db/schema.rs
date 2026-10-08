//! 用户数据库 schema 与版本迁移
//!
//! 表名与列名尽量沿用 Android Room（NeriUserDataDatabase v20）的命名，便于两端
//! 对照；桌面端需要无损往返的完整载荷额外放在 `*_payload_json` 列中

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

pub const SCHEMA_VERSION: i32 = 3;

const MIGRATIONS: &[&str] = &[V1, V2, V3];

const V1: &str = r#"
CREATE TABLE migration_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE local_playlist (
    playlist_id INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    display_position INTEGER NOT NULL,
    custom_cover_url TEXT,
    modified_at INTEGER NOT NULL,
    song_order_version INTEGER NOT NULL DEFAULT 0,
    is_system INTEGER NOT NULL DEFAULT 0,
    track_count INTEGER NOT NULL DEFAULT 0,
    cover_url TEXT,
    members_hash INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX index_local_playlist_display_position ON local_playlist(display_position);
CREATE INDEX index_local_playlist_modified_at ON local_playlist(modified_at);

CREATE TABLE playlist_member (
    playlist_id INTEGER NOT NULL REFERENCES local_playlist(playlist_id) ON DELETE CASCADE,
    display_position INTEGER NOT NULL,
    identity_key TEXT NOT NULL,
    track_id TEXT NOT NULL,
    source TEXT NOT NULL,
    name TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    cover_url TEXT,
    media_uri TEXT,
    added_at INTEGER NOT NULL,
    member_payload_json TEXT NOT NULL,
    PRIMARY KEY (playlist_id, display_position)
);
CREATE INDEX index_playlist_member_identity_key ON playlist_member(identity_key);
CREATE INDEX index_playlist_member_source ON playlist_member(source);

CREATE TABLE local_playlist_deletion (
    playlist_id INTEGER PRIMARY KEY NOT NULL,
    deleted_at INTEGER NOT NULL
);

CREATE TABLE playlist_song_deletion (
    position INTEGER PRIMARY KEY NOT NULL,
    playlist_id TEXT NOT NULL,
    song_identity_key TEXT NOT NULL,
    deleted_at INTEGER NOT NULL,
    device_id TEXT NOT NULL,
    deletion_payload_json TEXT NOT NULL
);
CREATE INDEX index_playlist_song_deletion_song
    ON playlist_song_deletion(playlist_id, song_identity_key);

CREATE TABLE favorite_playlist (
    playlist_id TEXT NOT NULL,
    source TEXT NOT NULL,
    display_position INTEGER NOT NULL,
    name TEXT NOT NULL,
    cover_url TEXT NOT NULL,
    track_count INTEGER NOT NULL,
    browse_id TEXT,
    remote_playlist_id TEXT,
    subtitle TEXT,
    added_time INTEGER NOT NULL,
    sort_order INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    is_deleted INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, source)
);
CREATE INDEX index_favorite_playlist_sort ON favorite_playlist(sort_order, modified_at);
CREATE INDEX index_favorite_playlist_visibility ON favorite_playlist(is_deleted, sort_order);

CREATE TABLE favorite_playlist_song (
    playlist_id TEXT NOT NULL,
    source TEXT NOT NULL,
    display_position INTEGER NOT NULL,
    song_payload_json TEXT NOT NULL,
    PRIMARY KEY (playlist_id, source, display_position),
    FOREIGN KEY (playlist_id, source)
        REFERENCES favorite_playlist(playlist_id, source) ON DELETE CASCADE
);

CREATE TABLE play_history (
    track_id TEXT PRIMARY KEY NOT NULL,
    played_at INTEGER NOT NULL,
    name TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    cover_url TEXT,
    source TEXT,
    track_payload_json TEXT NOT NULL
);
CREATE INDEX index_play_history_played_at ON play_history(played_at);

CREATE TABLE play_history_deletion (
    track_id TEXT PRIMARY KEY NOT NULL,
    deleted_at INTEGER NOT NULL,
    track_payload_json TEXT NOT NULL
);
CREATE INDEX index_play_history_deletion_deleted_at ON play_history_deletion(deleted_at);

CREATE TABLE playback_queue_state (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 0),
    current_index INTEGER NOT NULL,
    current_track_id TEXT,
    current_track_playlist_key TEXT,
    has_playback_session INTEGER NOT NULL,
    position_ms INTEGER,
    repeat_mode TEXT,
    shuffle_enabled INTEGER,
    volume REAL,
    queue_hash INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE playback_queue_song (
    position INTEGER PRIMARY KEY NOT NULL,
    track_id TEXT NOT NULL,
    track_payload_json TEXT NOT NULL
);

CREATE TABLE lyric_offset (
    track_key TEXT PRIMARY KEY NOT NULL,
    offset_ms INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE playback_stat (
    identity_key TEXT PRIMARY KEY NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    album_id TEXT NOT NULL,
    cover_url TEXT,
    duration_ms INTEGER NOT NULL,
    total_listen_ms INTEGER NOT NULL,
    play_count INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL,
    first_played_at INTEGER NOT NULL,
    media_uri TEXT,
    counter_base_listen_ms INTEGER NOT NULL,
    counter_base_play_count INTEGER NOT NULL,
    provenance_shards_json TEXT NOT NULL,
    row_hash INTEGER NOT NULL
);
CREATE INDEX index_playback_stat_last_played ON playback_stat(last_played_at);
CREATE INDEX index_playback_stat_play_count ON playback_stat(play_count, identity_key);
CREATE INDEX index_playback_stat_listen_time ON playback_stat(total_listen_ms, identity_key);

CREATE TABLE playback_stat_counter_shard (
    identity_key TEXT NOT NULL REFERENCES playback_stat(identity_key) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    epoch_started_at INTEGER NOT NULL,
    total_listen_ms INTEGER NOT NULL,
    play_count INTEGER NOT NULL,
    first_played_at INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL,
    PRIMARY KEY (identity_key, device_id, epoch_started_at)
);

CREATE TABLE playback_stat_bucket (
    day_start_at INTEGER NOT NULL,
    identity_key TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    album_id TEXT NOT NULL,
    cover_url TEXT,
    duration_ms INTEGER NOT NULL,
    total_listen_ms INTEGER NOT NULL,
    play_count INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL,
    first_played_at INTEGER NOT NULL,
    media_uri TEXT,
    counter_base_listen_ms INTEGER NOT NULL,
    counter_base_play_count INTEGER NOT NULL,
    provenance_shards_json TEXT NOT NULL,
    row_hash INTEGER NOT NULL,
    PRIMARY KEY (day_start_at, identity_key)
);
CREATE INDEX index_playback_stat_bucket_identity_day
    ON playback_stat_bucket(identity_key, day_start_at);

CREATE TABLE playback_stat_daily_counter_shard (
    day_start_at INTEGER NOT NULL,
    identity_key TEXT NOT NULL,
    device_id TEXT NOT NULL,
    epoch_started_at INTEGER NOT NULL,
    total_listen_ms INTEGER NOT NULL,
    play_count INTEGER NOT NULL,
    first_played_at INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL,
    PRIMARY KEY (day_start_at, identity_key, device_id, epoch_started_at),
    FOREIGN KEY (day_start_at, identity_key)
        REFERENCES playback_stat_bucket(day_start_at, identity_key) ON DELETE CASCADE
);

CREATE TABLE sync_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE sync_base_snapshot (
    scope TEXT NOT NULL,
    playlist_id TEXT NOT NULL,
    song_key TEXT NOT NULL,
    PRIMARY KEY (scope, playlist_id, song_key)
);

CREATE TABLE download_catalog (
    catalog_position INTEGER PRIMARY KEY NOT NULL,
    track_id TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    cover_url TEXT,
    source TEXT NOT NULL,
    file_path TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    downloaded_at INTEGER NOT NULL
);
CREATE INDEX index_download_catalog_track_id ON download_catalog(track_id);

CREATE TABLE cache_entry (
    bucket TEXT NOT NULL,
    cache_key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    payload_bytes INTEGER NOT NULL,
    saved_at_ms INTEGER NOT NULL,
    PRIMARY KEY (bucket, cache_key)
);
CREATE INDEX index_cache_entry_saved_at ON cache_entry(bucket, saved_at_ms);
"#;

// 播放历史改按曲目身份去重（对齐 Android identity_key），旧键由 play_history 的一次性迁移重算
const V2: &str = r#"
ALTER TABLE play_history RENAME COLUMN track_id TO identity_key;
ALTER TABLE play_history_deletion RENAME COLUMN track_id TO identity_key;
"#;

// 长音频续播位置（对齐 Android resume_position_ms）；NULL 表示本机还不知道，同步时沿用存档里的值
const V3: &str = r#"
ALTER TABLE play_history ADD COLUMN resume_position_ms INTEGER;
"#;

pub fn migrate(connection: &Connection) -> AppResult<()> {
    let current: i32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(AppError::Other(format!(
            "User database schema v{current} was created by a newer NeriPlayer build (supported: v{SCHEMA_VERSION})"
        )));
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        let target = index as i32 + 1;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", target)?;
        transaction.commit()?;
        log::info!(target: "database", "用户数据库 schema 已迁移到 v{target}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        migrate(&connection).unwrap();
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'local_playlist'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1);
    }

    #[test]
    fn schema_version_matches_migration_count() {
        assert_eq!(SCHEMA_VERSION as usize, MIGRATIONS.len());
    }
}
