//! 解码后端选择
//!
//! 文件头里能认出 symphonia 不支持的编码（E-AC-3、Opus、ALAC、APE……）就直接交给 FFmpeg；
//! 认不出的先试 symphonia，它报「不支持」时再换 FFmpeg。FFmpeg 不可用时只走 symphonia，
//! 并在错误里说明缺的是哪个组件。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::audio::ffmpeg::{self, ByteInput, FfmpegDecoder, FfmpegError, FfmpegRuntime, OpenOptions};
use crate::audio::pcm::{PcmSeekError, PcmSource};
use crate::audio::remote::{SourceAudioInfo, SymphoniaAudioDecoder};

/// 嗅探文件头时读取的字节数：MP4 的 moov、Matroska 的 Tracks 都在开头这一段里
pub const SNIFF_BYTES: usize = 64 * 1024;
const FFMPEG_BLOCK_FRAMES: usize = 4096;

/// AC-3/E-AC-3 是否保留码流自带的动态范围压缩（响处压低、轻处抬高，适合小音量收听）。
/// 默认关闭，保留完整动态；改动对之后打开的音轨生效
static KEEP_DYNAMIC_RANGE_COMPRESSION: AtomicBool = AtomicBool::new(false);

pub fn set_keep_dynamic_range_compression(keep: bool) {
    KEEP_DYNAMIC_RANGE_COMPRESSION.store(keep, Ordering::Relaxed);
}

fn ffmpeg_open_options(format_hint: Option<&str>, keep_dynamic_range_compression: bool) -> OpenOptions {
    OpenOptions {
        format_hint: format_hint.map(str::to_string),
        max_output_channels: 2,
        keep_dynamic_range_compression,
    }
}

pub trait AudioDecoder: PcmSource {
    fn source_audio_info(&self) -> SourceAudioInfo;
    fn backend(&self) -> DecoderBackend;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoderBackend {
    Symphonia,
    Ffmpeg,
}

impl AudioDecoder for SymphoniaAudioDecoder {
    fn source_audio_info(&self) -> SourceAudioInfo {
        SymphoniaAudioDecoder::source_audio_info(self)
    }

    fn backend(&self) -> DecoderBackend {
        DecoderBackend::Symphonia
    }
}

/// 把 FFmpeg 的块解码适配成旧引擎逐样本拉取的接口
pub struct FfmpegPcmSource {
    decoder: FfmpegDecoder,
    buffer: Vec<f32>,
    filled: usize,
    position: usize,
    finished: bool,
    keeps_dynamic_range_compression: bool,
}

impl FfmpegPcmSource {
    pub fn open(input: Box<dyn ByteInput>, format_hint: Option<&str>) -> Result<Self, FfmpegError> {
        let options = ffmpeg_open_options(
            format_hint,
            KEEP_DYNAMIC_RANGE_COMPRESSION.load(Ordering::Relaxed),
        );
        let decoder = FfmpegDecoder::open(input, &options)?;
        let channels = usize::from(decoder.info().output_channels.max(1));
        Ok(Self {
            decoder,
            buffer: vec![0.0; FFMPEG_BLOCK_FRAMES * channels],
            filled: 0,
            position: 0,
            finished: false,
            keeps_dynamic_range_compression: options.keep_dynamic_range_compression,
        })
    }

    fn refill(&mut self) -> bool {
        match self.decoder.read_frames(&mut self.buffer) {
            Ok(0) => false,
            Ok(frames) => {
                self.filled = frames * usize::from(self.decoder.info().output_channels.max(1));
                self.position = 0;
                true
            }
            // 换歌、seek 抢占或停止：调用方已经知道，安静结束
            Err(FfmpegError::Interrupted) => false,
            Err(error) => {
                log::warn!(target: "audio-decoder", "FFmpeg decoding stopped: {error}");
                false
            }
        }
    }
}

impl Iterator for FfmpegPcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.position >= self.filled {
            if self.finished || !self.refill() {
                self.finished = true;
                return None;
            }
        }
        let sample = self.buffer.get(self.position).copied();
        self.position += 1;
        sample
    }
}

impl PcmSource for FfmpegPcmSource {
    fn channels(&self) -> u16 {
        self.decoder.info().output_channels
    }

    fn sample_rate(&self) -> u32 {
        self.decoder.info().sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.decoder.info().duration
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), PcmSeekError> {
        self.decoder
            .seek(position)
            .map_err(|error| PcmSeekError::new(error.to_string()))?;
        self.filled = 0;
        self.position = 0;
        self.finished = false;
        Ok(())
    }
}

