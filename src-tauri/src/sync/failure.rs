// 同步失败里前端需要区分处理的几类：Display 是稳定代码，前端 parseSyncFailure 按代码本地化和重试
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncFailure {
    #[error("GITHUB_TOKEN_EXPIRED")]
    GitHubTokenExpired,
    #[error("GITHUB_RATE_LIMITED:{retry_at_ms}:{}", u8::from(*automatic))]
    GitHubRateLimited {
        status: u16,
        retry_at_ms: i64,
        automatic: bool,
    },
    #[error("WEBDAV_AUTH_FAILED")]
    WebDavAuth,
    #[error("WEBDAV_ACCESS_DENIED")]
    WebDavAccessDenied,
    #[error("WEBDAV_DIRECTORY_NOT_FOUND")]
    WebDavDirectoryNotFound,
    #[error("WEBDAV_NOT_DIRECTORY")]
    WebDavNotDirectory,
    /// 服务器既没有强 ETag 也给不了有限期的锁，没法安全地条件写入
    #[error("WEBDAV_MISSING_CONDITION")]
    WebDavMissingCondition,
}

/// 对齐 Android GitHubRateLimitException
pub const MIN_RETRY_DELAY_MS: i64 = 60_000;
const MIN_RETRY_LEAD_MS: i64 = 1_000;
/// 连续限流超过这个次数后不再自动重试，等用户手动同步
pub const MAX_AUTOMATIC_RETRIES: u32 = 3;
/// 退避只算到第 4 次（60/120/240/480 秒）
const MAX_TRACKED_ATTEMPTS: u32 = 4;

/// 失败响应是不是 GitHub 限流；是的话返回最早可以重试的时间（毫秒）
///
/// 429 一律算；403 只有带 Retry-After、剩余额度为 0，或正文提到 rate limit / abuse detection 时才算，
/// 其余 403 是权限问题
pub fn github_rate_limit_retry_at(
    status: u16,
    headers: &HeaderMap,
    body: &str,
    now_ms: i64,
) -> Option<i64> {
    let body = body.to_ascii_lowercase();
    let limited = status == 429
        || (status == 403
            && (headers.contains_key(RETRY_AFTER)
                || remaining_exhausted(headers)
                || body.contains("rate limit")
                || body.contains("abuse detection")));
    limited.then(|| github_rate_limit_resume_at(headers, now_ms))
}

/// 已经确定是限流（例如 GraphQL 的 RATE_LIMITED）时，按响应头算出重试时间
pub fn github_rate_limit_resume_at(headers: &HeaderMap, now_ms: i64) -> i64 {
    let future = |at: i64| (at > now_ms).then_some(at);
    let retry_after = header(headers, RETRY_AFTER.as_str())
        .and_then(|value| parse_retry_after(value, now_ms))
        .and_then(future);
    let reset_at = remaining_exhausted(headers)
        .then(|| header(headers, "x-ratelimit-reset"))
        .flatten()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .and_then(|seconds| seconds.checked_mul(1000))
        .and_then(future);
    retry_after
        .or(reset_at)
        .unwrap_or(now_ms + MIN_RETRY_DELAY_MS)
        .max(reset_at.unwrap_or(0))
        .max(now_ms + MIN_RETRY_LEAD_MS)
}

fn remaining_exhausted(headers: &HeaderMap) -> bool {
    header(headers, "x-ratelimit-remaining").is_some_and(|value| value.trim() == "0")
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Retry-After 可以是秒数，也可以是 HTTP 日期
fn parse_retry_after(value: &str, now_ms: i64) -> Option<i64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<i64>() {
        return (seconds >= 0).then(|| now_ms.saturating_add(seconds.saturating_mul(1000)));
    }
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| date.timestamp_millis())
}

/// 某个仓库的限流冷却，持久化后重启也不会立刻再撞限流（对齐 Android GitHubSyncCheckpoint）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubCooldown {
    pub target: String,
    pub status: u16,
    pub retry_at_ms: i64,
    pub attempts: u32,
}

impl GitHubCooldown {
    /// 记一次限流：退避 `60s << (attempts - 1)`，取服务端要求、上一次冷却和退避里最晚的时间
    pub fn record(
        previous: Option<&GitHubCooldown>,
        target: &str,
        status: u16,
        retry_at_ms: i64,
        now_ms: i64,
    ) -> Self {
        let previous = previous.filter(|cooldown| cooldown.target == target);
        let attempts = (previous.map_or(0, |cooldown| cooldown.attempts) + 1).min(MAX_TRACKED_ATTEMPTS);
        let backoff = MIN_RETRY_DELAY_MS << (attempts - 1);
        Self {
            target: target.to_string(),
            status,
            retry_at_ms: retry_at_ms
                .max(previous.map_or(0, |cooldown| cooldown.retry_at_ms))
                .max(now_ms + backoff),
            attempts,
        }
    }

