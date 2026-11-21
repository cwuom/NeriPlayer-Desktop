// 同步相关命令
use crate::settings::store::{self, AppSettings};
use crate::state::AppState;
use neri_common::config_backup::{self, ConfigBackupError};
use neri_common::error::{AppError, AppResult};
use neri_common::security;
use neri_common::sync_failure::{GitHubCooldown, SyncFailure};
use neri_library::library::playlist;
use neri_library::sync::manager;
use neri_model::sync::models::*;
use neri_platform::auth::state::{AuthState, BiliAuth, CookieEntry, NeteaseAuth, YouTubeAuth};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_store::StoreExt;

// 按职责拆分的命令与辅助逻辑，通过 `use super::*` 共享本模块的导入、常量与配置请求代际
mod config_file;
mod config_store;
mod github;
mod playlists;
mod webdav;
pub use config_file::*;
pub use config_store::*;
pub use github::*;
pub use playlists::*;
pub use webdav::*;

// 同步配置存储键
const GITHUB_CONFIG_KEY: &str = "githubSync";
const WEBDAV_CONFIG_KEY: &str = "webdavSync";
const SYNC_PREFERENCES_KEY: &str = "syncPreferences";
const SYNC_STORE: &str = "sync-config.json";
const CONFIG_FILE_KIND: &str = "moe.ouom.neriplayer.config";
const CONFIG_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
enum ConfigProvider {
    GitHub,
    WebDav,
}

#[derive(Debug, Clone, Copy)]
struct ConfigRequestToken {
    provider: ConfigProvider,
    generation: u64,
}

#[derive(Default)]
struct ConfigRequestGenerations {
    github: u64,
    webdav: u64,
    github_preferences: GitHubPreferenceRevisions,
    webdav_preferences: WebDavPreferenceRevisions,
    sync_preferences_revision: u64,
}

#[derive(Clone, Copy, Default)]
struct GitHubPreferenceRevisions {
    auto_sync: u64,
    data_saver: u64,
    silent_failures: u64,
    history_update_mode: u64,
}

#[derive(Clone, Copy, Default)]
struct WebDavPreferenceRevisions {
    auto_sync: u64,
    data_saver: u64,
}

fn advance_preference_revision(revision: &mut u64) -> AppResult<()> {
    *revision = revision
        .checked_add(1)
        .ok_or_else(|| AppError::Other("Sync preference revision exhausted".into()))?;
    Ok(())
}

impl ConfigRequestGenerations {
    fn snapshot(&self, provider: ConfigProvider) -> ConfigRequestToken {
        ConfigRequestToken {
            provider,
            generation: match provider {
                ConfigProvider::GitHub => self.github,
                ConfigProvider::WebDav => self.webdav,
            },
        }
    }

    fn begin(&mut self, provider: ConfigProvider) -> AppResult<ConfigRequestToken> {
        let generation = match provider {
            ConfigProvider::GitHub => &mut self.github,
            ConfigProvider::WebDav => &mut self.webdav,
        };
        *generation = generation
            .checked_add(1)
            .ok_or_else(|| AppError::Other("Sync configuration generation exhausted".into()))?;
        Ok(self.snapshot(provider))
    }

    fn invalidate(&mut self, provider: ConfigProvider) -> AppResult<()> {
        self.begin(provider).map(|_| ())
    }

    fn is_current(&self, token: ConfigRequestToken) -> bool {
        self.snapshot(token.provider).generation == token.generation
    }

    fn complete<R>(
        &self,
        token: ConfigRequestToken,
        apply: impl FnOnce() -> AppResult<R>,
    ) -> AppResult<R> {
        if !self.is_current(token) {
            return Err(AppError::Other(
                "Sync configuration request superseded by a newer action".into(),
            ));
        }
        apply()
    }
}

fn with_sync_config<R>(operation: impl FnOnce() -> R) -> R {
    with_config_generations(|_| operation())
}

