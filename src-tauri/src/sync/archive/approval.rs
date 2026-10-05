use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const PREFIX: &str = "SYNC_PROTOCOL_UPGRADE_REQUIRED:";
static APPROVALS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncProtocolUpgrade {
    pub backend: String,
    pub target: String,
    pub fingerprint: String,
    pub source_protocol: u8,
    pub target_protocol: u8,
}

impl SyncProtocolUpgrade {
    pub fn new(backend: &str, target: String, content: &[u8], source_protocol: u8) -> Self {
        Self {
            backend: backend.into(),
            target,
            fingerprint: super::digest(content),
            source_protocol,
            target_protocol: 4,
        }
    }

    fn key(&self) -> AppResult<String> {
        if !matches!(self.backend.as_str(), "github" | "webdav")
            || self.target.is_empty()
            || self.target.len() > 4096
            || !super::valid_hash(&self.fingerprint)
            || !matches!(self.source_protocol, 0 | 3)
            || self.target_protocol != 4
        {
            return Err(AppError::Other(
                "Invalid sync protocol upgrade challenge".into(),
            ));
        }
        Ok(serde_json::to_string(self)?)
    }
}

pub fn require(challenge: &SyncProtocolUpgrade) -> AppResult<()> {
    let key = challenge.key()?;
    let approvals = APPROVALS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| AppError::Other("Sync protocol approval lock poisoned".into()))?;
    if approvals.contains(&key) {
        return Ok(());
    }
    Err(AppError::Other(format!("{PREFIX}{key}")))
}

// 每次同步都重读远端，再匹配内存许可，地址或内容变化不会继承旧许可
pub fn approve_verified(
    requested: &SyncProtocolUpgrade,
    current: &SyncProtocolUpgrade,
) -> AppResult<()> {
    let key = requested.key()?;
    if requested != current {
        return Err(AppError::Other(
            "Sync target or content changed; refresh the upgrade confirmation".into(),
        ));
    }
    APPROVALS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| AppError::Other("Sync protocol approval lock poisoned".into()))?
        .insert(key);
    Ok(())
}

pub fn consume(challenge: &SyncProtocolUpgrade) -> AppResult<()> {
    let key = challenge.key()?;
    APPROVALS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| AppError::Other("Sync protocol approval lock poisoned".into()))?
        .remove(&key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_is_bound_to_content_target_provider_and_versions() {
        let challenge =
            SyncProtocolUpgrade::new("github", uuid::Uuid::new_v4().to_string(), b"old", 3);
        assert!(require(&challenge).is_err());
        approve_verified(&challenge, &challenge).unwrap();
        require(&challenge).unwrap();
        let mut changed = challenge.clone();
        changed.fingerprint = super::super::digest(b"new");
        assert!(require(&changed).is_err());
        assert!(approve_verified(&challenge, &changed).is_err());
        changed = challenge.clone();
        changed.target.push_str("/other");
        assert!(require(&changed).is_err());
        changed = challenge.clone();
        changed.backend = "webdav".into();
        assert!(require(&changed).is_err());
        changed = challenge.clone();
        changed.source_protocol = 0;
        assert!(require(&changed).is_err());
        changed = challenge.clone();
        changed.target_protocol = 5;
        assert!(require(&changed).is_err());
        consume(&challenge).unwrap();
        assert!(require(&challenge).is_err());
    }
}