impl AudioDecoder for FfmpegPcmSource {
    fn source_audio_info(&self) -> SourceAudioInfo {
        let info = self.decoder.info();
        SourceAudioInfo::from_parts(
            Some(info.sample_rate),
            Some(info.source_channels),
            info.bits_per_sample,
            Some(info.codec.clone()),
            info.bit_rate
                .and_then(|bits| u32::try_from((bits + 500) / 1_000).ok()),
        )
    }

    fn backend(&self) -> DecoderBackend {
        DecoderBackend::Ffmpeg
    }
}

/// 从文件头认出的、需要 FFmpeg 才能解码的编码
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SniffedCodec {
    pub codec: &'static str,
    /// FFmpeg 解封装器名；为空时让 FFmpeg 自己探测
    pub format_hint: Option<&'static str>,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// 只认确定的标记，认不出时返回 None，由「先试 symphonia、不支持再换 FFmpeg」兜底
pub fn sniff(header: &[u8]) -> Option<SniffedCodec> {
    let header = &header[..header.len().min(SNIFF_BYTES)];
    let is_mp4 = header.len() >= 8 && (contains(header, b"ftyp") || contains(header, b"moov"));
    if is_mp4 && (contains(header, b"ec-3") || contains(header, b"dec3")) {
        return Some(SniffedCodec { codec: "e-ac-3", format_hint: Some("mov") });
    }
    if is_mp4 && contains(header, b"ac-3") && contains(header, b"dac3") {
        return Some(SniffedCodec { codec: "ac-3", format_hint: Some("mov") });
    }
    if is_mp4 && contains(header, b"alac") {
        return Some(SniffedCodec { codec: "alac", format_hint: Some("mov") });
    }
    if is_mp4 && contains(header, b"Opus") && contains(header, b"dOps") {
        return Some(SniffedCodec { codec: "opus", format_hint: Some("mov") });
    }
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) && contains(header, b"A_OPUS") {
        return Some(SniffedCodec { codec: "opus", format_hint: Some("matroska") });
    }
    if header.starts_with(b"OggS") && contains(header, b"OpusHead") {
        return Some(SniffedCodec { codec: "opus", format_hint: Some("ogg") });
    }
    if header.starts_with(&[0x0b, 0x77]) {
        return Some(SniffedCodec { codec: "ac-3", format_hint: None });
    }
    if header.starts_with(b"MAC ") {
        return Some(SniffedCodec { codec: "ape", format_hint: Some("ape") });
    }
    if header.starts_with(b"wvpk") {
        return Some(SniffedCodec { codec: "wavpack", format_hint: Some("wv") });
    }
    if header.starts_with(b"DSD ") {
        return Some(SniffedCodec { codec: "dsd", format_hint: Some("dsf") });
    }
    if header.starts_with(b"FRM8") {
        return Some(SniffedCodec { codec: "dsd", format_hint: Some("iff") });
    }
    if header.len() >= 12 && header.starts_with(b"FORM") && (&header[8..12] == b"AIFF" || &header[8..12] == b"AIFC") {
        return Some(SniffedCodec { codec: "aiff", format_hint: Some("aiff") });
    }
    None
}

fn is_unsupported_by_symphonia(error: &str) -> bool {
    error.contains("unsupported feature")
}

/// 按「嗅探 → 主后端 → 回退后端」打开解码器
///
/// `ffmpeg_input` 每次调用都要给出从头开始的新输入：symphonia 失败后已经读过一段数据。
pub fn open_decoder(
    sniffed: Option<SniffedCodec>,
    open_symphonia: impl FnOnce() -> Result<SymphoniaAudioDecoder, String>,
    ffmpeg_input: impl Fn() -> io::Result<Box<dyn ByteInput>>,
) -> Result<Box<dyn AudioDecoder>, String> {
    let ffmpeg_ready = ffmpeg::runtime().is_ok();
    let open_ffmpeg = |hint: Option<&str>| -> Result<FfmpegPcmSource, FfmpegError> {
        let input = ffmpeg_input()
            .map_err(|error| FfmpegError::Failed(format!("could not reopen the input: {error}")))?;
        FfmpegPcmSource::open(input, hint)
    };

    if let Some(sniffed) = sniffed {
        if ffmpeg_ready {
            match open_ffmpeg(sniffed.format_hint) {
                Ok(decoder) => {
                    log_opened(&decoder);
                    return Ok(Box::new(decoder));
                }
                Err(FfmpegError::Interrupted) => return Err(format!("Decode error: {}", FfmpegError::Interrupted)),
                Err(error) => log::warn!(
                    target: "audio-decoder",
                    "FFmpeg could not open the {} stream, trying the built-in decoder: {error}",
                    sniffed.codec,
                ),
            }
        } else {
            log::warn!(
                target: "audio-decoder",
                "the stream looks like {}, which needs FFmpeg, but FFmpeg is unavailable",
                sniffed.codec,
            );
        }
    }

    match open_symphonia() {
        Ok(decoder) => Ok(Box::new(decoder)),
        Err(error) if ffmpeg_ready && sniffed.is_none() && is_unsupported_by_symphonia(&error) => {
            log::info!(
                target: "audio-decoder",
                "the built-in decoder does not support this stream, trying FFmpeg: {error}",
            );
            match open_ffmpeg(None) {
                Ok(decoder) => {
                    log_opened(&decoder);
                    Ok(Box::new(decoder))
                }
                Err(ffmpeg_error) => Err(format!("{error}; FFmpeg: {ffmpeg_error}")),
            }
        }
        Err(error) => Err(match sniffed {
            Some(sniffed) if !ffmpeg_ready => {
                format!("{error} (decoding {} needs FFmpeg, which is unavailable)", sniffed.codec)
            }
            _ => error,
        }),
    }
}