fn with_config_generations<R>(operation: impl FnOnce(&mut ConfigRequestGenerations) -> R) -> R {
    // 配置变更和本地同步提交共用临界区，网络请求始终在锁外执行
    static CONFIG_LOCK: std::sync::OnceLock<parking_lot::Mutex<ConfigRequestGenerations>> =
        std::sync::OnceLock::new();
    let mut guard = CONFIG_LOCK
        .get_or_init(|| parking_lot::Mutex::new(ConfigRequestGenerations::default()))
        .lock();
    operation(&mut guard)
}

fn github_connection_preferences(
    started: GitHubPreferenceRevisions,
    latest: GitHubPreferenceRevisions,
    current: &GitHubSyncConfig,
    mut proposed: GitHubSyncConfig,
) -> GitHubSyncConfig {
    if latest.auto_sync != started.auto_sync {
        proposed.auto_sync = current.auto_sync;
    }
    if latest.data_saver != started.data_saver {
        proposed.data_saver = current.data_saver;
    }
    if latest.silent_failures != started.silent_failures {
        proposed.silent_failures = current.silent_failures;
    }
    if latest.history_update_mode != started.history_update_mode {
        proposed.history_update_mode = current.history_update_mode.clone();
    }
    proposed
}

fn webdav_connection_preferences(
    started: WebDavPreferenceRevisions,
    latest: WebDavPreferenceRevisions,
    current: &WebDavSyncConfig,
    mut proposed: WebDavSyncConfig,
) -> WebDavSyncConfig {
    if latest.auto_sync != started.auto_sync {
        proposed.auto_sync = current.auto_sync;
    }
    if latest.data_saver != started.data_saver {
        proposed.data_saver = current.data_saver;
    }
    proposed
}

fn validated_github_identity(
    mut config: GitHubSyncConfig,
    token: String,
    owner: String,
) -> GitHubSyncConfig {
    if config.token != token || config.owner != owner {
        config.repo.clear();
        config.last_remote_sha.clear();
        config.last_sync_time = 0;
        config.auto_sync = false;
    }
    config.token = token;
    config.owner = owner;
    config
}

fn github_sync_configured(config: &GitHubSyncConfig) -> bool {
    !config.token.trim().is_empty()
        && !config.owner.trim().is_empty()
        && !config.repo.trim().is_empty()
}

fn same_github_target(current: &GitHubSyncConfig, requested: &GitHubSyncConfig) -> bool {
    !current.token.is_empty()
        && current.owner == requested.owner
        && current.repo == requested.repo
        && current.token == requested.token
}

fn same_webdav_target(current: &WebDavSyncConfig, requested: &WebDavSyncConfig) -> bool {
    !current.server_url.is_empty()
        && current.server_url == requested.server_url
        && current.username == requested.username
        && current.password == requested.password
        && current.base_path == requested.base_path
}

fn complete_for_current_target<C, R>(
    current: &mut C,
    requested: &C,
    same_target: impl FnOnce(&C, &C) -> bool,
    apply: impl FnOnce() -> AppResult<R>,
    record: impl FnOnce(&mut C),
) -> AppResult<R> {
    if !same_target(current, requested) {
        return Err(AppError::Other(
            "Sync target or credentials changed; retry with the current configuration".into(),
        ));
    }
    let result = apply()?;
    record(current);
    Ok(result)
}

