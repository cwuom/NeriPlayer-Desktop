use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_ATTEMPTS: u32 = 3;
pub const RETRY_COOLDOWN_MS: u64 = 30_000;

type SessionKey = [u8; 32];

/// Limits background account-profile recovery for an imported MUSIC_U session.
/// The raw cookie is never retained in the gate or exposed through diagnostics.
#[derive(Default)]
pub struct NeteaseHydrationGate {
    session_key: Option<SessionKey>,
    attempts: u32,
    next_attempt_at_ms: u64,
    in_flight: bool,
    completed: bool,
}

impl NeteaseHydrationGate {
    pub fn try_begin(&mut self, music_u: &str, now_ms: u64) -> bool {
        let session_key = session_key(music_u);
        if self.session_key.as_ref() != Some(&session_key) {
            self.session_key = Some(session_key);
            self.attempts = 0;
            self.next_attempt_at_ms = 0;
            self.in_flight = false;
            self.completed = false;
        }

        if self.completed
            || self.in_flight
            || self.attempts >= MAX_ATTEMPTS
            || now_ms < self.next_attempt_at_ms
        {
            return false;
        }

        self.attempts += 1;
        self.in_flight = true;
        true
    }

    pub fn record_failure(&mut self, music_u: &str, now_ms: u64) {
        if !self.matches(music_u) {
            return;
        }

        self.in_flight = false;
        let cooldown = RETRY_COOLDOWN_MS.saturating_mul(u64::from(self.attempts.max(1)));
        self.next_attempt_at_ms = now_ms.saturating_add(cooldown);
    }

    pub fn record_success(&mut self, music_u: &str) {
        if self.matches(music_u) {
            self.in_flight = false;
            self.completed = true;
        }
    }

    pub fn record_abandoned(&mut self, music_u: &str) {
        if self.matches(music_u) {
            self.in_flight = false;
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn matches(&self, music_u: &str) -> bool {
        self.session_key.as_ref() == Some(&session_key(music_u))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn session_key(music_u: &str) -> SessionKey {
    let digest = Sha256::digest(music_u.as_bytes());
    let mut key = [0; 32];
    key.copy_from_slice(&digest);
    key
}

#[cfg(test)]
mod tests {
    use super::{NeteaseHydrationGate, MAX_ATTEMPTS, RETRY_COOLDOWN_MS};

    #[test]
    fn blocks_duplicate_attempt_while_request_is_in_flight() {
        let mut gate = NeteaseHydrationGate::default();

        assert!(gate.try_begin("session-a", 0));
        assert!(!gate.try_begin("session-a", RETRY_COOLDOWN_MS * 10));
    }

    #[test]
    fn enforces_increasing_cooldown_between_failures() {
        let mut gate = NeteaseHydrationGate::default();

        assert!(gate.try_begin("session-a", 0));
        gate.record_failure("session-a", 0);
        assert!(!gate.try_begin("session-a", RETRY_COOLDOWN_MS - 1));
        assert!(gate.try_begin("session-a", RETRY_COOLDOWN_MS));

        gate.record_failure("session-a", RETRY_COOLDOWN_MS);
        assert!(!gate.try_begin("session-a", RETRY_COOLDOWN_MS * 3 - 1));
        assert!(gate.try_begin("session-a", RETRY_COOLDOWN_MS * 3));
    }

    #[test]
    fn stops_after_maximum_attempts() {
        let mut gate = NeteaseHydrationGate::default();
        let mut now = 0;

        for attempt in 1..=MAX_ATTEMPTS {
            assert!(gate.try_begin("session-a", now));
            gate.record_failure("session-a", now);
            now += RETRY_COOLDOWN_MS * u64::from(attempt);
        }

        assert!(!gate.try_begin("session-a", u64::MAX));
    }

    #[test]
    fn new_cookie_resets_attempts_and_in_flight_state() {
        let mut gate = NeteaseHydrationGate::default();

        assert!(gate.try_begin("session-a", 0));
        assert!(gate.try_begin("session-b", 1));
    }

    #[test]
    fn success_stops_more_attempts_for_the_same_cookie() {
        let mut gate = NeteaseHydrationGate::default();

        assert!(gate.try_begin("session-a", 0));
        gate.record_success("session-a");
        assert!(!gate.try_begin("session-a", u64::MAX));
        assert!(gate.try_begin("session-b", 1));
    }

    #[test]
    fn explicit_reset_allows_a_reimported_cookie() {
        let mut gate = NeteaseHydrationGate::default();

        assert!(gate.try_begin("session-a", 0));
        gate.record_success("session-a");
        gate.reset();
        assert!(gate.try_begin("session-a", 1));
    }

    #[test]
    fn stale_completion_does_not_change_new_session() {
        let mut gate = NeteaseHydrationGate::default();

        assert!(gate.try_begin("session-a", 0));
        assert!(gate.try_begin("session-b", 1));
        gate.record_failure("session-a", 1);
        assert!(!gate.try_begin("session-b", RETRY_COOLDOWN_MS * 10));
    }
}
