use super::archive;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub(super) const GRACE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Candidate {
    pub path: String,
    pub etag: String,
    first_seen_ms: i64,
    observed_age_ms: i64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Journal {
    version: u8,
    wall_ms: i64,
    uptime_ms: i64,
    candidates: Vec<Candidate>,
}

impl Journal {
    pub fn observe(
        &self,
        entries: &BTreeMap<String, String>,
        protected: &HashSet<String>,
        wall: i64,
        uptime: i64,
    ) -> Self {
        let stable = self.valid()
            && self.wall_ms > 0
            && wall >= self.wall_ms
            && uptime >= self.uptime_ms
            && (wall - self.wall_ms).abs_diff(uptime - self.uptime_ms) <= 5 * 60 * 1000;
        let prior: BTreeMap<_, _> = if stable {
            self.candidates
                .iter()
                .map(|entry| (entry.path.as_str(), entry))
                .collect()
        } else {
            BTreeMap::new()
        };
        let mut candidates = Vec::new();
        for (path, etag) in entries
            .iter()
            .filter(|(path, _)| !protected.contains(*path))
        {
            let previous = prior
                .get(path.as_str())
                .filter(|previous| previous.etag == *etag);
            let candidate = if let Some(previous) = previous {
                Candidate {
                    path: path.clone(),
                    etag: etag.clone(),
                    first_seen_ms: previous.first_seen_ms,
                    observed_age_ms: previous.observed_age_ms.saturating_add(
                        (wall - self.wall_ms)
                            .min(uptime - self.uptime_ms)
                            .min(GRACE_MS - previous.observed_age_ms),
                    ),
                }
            } else {
                Candidate {
                    path: path.clone(),
                    etag: etag.clone(),
                    first_seen_ms: wall,
                    observed_age_ms: 0,
                }
            };
            candidates.push(candidate);
        }
        candidates.sort_by(|left, right| {
            left.first_seen_ms
                .cmp(&right.first_seen_ms)
                .then_with(|| left.path.cmp(&right.path))
        });
        candidates.truncate(1024);
        Self {
            version: 1,
            wall_ms: wall,
            uptime_ms: uptime,
            candidates,
        }
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.wall_ms >= 0
            && self.uptime_ms >= 0
            && self.candidates.len() <= 1024
            && self
                .candidates
                .iter()
                .map(|entry| &entry.path)
                .collect::<HashSet<_>>()
                .len()
                == self.candidates.len()
            && self.candidates.iter().all(|entry| {
                archive::canonical_object_path(&entry.path)
                    && super::webdav_archive::strong_etag(&entry.etag)
                    && entry.first_seen_ms > 0
                    && entry.first_seen_ms <= self.wall_ms
                    && entry.observed_age_ms >= 0
                    && entry.observed_age_ms <= GRACE_MS.min(self.wall_ms - entry.first_seen_ms)
            })
    }
    pub fn eligible(&self) -> Vec<Candidate> {
        self.candidates
            .iter()
            .filter(|entry| entry.observed_age_ms >= GRACE_MS)
            .take(32)
            .cloned()
            .collect()
    }
    pub fn remove(&mut self, path: &str) {
        self.candidates.retain(|entry| entry.path != path);
    }
}

pub(super) fn uptime_ms() -> i64 {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetTickCount64() -> u64;
        }
        // 系统启动后的单调时钟让应用重启后也能安全累计观察期
        unsafe { GetTickCount64().min(i64::MAX as u64) as i64 }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/uptime") {
            if let Some(seconds) = content
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                return (seconds * 1000.0).min(i64::MAX as f64) as i64;
            }
        }
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis()
            .min(i64::MAX as u128) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> (String, String) {
        (
            format!("neriplayer-sync-v4-{}.zst", "a".repeat(64)),
            "\"a\"".into(),
        )
    }
    #[test]
    fn seven_day_retirement_resets_on_reference_identity_or_clock_changes() {
        let (path, etag) = entry();
        let entries = BTreeMap::from([(path.clone(), etag.clone())]);
        let empty = HashSet::new();
        let initial = Journal::default().observe(&entries, &empty, 1_000, 10_000);
        assert!(initial.eligible().is_empty());
        let ready = initial.observe(&entries, &empty, 1_000 + GRACE_MS, 10_000 + GRACE_MS);
        assert_eq!(ready.eligible().len(), 1);
        assert!(initial
            .observe(&entries, &empty, 1_000 + GRACE_MS, 10_000 + 100)
            .eligible()
            .is_empty());
        assert!(ready
            .observe(
                &entries,
                &HashSet::from([path.clone()]),
                2_000 + GRACE_MS,
                11_000 + GRACE_MS
            )
            .eligible()
            .is_empty());
        let replaced = BTreeMap::from([(path, "\"other\"".into())]);
        assert!(ready
            .observe(&replaced, &empty, 2_000 + GRACE_MS, 11_000 + GRACE_MS)
            .eligible()
            .is_empty());
        assert!(ready
            .observe(&entries, &empty, 500, 10)
            .eligible()
            .is_empty());
    }
}