/// 获取全局同步偏好
#[tauri::command]
pub async fn get_sync_preferences(app: AppHandle) -> AppResult<Value> {
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 更新全局同步偏好，兼容 Android 的频率枚举和旧版 batched 值
#[tauri::command]
pub async fn update_sync_preferences(
    app: AppHandle,
    history_update_mode: Option<String>,
) -> AppResult<()> {
    with_config_generations(|generations| {
        let mut preferences = load_sync_preferences_unlocked(&app);
        if let Some(mode) = history_update_mode {
            advance_preference_revision(&mut generations.sync_preferences_revision)?;
            preferences.history_update_mode = normalize_history_update_mode(&mode);
        }
        save_sync_preferences_unlocked(&app, &preferences);
        Ok(())
    })
}

/// 读取本地播放统计快照注入同步信封
fn local_stats_payload(state: &State<'_, AppState>) -> manager::SyncStatsPayload {
    let store = state.stats.lock();
    let (stats, buckets, cleared_at) = store.sync_snapshot();
    manager::SyncStatsPayload {
        stats,
        buckets,
        cleared_at,
    }
}

fn build_local_sync_snapshot(
    app: &AppHandle,
    history_entries: Option<&[manager::SyncHistoryEntry]>,
    history_deletions: Option<&[manager::SyncHistoryDeletion]>,
    stats: manager::SyncStatsPayload,
) -> AppResult<(SyncData, u64)> {
    // 快照与 epoch 必须在同一把歌单锁内取得。否则等待全局同步锁期间的
    // 本地编辑会让旧快照搭配新 epoch，随后通过检查并覆盖刚保存的歌单
    let _playlist_guard = playlist::lock_io();
    let local_data = manager::build_local_sync_data(
        &crate::sync_state::TauriSyncState(app),
        history_entries,
        history_deletions,
        Some(stats),
    )?;
    Ok((local_data, playlist::io_epoch()))
}

/// 写回完整统计来源，本机增量由 StatsStore 单独维护
fn apply_merged_stats(
    app: &AppHandle,
    state: &State<'_, AppState>,
    merged: &SyncData,
) -> AppResult<()> {
    let device_id = manager::get_or_create_device_id_pub(&crate::sync_state::TauriSyncState(app));
    let mut store = state.stats.lock();
    store.apply_merged(
        &merged.playback_stats,
        &merged.playback_stat_buckets,
        merged.playback_stats_cleared_at,
        &device_id,
    );
    neri_library::stats::save_checked(&store)
}

/// 执行 GitHub 同步
#[tauri::command]
pub async fn approve_sync_protocol_upgrade(
    app: AppHandle,
    state: State<'_, AppState>,
    challenge: neri_library::sync::archive::approval::SyncProtocolUpgrade,
    all_devices_updated: bool,
) -> AppResult<()> {
    ensure_all_devices_updated(all_devices_updated)?;
    let (current, request) = match challenge.backend.as_str() {
        "github" => {
            let (config, request) = with_config_generations(|generations| {
                (
                    load_github_config_unlocked(&app),
                    generations.snapshot(ConfigProvider::GitHub),
                )
            });
            (
                neri_library::sync::cloud::inspect_github_upgrade(&state.sync_http(), &config)
                    .await?,
                request,
            )
        }
        "webdav" => {
            let (config, request) = with_config_generations(|generations| {
                (
                    load_webdav_config_unlocked(&app),
                    generations.snapshot(ConfigProvider::WebDav),
                )
            });
            (
                neri_library::sync::cloud::inspect_webdav_upgrade(&state.sync_http(), &config)
                    .await?,
                request,
            )
        }
        _ => return Err(AppError::Other("Invalid sync upgrade provider".into())),
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            neri_library::sync::archive::approval::approve_verified(&challenge, &current)
        })
    })
}

/// 旧客户端读不懂升级后的归档，必须由用户确认所有设备都已更新（对齐 Android SyncProtocolUpgradeRepository）
fn ensure_all_devices_updated(confirmed: bool) -> AppResult<()> {
    if confirmed {
        Ok(())
    } else {
        Err(AppError::Other(
            "Confirm that every device runs a compatible version before upgrading the sync format"
                .into(),
        ))
    }
}

/// `playlists-changed` 的载荷，表示这次变化来自同步写回而不是用户编辑
const SYNC_WRITE_BACK: &str = "sync";

/// 本地歌单在同步期间被修改时什么都没有写回，这不是失败：转成 `None` 交给前端补一轮同步
fn deferred_on_local_change<T>(result: AppResult<T>) -> AppResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if manager::is_local_change_conflict(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn deferred_sync_result() -> SyncResult {
    SyncResult {
        message: "Local data changed during sync; another sync will follow".into(),
        deferred: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
