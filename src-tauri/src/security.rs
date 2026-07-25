// Release 构建使用系统凭据存储；Debug 构建完全使用随机路径明文文件
// 各类凭据使用独立目录，删除 Cookie 时不会影响同步凭据

const SERVICE_NAME: &str = "moe.ouom.neriplayer.desktop";

pub const AUTH_STATE_KEY: &str = "auth-state-v1";
pub const GITHUB_TOKEN_KEY: &str = "github-token-v1";
pub const WEBDAV_PASSWORD_KEY: &str = "webdav-password-v1";

#[cfg(any(not(debug_assertions), test, windows))]
const CHUNK_MANIFEST_PREFIX: &str = "neriplayer-keyring-chunks:";
#[cfg(any(not(debug_assertions), test, windows))]
const CHUNK_MANIFEST_VERSION: u8 = 1;
#[cfg(any(not(debug_assertions), test, windows))]
// Windows stores keyring passwords as UTF-16 in a 2560-byte credential blob.
// Keep headroom below that platform limit while also bounding UTF-8 allocations.
const KEYRING_CHUNK_UTF8_BYTES: usize = 2048;
#[cfg(any(not(debug_assertions), test, windows))]
const KEYRING_CHUNK_UTF16_UNITS: usize = 1024;
#[cfg(any(not(debug_assertions), test, windows))]
const MAX_CHUNKED_SECRET_BYTES: usize = 1024 * 1024;
#[cfg(any(not(debug_assertions), test, windows))]
const MAX_CHUNK_COUNT: usize = 2048;

#[cfg(any(not(debug_assertions), test, windows))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct KeyringChunkManifest {
    version: u8,
    generation: String,
    count: usize,
    length: usize,
}

#[cfg(any(not(debug_assertions), test, windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkStorageError {
    InvalidChunkSize,
    SecretTooLarge,
    UnsupportedVersion,
    InvalidGeneration,
    InvalidChunkCount,
    InvalidLength,
    ManifestSerialization,
    ManifestDeserialization,
    ChunkCountMismatch,
    ChunkTooLarge,
    ChunkLengthMismatch,
}

#[cfg(any(not(debug_assertions), test, windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyringSecretDeletion {
    Deleted,
    ChunkDeletionFailed,
    ManifestDeletionFailed,
}

#[cfg(any(not(debug_assertions), test, windows))]
fn validate_chunk_manifest(manifest: &KeyringChunkManifest) -> Result<(), ChunkStorageError> {
    if manifest.version != CHUNK_MANIFEST_VERSION {
        return Err(ChunkStorageError::UnsupportedVersion);
    }
    let generation = uuid::Uuid::parse_str(&manifest.generation)
        .map_err(|_| ChunkStorageError::InvalidGeneration)?;
    if generation.hyphenated().to_string() != manifest.generation {
        return Err(ChunkStorageError::InvalidGeneration);
    }
    if manifest.length > MAX_CHUNKED_SECRET_BYTES {
        return Err(ChunkStorageError::InvalidLength);
    }
    if manifest.count > MAX_CHUNK_COUNT {
        return Err(ChunkStorageError::InvalidChunkCount);
    }
    if (manifest.length == 0) != (manifest.count == 0) {
        return Err(ChunkStorageError::InvalidChunkCount);
    }
    if manifest.length > 0 {
        let capacity = manifest
            .count
            .checked_mul(KEYRING_CHUNK_UTF8_BYTES)
            .ok_or(ChunkStorageError::InvalidChunkCount)?;
        if manifest.length > capacity || manifest.count > manifest.length {
            return Err(ChunkStorageError::InvalidChunkCount);
        }
    }
    Ok(())
}

#[cfg(any(not(debug_assertions), test, windows))]
fn encode_chunk_manifest(manifest: &KeyringChunkManifest) -> Result<String, ChunkStorageError> {
    validate_chunk_manifest(manifest)?;
    let payload =
        serde_json::to_string(manifest).map_err(|_| ChunkStorageError::ManifestSerialization)?;
    Ok(format!("{CHUNK_MANIFEST_PREFIX}{payload}"))
}

