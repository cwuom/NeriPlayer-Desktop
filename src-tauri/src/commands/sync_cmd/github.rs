//! GitHub 同步命令：仓库配置、令牌校验、执行同步与限流冷却

use super::*;

/// 获取 GitHub 同步配置（不含 token 明文）
#[tauri::command]
pub async fn get_github_sync_config(app: AppHandle) -> AppResult<Value> {
    let config = load_github_config(&app);
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "configured": github_sync_configured(&config),
        "owner": config.owner,
        "repo": config.repo,
        "autoSync": config.auto_sync,
        "lastSyncTime": config.last_sync_time,
        "dataSaver": config.data_saver,
        "silentFailures": config.silent_failures,
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 验证 GitHub token，返回用户名
#[tauri::command]
pub async fn validate_github_token(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> AppResult<Value> {
    let request = with_config_generations(|generations| generations.begin(ConfigProvider::GitHub))?;
    let api = neri_library::sync::github_api::GitHubApiClient::new(&state.sync_http(), &token);
    let username = api.validate_token().await?;

    // 暂存 token（还没配置完，只保存 token 和 owner）
    with_config_generations(|generations| {
        generations.complete(request, || {
            let config = validated_github_identity(
                load_github_config_unlocked(&app),
                token,
                username.clone(),
            );
            save_github_config_unlocked(&app, &config);
            Ok(())
        })
    })?;

    Ok(serde_json::json!({
        "success": true,
        "username": username,
    }))
}

/// 创建新仓库
#[tauri::command]
pub async fn create_github_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    repo_name: String,
) -> AppResult<Value> {
    let (config, request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::GitHub)?;
        Ok::<_, AppError>((
            load_github_config_unlocked(&app),
            request,
            generations.github_preferences,
        ))
    })?;
    if config.token.is_empty() {
        return Err(AppError::Api("Token not validated yet".into()));
    }

    let api =
        neri_library::sync::github_api::GitHubApiClient::new(&state.sync_http(), &config.token);
    api.create_repository(&repo_name).await?;

    let updated = GitHubSyncConfig {
        token: config.token.clone(),
        owner: config.owner.clone(),
        repo: repo_name.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_github_config_unlocked(&app);
            let updated = github_connection_preferences(
                preferences,
                generations.github_preferences,
                &current,
                updated,
            );
            save_github_config_unlocked(&app, &updated);
            Ok(())
        })
    })?;

    Ok(serde_json::json!({
        "success": true,
        "owner": config.owner,
        "repo": repo_name,
    }))
}

/// 使用已有仓库
#[tauri::command]
pub async fn use_existing_github_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    owner: String,
    repo: String,
) -> AppResult<Value> {
    let (config, request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::GitHub)?;
        Ok::<_, AppError>((
            load_github_config_unlocked(&app),
            request,
            generations.github_preferences,
        ))
    })?;
    if config.token.is_empty() {
        return Err(AppError::Api("Token not validated yet".into()));
    }

    let api =
        neri_library::sync::github_api::GitHubApiClient::new(&state.sync_http(), &config.token);
    let _branch = api.check_repository(&owner, &repo).await?;

    let updated = GitHubSyncConfig {
        token: config.token.clone(),
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_github_config_unlocked(&app);
            let updated = github_connection_preferences(
                preferences,
                generations.github_preferences,
                &current,
                updated,
            );
            save_github_config_unlocked(&app, &updated);
            Ok(())
        })
    })?;

    Ok(serde_json::json!({
        "success": true,
        "owner": owner,
        "repo": repo,
    }))
}