    pub fn blocks(&self, target: &str, now_ms: i64) -> bool {
        self.target == target && self.retry_at_ms > now_ms
    }

    pub fn failure(&self) -> SyncFailure {
        SyncFailure::GitHubRateLimited {
            status: self.status,
            retry_at_ms: self.retry_at_ms,
            automatic: self.attempts <= MAX_AUTOMATIC_RETRIES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    const NOW: i64 = 1_800_000_000_000;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn exhausted_quota_waits_for_the_reset_time() {
        let reset = (NOW / 1000 + 120).to_string();
        let found = github_rate_limit_retry_at(
            403,
            &headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &reset)]),
            "",
            NOW,
        );
        assert_eq!(found, Some((NOW / 1000 + 120) * 1000));
    }

    #[test]
    fn retry_after_seconds_and_dates_are_honoured() {
        assert_eq!(
            github_rate_limit_retry_at(429, &headers(&[("retry-after", "30")]), "", NOW),
            Some(NOW + 30_000)
        );
        let date = chrono::DateTime::from_timestamp_millis(NOW + 90_000)
            .unwrap()
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        assert_eq!(
            github_rate_limit_retry_at(429, &headers(&[("retry-after", &date)]), "", NOW),
            Some(NOW + 90_000)
        );
    }

    #[test]
    fn secondary_limits_without_headers_wait_a_minute() {
        assert_eq!(
            github_rate_limit_retry_at(403, &HeaderMap::new(), "You have exceeded a secondary rate limit", NOW),
            Some(NOW + MIN_RETRY_DELAY_MS)
        );
        assert_eq!(
            github_rate_limit_retry_at(403, &HeaderMap::new(), "abuse detection mechanism", NOW),
            Some(NOW + MIN_RETRY_DELAY_MS)
        );
    }

    #[test]
    fn permission_failures_and_past_hints_are_not_misread() {
        assert_eq!(
            github_rate_limit_retry_at(403, &HeaderMap::new(), "Resource not accessible by integration", NOW),
            None
        );
        assert_eq!(github_rate_limit_retry_at(500, &HeaderMap::new(), "rate limit", NOW), None);
        // 已经过去的 reset 不算数，退回默认一分钟
        let stale = (NOW / 1000 - 10).to_string();
        assert_eq!(
            github_rate_limit_retry_at(
                403,
                &headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &stale)]),
                "",
                NOW,
            ),
            Some(NOW + MIN_RETRY_DELAY_MS)
        );
        // Reset 只在额度耗尽时才参考
        let reset = (NOW / 1000 + 600).to_string();
        assert_eq!(
            github_rate_limit_retry_at(429, &headers(&[("x-ratelimit-reset", &reset)]), "", NOW),
            Some(NOW + MIN_RETRY_DELAY_MS)
        );
    }

    #[test]
    fn consecutive_limits_back_off_and_stop_retrying_automatically() {
        let mut cooldown: Option<GitHubCooldown> = None;
        let mut delays = Vec::new();
        let mut automatic = Vec::new();
        for _ in 0..5 {
            let next = GitHubCooldown::record(cooldown.as_ref(), "owner/repo", 403, NOW + 1_000, NOW);
            delays.push((next.retry_at_ms - NOW) / 1000);
            automatic.push(matches!(next.failure(), SyncFailure::GitHubRateLimited { automatic: true, .. }));
            cooldown = Some(GitHubCooldown { retry_at_ms: NOW, ..next });
        }
        assert_eq!(delays, [60, 120, 240, 480, 480]);
        assert_eq!(automatic, [true, true, true, false, false]);
    }

    #[test]
    fn cooldowns_belong_to_one_repository() {
        let first = GitHubCooldown::record(None, "owner/old", 429, NOW + 600_000, NOW);
        let other = GitHubCooldown::record(Some(&first), "owner/new", 429, NOW + 1_000, NOW);
        assert_eq!(other.attempts, 1);
        assert_eq!(other.retry_at_ms, NOW + MIN_RETRY_DELAY_MS);
        assert!(first.blocks("owner/old", NOW));
        assert!(!first.blocks("owner/new", NOW));
        assert!(!first.blocks("owner/old", first.retry_at_ms));
    }

    #[test]
    fn failure_codes_are_stable() {
        assert_eq!(SyncFailure::GitHubTokenExpired.to_string(), "GITHUB_TOKEN_EXPIRED");
        assert_eq!(
            SyncFailure::GitHubRateLimited { status: 429, retry_at_ms: 42, automatic: false }.to_string(),
            "GITHUB_RATE_LIMITED:42:0"
        );
    }
}