#[cfg(any(not(debug_assertions), test, windows))]
fn parse_chunk_manifest(stored: &str) -> Result<Option<KeyringChunkManifest>, ChunkStorageError> {
    let Some(payload) = stored.strip_prefix(CHUNK_MANIFEST_PREFIX) else {
        return Ok(None);
    };
    let manifest: KeyringChunkManifest =
        serde_json::from_str(payload).map_err(|_| ChunkStorageError::ManifestDeserialization)?;
    validate_chunk_manifest(&manifest)?;
    Ok(Some(manifest))
}

#[cfg(any(not(debug_assertions), test, windows))]
fn split_utf8_chunks(value: &str) -> Result<Vec<String>, ChunkStorageError> {
    split_utf8_chunks_with_limits(value, KEYRING_CHUNK_UTF8_BYTES, KEYRING_CHUNK_UTF16_UNITS)
}

#[cfg(any(not(debug_assertions), test, windows))]
fn split_utf8_chunks_with_limits(
    value: &str,
    max_utf8_bytes: usize,
    max_utf16_units: usize,
) -> Result<Vec<String>, ChunkStorageError> {
    if max_utf8_bytes == 0
        || max_utf8_bytes > KEYRING_CHUNK_UTF8_BYTES
        || max_utf16_units == 0
        || max_utf16_units > KEYRING_CHUNK_UTF16_UNITS
    {
        return Err(ChunkStorageError::InvalidChunkSize);
    }
    if value.len() > MAX_CHUNKED_SECRET_BYTES {
        return Err(ChunkStorageError::SecretTooLarge);
    }

    let mut chunks = Vec::new();
    let mut chunk_start = 0usize;
    let mut chunk_utf8_bytes = 0usize;
    let mut chunk_utf16_units = 0usize;
    for (index, character) in value.char_indices() {
        let character_utf8_bytes = character.len_utf8();
        let character_utf16_units = character.len_utf16();
        if character_utf8_bytes > max_utf8_bytes || character_utf16_units > max_utf16_units {
            return Err(ChunkStorageError::ChunkTooLarge);
        }
        if chunk_utf8_bytes + character_utf8_bytes > max_utf8_bytes
            || chunk_utf16_units + character_utf16_units > max_utf16_units
        {
            if index == chunk_start {
                return Err(ChunkStorageError::ChunkTooLarge);
            }
            chunks.push(value[chunk_start..index].to_owned());
            chunk_start = index;
            chunk_utf8_bytes = 0;
            chunk_utf16_units = 0;
        }
        chunk_utf8_bytes += character_utf8_bytes;
        chunk_utf16_units += character_utf16_units;
    }
    if chunk_start < value.len() {
        chunks.push(value[chunk_start..].to_owned());
    }
    if chunks.len() > MAX_CHUNK_COUNT {
        return Err(ChunkStorageError::InvalidChunkCount);
    }
    Ok(chunks)
}

#[cfg(any(not(debug_assertions), test, windows))]
fn manifest_for_chunks(
    generation: String,
    value: &str,
    chunks: &[String],
) -> Result<KeyringChunkManifest, ChunkStorageError> {
    let manifest = KeyringChunkManifest {
        version: CHUNK_MANIFEST_VERSION,
        generation,
        count: chunks.len(),
        length: value.len(),
    };
    validate_chunk_manifest(&manifest)?;
    Ok(manifest)
}

#[cfg(any(not(debug_assertions), test, windows))]
fn assemble_chunked_secret(
    manifest: &KeyringChunkManifest,
    chunks: &[String],
) -> Result<String, ChunkStorageError> {
    validate_chunk_manifest(manifest)?;
    if chunks.len() != manifest.count {
        return Err(ChunkStorageError::ChunkCountMismatch);
    }

    let mut length = 0usize;
    for chunk in chunks {
        if chunk.is_empty()
            || chunk.len() > KEYRING_CHUNK_UTF8_BYTES
            || chunk.encode_utf16().count() > KEYRING_CHUNK_UTF16_UNITS
        {
            return Err(ChunkStorageError::ChunkTooLarge);
        }
        length = length
            .checked_add(chunk.len())
            .ok_or(ChunkStorageError::ChunkLengthMismatch)?;
    }
    if length != manifest.length {
        return Err(ChunkStorageError::ChunkLengthMismatch);
    }

    let mut value = String::with_capacity(length);
    for chunk in chunks {
        value.push_str(chunk);
    }
    Ok(value)
}