/// 配置 GitHub 同步（保留兼容，内部调用两阶段）
#[tauri::command]
pub async fn configure_github_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    repo: String,
) -> AppResult<Value> {
    let (request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::GitHub)?;
        Ok::<_, AppError>((request, generations.github_preferences))
    })?;
    let api = neri_library::sync::github_api::GitHubApiClient::new(&state.sync_http(), &token);
    let owner = api.validate_token().await?;

    match api.check_repository(&owner, &repo).await {
        Ok(_) => {}
        Err(error) if error.is_not_found() => api.create_repository(&repo).await?,
        Err(error) => return Err(error.into()),
    }

    let config = GitHubSyncConfig {
        token,
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_github_config_unlocked(&app);
            let config = github_connection_preferences(
                preferences,
                generations.github_preferences,
                &current,
                config,
            );
            save_github_config_unlocked(&app, &config);
            Ok(())
        })
    })?;

    Ok(serde_json::json!({
        "success": true,
        "owner": owner,
        "repo": repo,
    }))
}

#[tauri::command]
pub async fn sync_github(
    app: AppHandle,
    state: State<'_, AppState>,
    history_entries: Option<Vec<manager::SyncHistoryEntry>>,
    history_deletions: Option<Vec<manager::SyncHistoryDeletion>>,
) -> AppResult<SyncResult> {
    let (config, request) = with_config_generations(|generations| {
        (
            load_github_config_unlocked(&app),
            generations.snapshot(ConfigProvider::GitHub),
        )
    });
    if !github_sync_configured(&config) {
        return Err(AppError::Api("GitHub sync not configured".into()));
    }
    // 冷却期内不发任何请求：GitHub 对限流期间的继续请求会延长封禁
    let cooldown_target = github_cooldown_target(&config);
    let previous_cooldown = load_github_cooldown(&app);
    if let Some(cooldown) = previous_cooldown
        .as_ref()
        .filter(|cooldown| cooldown.blocks(&cooldown_target, chrono::Utc::now().timestamp_millis()))
    {
        return Err(cooldown.failure().into());
    }

    let (local_data, playlist_epoch) = build_local_sync_snapshot(
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
        local_stats_payload(&state),
    )?;
    let outcome = deferred_on_local_change(
        manager::sync_github(
            &state.sync_http(),
            &config,
            &local_data,
            playlist_epoch,
            |completed| {
                with_config_generations(|generations| {
                    generations.complete(request, || {
                        let mut current = load_github_config_unlocked(&app);
                        let outcome = complete_for_current_target(
                            &mut current,
                            &config,
                            same_github_target,
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
                                current.last_remote_sha = completed.version.clone();
                                current.last_sync_time = chrono::Utc::now().timestamp_millis();
                            },
                        )?;
                        save_github_config_unlocked(&app, &current);
                        Ok(outcome)
                    })
                })
            },
        )
        .await,
    )
    .map_err(|error| {
        github_sync_failed(
            &app,
            &config,
            previous_cooldown.as_ref(),
            &cooldown_target,
            error,
        )
    })?;
    if previous_cooldown.is_some() {
        save_github_cooldown(&app, None);
    }
    let Some(outcome) = outcome else {
        return Ok(deferred_sync_result());
    };
    // 仅本地数据确有变化时通知前端刷新；"sync" 标记让 App.vue 不再为这次写回触发自动同步
    if outcome.local_changed {
        let _ = app.emit("playlists-changed", SYNC_WRITE_BACK);
    }
    let _ = app.emit("playback-stats-changed", ());
    Ok(outcome.result)
}

const GITHUB_COOLDOWN_KEY: &str = "githubRateLimit";

pub(super) fn github_cooldown_target(config: &GitHubSyncConfig) -> String {
    format!("{}/{}", config.owner.trim(), config.repo.trim()).to_ascii_lowercase()
}

