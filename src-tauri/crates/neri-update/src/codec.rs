//! 内容编码：完整文件是普通 zstd 帧，补丁是以旧文件为前缀（ref prefix）的 zstd 帧，
//! 与 `zstd --patch-from` 原理相同。两者都带帧校验和，解码后还要再核对 SHA-256

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};
use zstd::stream::raw::{CParameter, DParameter};

use crate::error::{IoContext, UpdateError, UpdateResult};

/// 发布侧压缩级别；只在 CI 上跑一次，选压缩率而不是速度
pub const PACK_LEVEL: i32 = 19;
/// 64 位 zstd 允许的最大窗口
const MAX_WINDOW_LOG: u32 = 31;

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// 流式计算文件哈希，不把大文件整个读进内存
pub fn sha256_file(path: &Path) -> UpdateResult<String> {
    let mut file = File::open(path).at(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).at(path)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn compress_full(data: &[u8]) -> UpdateResult<Vec<u8>> {
    let mut encoder =
        zstd::stream::write::Encoder::new(Vec::new(), PACK_LEVEL).map_err(zstd_error)?;
    encoder.include_checksum(true).map_err(zstd_error)?;
    encoder
        .set_pledged_src_size(Some(data.len() as u64))
        .map_err(zstd_error)?;
    encoder.write_all(data).map_err(zstd_error)?;
    encoder.finish().map_err(zstd_error)
}

pub fn decompress_full(frame: &[u8], expected_size: u64) -> UpdateResult<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(frame).map_err(zstd_error)?;
    decoder.window_log_max(MAX_WINDOW_LOG).map_err(zstd_error)?;
    read_exact_size(decoder, expected_size)
}

/// 窗口要同时覆盖旧文件（前缀）与新文件，否则远处的匹配用不上
fn patch_window_log(base_len: usize, target_len: usize) -> u32 {
    let span = (base_len as u64)
        .saturating_add(target_len as u64)
        .max(1 << 10);
    let log = 64 - (span - 1).leading_zeros();
    log.clamp(10, MAX_WINDOW_LOG)
}

pub fn make_patch(base: &[u8], target: &[u8]) -> UpdateResult<Vec<u8>> {
    let mut encoder = zstd::stream::write::Encoder::with_ref_prefix(Vec::new(), PACK_LEVEL, base)
        .map_err(zstd_error)?;
    encoder
        .set_parameter(CParameter::WindowLog(patch_window_log(
            base.len(),
            target.len(),
        )))
        .map_err(zstd_error)?;
    encoder
        .set_parameter(CParameter::EnableLongDistanceMatching(true))
        .map_err(zstd_error)?;
    encoder.include_checksum(true).map_err(zstd_error)?;
    encoder
        .set_pledged_src_size(Some(target.len() as u64))
        .map_err(zstd_error)?;
    encoder.write_all(target).map_err(zstd_error)?;
    encoder.finish().map_err(zstd_error)
}

pub fn apply_patch(base: &[u8], patch: &[u8], expected_size: u64) -> UpdateResult<Vec<u8>> {
    let mut decoder =
        zstd::stream::read::Decoder::with_ref_prefix(patch, base).map_err(zstd_error)?;
    decoder
        .set_parameter(DParameter::WindowLogMax(MAX_WINDOW_LOG))
        .map_err(zstd_error)?;
    read_exact_size(decoder, expected_size)
}

/// 按清单声明的大小读取，多一个字节也不接受，防止解压炸弹撑爆内存
fn read_exact_size(reader: impl Read, expected_size: u64) -> UpdateResult<Vec<u8>> {
    let capacity =
        usize::try_from(expected_size).map_err(|_| UpdateError::Other("file too large".into()))?;
    let mut output = Vec::with_capacity(capacity);
    reader
        .take(expected_size.saturating_add(1))
        .read_to_end(&mut output)
        .map_err(zstd_error)?;
    if output.len() as u64 != expected_size {
        return Err(UpdateError::Other(format!(
            "decoded {} bytes, expected {expected_size}",
            output.len()
        )));
    }
    Ok(output)
}

fn zstd_error(error: std::io::Error) -> UpdateError {
    UpdateError::Other(format!("zstd: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect()
    }

    #[test]
    fn full_frames_round_trip_and_enforce_size() {
        let data = pseudo_random(200_000, 7);
        let frame = compress_full(&data).unwrap();
        assert_eq!(decompress_full(&frame, data.len() as u64).unwrap(), data);
        assert!(decompress_full(&frame, data.len() as u64 - 1).is_err());
        assert!(decompress_full(&frame, data.len() as u64 + 1).is_err());
    }

    #[test]
    fn patch_reconstructs_target_and_is_much_smaller_than_full() {
        // 模拟一次小改动：中间插入一段新内容、尾部改写几个字节，前后整体错位
        let base = pseudo_random(2_000_000, 1);
        let mut target = base[..900_000].to_vec();
        target.extend(pseudo_random(4_096, 2));
        target.extend_from_slice(&base[900_000..]);
        let tail = target.len() - 10;
        target[tail..].copy_from_slice(b"0123456789");

        let patch = make_patch(&base, &target).unwrap();
        let full = compress_full(&target).unwrap();
        assert!(
            patch.len() * 20 < full.len(),
            "patch {} vs full {}",
            patch.len(),
            full.len()
        );
        assert_eq!(
            apply_patch(&base, &patch, target.len() as u64).unwrap(),
            target
        );
    }

    #[test]
    fn patch_fails_against_a_different_base() {
        let base = pseudo_random(100_000, 3);
        let mut target = base.clone();
        target[50] ^= 0xff;
        let patch = make_patch(&base, &target).unwrap();
        let other = pseudo_random(100_000, 4);
        // 帧校验和或长度校验会拒绝用错基准还原出的内容
        let result = apply_patch(&other, &patch, target.len() as u64);
        assert!(result.map(|out| out != target).unwrap_or(true));
    }

    #[test]
    fn window_covers_base_and_target() {
        assert_eq!(patch_window_log(0, 0), 10);
        assert_eq!(patch_window_log(1 << 20, 1 << 20), 21);
        assert_eq!(
            patch_window_log(usize::MAX / 2, usize::MAX / 2),
            MAX_WINDOW_LOG
        );
    }

    #[test]
    fn file_hash_matches_in_memory_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        let data = pseudo_random(700_000, 9);
        std::fs::write(&path, &data).unwrap();
        assert_eq!(sha256_file(&path).unwrap(), sha256_hex(&data));
    }
}
