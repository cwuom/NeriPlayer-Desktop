//! 同步引擎的设备 ID 与因果计数器存在 tauri-plugin-store 的 sync-state.json

use neri_common::error::{AppError, AppResult};
use neri_library::sync::state_store::SyncStateStore;
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

const STORE_FILE: &str = "sync-state.json";

pub struct TauriSyncState<'a>(pub &'a AppHandle);

impl TauriSyncState<'_> {
    fn store(&self) -> AppResult<std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>> {
        self.0
            .store(STORE_FILE)
            .map_err(|error| AppError::Other(format!("Failed to open sync state: {}", error)))
    }
}

impl SyncStateStore for TauriSyncState<'_> {
    fn get(&self, key: &str) -> AppResult<Option<Value>> {
        Ok(self.store()?.get(key))
    }

    fn set(&self, key: &str, value: Value) -> AppResult<()> {
        self.store()?.set(key, value);
        Ok(())
    }

    fn save(&self) -> AppResult<()> {
        self.store()?
            .save()
            .map_err(|error| AppError::Other(format!("Failed to persist sync state: {}", error)))
    }
}
