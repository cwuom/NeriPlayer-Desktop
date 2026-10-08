use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use super::ws_client::LtWsClient;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LtSessionCredentials {
    pub member_secret: Option<String>,
    pub join_secret: Option<String>,
    pub token: Option<String>,
}

pub struct LtSessionUpdate {
    pub base_url: String,
    pub room_id: Option<String>,
    pub user_uuid: String,
    pub nickname: String,
    pub token: Option<String>,
    pub ws_url: Option<String>,
    pub member_secret: Option<String>,
    pub join_secret: Option<String>,
}

pub struct LtSessionHttpCredentials {
    pub base_url: String,
    pub room_id: String,
    pub token: String,
}

/// 一起听会话状态
pub struct LtSession {
    generation: u64,
    pub base_url: Option<String>,
    pub room_id: Option<String>,
    pub token: Option<String>,
    pub member_secret: Option<String>,
    pub join_secret: Option<String>,
    pub ws_url: Option<String>,
    pub user_uuid: String,
    pub nickname: String,
    pub ws_client: Arc<TokioMutex<Option<LtWsClient>>>,
}

impl Default for LtSession {
    fn default() -> Self {
        Self::new()
    }
}

impl LtSession {
    pub fn new() -> Self {
        Self {
            generation: 0,
            base_url: None,
            room_id: None,
            token: None,
            member_secret: None,
            join_secret: None,
            ws_url: None,
            user_uuid: String::new(),
            nickname: String::new(),
            ws_client: Arc::new(TokioMutex::new(None)),
        }
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.base_url = None;
        self.room_id = None;
        self.token = None;
        self.member_secret = None;
        self.join_secret = None;
        self.ws_url = None;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn begin_leave(&mut self) -> Option<LtSessionHttpCredentials> {
        self.generation = self.generation.wrapping_add(1);
        self.http_credentials()
    }

    pub fn credentials_for_join(
        &self,
        base_url: &str,
        room_id: &str,
        user_uuid: &str,
    ) -> LtSessionCredentials {
        if !self.matches_join_session(base_url, room_id, user_uuid) {
            return LtSessionCredentials::default();
        }
        LtSessionCredentials {
            member_secret: self.member_secret.clone(),
            join_secret: self.join_secret.clone(),
            token: self.token.clone().filter(|token| !token.trim().is_empty()),
        }
    }

    pub fn token_for_room_state(&self, base_url: &str, room_id: &str) -> Option<String> {
        self.matches_room(base_url, room_id)
            .then(|| self.token.clone())
            .flatten()
            .filter(|token| !token.trim().is_empty())
    }

    pub fn http_credentials(&self) -> Option<LtSessionHttpCredentials> {
        let base_url = normalize_base_url(self.base_url.as_deref()?);
        let room_id = self.room_id.as_deref()?.trim();
        let token = self.token.as_deref()?.trim();
        if base_url.is_empty() || room_id.is_empty() || token.is_empty() {
            return None;
        }
        Some(LtSessionHttpCredentials {
            base_url: base_url.to_string(),
            room_id: room_id.to_string(),
            token: token.to_string(),
        })
    }

    pub fn update_room_if_current(&mut self, generation: u64, update: LtSessionUpdate) -> bool {
        if generation != self.generation {
            return false;
        }
        self.update_room(update);
        true
    }

    fn update_room(&mut self, update: LtSessionUpdate) {
        let LtSessionUpdate {
            base_url,
            room_id,
            user_uuid,
            nickname,
            token,
            ws_url,
            member_secret,
            join_secret,
        } = update;
        let keep_credentials = room_id
            .as_deref()
            .is_some_and(|room_id| self.matches_join_session(&base_url, room_id, &user_uuid));
        let previous_member_secret = keep_credentials
            .then(|| self.member_secret.clone())
            .flatten();
        let previous_join_secret = keep_credentials.then(|| self.join_secret.clone()).flatten();

        self.base_url = Some(base_url);
        self.room_id = room_id;
        self.token = token;
        self.member_secret = member_secret.or(previous_member_secret);
        self.join_secret = join_secret.or(previous_join_secret);
        self.ws_url = ws_url;
        self.user_uuid = user_uuid;
        self.nickname = nickname;
    }

    fn matches_join_session(&self, base_url: &str, room_id: &str, user_uuid: &str) -> bool {
        self.matches_room(base_url, room_id) && self.user_uuid.eq_ignore_ascii_case(user_uuid)
    }

    fn matches_room(&self, base_url: &str, room_id: &str) -> bool {
        self.base_url.as_deref().is_some_and(|current_base_url| {
            normalize_base_url(current_base_url).eq_ignore_ascii_case(normalize_base_url(base_url))
        }) && self
            .room_id
            .as_deref()
            .is_some_and(|current_room_id| current_room_id.eq_ignore_ascii_case(room_id))
    }
}

fn normalize_base_url(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> LtSession {
        let mut session = LtSession::new();
        session.base_url = Some("https://listen.example/".into());
        session.room_id = Some("ABC123".into());
        session.user_uuid = "user-1".into();
        session.token = Some("token".into());
        session.member_secret = Some("member-secret".into());
        session.join_secret = Some("join-secret".into());
        session
    }

    fn room_update(room_id: &str, token: &str) -> LtSessionUpdate {
        LtSessionUpdate {
            base_url: "https://listen.example".into(),
            room_id: Some(room_id.into()),
            user_uuid: "user-1".into(),
            nickname: "Neri".into(),
            token: Some(token.into()),
            ws_url: None,
            member_secret: None,
            join_secret: None,
        }
    }

    #[test]
    fn matching_session_reuses_join_credentials_and_token() {
        let session = session();

        let credentials =
            session.credentials_for_join("https://listen.example", "abc123", "USER-1");

        assert_eq!(credentials.member_secret.as_deref(), Some("member-secret"));
        assert_eq!(credentials.join_secret.as_deref(), Some("join-secret"));
        assert_eq!(credentials.token.as_deref(), Some("token"));
        assert_eq!(
            session.token_for_room_state("https://listen.example", "ABC123"),
            Some("token".into())
        );
    }

    #[test]
    fn credentials_are_not_reused_for_another_endpoint_or_member() {
        let session = session();

        assert_eq!(
            session.credentials_for_join("https://other.example", "ABC123", "user-1"),
            LtSessionCredentials::default()
        );
        assert_eq!(
            session.credentials_for_join("https://listen.example", "ABC123", "user-2"),
            LtSessionCredentials::default()
        );
        assert!(session
            .token_for_room_state("https://other.example", "ABC123")
            .is_none());
    }

    #[test]
    fn room_update_keeps_omitted_credentials_for_the_same_member() {
        let mut session = session();

        session.update_room(LtSessionUpdate {
            base_url: "https://listen.example".into(),
            room_id: Some("ABC123".into()),
            user_uuid: "user-1".into(),
            nickname: "Neri".into(),
            token: Some("new-token".into()),
            ws_url: None,
            member_secret: None,
            join_secret: None,
        });

        assert_eq!(session.member_secret.as_deref(), Some("member-secret"));
        assert_eq!(session.join_secret.as_deref(), Some("join-secret"));
        assert_eq!(session.token.as_deref(), Some("new-token"));
        session.reset();
        assert!(session.member_secret.is_none());
        assert!(session.join_secret.is_none());
    }

    #[test]
    fn http_credentials_require_an_active_authenticated_room() {
        let mut session = session();
        let credentials = session.http_credentials().unwrap();
        assert_eq!(credentials.base_url, "https://listen.example");
        assert_eq!(credentials.room_id, "ABC123");
        assert_eq!(credentials.token, "token");

        session.token = Some("   ".into());
        assert!(session.http_credentials().is_none());
        session.token = Some("token".into());
        session.room_id = Some("".into());
        assert!(session.http_credentials().is_none());
        session.reset();
        assert!(session.http_credentials().is_none());
    }

    #[test]
    fn reset_rejects_a_pending_room_response() {
        let mut session = session();
        let pending_generation = session.generation();
        session.reset();

        assert!(!session
            .update_room_if_current(pending_generation, room_update("ABC123", "late-token")));
        assert!(session.room_id.is_none());
        assert!(session.token.is_none());
    }

    #[test]
    fn late_room_response_does_not_overwrite_a_new_session() {
        let mut session = session();
        let previous_generation = session.generation();
        session.reset();
        assert!(session
            .update_room_if_current(session.generation(), room_update("NEW123", "current-token")));

        assert!(!session
            .update_room_if_current(previous_generation, room_update("ABC123", "late-token")));
        assert_eq!(session.room_id.as_deref(), Some("NEW123"));
        assert_eq!(session.token.as_deref(), Some("current-token"));
    }

    #[test]
    fn leave_invalidates_pending_room_requests_before_the_http_request() {
        let mut session = session();
        let pending_generation = session.generation();
        let leave_credentials = session.begin_leave().unwrap();

        assert_eq!(leave_credentials.room_id, "ABC123");
        assert_eq!(leave_credentials.token, "token");
        assert!(!session
            .update_room_if_current(pending_generation, room_update("NEW123", "late-token")));
        assert_eq!(session.room_id.as_deref(), Some("ABC123"));
        assert_eq!(session.token.as_deref(), Some("token"));
    }
}
