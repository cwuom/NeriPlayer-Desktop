//! 同步引擎需要跨启动保留的少量键值：设备 ID 与因果计数器。
//! 存储位置由宿主决定（桌面端为 tauri-plugin-store 的 sync-state.json）

use serde_json::Value;

use neri_common::error::AppResult;

pub const DEVICE_ID_KEY: &str = "deviceId";
pub const CAUSAL_COUNTER_KEY: &str = "syncCausalCounter";

pub trait SyncStateStore: Send + Sync {
    /// 存储无法打开时返回错误；键不存在时返回 `Ok(None)`
    fn get(&self, key: &str) -> AppResult<Option<Value>>;
    fn set(&self, key: &str, value: Value) -> AppResult<()>;
    /// 立即落盘；因果计数器必须在返回令牌前持久化，否则重启后会发出重复令牌
    fn save(&self) -> AppResult<()>;
}

#[cfg(test)]
pub(crate) mod memory {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// 测试用内存存储；`saves` 记录落盘次数
    #[derive(Default)]
    pub(crate) struct MemoryStateStore {
        pub(crate) values: Mutex<HashMap<String, Value>>,
        pub(crate) saves: Mutex<usize>,
    }

    impl SyncStateStore for MemoryStateStore {
        fn get(&self, key: &str) -> AppResult<Option<Value>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: Value) -> AppResult<()> {
            self.values.lock().unwrap().insert(key.to_owned(), value);
            Ok(())
        }

        fn save(&self) -> AppResult<()> {
            *self.saves.lock().unwrap() += 1;
            Ok(())
        }
    }
}
