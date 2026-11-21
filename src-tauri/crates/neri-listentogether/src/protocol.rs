use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtTrack {
    pub stable_key: String,
    pub channel_id: String,
    pub audio_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_audio_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
    #[serde(default)]
    pub stream_urls: Vec<String>,
    pub name: String,
    pub artist: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtRoomSettings {
    #[serde(default = "default_true")]
    pub allow_member_control: bool,
    #[serde(default = "default_true")]
    pub auto_pause_on_member_change: bool,
    #[serde(default = "default_true")]
    pub share_audio_links: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LtRoomSettings {
    fn default() -> Self {
        Self {
            allow_member_control: true,
            auto_pause_on_member_change: true,
            share_audio_links: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtMember {
    #[serde(default)]
    pub user_uuid: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub role: String,
    pub joined_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtPlaybackState {
    #[serde(default = "default_paused")]
    pub state: String,
    #[serde(default)]
    pub base_position_ms: i64,
    #[serde(default)]
    pub base_timestamp_ms: i64,
    #[serde(default = "default_rate")]
    pub playback_rate: f64,
    // Align Android ListenTogetherPlaybackState / ExoPlayer:
    // REPEAT_MODE_OFF=0, ONE=1, ALL=2
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_mode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_enabled: Option<bool>,
}

fn default_paused() -> String {
    "paused".to_string()
}
fn default_rate() -> f64 {
    1.0
}

impl Default for LtPlaybackState {
    fn default() -> Self {
        Self {
            state: "paused".to_string(),
            base_position_ms: 0,
            base_timestamp_ms: 0,
            playback_rate: 1.0,
            repeat_mode: None,
            shuffle_enabled: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtRoomState {
    pub room_id: String,
    pub version: i64,
    #[serde(default = "default_schema")]
    pub schema_version: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_user_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_heartbeat_at: Option<i64>,
    #[serde(default)]
    pub settings: LtRoomSettings,
    #[serde(default)]
    pub members: Vec<LtMember>,
    #[serde(default)]
    pub queue: Vec<LtTrack>,
    #[serde(default)]
    pub current_index: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<LtTrack>,
    #[serde(default)]
    pub playback: LtPlaybackState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_offline_since: Option<i64>,
    #[serde(default = "default_active")]
    pub room_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_schema() -> i32 {
    1
}
fn default_active() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtCause {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtInitialSnapshot {
    #[serde(default)]
    pub queue: Vec<LtTrack>,
    #[serde(default)]
    pub current_index: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<LtTrack>,
    #[serde(default)]
    pub settings: LtRoomSettings,
    #[serde(default)]
    pub is_playing: bool,
    #[serde(default)]
    pub position_ms: i64,
    // Align Android ListenTogetherInitialSnapshot
    #[serde(default)]
    pub repeat_mode: i32,
    #[serde(default)]
    pub shuffle_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_restore_queue: Option<Vec<LtTrack>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtCreateRoomRequest {
    pub user_uuid: String,
    pub nickname: String,
    pub initial_snapshot: LtInitialSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtJoinRoomRequest {
    pub user_uuid: String,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtRoomResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_secret: Option<String>,
    #[serde(default)]
    pub auto_pause_on_join: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<LtRoomState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtStateResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<LtRoomState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_position_ms: Option<i64>,
    // Align Android ListenTogetherStateResponse.serverNowMs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_now_ms: Option<i64>,
    #[serde(default)]
    pub auto_pause_on_join: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtEvent {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_time_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<LtTrack>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<Vec<LtTrack>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_settings: Option<LtRoomSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub should_play: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    // PLAYBACK_MODE / REQUEST_PLAYBACK_MODE payload (Android-aligned)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_mode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_mutation: Option<LtQueueMutation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_track_stable_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_refresh: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_track_stable_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtQueueReference {
    pub stable_key: String,
    pub occurrence: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtQueueOperation {
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<LtQueueReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<LtQueueReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<LtTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<LtQueueReference>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtQueueMutation {
    pub base_room_version: i64,
    pub operations: Vec<LtQueueOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_current: Option<LtQueueReference>,
}

/// 服务端实际落地的事件（对齐 Android ListenTogetherAppliedEvent）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtAppliedEvent {
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<LtRoomState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_position_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<LtCause>,
}

/// 控制命令的应答（对齐 Android ListenTogetherControlResponse）
///
/// 缺这个字段时，服务端拒绝控制（成员无权限、房间已关闭等）在桌面端
/// 会被静默丢弃：本地乐观改动照旧生效，用户也收不到任何提示，
/// 于是两端状态就此分叉。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtControlResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<LtAppliedEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LtSocketEnvelope {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub auto_pause_on_join: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<LtRoomState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_position_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now_ms: Option<i64>,
    // Android envelope also carries short alias `t`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// 控制命令应答；serde 默认忽略未知字段，缺这一项会被静默丢弃
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<LtControlResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<LtCause>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<LtTrack>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<Vec<LtTrack>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_track_stable_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub should_play: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_mode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_mutation: Option<LtQueueMutation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_time_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_sequence: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track_json() -> serde_json::Value {
        json!({
            "stableKey": "netease:42",
            "channelId": "netease",
            "audioId": "42",
            "name": "Song",
            "artist": "Artist"
        })
    }

    #[test]
    fn room_join_credentials_use_the_android_wire_contract() {
        let request = LtJoinRoomRequest {
            user_uuid: "user-1".into(),
            nickname: "Neri".into(),
            member_secret: Some("member-secret".into()),
            join_secret: Some("join-secret".into()),
        };

        let encoded = serde_json::to_value(request).unwrap();
        assert_eq!(encoded["memberSecret"], "member-secret");
        assert_eq!(encoded["joinSecret"], "join-secret");

        let response: LtRoomResponse = serde_json::from_value(json!({
            "ok": true,
            "memberSecret": "member-secret",
            "joinSecret": "join-secret"
        }))
        .unwrap();
        assert_eq!(response.member_secret.as_deref(), Some("member-secret"));
        assert_eq!(response.join_secret.as_deref(), Some("join-secret"));
    }

    #[test]
    fn track_stream_candidates_survive_the_ipc_round_trip() {
        let mut payload = track_json();
        payload["streamUrl"] = json!("https://audio.example/primary");
        payload["streamUrls"] = json!([
            "https://audio.example/primary",
            "https://audio.example/backup"
        ]);

        let track: LtTrack = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(track.stream_urls.len(), 2);
        let encoded = serde_json::to_value(track).unwrap();
        assert_eq!(encoded["streamUrl"], payload["streamUrl"]);
        assert_eq!(encoded["streamUrls"], payload["streamUrls"]);

        let legacy: LtTrack = serde_json::from_value(track_json()).unwrap();
        assert!(legacy.stream_urls.is_empty());
    }

    #[test]
    fn forwarded_queue_mutation_preserves_duplicate_track_references() {
        let mutation = json!({
            "baseRoomVersion": 12,
            "operations": [{
                "type": "remove",
                "target": { "stableKey": "netease:42", "occurrence": 1 }
            }, {
                "type": "insert",
                "anchor": { "stableKey": "netease:42", "occurrence": 0 },
                "placement": "after",
                "track": track_json()
            }, {
                "type": "reorder",
                "order": [
                    { "stableKey": "netease:42", "occurrence": 1 },
                    { "stableKey": "netease:42", "occurrence": 0 }
                ]
            }],
            "targetCurrent": { "stableKey": "netease:42", "occurrence": 0 }
        });
        let envelope: LtSocketEnvelope = serde_json::from_value(json!({
            "type": "member_control_requested",
            "causedBy": { "type": "REQUEST_SET_QUEUE" },
            "queueMutation": mutation,
            "requestSequence": 7
        }))
        .unwrap();

        let encoded = serde_json::to_value(envelope).unwrap();
        let actual = &encoded["queueMutation"];
        assert_eq!(actual["baseRoomVersion"], 12);
        assert_eq!(actual["operations"][0]["target"]["occurrence"], 1);
        assert_eq!(actual["operations"][1]["placement"], "after");
        assert_eq!(
            actual["operations"][2]["order"],
            mutation["operations"][2]["order"]
        );
        assert_eq!(actual["targetCurrent"], mutation["targetCurrent"]);
        assert_eq!(encoded["requestSequence"], 7);

        let event: LtEvent = serde_json::from_value(json!({
            "type": "SET_QUEUE",
            "queueMutation": mutation
        }))
        .unwrap();
        assert!(event.queue_mutation.is_some());
        assert_eq!(
            serde_json::to_value(event).unwrap()["queueMutation"]["baseRoomVersion"],
            12
        );
    }

    #[test]
    fn shuffle_snapshot_and_forced_link_refresh_use_android_fields() {
        let snapshot: LtInitialSnapshot = serde_json::from_value(json!({
            "shuffleEnabled": true,
            "shuffleRestoreQueue": [track_json()]
        }))
        .unwrap();
        let encoded = serde_json::to_value(snapshot).unwrap();
        assert_eq!(encoded["shuffleRestoreQueue"][0]["stableKey"], "netease:42");

        let event: LtEvent = serde_json::from_value(json!({
            "type": "REQUEST_TRACK_LINK",
            "requestTrackStableKey": "netease:42",
            "forceRefresh": true
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(event).unwrap()["forceRefresh"], true);

        let legacy: LtInitialSnapshot = serde_json::from_value(json!({})).unwrap();
        assert!(legacy.shuffle_restore_queue.is_none());
        let legacy_event: LtEvent = serde_json::from_value(json!({"type": "PLAY"})).unwrap();
        assert!(legacy_event.queue_mutation.is_none());
        assert!(legacy_event.force_refresh.is_none());
    }
}
