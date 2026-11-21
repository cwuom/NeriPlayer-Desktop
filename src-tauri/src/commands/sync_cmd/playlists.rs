//! 歌单 JSON 导入导出（兼容 Android BackupData）

use super::*;

/// 导出播放列表为 JSON（Android BackupData 兼容格式）
#[tauri::command]
pub async fn export_playlists(app: AppHandle) -> AppResult<Value> {
    use neri_library::library::playlist::PlaylistStore;
    let store = PlaylistStore::load()?;

    // 转换为 SyncPlaylist 格式（Android 兼容）
    let sync_playlists: Vec<neri_model::sync::models::SyncPlaylist> = store
        .playlists
        .iter()
        .map(|pl| neri_model::sync::models::SyncPlaylist {
            id: pl.id.to_string(),
            name: pl.name.clone(),
            songs: neri_library::sync::manager::tracks_to_sync_songs_pub(&pl.tracks),
            created_at: pl.modified_at as i64,
            modified_at: pl.modified_at as i64,
            is_deleted: false,
            song_order_version: 1,
        })
        .collect();

    let backup_data = serde_json::json!({
        "version": "2.0",
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "exportDate": chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
        "playlists": sync_playlists,
    });

    let json_data = serde_json::to_string_pretty(&backup_data)
        .map_err(|e| AppError::Other(format!("Serialize failed: {}", e)))?;

    let Some(path) = pick_file_path(&app, Some("neriplayer-playlists.json".into())).await? else {
        return Ok(dialog_cancelled());
    };
    std::fs::write(&path, &json_data)
        .map_err(|e| AppError::Other(format!("Write failed: {}", e)))?;
    Ok(serde_json::json!({ "success": true, "count": store.playlists.len() }))
}

/// 弹出文件对话框，且不占住 async 运行时
///
/// `blocking_save_file` / `blocking_pick_file` 会一直阻塞到用户选完，
/// 可能是几分钟。直接在 `#[tauri::command] async fn` 里调用等于霸占一个
/// tokio worker，插件文档也明确警告这么用会死锁。挪到阻塞线程池里执行。
///
/// 返回的 `FilePath` 可能是 `Url`（`file://`、Android `content://`），
/// `as_path()` 对它返回 `None`——之前那里直接 unwrap，用户从虚拟位置选文件
/// 就会 panic。`into_path()` 能把 `file://` 正确还原成路径。
pub(super) async fn pick_file_path(
    app: &AppHandle,
    save_as: Option<String>,
) -> AppResult<Option<std::path::PathBuf>> {
    use tauri_plugin_dialog::DialogExt;

    let dialog = app.dialog().clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let builder = dialog.file().add_filter("JSON", &["json"]);
        match save_as {
            Some(name) => builder.set_file_name(name).blocking_save_file(),
            None => builder.blocking_pick_file(),
        }
    })
    .await
    .map_err(|error| AppError::Other(format!("File dialog failed: {error}")))?;

    let Some(picked) = picked else {
        return Ok(None);
    };
    picked
        .into_path()
        .map(Some)
        .map_err(|error| AppError::Other(format!("Unsupported file location: {error}")))
}

/// 取消选择时统一的返回体，避免各处自己拼 JSON 拼歪
pub(super) fn dialog_cancelled() -> Value {
    serde_json::json!({ "success": false, "reason": "cancelled" })
}

/// 合并导入歌单到本地快照（修复 SY-1）
///
/// 旧实现把导入的歌单当作全量集合直接喂给 save_synced_playlists，而后者会以
/// merged.playlists 全量替换 store.playlists、以 merged.favorite_playlists 覆盖
/// favorites 文件、以 merged.playlist_song_deletions 覆盖删除墓碑——传入的
/// SyncData 除 playlists 外全为空，于是本地独有歌单 / 收藏 / 删除墓碑被一并清空。
///
/// 正确语义：以本地完整快照为基底，导入项按身份（sync id / 名称）覆盖匹配项，
/// 本地独有歌单必须保留；导入不传播删除墓碑，避免误删本地歌单。
fn build_import_merged_playlists(
    app: &AppHandle,
    imported: Vec<neri_model::sync::models::SyncPlaylist>,
) -> AppResult<neri_model::sync::models::SyncData> {
    let mut merged = neri_library::sync::manager::build_local_sync_data(
        &crate::sync_state::TauriSyncState(app),
        None,
        None,
        None,
    )?;
    for imp in imported {
        if imp.is_deleted {
            continue;
        }
        let pos = merged
            .playlists
            .iter()
            .position(|p| !p.is_deleted && (p.id == imp.id || p.name == imp.name));
        match pos {
            Some(i) => merged.playlists[i] = imp,
            None => merged.playlists.push(imp),
        }
    }
    Ok(merged)
}