#[cfg(any(not(debug_assertions), test, windows))]
fn keyring_chunk_key(key: &str, generation: &str, index: usize) -> String {
    format!("{key}.chunks.v{CHUNK_MANIFEST_VERSION}.{generation}.{index}")
}

#[cfg(any(not(debug_assertions), test, windows))]
fn delete_keyring_secret_with<DeleteChunks, DeleteManifest>(
    key: &str,
    manifest: Option<&KeyringChunkManifest>,
    delete_chunks: DeleteChunks,
    delete_manifest: DeleteManifest,
) -> KeyringSecretDeletion
where
    DeleteChunks: FnOnce(&str, &KeyringChunkManifest) -> bool,
    DeleteManifest: FnOnce(&str) -> bool,
{
    if let Some(manifest) = manifest {
        if !delete_chunks(key, manifest) {
            return KeyringSecretDeletion::ChunkDeletionFailed;
        }
    }
    if !delete_manifest(key) {
        return KeyringSecretDeletion::ManifestDeletionFailed;
    }
    KeyringSecretDeletion::Deleted
}

/// 从当前构建配置的凭据存储读取秘密值
pub fn get_secret(key: &str) -> Option<String> {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::get(key)
    }

    #[cfg(not(debug_assertions))]
    {
        get_keyring_secret(key)
    }
}

/// 写入当前构建配置的凭据存储，失败时返回 false
pub fn set_secret(key: &str, value: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::set(key, value)
    }

    #[cfg(not(debug_assertions))]
    {
        set_keyring_secret(key, value)
    }
}

/// 删除当前构建配置的凭据存储中的秘密值
pub fn delete_secret(key: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::delete(key)
    }

    #[cfg(not(debug_assertions))]
    {
        delete_keyring_secret(key)
    }
}

/// Debug 构建中检查指定凭据是否存在，不会访问系统钥匙串
pub fn debug_secret_exists(key: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::exists(key)
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = key;
        false
    }
}

#[cfg(any(not(debug_assertions), windows))]
static KEYRING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(any(not(debug_assertions), windows))]
fn get_keyring_secret(key: &str) -> Option<String> {
    let _guard = KEYRING_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if key != AUTH_STATE_KEY {
        return match get_keyring_value(key) {
            Ok(value) => value,
            Err(error) => {
                log::error!(target: "security", "Failed to read credential from keyring: {error:?}");
                None
            }
        };
    }

    let stored = match get_keyring_value(key) {
        Ok(Some(stored)) => stored,
        Ok(None) => return None,
        Err(error) => {
            log::error!(target: "security", "Failed to read authentication manifest: {error:?}");
            return None;
        }
    };
    if stored.is_empty() {
        return None;
    }
    let manifest = match parse_chunk_manifest(&stored) {
        Ok(None) => return Some(stored),
        Ok(Some(manifest)) => manifest,
        Err(error) => {
            log::error!(target: "security", "Invalid authentication chunk manifest: {error:?}");
            return None;
        }
    };

    let mut chunks = Vec::with_capacity(manifest.count);
    for index in 0..manifest.count {
        let chunk_key = keyring_chunk_key(key, &manifest.generation, index);
        let chunk = match get_keyring_value(&chunk_key) {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                log::error!(
                    target: "security",
                    "Authentication credential chunk {index}/{} is missing",
                    manifest.count
                );
                return None;
            }
            Err(error) => {
                log::error!(
                    target: "security",
                    "Failed to read authentication credential chunk {index}/{}: {error:?}",
                    manifest.count
                );
                return None;
            }
        };
        chunks.push(chunk);
    }
    match assemble_chunked_secret(&manifest, &chunks) {
        Ok(value) => Some(value),
        Err(error) => {
            log::error!(target: "security", "Authentication credential chunks are invalid: {error:?}");
            None
        }
    }
}

