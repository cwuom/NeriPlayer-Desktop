use std::time::{Duration, Instant};

const PROFILE_RETRY_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthPlatform {
    Netease,
    Bilibili,
    Youtube,
}

impl AuthPlatform {
    pub const ALL: [Self; 3] = [Self::Netease, Self::Bilibili, Self::Youtube];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "netease" => Some(Self::Netease),
            "bilibili" => Some(Self::Bilibili),
            "youtube" => Some(Self::Youtube),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Netease => "netease",
            Self::Bilibili => "bilibili",
            Self::Youtube => "youtube",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Netease => 0,
            Self::Bilibili => 1,
            Self::Youtube => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AuthOperation {
    pub platform: AuthPlatform,
    generation: u64,
}

impl AuthOperation {
    pub fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Default)]
pub struct AuthOperations {
    generations: [u64; 3],
    profile_attempts: [Option<ProfileAttempt>; 3],
    pending_logins: [Option<u64>; 3],
}

struct ProfileAttempt {
    generation: u64,
    running: bool,
    retry_at: Option<Instant>,
}

impl AuthOperations {
    pub fn begin(&mut self, platform: AuthPlatform) -> AuthOperation {
        let generation = &mut self.generations[platform.index()];
        *generation = generation
            .checked_add(1)
            .expect("Authentication generation overflow");
        self.pending_logins[platform.index()] = None;
        self.current(platform)
    }

    pub fn begin_login(&mut self, platform: AuthPlatform) -> AuthOperation {
        let operation = self.begin(platform);
        self.pending_logins[platform.index()] = Some(operation.generation);
        operation
    }

    pub fn finish_login(&mut self, operation: AuthOperation) {
        let pending = &mut self.pending_logins[operation.platform.index()];
        if *pending == Some(operation.generation) {
            *pending = None;
        }
    }

    pub fn invalidate_all(&mut self) -> [AuthOperation; 3] {
        AuthPlatform::ALL.map(|platform| self.begin(platform))
    }

    pub fn current(&self, platform: AuthPlatform) -> AuthOperation {
        AuthOperation {
            platform,
            generation: self.generations[platform.index()],
        }
    }

    pub fn is_current(&self, operation: AuthOperation) -> bool {
        self.generations[operation.platform.index()] == operation.generation
    }

    pub fn claim_profile_refresh(
        &mut self,
        platform: AuthPlatform,
        now: Instant,
    ) -> Option<AuthOperation> {
        let operation = self.current(platform);
        if self.pending_logins[platform.index()] == Some(operation.generation) {
            return None;
        }
        let attempted = &mut self.profile_attempts[platform.index()];
        if attempted
            .as_ref()
            .filter(|attempt| attempt.generation == operation.generation)
            .is_some_and(|attempt| {
                attempt.running || attempt.retry_at.is_none_or(|retry_at| now < retry_at)
            })
        {
            return None;
        }
        *attempted = Some(ProfileAttempt {
            generation: operation.generation,
            running: true,
            retry_at: None,
        });
        Some(operation)
    }

    pub fn finish_profile_refresh(
        &mut self,
        operation: AuthOperation,
        success: bool,
        now: Instant,
    ) {
        if !self.is_current(operation) {
            return;
        }
        if let Some(attempt) = self.profile_attempts[operation.platform.index()]
            .as_mut()
            .filter(|attempt| attempt.generation == operation.generation)
        {
            attempt.running = false;
            attempt.retry_at = (!success).then(|| now + PROFILE_RETRY_COOLDOWN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthOperations, AuthPlatform};

    #[test]
    fn later_login_logout_or_import_prevents_an_old_commit() {
        for platform in AuthPlatform::ALL {
            let mut operations = AuthOperations::default();
            let old = operations.begin(platform);
            let current = operations.begin(platform);
            assert!(!operations.is_current(old));
            assert!(operations.is_current(current));
        }
    }

    #[test]
    fn updating_one_platform_keeps_other_platform_operations_current() {
        let mut operations = AuthOperations::default();
        let netease = operations.begin(AuthPlatform::Netease);
        operations.begin(AuthPlatform::Bilibili);
        assert!(operations.is_current(netease));
        operations.invalidate_all();
        assert!(!operations.is_current(netease));
    }

    #[test]
    fn profile_requests_deduplicate_and_retry_failures_only_after_cooldown() {
        let now = std::time::Instant::now();
        let mut operations = AuthOperations::default();
        let old = operations
            .claim_profile_refresh(AuthPlatform::Netease, now)
            .unwrap();
        assert!(operations
            .claim_profile_refresh(AuthPlatform::Netease, now)
            .is_none());
        operations.finish_profile_refresh(old, false, now);
        assert!(operations
            .claim_profile_refresh(
                AuthPlatform::Netease,
                now + super::PROFILE_RETRY_COOLDOWN - std::time::Duration::from_millis(1)
            )
            .is_none());
        let retry = operations
            .claim_profile_refresh(AuthPlatform::Netease, now + super::PROFILE_RETRY_COOLDOWN)
            .unwrap();
        operations.finish_profile_refresh(retry, true, now + super::PROFILE_RETRY_COOLDOWN);
        assert!(operations
            .claim_profile_refresh(
                AuthPlatform::Netease,
                now + super::PROFILE_RETRY_COOLDOWN * 2
            )
            .is_none());
        operations.begin(AuthPlatform::Netease);
        assert!(!operations.is_current(old));
        assert!(operations
            .claim_profile_refresh(AuthPlatform::Netease, now)
            .is_some());
    }

    #[test]
    fn completing_an_old_profile_cannot_release_a_new_session_request() {
        let now = std::time::Instant::now();
        let mut operations = AuthOperations::default();
        let old = operations
            .claim_profile_refresh(AuthPlatform::Bilibili, now)
            .unwrap();
        operations.begin(AuthPlatform::Bilibili);
        operations
            .claim_profile_refresh(AuthPlatform::Bilibili, now)
            .unwrap();
        operations.finish_profile_refresh(old, false, now);
        assert!(operations
            .claim_profile_refresh(AuthPlatform::Bilibili, now + super::PROFILE_RETRY_COOLDOWN)
            .is_none());
    }

    #[test]
    fn pending_login_cannot_refresh_the_previous_saved_account_under_a_new_generation() {
        let now = std::time::Instant::now();
        let mut operations = AuthOperations::default();
        let old = operations.begin_login(AuthPlatform::Netease);
        assert!(operations
            .claim_profile_refresh(AuthPlatform::Netease, now)
            .is_none());
        let current = operations.begin_login(AuthPlatform::Netease);
        operations.finish_login(old);
        assert!(operations
            .claim_profile_refresh(AuthPlatform::Netease, now)
            .is_none());
        operations.finish_login(current);
        assert!(operations
            .claim_profile_refresh(AuthPlatform::Netease, now)
            .is_some());
    }
}