/// 合并桌面格式导出的歌单：同名歌单保留本地版本，新歌单 ID 与本地冲突时换发新 ID
fn merge_desktop_playlists(
    store: &mut neri_library::library::playlist::PlaylistStore,
    imported: Vec<neri_library::library::playlist::Playlist>,
) -> bool {
    let mut changed = false;
    for mut playlist in imported {
        if store
            .playlists
            .iter()
            .any(|existing| existing.name == playlist.name)
        {
            continue;
        }
        if store
            .playlists
            .iter()
            .any(|existing| existing.id == playlist.id)
        {
            playlist.id = store.allocate_id();
        }
        store.deleted_playlist_ids.retain(|id| *id != playlist.id);
        store.playlists.push(playlist);
        changed = true;
    }
    if changed {
        store.fix_next_id();
    }
    changed
}

/// 导入播放列表 JSON（兼容 Android BackupData 和 Desktop 两种格式）
#[tauri::command]
pub async fn import_playlists(app: AppHandle) -> AppResult<Value> {
    use neri_library::library::playlist::{Playlist, PlaylistStore};
    use neri_library::sync::manager::save_synced_playlists;
    use neri_model::sync::models::SyncPlaylist;
    let Some(path) = pick_file_path(&app, None).await? else {
        return Ok(dialog_cancelled());
    };
    {
        let data = std::fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("Read failed: {}", e)))?;

        let parsed: serde_json::Value = serde_json::from_str(&data)
            .map_err(|e| AppError::Other(format!("Parse failed: {}", e)))?;

        // 检测格式：Android BackupData 有 "playlists" 顶层数组（每项有 "songs"）
        //           Desktop 格式是直接的 Playlist 数组（每项有 "tracks"）
        let count;

        if parsed.is_object() && parsed.get("playlists").is_some() {
            // Android BackupData 格式：{ version, playlists: [SyncPlaylist] }
            let sync_playlists: Vec<SyncPlaylist> =
                serde_json::from_value(parsed["playlists"].clone())
                    .map_err(|e| AppError::Other(format!("Parse sync playlists: {}", e)))?;
            count = sync_playlists.len();

            // 并入本地快照后回写：保留本地独有歌单 / 收藏 / 删除墓碑（修复 SY-1）
            let sync_data = build_import_merged_playlists(&app, sync_playlists)?;
            save_synced_playlists(&sync_data)?;
        } else if parsed.is_array() {
            // 尝试 Desktop 格式
            if let Ok(imported) = serde_json::from_value::<Vec<Playlist>>(parsed.clone()) {
                count = imported.len();
                PlaylistStore::update(|store| {
                    let changed = merge_desktop_playlists(store, imported);
                    Ok(((), changed))
                })?;
            } else {
                // 可能是 SyncPlaylist 数组（无外层包装）
                let sync_playlists: Vec<SyncPlaylist> = serde_json::from_value(parsed)
                    .map_err(|e| AppError::Other(format!("Parse playlists array: {}", e)))?;
                count = sync_playlists.len();
                // 并入本地快照后回写（修复 SY-1）
                let sync_data = build_import_merged_playlists(&app, sync_playlists)?;
                save_synced_playlists(&sync_data)?;
            }
        } else {
            return Err(AppError::Other("Unrecognized playlist format".into()));
        }

        let _ = app.emit("playlists-changed", ());
        Ok(serde_json::json!({ "success": true, "imported": count }))
    }
}
