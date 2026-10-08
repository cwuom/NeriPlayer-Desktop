//! FFmpeg 解码后端
//!
//! 动态库在运行时经 `native/ffmpeg` 下的 C++ 垫片加载；找不到或版本不符时返回原因，
//! 调用方回退 symphonia。FFmpeg 的结构体只在垫片里访问，这里只面对一组稳定的 C 接口。

use std::ffi::{c_char, c_void, CStr, CString};
use std::io::{self, Read, Seek, SeekFrom};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::time::Duration;

mod sys {
    use std::ffi::{c_char, c_void};

    #[repr(C)]
    pub struct Io {
        pub opaque: *mut c_void,
        pub read: Option<unsafe extern "C" fn(*mut c_void, *mut u8, i32) -> i32>,
        pub seek: Option<unsafe extern "C" fn(*mut c_void, i64, i32) -> i64>,
        pub interrupted: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    }

    #[repr(C)]
    pub struct StreamInfo {
        pub sample_rate: i32,
        pub source_channels: i32,
        pub output_channels: i32,
        pub bits_per_sample: i32,
        pub bit_rate: i64,
        pub duration_us: i64,
        pub initial_padding: i32,
        pub trailing_padding: i32,
        pub seek_preroll: i32,
        pub decoder_downmix: i32,
        pub matrix_downmix: i32,
        pub codec: [c_char; 32],
        pub container: [c_char; 64],
    }

    #[repr(C)]
    pub struct Decoder {
        _opaque: [u8; 0],
    }

    extern "C" {
        pub fn neri_ff_set_log_sink(sink: Option<unsafe extern "C" fn(i32, *const c_char)>);
        pub fn neri_ff_load(directory: *const c_char) -> i32;
        pub fn neri_ff_load_error() -> *const c_char;
        pub fn neri_ff_versions(avutil: *mut u32, avcodec: *mut u32, avformat: *mut u32);
        pub fn neri_ff_has_decoder(name: *const c_char) -> i32;
        pub fn neri_ff_open(
            io: *const Io,
            format_hint: *const c_char,
            max_output_channels: i32,
            drc_off: i32,
            out: *mut *mut Decoder,
            error: *mut c_char,
            error_len: i32,
        ) -> i32;
        pub fn neri_ff_info(decoder: *const Decoder, out: *mut StreamInfo) -> i32;
        pub fn neri_ff_read(decoder: *mut Decoder, out: *mut f32, max_frames: i32) -> i32;
        pub fn neri_ff_seek(decoder: *mut Decoder, target_us: i64) -> i32;
        pub fn neri_ff_decoder_error(decoder: *const Decoder) -> *const c_char;
        pub fn neri_ff_close(decoder: *mut Decoder);
    }
}

/// 覆盖默认搜索路径的目录（开发、测试或用户替换动态库时使用）
pub const FFMPEG_DIR_ENV: &str = "NERI_FFMPEG_DIR";
const AVSEEK_SIZE: i32 = 0x10000;
const IO_ERROR: i32 = -1;
const IO_INTERRUPTED: i32 = -2;
const AV_LOG_ERROR: i32 = 16;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegRuntime {
    pub directory: String,
    pub avutil: String,
    pub avcodec: String,
    pub avformat: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfmpegError {
    /// 动态库不可用：调用方应回退其他解码器
    Unavailable(String),
    /// 输入被调用方取消（换歌、seek 抢占、停止）
    Interrupted,
    Failed(String),
}

impl std::fmt::Display for FfmpegError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(formatter, "FFmpeg is unavailable: {reason}"),
            Self::Interrupted => formatter.write_str("FFmpeg input was interrupted"),
            Self::Failed(reason) => write!(formatter, "FFmpeg: {reason}"),
        }
    }
}

impl std::error::Error for FfmpegError {}

static RUNTIME: OnceLock<Result<FfmpegRuntime, String>> = OnceLock::new();
static BUNDLED_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// 安装包里随附动态库的目录（应用资源目录下的 `ffmpeg/`），要在第一次加载前设置
///
/// 各平台、各种包格式的资源目录不同（Linux 的 deb、rpm、AppImage 各不一样），
/// 由应用按 Tauri 给出的资源目录告诉这里，不在加载器里猜。
pub fn set_bundled_directory(directory: PathBuf) {
    if BUNDLED_DIRECTORY.set(directory).is_err() {
        log::warn!(target: "audio-decoder", "FFmpeg bundled directory was already set");
    }
}

