// 收藏的平台歌单与关注的创作者：SQLite 持久化（对齐 Android FavoritePlaylistRoomStore）
//
// 取消收藏以 is_deleted 墓碑保留，下一轮同步才不会被旧端的收藏重新恢复
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Transaction};

use crate::db::{self, UserDatabase};
use crate::error::{AppError, AppResult};
use crate::sync::models::{SyncFavoritePlaylist, SyncSong};

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.favorites";
const LEGACY_FILE: &str = "favorites.json";

/// 读取收藏；`include_deleted` 为 false 时隐藏墓碑（列表展示用）
pub fn load(include_deleted: bool) -> AppResult<Vec<SyncFavoritePlaylist>> {
    db::user_db()?.read(|connection| load_from(connection, include_deleted))
}

pub fn load_from(connection: &Connection, include_deleted: bool) -> AppResult<Vec<SyncFavoritePlaylist>> {
    let mut songs: HashMap<(String, String), Vec<SyncSong>> = HashMap::new();
    {
        let mut statement = connection.prepare(
            "SELECT playlist_id, source, song_payload_json FROM favorite_playlist_song
             ORDER BY playlist_id, source, display_position",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let song: SyncSong = serde_json::from_str(&row.get::<_, String>(2)?).map_err(|error| {
                AppError::Other(format!("Stored favorite song is invalid: {error}"))
            })?;
            songs
                .entry((row.get(0)?, row.get(1)?))
                .or_default()
                .push(song);
        }
    }
    let mut statement = connection.prepare(
        "SELECT playlist_id, source, name, cover_url, track_count, browse_id, remote_playlist_id,
             subtitle, added_time, sort_order, modified_at, is_deleted
         FROM favorite_playlist ORDER BY display_position",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(SyncFavoritePlaylist {
            id: row.get(0)?,
            source: row.get(1)?,
            name: row.get(2)?,
            cover_url: row.get(3)?,
            track_count: row.get(4)?,
            browse_id: row.get(5)?,
            playlist_id: row.get(6)?,
            subtitle: row.get(7)?,
            added_time: row.get(8)?,
            sort_order: row.get(9)?,
            modified_at: row.get(10)?,
            is_deleted: row.get(11)?,
            songs: Vec::new(),
        })
    })?;
    let mut favorites = Vec::new();
    for favorite in rows {
        let mut favorite = favorite?;
        if !include_deleted && favorite.is_deleted {
            continue;
        }
        favorite.songs = songs
            .remove(&(favorite.id.clone(), favorite.source.clone()))
            .unwrap_or_default();
        favorites.push(favorite.normalized_for_sync());
    }
    Ok(favorites)
}