#[cfg(any(not(debug_assertions), windows))]
fn get_keyring_value(key: &str) -> Result<Option<String>, keyring::Error> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(any(not(debug_assertions), windows))]
fn set_keyring_secret(key: &str, value: &str) -> bool {
    let _guard = KEYRING_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if key != AUTH_STATE_KEY {
        return set_keyring_value(key, value);
    }

    let chunks = match split_utf8_chunks(value) {
        Ok(chunks) => chunks,
        Err(error) => {
            log::error!(target: "security", "Authentication credentials cannot be chunked: {error:?}");
            return false;
        }
    };
    let manifest = match manifest_for_chunks(
        uuid::Uuid::new_v4().hyphenated().to_string(),
        value,
        &chunks,
    ) {
        Ok(manifest) => manifest,
        Err(error) => {
            log::error!(target: "security", "Authentication chunk manifest is invalid: {error:?}");
            return false;
        }
    };
    let encoded_manifest = match encode_chunk_manifest(&manifest) {
        Ok(encoded) => encoded,
        Err(error) => {
            log::error!(target: "security", "Authentication chunk manifest cannot be encoded: {error:?}");
            return false;
        }
    };

    let previous_value = match get_keyring_value(key) {
        Ok(value) => value,
        Err(error) => {
            log::error!(target: "security", "Failed to read the previous authentication manifest: {error:?}");
            return false;
        }
    };
    let previous_manifest = previous_value.as_deref().and_then(|stored| {
        match parse_chunk_manifest(stored) {
            Ok(manifest) => manifest,
            Err(error) => {
                log::warn!(target: "security", "Previous authentication chunk manifest is invalid: {error:?}");
                None
            }
        }
    });

    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_key = keyring_chunk_key(key, &manifest.generation, index);
        if !set_keyring_value(&chunk_key, chunk) {
            log::error!(target: "security", "Failed to write authentication credential chunk {index}");
            let _ = delete_keyring_chunks(key, &manifest);
            return false;
        }
    }

    if !set_keyring_value(key, &encoded_manifest) {
        log::error!(target: "security", "Failed to commit authentication chunk manifest");
        let rollback_succeeded = previous_value.as_deref().map_or_else(
            || delete_keyring_value(key),
            |value| set_keyring_value(key, value),
        );
        if rollback_succeeded {
            let _ = delete_keyring_chunks(key, &manifest);
        } else {
            log::error!(
                target: "security",
                "Failed to restore the previous authentication manifest; retaining new chunks"
            );
        }
        return false;
    }

    if let Some(previous_manifest) = previous_manifest {
        if previous_manifest.generation != manifest.generation
            && !delete_keyring_chunks(key, &previous_manifest)
        {
            log::warn!(target: "security", "Failed to clean up previous authentication chunks");
        }
    }
    true
}

#[cfg(any(not(debug_assertions), windows))]
fn set_keyring_value(key: &str, value: &str) -> bool {
    let Ok(entry) = keyring::Entry::new(SERVICE_NAME, key) else {
        return false;
    };
    entry.set_password(value).is_ok()
}

#[cfg(any(not(debug_assertions), windows))]
fn delete_keyring_secret(key: &str) -> bool {
    let _guard = KEYRING_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if key != AUTH_STATE_KEY {
        return delete_keyring_value(key);
    }

    let stored = match get_keyring_value(key) {
        Ok(Some(stored)) => stored,
        Ok(None) => return true,
        Err(error) => {
            log::error!(target: "security", "Failed to read authentication manifest before deletion: {error:?}");
            return false;
        }
    };
    let manifest = match parse_chunk_manifest(&stored) {
        Ok(manifest) => manifest,
        Err(error) => {
            log::error!(target: "security", "Authentication chunk manifest is invalid during deletion: {error:?}");
            return false;
        }
    };

    match delete_keyring_secret_with(
        key,
        manifest.as_ref(),
        delete_keyring_chunks,
        delete_keyring_value,
    ) {
        KeyringSecretDeletion::Deleted => true,
        KeyringSecretDeletion::ChunkDeletionFailed => {
            log::error!(target: "security", "Failed to delete authentication chunks; retaining the manifest for retry");
            false
        }
        KeyringSecretDeletion::ManifestDeletionFailed => {
            log::error!(target: "security", "Failed to delete authentication manifest after clearing its chunks");
            false
        }
    }
}