/// 加载（仅首次调用时）并返回 FFmpeg 运行时信息
pub fn runtime() -> Result<&'static FfmpegRuntime, FfmpegError> {
    RUNTIME
        .get_or_init(|| {
            let result = load_first(&candidate_directories(), load_directory);
            match &result {
                Ok(runtime) => log::info!(
                    target: "audio-decoder",
                    "FFmpeg loaded from {} (avutil {}, avcodec {}, avformat {})",
                    runtime.directory,
                    runtime.avutil,
                    runtime.avcodec,
                    runtime.avformat,
                ),
                Err(reason) => log::warn!(
                    target: "audio-decoder",
                    "FFmpeg unavailable, falling back to the built-in decoder: {reason}",
                ),
            }
            result
        })
        .as_ref()
        .map_err(|reason| FfmpegError::Unavailable(reason.clone()))
}

/// 按优先级给出动态库可能所在的目录
fn candidate_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(directory) = std::env::var_os(FFMPEG_DIR_ENV) {
        directories.push(PathBuf::from(directory));
    }
    if let Some(directory) = BUNDLED_DIRECTORY.get() {
        directories.push(directory.clone());
    }
    if let Some(executable_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        directories.push(executable_dir.join("ffmpeg"));
        directories.push(executable_dir.clone());
        if cfg!(target_os = "macos") {
            // 安装包把动态库放进 Frameworks，签名应用时一并签名
            directories.push(executable_dir.join("../Frameworks"));
        }
    }
    if cfg!(debug_assertions) {
        directories.extend(development_directories());
    }
    directories
}

/// 开发机上的动态库（仅调试构建会找这里）：先找 scripts/ffmpeg/build-ffmpeg.sh 的输出
/// `.cache/ffmpeg-build/<os>-<arch>/`，再找放在 `.cache/ffmpeg/<os>-<arch>/` 的预编译共享库
fn development_directories() -> [PathBuf; 2] {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".cache");
    let library_dir = if cfg!(windows) { "bin" } else { "lib" };
    [
        cache.join("ffmpeg-build").join(&platform),
        cache.join("ffmpeg").join(&platform).join(library_dir),
    ]
}

fn load_first(
    directories: &[PathBuf],
    mut load: impl FnMut(&Path) -> Result<FfmpegRuntime, String>,
) -> Result<FfmpegRuntime, String> {
    let mut failures = Vec::new();
    for directory in directories {
        if !directory.is_dir() {
            continue;
        }
        match load(directory) {
            Ok(runtime) => return Ok(runtime),
            Err(reason) => failures.push(reason),
        }
    }
    if failures.is_empty() {
        Err("no FFmpeg libraries were found".into())
    } else {
        Err(failures.join("; "))
    }
}

fn load_directory(directory: &Path) -> Result<FfmpegRuntime, String> {
    let directory_text = directory.to_string_lossy().into_owned();
    let encoded = CString::new(directory_text.clone())
        .map_err(|_| format!("directory contains a NUL byte: {directory_text}"))?;
    unsafe { sys::neri_ff_set_log_sink(Some(forward_ffmpeg_log)) };
    let status = unsafe { sys::neri_ff_load(encoded.as_ptr()) };
    if status != 0 {
        let reason = unsafe { c_text(sys::neri_ff_load_error()) };
        return Err(if reason.is_empty() {
            format!("could not load FFmpeg from {directory_text}")
        } else {
            reason
        });
    }
    let (mut avutil, mut avcodec, mut avformat) = (0u32, 0u32, 0u32);
    unsafe { sys::neri_ff_versions(&mut avutil, &mut avcodec, &mut avformat) };
    Ok(FfmpegRuntime {
        directory: directory_text,
        avutil: format_version(avutil),
        avcodec: format_version(avcodec),
        avformat: format_version(avformat),
    })
}

fn format_version(version: u32) -> String {
    format!("{}.{}.{}", version >> 16, (version >> 8) & 0xff, version & 0xff)
}

unsafe fn c_text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    CStr::from_ptr(pointer).to_string_lossy().into_owned()
}