/// 整体替换收藏（含墓碑）；同一 (id, source) 只保留最新修改的一份
pub fn save_into(transaction: &Transaction<'_>, favorites: &[SyncFavoritePlaylist]) -> AppResult<()> {
    let mut ordered: Vec<&SyncFavoritePlaylist> = Vec::with_capacity(favorites.len());
    let mut positions: HashMap<(&str, &str), usize> = HashMap::new();
    for favorite in favorites {
        let key = (favorite.id.as_str(), favorite.source.as_str());
        match positions.get(&key) {
            Some(&index) => {
                log::warn!(
                    target: "playlist-io",
                    "收藏存在重复项 {}_{}, 保留最近修改的一份",
                    favorite.id,
                    favorite.source,
                );
                if favorite.modified_at >= ordered[index].modified_at {
                    ordered[index] = favorite;
                }
            }
            None => {
                positions.insert(key, ordered.len());
                ordered.push(favorite);
            }
        }
    }

    transaction.execute("DELETE FROM favorite_playlist", [])?;
    let mut insert = transaction.prepare_cached(
        "INSERT INTO favorite_playlist (playlist_id, source, display_position, name, cover_url,
             track_count, browse_id, remote_playlist_id, subtitle, added_time, sort_order,
             modified_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?;
    let mut insert_song = transaction.prepare_cached(
        "INSERT INTO favorite_playlist_song (playlist_id, source, display_position, song_payload_json)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (position, favorite) in ordered.into_iter().enumerate() {
        insert.execute(params![
            favorite.id,
            favorite.source,
            position as i64,
            favorite.name,
            favorite.cover_url,
            favorite.track_count,
            favorite.browse_id,
            favorite.playlist_id,
            favorite.subtitle,
            favorite.added_time,
            favorite.sort_order,
            favorite.modified_at,
            favorite.is_deleted,
        ])?;
        for (song_position, song) in favorite.songs.iter().enumerate() {
            insert_song.execute(params![
                favorite.id,
                favorite.source,
                song_position as i64,
                serde_json::to_string(song)?,
            ])?;
        }
    }
    Ok(())
}

/// 读改写收藏，期间持有歌单锁；闭包失败时整体回滚
pub fn update<T>(mutation: impl FnOnce(&mut Vec<SyncFavoritePlaylist>) -> AppResult<T>) -> AppResult<T> {
    update_with(db::user_db()?, mutation)
}

fn update_with<T>(
    database: &UserDatabase,
    mutation: impl FnOnce(&mut Vec<SyncFavoritePlaylist>) -> AppResult<T>,
) -> AppResult<T> {
    let _guard = super::playlist::lock_io();
    let result = database.write(|transaction| {
        let mut favorites = load_from(transaction, true)?;
        let result = mutation(&mut favorites)?;
        save_into(transaction, &favorites)?;
        Ok(result)
    })?;
    // 收藏也是同步快照的一部分，网络窗口中的修改必须让旧快照失效
    super::playlist::mark_io_changed();
    Ok(result)
}

/// 旧版 favorites.json 一次性导入
pub(crate) fn import_legacy_json(transaction: &Transaction<'_>, directory: &Path) -> AppResult<Vec<PathBuf>> {
    let path = directory.join(LEGACY_FILE);
    let Some(favorites) = db::legacy::read_json::<Vec<SyncFavoritePlaylist>>(&path)? else {
        return Ok(Vec::new());
    };
    let favorites: Vec<_> = favorites.iter().map(SyncFavoritePlaylist::normalized_for_sync).collect();
    save_into(transaction, &favorites)?;
    log::info!(target: "playlist-io", "旧版收藏已导入数据库: count={}", favorites.len());
    Ok(vec![path])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::merge;
    use crate::sync::models::SyncData;

    fn favorite(value: serde_json::Value) -> SyncFavoritePlaylist {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn favorite_updates_preserve_tombstones_and_invalidate_sync_snapshots() {
        let database = UserDatabase::open_in_memory().unwrap();
        let deleted = favorite(serde_json::json!({"id":42,"name":"deleted artist","source":"neteaseArtist","modifiedAt":20,"isDeleted":true}));
        database.write(|transaction| save_into(transaction, &[deleted])).unwrap();
        let epoch = super::super::playlist::io_epoch();
        update_with(&database, |favorites| {
            assert!(favorites[0].is_deleted);
            favorites.push(favorite(serde_json::json!({"id":43,"name":"new artist","source":"biliArtist","modifiedAt":21})));
            Ok(())
        })
        .unwrap();
        assert!(super::super::playlist::io_epoch() > epoch);
        assert_eq!(database.read(|connection| load_from(connection, true)).unwrap().len(), 2);
        assert_eq!(database.read(|connection| load_from(connection, false)).unwrap().len(), 1);

        let failed = update_with(&database, |favorites| {
            favorites.clear();
            Err::<(), _>(AppError::Other("fixture error".into()))
        });
        assert!(failed.is_err());
        assert_eq!(database.read(|connection| load_from(connection, true)).unwrap().len(), 2);
    }

    #[test]
    fn favorite_tombstone_survives_two_sync_rounds_and_is_hidden_from_listing() {
        let database = UserDatabase::open_in_memory().unwrap();
        let active = favorite(serde_json::json!({"id":42,"name":"phone favorite","source":"netease","modifiedAt":10}));
        let mut deleted = active.clone();
        deleted.is_deleted = true;
        deleted.modified_at = 20;
        database.write(|transaction| save_into(transaction, std::slice::from_ref(&deleted))).unwrap();
        assert!(database.read(|connection| load_from(connection, false)).unwrap().is_empty());

        let local = SyncData {
            favorite_playlists: database.read(|connection| load_from(connection, true)).unwrap(),
            ..Default::default()
        };
        let remote = SyncData { favorite_playlists: vec![active], ..Default::default() };
        let next = merge::three_way_merge(&local, &remote, 0, &HashMap::new());
        assert!(next.favorite_playlists[0].is_deleted);
        database.write(|transaction| save_into(transaction, &next.favorite_playlists)).unwrap();
        assert!(database.read(|connection| load_from(connection, true)).unwrap()[0].is_deleted);
        assert!(database.read(|connection| load_from(connection, false)).unwrap().is_empty());
    }

    #[test]
    fn favorites_round_trip_songs_and_optional_fields() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut stored = favorite(serde_json::json!({
            "id": 7, "name": "Fav", "source": "netease", "coverUrl": "https://a", "trackCount": 1,
            "addedTime": 1, "modifiedAt": 2, "sortOrder": 3, "browseId": "B", "playlistId": "P",
            "subtitle": "sub"
        }));
        stored.songs = vec![SyncSong { id: "42".into(), name: "Song".into(), ..Default::default() }];
        database.write(|transaction| save_into(transaction, std::slice::from_ref(&stored))).unwrap();

        let restored = database.read(|connection| load_from(connection, true)).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(vec![stored.normalized_for_sync()]).unwrap()
        );
    }

    #[test]
    fn duplicate_favorites_keep_the_latest_modification() {
        let database = UserDatabase::open_in_memory().unwrap();
        let older = favorite(serde_json::json!({"id":1,"name":"old","source":"netease","modifiedAt":5}));
        let newer = favorite(serde_json::json!({"id":1,"name":"new","source":"netease","modifiedAt":9}));
        database.write(|transaction| save_into(transaction, &[newer, older])).unwrap();
        let restored = database.read(|connection| load_from(connection, true)).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].name, "new");
    }

    #[test]
    fn legacy_favorites_json_is_imported() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(LEGACY_FILE),
            br#"[{"id":42,"name":"Artist","source":"neteaseArtist","modifiedAt":20,"isDeleted":true},
                 {"id":7,"name":"List","source":"netease","modifiedAt":3}]"#,
        )
        .unwrap();
        let database = UserDatabase::open_in_memory().unwrap();
        db::legacy::run_once(&database, directory.path(), LEGACY_IMPORT_KEY, import_legacy_json).unwrap();

        let all = database.read(|connection| load_from(connection, true)).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].is_deleted);
        assert_eq!(database.read(|connection| load_from(connection, false)).unwrap().len(), 1);
    }
}
