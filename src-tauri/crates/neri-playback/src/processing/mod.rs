//! 输出前的音频处理：原生 DSP 桥接与设置、音效链、保调变速与电平/频谱分析。
//! 子模块经 crate 根重导出，对外路径保持 `neri_playback::{analyzer, dsp, ...}` 不变

pub mod analyzer;
pub mod dsp;
pub mod dsp_settings;
pub mod effects;
pub mod stretch;
