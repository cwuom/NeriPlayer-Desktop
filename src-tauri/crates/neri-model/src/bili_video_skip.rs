//! B 站视频片段跳过规则：随云同步扩展字段跨端传输

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliVideoSkipInterval {
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliVideoSkipRule {
    pub bvid: String,
    pub cid: i64,
    #[serde(default)]
    pub intervals: Vec<BiliVideoSkipInterval>,
    #[serde(default)]
    pub modified_at: i64,
    #[serde(default)]
    pub is_deleted: bool,
}
