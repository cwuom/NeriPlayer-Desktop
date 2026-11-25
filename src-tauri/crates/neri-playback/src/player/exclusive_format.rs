//! WASAPI 独占模式的格式协商（纯计算部分，三端都编译以便单测）
//!
//! 独占模式绕过系统混音器，设备只接受它硬件真正支持的格式，所以按「32 位浮点 →
//! 32 位整数 → 24 位有效位的 32 位容器 → 16 位整数」依次尝试。采样率和声道数沿用
//! 共享模式的混音格式：这样独占失败回退共享流时，ring、时钟和重采样都不需要重建。

use std::sync::atomic::{AtomicBool, Ordering};

static EXCLUSIVE_OUTPUT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 设置是否请求独占输出，返回之前的值；只影响之后新建的输出流
pub fn set_exclusive_output(enabled: bool) -> bool {
    EXCLUSIVE_OUTPUT_REQUESTED.swap(enabled, Ordering::AcqRel)
}

pub fn exclusive_output_requested() -> bool {
    EXCLUSIVE_OUTPUT_REQUESTED.load(Ordering::Acquire)
}

/// 平台是否提供独占输出（目前只有 Windows WASAPI）
pub const EXCLUSIVE_OUTPUT_SUPPORTED: bool = cfg!(target_os = "windows");

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusiveSampleKind {
    F32,
    I32,
    I16,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExclusiveFormat {
    pub kind: ExclusiveSampleKind,
    pub container_bits: u16,
    pub valid_bits: u16,
}

#[cfg(any(target_os = "windows", test))]
impl ExclusiveFormat {
    pub fn block_align(self, channels: u16) -> u16 {
        channels * (self.container_bits / 8)
    }
}

#[cfg(any(target_os = "windows", test))]
/// 依次尝试的独占格式；24-in-32 写入时仍按满幅 i32，驱动只取高 24 位
pub const EXCLUSIVE_FORMAT_CANDIDATES: [ExclusiveFormat; 4] = [
    ExclusiveFormat {
        kind: ExclusiveSampleKind::F32,
        container_bits: 32,
        valid_bits: 32,
    },
    ExclusiveFormat {
        kind: ExclusiveSampleKind::I32,
        container_bits: 32,
        valid_bits: 32,
    },
    ExclusiveFormat {
        kind: ExclusiveSampleKind::I32,
        container_bits: 32,
        valid_bits: 24,
    },
    ExclusiveFormat {
        kind: ExclusiveSampleKind::I16,
        container_bits: 16,
        valid_bits: 16,
    },
];

#[cfg(any(target_os = "windows", test))]
/// 标准扬声器布局掩码；独占模式下不少驱动拒绝 DIRECTOUT(0)，常见声道数要给真实布局
pub fn channel_mask(channels: u16) -> u32 {
    const FL: u32 = 0x1;
    const FR: u32 = 0x2;
    const FC: u32 = 0x4;
    const LFE: u32 = 0x8;
    const BL: u32 = 0x10;
    const BR: u32 = 0x20;
    const SL: u32 = 0x200;
    const SR: u32 = 0x400;
    match channels {
        1 => FC,
        2 => FL | FR,
        4 => FL | FR | BL | BR,
        6 => FL | FR | FC | LFE | BL | BR,
        8 => FL | FR | FC | LFE | BL | BR | SL | SR,
        _ => 0,
    }
}

#[cfg(any(target_os = "windows", test))]
/// 帧数换算成 100ns 单位的周期（四舍五入），用于 BUFFER_SIZE_NOT_ALIGNED 后按对齐帧数重试
pub fn frames_to_hns(frames: u32, sample_rate: u32) -> i64 {
    let rate = u64::from(sample_rate.max(1));
    ((10_000_000u64 * u64::from(frames) + rate / 2) / rate) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_prefer_float_then_wide_integers() {
        let kinds: Vec<_> = EXCLUSIVE_FORMAT_CANDIDATES
            .iter()
            .map(|format| (format.kind, format.valid_bits))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (ExclusiveSampleKind::F32, 32),
                (ExclusiveSampleKind::I32, 32),
                (ExclusiveSampleKind::I32, 24),
                (ExclusiveSampleKind::I16, 16),
            ]
        );
        assert_eq!(EXCLUSIVE_FORMAT_CANDIDATES[0].block_align(2), 8);
        assert_eq!(EXCLUSIVE_FORMAT_CANDIDATES[3].block_align(2), 4);
    }

    #[test]
    fn channel_masks_follow_standard_layouts() {
        assert_eq!(channel_mask(1), 0x4);
        assert_eq!(channel_mask(2), 0x3);
        assert_eq!(channel_mask(6), 0x3F);
        assert_eq!(channel_mask(8), 0x63F);
        assert_eq!(channel_mask(3), 0);
    }

    #[test]
    fn aligned_period_rounds_to_nearest_hundred_nanoseconds() {
        // 48 kHz 下 480 帧正好 10ms
        assert_eq!(frames_to_hns(480, 48_000), 100_000);
        // 44.1 kHz 下 441 帧也是 10ms；448 帧（常见对齐值）≈ 10.1587ms
        assert_eq!(frames_to_hns(441, 44_100), 100_000);
        assert_eq!(frames_to_hns(448, 44_100), 101_587);
        assert_eq!(frames_to_hns(10, 0), 100_000_000);
    }

    #[test]
    fn exclusive_request_flag_reports_previous_value() {
        let original = exclusive_output_requested();
        assert_eq!(set_exclusive_output(true), original);
        assert!(exclusive_output_requested());
        assert!(set_exclusive_output(original));
        assert_eq!(exclusive_output_requested(), original);
    }
}
