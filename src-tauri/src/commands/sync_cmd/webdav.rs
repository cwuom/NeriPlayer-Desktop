//! WebDAV 同步命令：配置、执行同步与断开

use super::*;

/// 获取 WebDAV 同步配置
#[tauri::command]
pub async fn get_webdav_sync_config(app: AppHandle) -> AppResult<Value> {
    let config = load_webdav_config(&app);
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "configured": !config.server_url.is_empty(),
        "serverUrl": config.server_url,
        "basePath": config.base_path,
        "autoSync": config.auto_sync,
        "lastSyncTime": config.last_sync_time,
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 配置 WebDAV 同步
#[tauri::command]
pub async fn configure_webdav_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    server_url: String,
    username: String,
    password: String,
    base_path: Option<String>,
) -> AppResult<Value> {
    // 对齐 Android WebDavStorage：服务器、用户名、密码缺一不可
    if server_url.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
        return Err(AppError::Other(
            "WebDAV server, username and password are required".into(),
        ));
    }
    let (request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::WebDav)?;
        Ok::<_, AppError>((request, generations.webdav_preferences))
    })?;
    let bp = base_path.unwrap_or_default();
    let config = WebDavSyncConfig {
        server_url: server_url.clone(),
        username,
        password,
        base_path: bp,
        auto_sync: true,
        ..Default::default()
    };
    neri_library::sync::webdav_archive::WebDavArchiveClient::new(&state.sync_http(), &config)?
        .validate_connection()
        .await?;
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_webdav_config_unlocked(&app);
            let config = webdav_connection_preferences(
                preferences,
                generations.webdav_preferences,
                &current,
                config,
            );
            save_webdav_config_unlocked(&app, &config);
            Ok(())
        })
    })?;

    Ok(serde_json::json!({
        "success": true,
        "serverUrl": server_url,
    }))
}

/// 执行 WebDAV 同步
#[tauri::command]
pub async fn sync_webdav(
    app: AppHandle,
    state: State<'_, AppState>,
    history_entries: Option<Vec<manager::SyncHistoryEntry>>,
    history_deletions: Option<Vec<manager::SyncHistoryDeletion>>,
) -> AppResult<SyncResult> {
    let (config, request) = with_config_generations(|generations| {
        (
            load_webdav_config_unlocked(&app),
            generations.snapshot(ConfigProvider::WebDav),
        )
    });
    if config.server_url.is_empty() {
        return Err(AppError::Api("WebDAV sync not configured".into()));
    }

    let (local_data, playlist_epoch) = build_local_sync_snapshot(
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
        local_stats_payload(&state),
    )?;
    let outcome = deferred_on_local_change(
        manager::sync_webdav(
            &state.sync_http(),
            &config,
            &local_data,
            playlist_epoch,
            |completed| {
                with_config_generations(|generations| {
                    generations.complete(request, || {
                        let mut current = load_webdav_config_unlocked(&app);
                        let outcome = complete_for_current_target(
                            &mut current,
                            &config,
                            same_webdav_target,
                            || {
                                let outcome = manager::complete_cloud_sync(
                                    &completed,
                                    &local_data,
                                    playlist_epoch,
                                )?;
                                apply_merged_stats(&app, &state, &outcome.merged)?;
                                Ok(outcome)
                            },
                            |current| {
                                current.last_remote_fingerprint = completed.version.clone();
                                current.last_sync_time = chrono::Utc::now().timestamp_millis();
                            },
                        )?;
                        save_webdav_config_unlocked(&app, &current);
                        Ok(outcome)
                    })
                })
            },
        )
        .await,
    )?;
    let Some(outcome) = outcome else {
        return Ok(deferred_sync_result());
    };
    // 与 sync_github 同理
    if outcome.local_changed {
        let _ = app.emit("playlists-changed", SYNC_WRITE_BACK);
    }
    let _ = app.emit("playback-stats-changed", ());
    Ok(outcome.result)
}

/// 更新 WebDAV 同步子设置
#[tauri::command]
pub async fn update_webdav_sync_settings(app: AppHandle, auto_sync: Option<bool>) -> AppResult<()> {
    with_config_generations(|generations| {
        let mut config = load_webdav_config_unlocked(&app);
        if config.server_url.is_empty() {
            return Err(AppError::Api("WebDAV sync not configured".into()));
        }
        if let Some(v) = auto_sync {
            advance_preference_revision(&mut generations.webdav_preferences.auto_sync)?;
            config.auto_sync = v;
        }
        save_webdav_config_unlocked(&app, &config);
        Ok(())
    })
}

/// 断开 WebDAV 同步
#[tauri::command]
pub async fn disconnect_webdav_sync(app: AppHandle) -> AppResult<()> {
    with_config_generations(|generations| {
        generations.invalidate(ConfigProvider::WebDav)?;
        save_webdav_config_unlocked(&app, &WebDavSyncConfig::default());
        Ok(())
    })
}