fn log_opened(decoder: &FfmpegPcmSource) {
    let info = decoder.decoder.info();
    let downmix = if info.decoder_downmix {
        "decoder (bitstream mix levels)"
    } else if info.matrix_downmix {
        "matrix (ITU coefficients)"
    } else {
        "none"
    };
    let drc = match (info.codec.as_str(), decoder.keeps_dynamic_range_compression) {
        ("ac3" | "eac3", true) => " drc=kept",
        ("ac3" | "eac3", false) => " drc=off",
        _ => "",
    };
    log::info!(
        target: "audio-decoder",
        "FFmpeg opened codec={} container={} rate={} channels={}->{} downmix={}{}",
        info.codec,
        info.container,
        info.sample_rate,
        info.source_channels,
        info.output_channels,
        downmix,
        drc,
    );
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecoderCapabilities {
    pub ffmpeg: Option<FfmpegRuntime>,
    /// FFmpeg 不可用的原因，可用时为空
    pub ffmpeg_error: Option<String>,
    /// 能解码的编码，名称与前端 `normalizeCodecName` 的结果一致（小写比较）
    pub codecs: Vec<&'static str>,
}

const BUILT_IN_CODECS: [&str; 5] = ["aac", "mp3", "flac", "vorbis", "pcm"];
/// (前端编码名, FFmpeg 解码器名)
const FFMPEG_CODECS: [(&str, &str); 7] = [
    ("opus", "opus"),
    ("e-ac-3", "eac3"),
    ("ac-3", "ac3"),
    ("alac", "alac"),
    ("ape", "ape"),
    ("wavpack", "wavpack"),
    ("dsd", "dsd_lsbf"),
];

pub fn decoder_capabilities() -> DecoderCapabilities {
    let mut codecs = BUILT_IN_CODECS.to_vec();
    match ffmpeg::runtime() {
        Ok(runtime) => {
            let names: Vec<&str> = FFMPEG_CODECS.iter().map(|(_, decoder)| *decoder).collect();
            let available = ffmpeg::available_decoders(&names);
            codecs.extend(
                FFMPEG_CODECS
                    .iter()
                    .filter(|(_, decoder)| available.iter().any(|name| name == decoder))
                    .map(|(codec, _)| *codec),
            );
            DecoderCapabilities { ffmpeg: Some(runtime.clone()), ffmpeg_error: None, codecs }
        }
        Err(error) => DecoderCapabilities {
            ffmpeg: None,
            ffmpeg_error: Some(error.to_string()),
            codecs,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{open_decoder, sniff, AudioDecoder, DecoderBackend, SniffedCodec};
    use crate::audio::remote::SymphoniaAudioDecoder;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    fn fixture(name: &str) -> Arc<[u8]> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/audio/fixtures/ffmpeg").join(name);
        Arc::from(std::fs::read(&path).unwrap_or_else(|error| panic!("missing fixture {}: {error}", path.display())))
    }

    fn ffmpeg_ready() -> bool {
        match crate::audio::ffmpeg::runtime() {
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

    fn open_bytes(bytes: Arc<[u8]>, sniffed: Option<SniffedCodec>) -> Result<Box<dyn AudioDecoder>, String> {
        let symphonia_bytes = Arc::clone(&bytes);
        open_decoder(
            sniffed,
            move || SymphoniaAudioDecoder::new(Box::new(Cursor::new(symphonia_bytes)), None),
            move || Ok(Box::new(Cursor::new(Arc::clone(&bytes)))),
        )
    }

    #[test]
    fn sniffing_recognizes_only_codecs_the_built_in_decoder_lacks() {
        assert_eq!(sniff(&fixture("eac3-5.1-center.mp4")).map(|found| found.codec), Some("e-ac-3"));
        assert_eq!(sniff(&fixture("eac3-5.1-center.mp4")).and_then(|found| found.format_hint), Some("mov"));
        assert_eq!(sniff(&fixture("opus-stereo-1s.webm")).map(|found| found.codec), Some("opus"));
        for supported in ["flac-s16-stereo-0.5s.flac", "mp3-stereo-1s.mp3", "aac-stereo-1s.m4a", "pcm-s16-stereo-0.5s.wav"] {
            assert_eq!(sniff(&fixture(supported)), None, "{supported} 应留给内置解码器");
        }
        assert_eq!(sniff(b"MAC \x96\x0f"), Some(SniffedCodec { codec: "ape", format_hint: Some("ape") }));
        assert_eq!(sniff(b"wvpk\x00\x00"), Some(SniffedCodec { codec: "wavpack", format_hint: Some("wv") }));
        assert_eq!(sniff(b"ec-3 without an mp4 box"), None, "没有 MP4 结构时不能凭四个字节下结论");
    }

    #[test]
    fn sniffed_dolby_and_opus_go_straight_to_ffmpeg() {
        if !ffmpeg_ready() {
            return;
        }
        for name in ["eac3-5.1-center.mp4", "opus-stereo-1s.webm"] {
            let bytes = fixture(name);
            let decoder = open_bytes(Arc::clone(&bytes), sniff(&bytes)).expect("decoder should open");
            assert_eq!(decoder.backend(), DecoderBackend::Ffmpeg, "{name}");
            assert!(decoder.channels() <= 2, "{name}: 旧引擎只接收立体声以内的输出");
        }
    }

    #[test]
    fn unrecognized_streams_fall_back_to_ffmpeg_when_the_built_in_decoder_refuses() {
        if !ffmpeg_ready() {
            return;
        }
        // 不给嗅探结果，模拟文件头里认不出编码的情况
        let decoder = open_bytes(fixture("opus-stereo-1s.webm"), None).expect("fallback should open Opus");
        assert_eq!(decoder.backend(), DecoderBackend::Ffmpeg);
        assert_eq!(decoder.source_audio_info().codec.as_deref(), Some("opus"));
    }

    /// 多声道 DRC 设置交给 FFmpeg；同时固定下混到最多 2 声道
    #[test]
    fn open_options_carry_the_dynamic_range_compression_setting() {
        let kept = super::ffmpeg_open_options(Some("mov"), true);
        assert!(kept.keep_dynamic_range_compression);
        assert_eq!(kept.format_hint.as_deref(), Some("mov"));
        assert_eq!(kept.max_output_channels, 2);
        assert!(!super::ffmpeg_open_options(None, false).keep_dynamic_range_compression);
        assert!(
            !super::KEEP_DYNAMIC_RANGE_COMPRESSION.load(std::sync::atomic::Ordering::Relaxed),
            "默认保留完整动态",
        );
    }

    #[test]
    fn formats_the_built_in_decoder_handles_stay_on_it() {
        let bytes = fixture("flac-s16-stereo-0.5s.flac");
        let decoder = open_bytes(Arc::clone(&bytes), sniff(&bytes)).expect("FLAC should open");
        assert_eq!(decoder.backend(), DecoderBackend::Symphonia);
    }

    #[test]
    fn ffmpeg_source_reports_the_original_layout_and_seeks_exactly() {
        if !ffmpeg_ready() {
            return;
        }
        let bytes = fixture("eac3-5.1-center.mp4");
        let mut decoder = open_bytes(Arc::clone(&bytes), sniff(&bytes)).expect("E-AC-3 should open");
        let info = decoder.source_audio_info();
        assert_eq!(info.channel_count, Some(6), "音频信息要显示源声道数，而不是下混后的 2");
        assert_eq!(info.codec.as_deref(), Some("eac3"));
        assert!(decoder.next().is_some());
        decoder.try_seek(Duration::from_millis(100)).expect("seek should succeed");
        let after_seek = decoder.by_ref().take(4_800).collect::<Vec<_>>();
        assert_eq!(after_seek.len(), 4_800, "seek 之后应能继续解出样本");
    }
}
