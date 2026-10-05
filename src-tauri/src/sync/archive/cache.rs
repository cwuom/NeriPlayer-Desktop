use super::{decompress, digest, ObjectRef, MAX_OBJECT_BYTES};
use std::path::{Path, PathBuf};

const MAX_CACHE_BYTES: u64 = 256 * 1024 * 1024;

fn directory() -> PathBuf {
    dirs_next::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("NeriPlayer")
        .join("sync-archive")
}
fn pointer(raw: &str, index: bool, protocol: u8) -> String {
    format!("{raw}-{index}-v{protocol}.ref")
}
fn object(reference: &ObjectRef, protocol: u8) -> String {
    format!("neriplayer-sync-v{protocol}-{}.zst", reference.hash)
}

pub(super) fn read(raw: &[u8], index: bool, protocol: u8) -> Option<(ObjectRef, Vec<u8>)> {
    if cfg!(test) {
        return None;
    }
    read_at(&directory(), raw, index, protocol)
}

fn read_at(
    directory: &Path,
    raw: &[u8],
    index: bool,
    protocol: u8,
) -> Option<(ObjectRef, Vec<u8>)> {
    let hash = digest(raw);
    let pointer_path = directory.join(pointer(&hash, index, protocol));
    if std::fs::metadata(&pointer_path).ok()?.len() > 512 {
        return None;
    }
    let reference: ObjectRef = serde_json::from_slice(&std::fs::read(&pointer_path).ok()?).ok()?;
    super::validate_ref(&reference, protocol).ok()?;
    if reference.raw_hash != hash
        || reference.raw_bytes as usize != raw.len()
        || reference.index != index
    {
        return None;
    }
    let object_path = directory.join(object(&reference, protocol));
    if reference.compressed_bytes <= 0
        || reference.compressed_bytes as usize > MAX_OBJECT_BYTES
        || std::fs::metadata(&object_path).ok()?.len() != reference.compressed_bytes as u64
    {
        return None;
    }
    let compressed = std::fs::read(&object_path).ok()?;
    if digest(&compressed) != reference.hash {
        return None;
    }
    let decoded = decompress(
        &compressed,
        raw.len(),
        if index {
            128 * 1024
        } else if protocol == 4 {
            4 * 1024 * 1024
        } else {
            1024 * 1024
        },
        if protocol == 4 { 22 } else { 20 },
    )
    .ok()?;
    if decoded != raw {
        return None;
    }
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(object_path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
    Some((reference, compressed))
}

pub(super) fn write(reference: &ObjectRef, compressed: &[u8], protocol: u8) {
    if cfg!(test) {
        return;
    }
    let directory = directory();
    let result = (|| -> crate::error::AppResult<()> {
        std::fs::create_dir_all(&directory)?;
        crate::fsutil::atomic_write(directory.join(object(reference, protocol)), compressed)?;
        crate::fsutil::atomic_write(
            directory.join(pointer(&reference.raw_hash, reference.index, protocol)),
            serde_json::to_vec(reference)?,
        )?;
        trim(&directory)?;
        Ok(())
    })();
    if let Err(error) = result {
        log::warn!(target:"sync","archive compression cache unavailable: {error}");
    }
}

fn trim(directory: &Path) -> crate::error::AppResult<()> {
    let mut files = Vec::new();
    let mut size = 0_u64;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let owned = super::canonical_object_path(&name)
            || [
                "-true-v3.ref",
                "-false-v3.ref",
                "-true-v4.ref",
                "-false-v4.ref",
            ]
            .iter()
            .any(|suffix| name.strip_suffix(suffix).is_some_and(super::valid_hash));
        if !owned || !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = entry.metadata()?;
        size = size.saturating_add(metadata.len());
        files.push((
            metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            entry.path(),
            metadata.len(),
        ));
    }
    files.sort_by_key(|entry| entry.0);
    for (_, path, bytes) in files {
        if size <= MAX_CACHE_BYTES {
            break;
        }
        std::fs::remove_file(path)?;
        size = size.saturating_sub(bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compression_cache_revalidates_checksum_kind_and_descriptor_before_reuse() {
        let directory =
            std::env::temp_dir().join(format!("neri-archive-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let raw = b"cached archive";
        let compressed = super::super::compress(raw, 22).unwrap();
        let reference = ObjectRef {
            hash: digest(&compressed),
            raw_hash: digest(raw),
            raw_bytes: raw.len() as i32,
            compressed_bytes: compressed.len() as i32,
            index: false,
        };
        let pointer_path = directory.join(pointer(&reference.raw_hash, false, 4));
        let object_path = directory.join(object(&reference, 4));
        std::fs::write(&pointer_path, serde_json::to_vec(&reference).unwrap()).unwrap();
        std::fs::write(&object_path, &compressed).unwrap();
        assert!(read_at(&directory, raw, false, 4).is_some());
        assert!(read_at(&directory, raw, true, 4).is_none());
        let mut malicious = reference.clone();
        malicious.hash = "../outside".into();
        std::fs::write(&pointer_path, serde_json::to_vec(&malicious).unwrap()).unwrap();
        assert!(read_at(&directory, raw, false, 4).is_none());
        std::fs::write(&pointer_path, serde_json::to_vec(&reference).unwrap()).unwrap();
        std::fs::write(&object_path, b"corrupt").unwrap();
        assert!(read_at(&directory, raw, false, 4).is_none());
        std::fs::remove_file(pointer_path).unwrap();
        std::fs::remove_file(object_path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