fn array_text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|value| **value != 0)
        .map(|value| *value as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

unsafe extern "C" fn forward_ffmpeg_log(level: i32, line: *const c_char) {
    let _ = catch_unwind(|| {
        let text = c_text(line);
        if level <= AV_LOG_ERROR {
            log::warn!(target: "ffmpeg", "{text}");
        } else {
            log::debug!(target: "ffmpeg", "{text}");
        }
    });
}

/// 解码器的输入：现有的文件、内存、远程 Range 读取器都实现它
pub trait ByteInput: Read + Seek + Send {
    fn byte_len(&mut self) -> Option<u64>;
    /// 调用方已取消时返回真，FFmpeg 阻塞中的读会尽快退出
    fn interrupted(&self) -> bool {
        false
    }
}

impl ByteInput for std::fs::File {
    fn byte_len(&mut self) -> Option<u64> {
        self.metadata().ok().map(|metadata| metadata.len())
    }
}

impl<T: AsRef<[u8]> + Send> ByteInput for io::Cursor<T> {
    fn byte_len(&mut self) -> Option<u64> {
        u64::try_from(self.get_ref().as_ref().len()).ok()
    }
}

struct IoState {
    input: Box<dyn ByteInput>,
    interrupted: bool,
    last_error: Option<String>,
}

fn guarded<T>(fallback: T, operation: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|_| {
        log::error!(target: "audio-decoder", "FFmpeg I/O callback panicked");
        fallback
    })
}

unsafe extern "C" fn io_read(opaque: *mut c_void, buffer: *mut u8, size: i32) -> i32 {
    guarded(IO_ERROR, || {
        let state = &mut *(opaque as *mut IoState);
        let Ok(length) = usize::try_from(size) else { return IO_ERROR };
        let target = std::slice::from_raw_parts_mut(buffer, length);
        match state.input.read(target) {
            Ok(read) => i32::try_from(read).unwrap_or(IO_ERROR),
            // 远程读取器取消时报 Interrupted，边下边播的读取器报普通错误，两者都要按取消处理
            Err(error) if error.kind() == io::ErrorKind::Interrupted || state.input.interrupted() => {
                state.interrupted = true;
                IO_INTERRUPTED
            }
            Err(error) => {
                state.last_error = Some(error.to_string());
                IO_ERROR
            }
        }
    })
}

unsafe extern "C" fn io_seek(opaque: *mut c_void, offset: i64, whence: i32) -> i64 {
    guarded(-1, || {
        let state = &mut *(opaque as *mut IoState);
        if whence == AVSEEK_SIZE {
            return state.input.byte_len().and_then(|length| i64::try_from(length).ok()).unwrap_or(-1);
        }
        let target = match whence {
            0 => match u64::try_from(offset) {
                Ok(position) => SeekFrom::Start(position),
                Err(_) => return -1,
            },
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return -1,
        };
        match state.input.seek(target) {
            Ok(position) => i64::try_from(position).unwrap_or(-1),
            Err(error) => {
                if error.kind() == io::ErrorKind::Interrupted {
                    state.interrupted = true;
                } else {
                    state.last_error = Some(error.to_string());
                }
                -1
            }
        }
    })
}