#[cfg(any(not(debug_assertions), windows))]
fn delete_keyring_chunks(key: &str, manifest: &KeyringChunkManifest) -> bool {
    if validate_chunk_manifest(manifest).is_err() {
        return false;
    }
    let mut success = true;
    for index in 0..manifest.count {
        let chunk_key = keyring_chunk_key(key, &manifest.generation, index);
        success &= delete_keyring_value(&chunk_key);
    }
    success
}

#[cfg(any(not(debug_assertions), windows))]
fn delete_keyring_value(key: &str) -> bool {
    let Ok(entry) = keyring::Entry::new(SERVICE_NAME, key) else {
        return false;
    };
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => return true,
        Err(_) => {}
    }
    // 某些后端只支持覆盖值，清空后也不会再携带可用凭据
    entry.set_password("").is_ok()
}

#[cfg(test)]
mod chunk_storage_tests {
    use super::*;
    use std::cell::Cell;

    const GENERATION: &str = "123e4567-e89b-42d3-a456-426614174000";

    #[test]
    fn utf8_chunks_round_trip_without_splitting_code_points() {
        let value = format!("prefix-{}-suffix", "你🙂é".repeat(700));
        let chunks = split_utf8_chunks(&value).expect("split secret");

        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| !chunk.is_empty()
            && chunk.len() <= KEYRING_CHUNK_UTF8_BYTES
            && chunk.encode_utf16().count() <= KEYRING_CHUNK_UTF16_UNITS));
        let manifest =
            manifest_for_chunks(GENERATION.into(), &value, &chunks).expect("create manifest");
        assert_eq!(
            assemble_chunked_secret(&manifest, &chunks).expect("assemble secret"),
            value
        );
    }

    #[test]
    fn small_chunk_limit_still_preserves_utf8_boundaries() {
        let value = "ab你🙂cd";
        let chunks = split_utf8_chunks_with_limits(value, 5, 3).expect("split small chunks");

        assert!(chunks.iter().all(|chunk| chunk.len() <= 5));
        assert!(chunks.iter().all(|chunk| chunk.encode_utf16().count() <= 3));
        assert_eq!(chunks.concat(), value);
    }

    #[test]
    fn ascii_heavy_auth_json_respects_windows_utf16_blob_limit() {
        let value = format!(
            r#"{{"netease":{{"cookies":[{{"name":"MUSIC_U","value":"{}","domain":"music.163.com"}}],"user_id":1,"nickname":null,"avatar_url":null}},"bilibili":null,"youtube":null}}"#,
            "a".repeat(12_000)
        );
        let chunks = split_utf8_chunks(&value).expect("split ASCII-heavy auth JSON");

        assert!(chunks.len() > 10);
        assert!(chunks.iter().all(|chunk| {
            chunk.len() <= KEYRING_CHUNK_UTF8_BYTES
                && chunk.encode_utf16().count() <= KEYRING_CHUNK_UTF16_UNITS
                && chunk.encode_utf16().count() * 2 <= 2048
        }));
        let manifest =
            manifest_for_chunks(GENERATION.into(), &value, &chunks).expect("create manifest");
        assert_eq!(
            assemble_chunked_secret(&manifest, &chunks).expect("assemble auth JSON"),
            value
        );
    }

    #[test]
    fn manifest_round_trip_keeps_legacy_inline_values_compatible() {
        let value = r#"{"youtube":{"cookies":["legacy"]}}"#;
        assert_eq!(parse_chunk_manifest(value), Ok(None));

        let chunks = split_utf8_chunks(value).expect("split secret");
        let manifest =
            manifest_for_chunks(GENERATION.into(), value, &chunks).expect("create manifest");
        let encoded = encode_chunk_manifest(&manifest).expect("encode manifest");

        assert_eq!(parse_chunk_manifest(&encoded), Ok(Some(manifest)));
    }

    #[test]
    fn empty_secret_uses_a_zero_chunk_manifest() {
        let chunks = split_utf8_chunks("").expect("split empty secret");
        let manifest =
            manifest_for_chunks(GENERATION.into(), "", &chunks).expect("create empty manifest");

        assert!(chunks.is_empty());
        assert_eq!(manifest.count, 0);
        assert_eq!(manifest.length, 0);
        assert_eq!(
            assemble_chunked_secret(&manifest, &chunks).expect("assemble empty secret"),
            ""
        );
    }

    #[test]
    fn assembly_rejects_count_and_length_mismatches() {
        let value = "credential-json";
        let chunks = split_utf8_chunks(value).expect("split secret");
        let manifest =
            manifest_for_chunks(GENERATION.into(), value, &chunks).expect("create manifest");

        assert_eq!(
            assemble_chunked_secret(&manifest, &[]),
            Err(ChunkStorageError::ChunkCountMismatch)
        );

        let mut wrong_length = manifest;
        wrong_length.length += 1;
        assert_eq!(
            assemble_chunked_secret(&wrong_length, &chunks),
            Err(ChunkStorageError::ChunkLengthMismatch)
        );
    }

    #[test]
    fn manifest_parser_rejects_invalid_generation_and_shape() {
        let invalid_generation = format!(
            r#"{CHUNK_MANIFEST_PREFIX}{{"version":1,"generation":"../old","count":1,"length":1}}"#
        );
        assert_eq!(
            parse_chunk_manifest(&invalid_generation),
            Err(ChunkStorageError::InvalidGeneration)
        );

        let invalid_shape = format!(
            r#"{CHUNK_MANIFEST_PREFIX}{{"version":1,"generation":"{GENERATION}","count":0,"length":1}}"#
        );
        assert_eq!(
            parse_chunk_manifest(&invalid_shape),
            Err(ChunkStorageError::InvalidChunkCount)
        );

        let unknown_field = format!(
            r#"{CHUNK_MANIFEST_PREFIX}{{"version":1,"generation":"{GENERATION}","count":0,"length":0,"extra":true}}"#
        );
        assert_eq!(
            parse_chunk_manifest(&unknown_field),
            Err(ChunkStorageError::ManifestDeserialization)
        );
    }

    #[test]
    fn failed_chunk_deletion_retains_manifest_for_retry() {
        let chunks = split_utf8_chunks("credential-json").expect("split secret");
        let manifest = manifest_for_chunks(GENERATION.into(), "credential-json", &chunks)
            .expect("create manifest");
        let manifest_deleted = Cell::new(false);

        let result = delete_keyring_secret_with(
            AUTH_STATE_KEY,
            Some(&manifest),
            |_, _| false,
            |_| {
                manifest_deleted.set(true);
                true
            },
        );

        assert_eq!(result, KeyringSecretDeletion::ChunkDeletionFailed);
        assert!(!manifest_deleted.get());
    }

    #[test]
    fn manifest_deletion_failure_is_reported_after_chunks_are_cleared() {
        let chunks = split_utf8_chunks("credential-json").expect("split secret");
        let manifest = manifest_for_chunks(GENERATION.into(), "credential-json", &chunks)
            .expect("create manifest");
        let chunks_deleted = Cell::new(false);

        let result = delete_keyring_secret_with(
            AUTH_STATE_KEY,
            Some(&manifest),
            |_, _| {
                chunks_deleted.set(true);
                true
            },
            |_| false,
        );

        assert!(chunks_deleted.get());
        assert_eq!(result, KeyringSecretDeletion::ManifestDeletionFailed);
    }
}

