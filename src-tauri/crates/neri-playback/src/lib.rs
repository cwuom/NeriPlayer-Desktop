//! 播放引擎：cpal 输出、symphonia/FFmpeg 解码、HLS 与远程拉流缓存、原生 DSP 音效、
//! 播放队列与系统媒体会话。不依赖 Tauri，异步运行时由宿主通过 [`runtime::install`] 注入；
//! 原生 DSP 与 FFmpeg 垫片由本 crate 的 build.rs 编译

pub mod buffered;
pub mod decoder;
pub mod ffmpeg;
pub mod growing;
pub mod hls;
pub mod media_session;
pub mod metrics;
pub mod pcm;
pub mod player;
mod processing;
pub mod queue;
pub mod quiet;
pub mod remote;
pub mod runtime;
/// Sound server sinks are a Linux desktop concept; other targets keep using cpal.
#[cfg(target_os = "linux")]
pub mod system_sinks;

pub use processing::{analyzer, dsp, dsp_settings, effects, stretch};