fn load_github_cooldown(app: &AppHandle) -> Option<GitHubCooldown> {
    app.store(SYNC_STORE)
        .ok()?
        .get(GITHUB_COOLDOWN_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
}

fn save_github_cooldown(app: &AppHandle, cooldown: Option<&GitHubCooldown>) {
    let Ok(store) = app.store(SYNC_STORE) else {
        return;
    };
    match cooldown {
        Some(cooldown) => store.set(GITHUB_COOLDOWN_KEY, serde_json::json!(cooldown)),
        None => {
            store.delete(GITHUB_COOLDOWN_KEY);
        }
    }
    let _ = store.save();
}

/// 限流时记一次冷却，交给前端的错误带上冷却后的重试时间和是否还自动重试
pub(super) fn classify_github_sync_failure(
    error: AppError,
    previous: Option<&GitHubCooldown>,
    target: &str,
    now_ms: i64,
) -> (AppError, Option<GitHubCooldown>) {
    match error {
        AppError::Sync(SyncFailure::GitHubRateLimited {
            status,
            retry_at_ms,
            ..
        }) => {
            let cooldown = GitHubCooldown::record(previous, target, status, retry_at_ms, now_ms);
            (cooldown.failure().into(), Some(cooldown))
        }
        error => (error, None),
    }
}

/// 只清掉这次被拒的 token：同步途中用户可能已经换上了新 token。owner/repo 保留，方便重新登录
pub(super) fn clear_rejected_github_token(
    current: &mut GitHubSyncConfig,
    rejected: &GitHubSyncConfig,
) -> bool {
    if current.token.is_empty() || current.token != rejected.token {
        return false;
    }
    current.token.clear();
    true
}

fn github_sync_failed(
    app: &AppHandle,
    config: &GitHubSyncConfig,
    previous: Option<&GitHubCooldown>,
    target: &str,
    error: AppError,
) -> AppError {
    if matches!(error, AppError::Sync(SyncFailure::GitHubTokenExpired)) {
        // 对齐 Android：失效 token 不会自己恢复，清掉后自动同步不再反复请求
        let cleared = with_config_generations(|generations| {
            let mut current = load_github_config_unlocked(app);
            if !clear_rejected_github_token(&mut current, config) {
                return Ok(());
            }
            generations.invalidate(ConfigProvider::GitHub)?;
            save_github_config_unlocked(app, &current);
            Ok::<_, AppError>(())
        });
        if let Err(clear_error) = cleared {
            log::warn!(target: "sync", "clearing the rejected GitHub token failed: {clear_error}");
        }
        return error;
    }
    let (error, cooldown) = classify_github_sync_failure(
        error,
        previous,
        target,
        chrono::Utc::now().timestamp_millis(),
    );
    if let Some(cooldown) = cooldown {
        save_github_cooldown(app, Some(&cooldown));
    }
    error
}

/// 断开 GitHub 同步
#[tauri::command]
pub async fn disconnect_github_sync(app: AppHandle) -> AppResult<()> {
    with_config_generations(|generations| {
        generations.invalidate(ConfigProvider::GitHub)?;
        save_github_config_unlocked(&app, &GitHubSyncConfig::default());
        Ok(())
    })
}

/// 更新 GitHub 同步子设置（不影响 token/owner/repo）
#[tauri::command]
pub async fn update_github_sync_settings(
    app: AppHandle,
    auto_sync: Option<bool>,
    data_saver: Option<bool>,
    silent_failures: Option<bool>,
    history_update_mode: Option<String>,
) -> AppResult<()> {
    with_config_generations(|generations| {
        let mut config = load_github_config_unlocked(&app);
        if config.token.is_empty() {
            return Err(AppError::Api("GitHub sync not configured".into()));
        }
        if let Some(v) = auto_sync {
            advance_preference_revision(&mut generations.github_preferences.auto_sync)?;
            config.auto_sync = v;
        }
        if let Some(v) = data_saver {
            advance_preference_revision(&mut generations.github_preferences.data_saver)?;
            config.data_saver = v;
        }
        if let Some(v) = silent_failures {
            advance_preference_revision(&mut generations.github_preferences.silent_failures)?;
            config.silent_failures = v;
        }
        if let Some(v) = history_update_mode {
            advance_preference_revision(&mut generations.github_preferences.history_update_mode)?;
            advance_preference_revision(&mut generations.sync_preferences_revision)?;
            let normalized = normalize_history_update_mode(&v);
            config.history_update_mode = normalized.clone();
            save_sync_preferences_unlocked(
                &app,
                &SyncPreferencesConfig {
                    history_update_mode: normalized,
                },
            );
        }
        save_github_config_unlocked(&app, &config);
        Ok(())
    })
}