#[cfg(debug_assertions)]
mod debug_secret_storage {
    use super::{AUTH_STATE_KEY, GITHUB_TOKEN_KEY, SERVICE_NAME, WEBDAV_PASSWORD_KEY};
    use serde::{Deserialize, Serialize};
    use std::fs::{self, OpenOptions};
    use std::io::{self, ErrorKind, Write};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard};
    use uuid::Uuid;

    const AUTH_STORE_DIRECTORY: &str = "debug-auth";
    const GITHUB_STORE_DIRECTORY: &str = "debug-github-token";
    const WEBDAV_STORE_DIRECTORY: &str = "debug-webdav-password";
    const LOCATION_FILE: &str = "location.json";
    static STORE_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Deserialize, Serialize)]
    struct StorageLocation {
        directory: String,
        file: String,
    }

    impl StorageLocation {
        fn new() -> Self {
            Self {
                directory: Uuid::new_v4().to_string(),
                file: format!("{}.json", Uuid::new_v4()),
            }
        }

        fn is_valid(&self) -> bool {
            if Uuid::parse_str(&self.directory).is_err() {
                return false;
            }

            let Some(file_id) = self.file.strip_suffix(".json") else {
                return false;
            };
            Uuid::parse_str(file_id).is_ok()
        }

        fn credential_path(&self, root: &Path) -> PathBuf {
            root.join(&self.directory).join(&self.file)
        }
    }

    pub(super) fn get(key: &str) -> Option<String> {
        let root = storage_root(key)?;
        let _guard = lock_store();
        match read_secret_from_root(&root) {
            Ok(value) => value,
            Err(error) => {
                log::error!(target: "security", "调试凭据存储读取失败: {error}");
                if let Err(cleanup_error) = delete_store_root(&root) {
                    log::error!(target: "security", "损坏的调试凭据存储清理失败: {cleanup_error}");
                }
                None
            }
        }
    }

    pub(super) fn set(key: &str, value: &str) -> bool {
        let Some(root) = storage_root(key) else {
            return false;
        };
        let _guard = lock_store();
        match write_secret_to_root(&root, value) {
            Ok(()) => true,
            Err(error) => {
                log::error!(target: "security", "调试凭据存储写入失败: {error}");
                false
            }
        }
    }

    pub(super) fn delete(key: &str) -> bool {
        let Some(root) = storage_root(key) else {
            return false;
        };
        let _guard = lock_store();
        match delete_store_root(&root) {
            Ok(()) => true,
            Err(error) => {
                log::error!(target: "security", "调试凭据存储删除失败: {error}");
                false
            }
        }
    }

    pub(super) fn exists(key: &str) -> bool {
        let Some(root) = storage_root(key) else {
            return false;
        };
        let _guard = lock_store();
        read_location(&root)
            .ok()
            .flatten()
            .is_some_and(|location| location.credential_path(&root).is_file())
    }

    fn storage_root(key: &str) -> Option<PathBuf> {
        let directory = match key {
            AUTH_STATE_KEY => AUTH_STORE_DIRECTORY,
            GITHUB_TOKEN_KEY => GITHUB_STORE_DIRECTORY,
            WEBDAV_PASSWORD_KEY => WEBDAV_STORE_DIRECTORY,
            _ => return None,
        };
        dirs_next::cache_dir().map(|path| path.join(SERVICE_NAME).join(directory))
    }

    fn lock_store() -> MutexGuard<'static, ()> {
        STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn read_secret_from_root(root: &Path) -> io::Result<Option<String>> {
        let Some(location) = read_location(root)? else {
            return Ok(None);
        };
        match fs::read_to_string(location.credential_path(root)) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                delete_store_root(root)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn write_secret_to_root(root: &Path, value: &str) -> io::Result<()> {
        let location = match read_location(root) {
            Ok(Some(location)) => location,
            Ok(None) => {
                delete_store_root(root)?;
                StorageLocation::new()
            }
            Err(_) => {
                delete_store_root(root)?;
                StorageLocation::new()
            }
        };

        let credential_path = location.credential_path(root);
        let credential_directory = credential_path
            .parent()
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "调试凭据路径缺少父目录"))?;
        create_private_directory(root)?;
        create_private_directory(credential_directory)?;
        write_private_file(&credential_path, value.as_bytes())?;

        let serialized_location = serde_json::to_vec(&location)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if let Err(error) = write_private_file(&root.join(LOCATION_FILE), &serialized_location) {
            let _ = delete_store_root(root);
            return Err(error);
        }
        Ok(())
    }

    fn read_location(root: &Path) -> io::Result<Option<StorageLocation>> {
        let bytes = match fs::read(root.join(LOCATION_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let location: StorageLocation = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if !location.is_valid() {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "调试凭据位置元数据无效",
            ));
        }
        Ok(Some(location))
    }

    fn create_private_directory(path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(contents)?;
        file.sync_all()
    }

    fn delete_store_root(root: &Path) -> io::Result<()> {
        match fs::remove_dir_all(root) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stores_cookie_in_random_directory_and_file() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let root = temp.path().join(AUTH_STORE_DIRECTORY);

            write_secret_to_root(&root, "cookie-json").expect("write debug cookie");

            let location = read_location(&root)
                .expect("read location")
                .expect("location exists");
            assert!(Uuid::parse_str(&location.directory).is_ok());
            let file_id = location
                .file
                .strip_suffix(".json")
                .expect("random cookie file suffix");
            assert!(Uuid::parse_str(file_id).is_ok());
            assert_eq!(
                read_secret_from_root(&root).expect("read debug cookie"),
                Some("cookie-json".to_string())
            );
        }

        #[test]
        fn deleting_cookie_removes_random_storage() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let root = temp.path().join(AUTH_STORE_DIRECTORY);
            write_secret_to_root(&root, "cookie-json").expect("write debug cookie");

            delete_store_root(&root).expect("delete debug cookie storage");

            assert!(!root.exists());
            assert_eq!(
                read_secret_from_root(&root).expect("read deleted debug cookie"),
                None
            );
        }

        #[test]
        fn rejects_location_outside_debug_storage() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let root = temp.path().join(AUTH_STORE_DIRECTORY);
            let outside = temp.path().join("outside.json");
            fs::create_dir_all(&root).expect("create debug storage root");
            fs::write(&outside, "do-not-read").expect("write outside file");
            fs::write(
                root.join(LOCATION_FILE),
                r#"{"directory":"..","file":"outside.json"}"#,
            )
            .expect("write invalid location");

            let error = read_secret_from_root(&root).expect_err("reject invalid location");

            assert_eq!(error.kind(), ErrorKind::InvalidData);
            assert_eq!(
                fs::read_to_string(outside).expect("outside file remains"),
                "do-not-read"
            );
        }

        #[test]
        fn deleting_cookie_storage_keeps_sync_credentials() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let auth_root = temp.path().join(AUTH_STORE_DIRECTORY);
            let github_root = temp.path().join(GITHUB_STORE_DIRECTORY);
            write_secret_to_root(&auth_root, "cookie-json").expect("write debug cookie");
            write_secret_to_root(&github_root, "github-token").expect("write debug token");

            delete_store_root(&auth_root).expect("delete debug cookie storage");

            assert!(!auth_root.exists());
            assert_eq!(
                read_secret_from_root(&github_root).expect("read debug token"),
                Some("github-token".to_string())
            );
        }

        #[test]
        fn known_secrets_use_independent_debug_directories() {
            let auth_root = storage_root(AUTH_STATE_KEY).expect("auth storage root");
            let github_root = storage_root(GITHUB_TOKEN_KEY).expect("GitHub storage root");
            let webdav_root = storage_root(WEBDAV_PASSWORD_KEY).expect("WebDAV storage root");

            assert_ne!(auth_root, github_root);
            assert_ne!(auth_root, webdav_root);
            assert_ne!(github_root, webdav_root);
            assert!(storage_root("unknown-secret").is_none());
        }
    }
}