unsafe extern "C" fn io_interrupted(opaque: *mut c_void) -> i32 {
    guarded(1, || {
        let state = &mut *(opaque as *mut IoState);
        if state.input.interrupted() {
            state.interrupted = true;
            1
        } else {
            0
        }
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// FFmpeg 解封装器名（如 `mov`、`matroska`、`flac`），为空时按内容探测
    pub format_hint: Option<String>,
    /// 交付的最多声道数（1 或 2）；多出的声道在解码端下混
    pub max_output_channels: u16,
    /// 是否保留 AC-3/E-AC-3 码流里的动态范围压缩
    pub keep_dynamic_range_compression: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FfmpegStreamInfo {
    pub codec: String,
    pub container: String,
    pub sample_rate: u32,
    pub source_channels: u16,
    pub output_channels: u16,
    pub bits_per_sample: Option<u32>,
    pub bit_rate: Option<u64>,
    pub duration: Option<Duration>,
    /// AC-3/E-AC-3 由解码器按码流元数据下混
    pub decoder_downmix: bool,
    /// 其他多声道编码按 ITU 系数下混
    pub matrix_downmix: bool,
}

pub struct FfmpegDecoder {
    handle: NonNull<sys::Decoder>,
    io: Box<IoState>,
    info: FfmpegStreamInfo,
}

// 垫片里的解码器只经 `&mut self` 访问，FFmpeg 上下文不会被两个线程同时使用
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    pub fn open(input: Box<dyn ByteInput>, options: &OpenOptions) -> Result<Self, FfmpegError> {
        runtime()?;
        let mut io = Box::new(IoState {
            input,
            interrupted: false,
            last_error: None,
        });
        let raw_io = sys::Io {
            opaque: (&mut *io as *mut IoState).cast::<c_void>(),
            read: Some(io_read),
            seek: Some(io_seek),
            interrupted: Some(io_interrupted),
        };
        let hint = options
            .format_hint
            .as_deref()
            .filter(|hint| !hint.is_empty())
            .map(CString::new)
            .transpose()
            .map_err(|_| FfmpegError::Failed("format hint contains a NUL byte".into()))?;
        let mut handle: *mut sys::Decoder = std::ptr::null_mut();
        let mut error = [0 as c_char; 256];
        let status = unsafe {
            sys::neri_ff_open(
                &raw_io,
                hint.as_ref().map_or(std::ptr::null(), |hint| hint.as_ptr()),
                i32::from(options.max_output_channels.clamp(1, 2)),
                i32::from(!options.keep_dynamic_range_compression),
                &mut handle,
                error.as_mut_ptr(),
                error.len() as i32,
            )
        };
        let Some(handle) = NonNull::new(handle).filter(|_| status >= 0) else {
            if io.interrupted {
                return Err(FfmpegError::Interrupted);
            }
            return Err(FfmpegError::Failed(with_io_error(array_text(&error), &io)));
        };
        let mut raw_info = unsafe { std::mem::zeroed::<sys::StreamInfo>() };
        unsafe { sys::neri_ff_info(handle.as_ptr(), &mut raw_info) };
        Ok(Self {
            handle,
            io,
            info: convert_info(&raw_info),
        })
    }

    pub fn info(&self) -> &FfmpegStreamInfo {
        &self.info
    }

    /// 解出交错 f32 样本填进 `out`，返回帧数；0 表示流已结束
    pub fn read_frames(&mut self, out: &mut [f32]) -> Result<usize, FfmpegError> {
        let channels = usize::from(self.info.output_channels.max(1));
        let capacity = i32::try_from(out.len() / channels).unwrap_or(i32::MAX);
        if capacity == 0 {
            return Ok(0);
        }
        let frames = unsafe { sys::neri_ff_read(self.handle.as_ptr(), out.as_mut_ptr(), capacity) };
        if frames >= 0 {
            return Ok(frames as usize);
        }
        Err(self.last_error())
    }

    /// 样本级精确 seek：解封装器回到目标之前的同步点，再丢弃到目标样本
    pub fn seek(&mut self, position: Duration) -> Result<(), FfmpegError> {
        let target_us = i64::try_from(position.as_micros()).unwrap_or(i64::MAX);
        if unsafe { sys::neri_ff_seek(self.handle.as_ptr(), target_us) } >= 0 {
            return Ok(());
        }
        Err(self.last_error())
    }

    fn last_error(&mut self) -> FfmpegError {
        if std::mem::take(&mut self.io.interrupted) {
            return FfmpegError::Interrupted;
        }
        let reason = unsafe { c_text(sys::neri_ff_decoder_error(self.handle.as_ptr())) };
        let message = with_io_error(reason, &self.io);
        self.io.last_error = None;
        FfmpegError::Failed(message)
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        // 先关解码器再释放 I/O 状态：关闭过程中 FFmpeg 仍可能持有回调指针
        unsafe { sys::neri_ff_close(self.handle.as_ptr()) };
    }
}

fn with_io_error(reason: String, io: &IoState) -> String {
    match io.last_error.as_deref() {
        Some(detail) if !reason.is_empty() => format!("{reason} ({detail})"),
        Some(detail) => detail.to_string(),
        None if reason.is_empty() => "unknown error".into(),
        None => reason,
    }
}

fn convert_info(raw: &sys::StreamInfo) -> FfmpegStreamInfo {
    FfmpegStreamInfo {
        codec: array_text(&raw.codec),
        container: array_text(&raw.container),
        sample_rate: u32::try_from(raw.sample_rate).unwrap_or(0),
        source_channels: u16::try_from(raw.source_channels).unwrap_or(0),
        output_channels: u16::try_from(raw.output_channels).unwrap_or(0),
        bits_per_sample: u32::try_from(raw.bits_per_sample).ok().filter(|bits| *bits > 0),
        bit_rate: u64::try_from(raw.bit_rate).ok().filter(|rate| *rate > 0),
        duration: u64::try_from(raw.duration_us).ok().map(Duration::from_micros),
        decoder_downmix: raw.decoder_downmix != 0,
        matrix_downmix: raw.matrix_downmix != 0,
    }
}

/// 运行时里实际可用的解码器（按 FFmpeg 解码器名查询）
pub fn available_decoders(names: &[&str]) -> Vec<String> {
    if runtime().is_err() {
        return Vec::new();
    }
    names
        .iter()
        .filter(|name| {
            CString::new(**name).is_ok_and(|encoded| unsafe { sys::neri_ff_has_decoder(encoded.as_ptr()) } == 1)
        })
        .map(|name| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{load_first, FfmpegDecoder, FfmpegError, FfmpegRuntime, OpenOptions};
    use std::path::PathBuf;
    use std::time::Duration;

    /// 需要 FFmpeg 的测试：开发机找不到动态库时跳过；CI 设 NERI_REQUIRE_FFMPEG=1 后改为失败
    fn ffmpeg_ready() -> bool {
        match super::runtime() {
            Ok(_) => true,
            Err(error) => {
                if std::env::var("NERI_REQUIRE_FFMPEG").is_ok_and(|value| value == "1") {
                    panic!("FFmpeg is required for this test run: {error}");
                }
                eprintln!("skipping FFmpeg test: {error}");
                false
            }
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/audio/fixtures/ffmpeg")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("missing fixture {}: {error}", path.display()))
    }

    fn open_fixture(name: &str, hint: Option<&str>) -> FfmpegDecoder {
        FfmpegDecoder::open(
            Box::new(std::io::Cursor::new(fixture(name))),
            &OpenOptions {
                format_hint: hint.map(str::to_string),
                max_output_channels: 2,
                keep_dynamic_range_compression: false,
            },
        )
        .unwrap_or_else(|error| panic!("could not open {name}: {error}"))
    }

    fn decode_all(decoder: &mut FfmpegDecoder) -> Vec<f32> {
        let channels = usize::from(decoder.info().output_channels);
        let mut samples = Vec::new();
        let mut block = vec![0.0f32; 1024 * channels];
        loop {
            let frames = decoder.read_frames(&mut block).expect("decoding should not fail");
            if frames == 0 {
                return samples;
            }
            samples.extend_from_slice(&block[..frames * channels]);
        }
    }

    fn rms(samples: impl Iterator<Item = f32>) -> f64 {
        let (sum, count) = samples.fold((0.0f64, 0usize), |(sum, count), value| {
            (sum + f64::from(value) * f64::from(value), count + 1)
        });
        (sum / count.max(1) as f64).sqrt()
    }

    /// 安装包里的目录排在手动覆盖之后、可执行文件旁边的目录之前
    #[test]
    fn the_bundled_directory_is_searched_right_after_the_override() {
        let bundled = std::env::temp_dir().join("neri-ffmpeg-bundled-test-missing");
        super::set_bundled_directory(bundled.clone());

        let directories = super::candidate_directories();

        let overrides = usize::from(std::env::var_os(super::FFMPEG_DIR_ENV).is_some());
        assert_eq!(directories.iter().position(|directory| *directory == bundled), Some(overrides));
    }

    #[test]
    fn loading_reports_every_failed_directory_and_stops_at_the_first_success() {
        let existing = std::env::temp_dir();
        let missing = existing.join("neri-ffmpeg-does-not-exist");
        let failures = load_first(&[missing.clone(), existing.clone()], |directory| {
            Err(format!("cannot load from {}", directory.display()))
        })
        .unwrap_err();
        assert!(failures.contains(&existing.display().to_string()), "{failures}");
        assert!(!failures.contains("neri-ffmpeg-does-not-exist"), "不存在的目录不应尝试加载");

        let mut attempts = 0;
        let loaded = load_first(&[existing.clone(), existing.clone()], |directory| {
            attempts += 1;
            Ok(FfmpegRuntime {
                directory: directory.display().to_string(),
                avutil: "61.1.102".into(),
                avcodec: "63.1.102".into(),
                avformat: "63.1.102".into(),
            })
        });
        assert!(loaded.is_ok());
        assert_eq!(attempts, 1);
        assert_eq!(load_first(&[missing], |_| unreachable!()).unwrap_err(), "no FFmpeg libraries were found");
    }

    #[test]
    fn eac3_center_dialogue_survives_the_stereo_downmix() {
        if !ffmpeg_ready() {
            return;
        }
        // 5.1 声床只有中置有信号：旧引擎只取前两个声道，这样的对白会整段消失
        let mut decoder = open_fixture("eac3-5.1-center.mp4", Some("mov"));
        let info = decoder.info().clone();
        assert_eq!(info.codec, "eac3");
        assert_eq!((info.source_channels, info.output_channels, info.sample_rate), (6, 2, 48_000));
        assert!(info.decoder_downmix, "E-AC-3 应当由解码器按码流元数据下混");
        let samples = decode_all(&mut decoder);
        let left = rms(samples.iter().step_by(2).copied());
        let right = rms(samples.iter().skip(1).step_by(2).copied());
        assert!(left > 0.05 && right > 0.05, "中置对白下混后两侧都应有声: L={left:.4} R={right:.4}");
        assert!((left / right - 1.0).abs() < 0.05, "中置应均匀分到左右: L={left:.4} R={right:.4}");
    }

    #[test]
    fn opus_in_webm_decodes_with_encoder_delay_removed() {
        if !ffmpeg_ready() {
            return;
        }
        let mut decoder = open_fixture("opus-stereo-1s.webm", Some("matroska"));
        assert_eq!(decoder.info().codec, "opus");
        assert_eq!(decoder.info().sample_rate, 48_000);
        let frames = decode_all(&mut decoder).len() / 2;
        // 源是 1 秒 48 kHz：去掉 pre-skip 之后应该正好 48 000 帧（Opus 以 20 ms 为单位，容许一包误差）
        assert!((frames as i64 - 48_000).abs() <= 960, "decoded {frames} frames");
    }

    #[test]
    fn flac_decodes_bit_exact_against_the_wav_source() {
        if !ffmpeg_ready() {
            return;
        }
        let mut source = open_fixture("pcm-s16-stereo-0.5s.wav", Some("wav"));
        let mut flac = open_fixture("flac-s16-stereo-0.5s.flac", Some("flac"));
        assert_eq!(flac.info().codec, "flac");
        assert_eq!(flac.info().bits_per_sample, Some(16));
        let expected = decode_all(&mut source);
        let decoded = decode_all(&mut flac);
        assert_eq!(decoded.len(), expected.len());
        assert!(decoded == expected, "FLAC 解码必须与 WAV 源逐样本一致");
    }

    #[test]
    fn mp3_and_aac_trim_encoder_delay_and_padding() {
        if !ffmpeg_ready() {
            return;
        }
        for (name, hint) in [("mp3-stereo-1s.mp3", "mp3"), ("aac-stereo-1s.m4a", "mov")] {
            let mut decoder = open_fixture(name, Some(hint));
            let frames = decode_all(&mut decoder).len() / 2;
            assert_eq!(frames, 44_100, "{name}: 无缝衔接要求去掉编码器延迟与尾部填充");
        }
    }

    #[test]
    fn seek_lands_on_the_exact_sample() {
        if !ffmpeg_ready() {
            return;
        }
        let mut reference = open_fixture("flac-s16-stereo-0.5s.flac", Some("flac"));
        let all = decode_all(&mut reference);
        let mut decoder = open_fixture("flac-s16-stereo-0.5s.flac", Some("flac"));
        decoder.seek(Duration::from_millis(250)).expect("seek should succeed");
        let mut block = vec![0.0f32; 64 * 2];
        let frames = decoder.read_frames(&mut block).expect("read after seek");
        assert!(frames > 0);
        let offset = 250 * 48_000 / 1000 * 2;
        assert_eq!(&block[..frames * 2], &all[offset..offset + frames * 2], "seek 后的第一个样本必须是目标样本");
    }

    #[test]
    fn undecodable_input_fails_on_open_instead_of_mid_playback() {
        if !ffmpeg_ready() {
            return;
        }
        let garbage = vec![0x5au8; 64 * 1024];
        let result = FfmpegDecoder::open(
            Box::new(std::io::Cursor::new(garbage)),
            &OpenOptions { max_output_channels: 2, ..OpenOptions::default() },
        );
        assert!(matches!(result, Err(FfmpegError::Failed(_))), "{:?}", result.err());
    }
}
