//! 基于 symphonia 的远程/本地音频解码器与源音频信息

use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAudioInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sample_rate_hz: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) channel_count: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bit_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bitrate: Option<u32>,
}

impl SourceAudioInfo {
    pub(crate) fn from_parts(
        sample_rate_hz: Option<u32>,
        channel_count: Option<u16>,
        bit_depth: Option<u32>,
        codec: Option<String>,
        bitrate_kbps: Option<u32>,
    ) -> Self {
        Self {
            sample_rate_hz: sample_rate_hz.filter(|value| *value > 0),
            channel_count: channel_count.filter(|value| *value > 0),
            bit_depth: bit_depth.filter(|value| *value > 0),
            codec: codec.filter(|value| !value.is_empty()),
            bitrate: bitrate_kbps.filter(|value| *value > 0),
        }
    }

    pub(crate) fn with_encoded_bitrate(
        mut self,
        byte_length: Option<u64>,
        duration: Option<Duration>,
    ) -> Self {
        if self.bitrate.is_none() {
            if let (Some(bytes), Some(duration)) = (
                byte_length.filter(|bytes| *bytes > 0),
                duration.filter(|duration| !duration.is_zero()),
            ) {
                // 压缩音频只能按完整编码文件和真实时长算平均码率，不能用解码后的 PCM 大小
                let nanos = duration.as_nanos();
                let kbps = (u128::from(bytes) * 8 * 1_000_000 + nanos / 2) / nanos;
                self.bitrate = u32::try_from(kbps).ok().filter(|value| *value > 0);
            }
        }
        self
    }
}

pub(super) struct RemoteDecoderSource {
    pub(super) source: RemoteAudioSource,
    pub(super) reached_eof: Arc<AtomicBool>,
}

impl Read for RemoteDecoderSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        self.reached_eof.store(false, Ordering::Release);
        let read = self.source.read(out)?;
        self.reached_eof.store(
            read == 0 && self.source.pos >= self.source.logical_len(),
            Ordering::Release,
        );
        Ok(read)
    }
}

impl Seek for RemoteDecoderSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let position = self.source.seek(pos)?;
        self.reached_eof.store(false, Ordering::Release);
        Ok(position)
    }
}

impl MediaSource for RemoteDecoderSource {
    fn is_seekable(&self) -> bool {
        self.source.is_seekable()
    }
    fn byte_len(&self) -> Option<u64> {
        MediaSource::byte_len(&self.source)
    }
}

pub struct SymphoniaAudioDecoder {
    decoder: Box<dyn Decoder>,
    current_frame_offset: usize,
    pub(super) format: Box<dyn FormatReader>,
    track_id: u32,
    total_duration: Option<Time>,
    buffer: SampleBuffer<f32>,
    spec: SignalSpec,
    retry_virtual_body_edges: bool,
    source_eof: Option<Arc<AtomicBool>>,
    exhausted: bool,
}

impl SymphoniaAudioDecoder {
    pub fn source_audio_info(&self) -> SourceAudioInfo {
        let params = self.decoder.codec_params();
        let sample_rate_hz = Some(self.spec.rate).filter(|value| *value > 0);
        let channel_count = u16::try_from(self.spec.channels.count())
            .ok()
            .filter(|value| *value > 0);
        let encoded_width = match params.codec {
            codecs::CODEC_TYPE_PCM_S8
            | codecs::CODEC_TYPE_PCM_U8
            | codecs::CODEC_TYPE_PCM_ALAW
            | codecs::CODEC_TYPE_PCM_MULAW => Some(8u32),
            codecs::CODEC_TYPE_PCM_S16LE
            | codecs::CODEC_TYPE_PCM_S16BE
            | codecs::CODEC_TYPE_PCM_U16LE
            | codecs::CODEC_TYPE_PCM_U16BE => Some(16),
            codecs::CODEC_TYPE_PCM_S24LE
            | codecs::CODEC_TYPE_PCM_S24BE
            | codecs::CODEC_TYPE_PCM_U24LE
            | codecs::CODEC_TYPE_PCM_U24BE => Some(24),
            codecs::CODEC_TYPE_PCM_S32LE
            | codecs::CODEC_TYPE_PCM_S32BE
            | codecs::CODEC_TYPE_PCM_U32LE
            | codecs::CODEC_TYPE_PCM_U32BE
            | codecs::CODEC_TYPE_PCM_F32LE
            | codecs::CODEC_TYPE_PCM_F32BE => Some(32),
            codecs::CODEC_TYPE_PCM_F64LE | codecs::CODEC_TYPE_PCM_F64BE => Some(64),
            _ => None,
        };
        let bitrate = encoded_width.and_then(|width| {
            sample_rate_hz
                .zip(channel_count)
                .and_then(|(rate, channels)| {
                    let bps = u64::from(rate) * u64::from(channels) * u64::from(width);
                    u32::try_from((bps + 500) / 1_000)
                        .ok()
                        .filter(|value| *value > 0)
                })
        });
        SourceAudioInfo {
            sample_rate_hz,
            channel_count,
            bit_depth: params.bits_per_sample.filter(|value| *value > 0),
            codec: get_codecs()
                .get_codec(params.codec)
                .map(|descriptor| descriptor.short_name.to_string()),
            bitrate,
        }
    }

    pub fn new(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Self, String> {
        let options = MediaSourceStreamOptions {
            buffer_len: REMOTE_DECODER_BUFFER_BYTES,
        };
        let mss = MediaSourceStream::new(source, options);
        Self::init(mss, extension).map_err(|err| format!("Decode error: {}", err))
    }

    /// 远程源：若处于 demuxer_open_sequential，打开时对 isomp4 隐藏 seekable
    pub fn new_remote(source: RemoteAudioSource) -> Result<Self, String> {
        Self::new_remote_virtual(source, false)
    }

    /// 远程源：virtual_body 模式用于长内容 seek（header + 目标 moof）
    pub fn new_remote_virtual(
        mut source: RemoteAudioSource,
        keep_virtual_body: bool,
    ) -> Result<Self, String> {
        let finish_handle = RemoteAudioSource {
            inner: Arc::clone(&source.inner),
            pos: 0,
            access_mode: source.access_mode,
            read_cancellation: source.read_cancellation.clone(),
        };
        source.pos = 0;
        if keep_virtual_body {
            source
                .inner
                .demuxer_open_sequential
                .store(true, Ordering::Release);

            // 关键修复：把 header + 目标 moof/mdat 拼成连续内存 demux。
            // 旧路径让 isomp4 在 virtual-body 上自己 next_packet：
            // - MoovSegment stco 样本会被映射到中段字节 → 假 AAC
            // - Unsupported("coupling channel element") 会直接杀 worker
            // - 段边界假 EOF 会让 worker frames≈1k 就 exhausted
            // 拼接后 demuxer 看到的是真实连续字节，坐标无歧义。
            return Self::new_remote_virtual_spliced(source, finish_handle);
        }
        let buffer_len = REMOTE_DECODER_BUFFER_BYTES;
        let options = MediaSourceStreamOptions { buffer_len };
        let mss = MediaSourceStream::new(Box::new(source), options);
        let decoder =
            Self::init_remote(mss, None).map_err(|err| format!("Decode error: {}", err))?;
        finish_handle.finish_demuxer_open();
        finish_handle.clear_virtual_body();
        log::info!(
            target: "remote-audio",
            "demuxer open finished sequential_cleared=true virtual_body=false",
        );
        Ok(decoder)
    }

    /// virtual-body：header ∥ 目标 moof 的**惰性**拼接视图
    ///
    /// 不预先把 body 拉进内存。`RemoteAudioSource` 自己做逻辑↔物理地址翻译
    /// （`map_logical_to_physical` / `logical_len` / `normalize_seek_offset`），
    /// demuxer 读到哪就按需拉哪一块。
    ///
    /// 之前那版先阻塞拉满 2MB 再交给 `Cursor`，两个后果都致命：
    /// - 慢：4 次串行 Range 请求，实测 seek 要等 8 秒
    /// - 错：2MB 处凭空多出一个 EOF，probe 扫 atom 直接撞上 end of stream
    fn new_remote_virtual_spliced(
        source: RemoteAudioSource,
        finish_handle: RemoteAudioSource,
    ) -> Result<Self, String> {
        let header_end = source.inner.header_end.load(Ordering::Acquire);
        let origin = source.inner.virtual_body_origin.load(Ordering::Acquire);
        if header_end == 0 || origin == 0 {
            return Err("virtual-body splice requires header_end and body_origin".into());
        }
        log::info!(
            target: "remote-audio",
            "virtual-body lazy view header_end={}, body_origin={}, logical_len={}",
            header_end,
            origin,
            source.logical_len(),
        );

        let options = MediaSourceStreamOptions {
            buffer_len: REMOTE_VIRTUAL_DECODER_BUFFER_BYTES,
        };
        let source_eof = Arc::new(AtomicBool::new(false));
        let tracked = RemoteDecoderSource {
            source,
            reached_eof: Arc::clone(&source_eof),
        };
        let mss = MediaSourceStream::new(Box::new(tracked), options);
        // 探测期间必须维持 demuxer_open_sequential：
        // is_seekable 一旦为真，try_new 会把整条逻辑流的顶层 atom 扫一遍去找
        // 全部 sidx，等于把刚省下的顺序读又加回来。顺序模式下它遇到第一个
        // moof 就 break，正好停在目标分片上。探测结束后由 init 清标志，
        // 之后的样本级 Seek 与 ignore_bytes 跳段才能生效。
        let mut decoder = Self::init_remote_spliced(mss, Some(finish_handle))
            .map_err(|err| format!("Decode error: {}", err))?;
        decoder.source_eof = Some(source_eof);
        log::info!(
            target: "remote-audio",
            "demuxer open finished sequential_cleared=true virtual_body=true lazy=true",
        );
        Ok(decoder)
    }

    pub fn new_file(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|err| format!("Cannot open file: {}", err))?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !matches!(*value, "audio" | "part"));
        Self::new(Box::new(file), extension)
    }

    fn init(
        mss: MediaSourceStream,
        extension: Option<&str>,
    ) -> symphonia::core::errors::Result<Self> {
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let mut probed = get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        let track_id = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(SymphoniaError::Unsupported("No track with supported codec"))?
            .id;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .ok_or(SymphoniaError::Unsupported("Selected track disappeared"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
        let total_duration =
            track_duration(track.codec_params.time_base, track.codec_params.n_frames);

        let mut decode_errors = 0usize;
        let decoded = loop {
            let packet = probed.format.next_packet()?;
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => break decoded,
                Err(SymphoniaError::DecodeError(_)) if decode_errors < MAX_DECODE_RETRIES => {
                    decode_errors += 1;
                }
                Err(err) => return Err(err),
            }
        };
        let spec = decoded.spec().to_owned();
        let buffer = Self::copy_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            track_id,
            total_duration,
            buffer,
            spec,
            retry_virtual_body_edges: false,
            source_eof: None,
            exhausted: false,
        })
    }

    /// 远程打开：try_new 期间可保持 sequential；探测完成后立刻恢复 seekable 再读首包
    fn init_remote(
        mss: MediaSourceStream,
        enable_seek_after_probe: Option<RemoteAudioSource>,
    ) -> symphonia::core::errors::Result<Self> {
        let hint = Hint::new();
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let mut probed = get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        // try_new 已结束：允许样本级 Seek（virtual-body 必需）
        if let Some(handle) = enable_seek_after_probe.as_ref() {
            handle.finish_demuxer_open();
        }

        let track_id = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(SymphoniaError::Unsupported("No track with supported codec"))?
            .id;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .ok_or(SymphoniaError::Unsupported("Selected track disappeared"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
        let total_duration =
            track_duration(track.codec_params.time_base, track.codec_params.n_frames);

        // 中段 moof 首帧可能非 RAP，多跳过一些 DecodeError。
        // 注意：fragmented 流里 moov 常无样本，首个 moof 的 first_ts 也可能是 0，
        // 绝不能按 ts==0 丢弃，否则会把目标段整段扔掉。
        let max_errors = if enable_seek_after_probe.is_some() {
            MAX_VIRTUAL_BODY_SKIP_PACKETS
        } else {
            MAX_DECODE_RETRIES
        };
        let mut decode_errors = 0usize;
        let mut packet_idx = 0usize;
        let decoded = loop {
            let packet = probed.format.next_packet()?;
            if packet.track_id() != track_id {
                continue;
            }
            packet_idx += 1;
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if enable_seek_after_probe.is_some() {
                        log::info!(
                            target: "remote-audio",
                            "virtual-body first good packet idx={} ts={} dur={} bytes={} after_errors={}",
                            packet_idx,
                            packet.ts(),
                            packet.dur(),
                            packet.buf().len(),
                            decode_errors,
                        );
                    }
                    break decoded;
                }
                Err(SymphoniaError::DecodeError(msg)) if decode_errors < max_errors => {
                    decode_errors += 1;
                    if enable_seek_after_probe.is_some() && decode_errors <= 4 {
                        log::warn!(
                            target: "remote-audio",
                            "virtual-body decode error idx={} ts={} bytes={} err={}",
                            packet_idx,
                            packet.ts(),
                            packet.buf().len(),
                            msg,
                        );
                    }
                    decoder.reset();
                }
                Err(err) => return Err(err),
            }
        };
        let spec = decoded.spec().to_owned();
        let buffer = Self::copy_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            track_id,
            total_duration,
            buffer,
            spec,
            retry_virtual_body_edges: enable_seek_after_probe.is_some(),
            source_eof: None,
            exhausted: false,
        })
    }

    /// 拼接 demux：本地 Cursor 全程 seekable；首包必须来自 moof（跳过 moov stco 假样本）
    fn init_remote_spliced(
        mss: MediaSourceStream,
        enable_seek_after_probe: Option<RemoteAudioSource>,
    ) -> symphonia::core::errors::Result<Self> {
        let hint = Hint::new();
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let mut probed = get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        // try_new 已结束：放开可 seek，样本级跳转与 ignore_bytes 跳段都要靠它
        if let Some(handle) = enable_seek_after_probe.as_ref() {
            handle.finish_demuxer_open();
        }

        let track_id = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(SymphoniaError::Unsupported("No track with supported codec"))?
            .id;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .ok_or(SymphoniaError::Unsupported("Selected track disappeared"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
        let total_duration =
            track_duration(track.codec_params.time_base, track.codec_params.n_frames);

        // 先吞掉 moov 段样本（stco 指向文件头 mdat，在拼接视图里是错误数据）
        // 策略：连续 DecodeError / Unsupported 都跳；一旦解出好包就停
        let mut skipped = 0usize;
        let mut decode_errors = 0usize;
        let decoded = loop {
            let packet = match probed.format.next_packet() {
                Ok(p) => p,
                Err(err)
                    if skipped < MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES
                        && splice_skip_retryable(&err) =>
                {
                    // 可能还在假样本区；再试
                    skipped += 1;
                    if skipped <= 4 {
                        log::warn!(
                            target: "remote-audio",
                            "splice next_packet err while skipping moov: {}",
                            err,
                        );
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            if packet.track_id() != track_id {
                continue;
            }
            skipped += 1;
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    log::info!(
                        target: "remote-audio",
                        "splice first good packet after_skip={} ts={} dur={} bytes={}",
                        skipped,
                        packet.ts(),
                        packet.dur(),
                        packet.buf().len(),
                    );
                    break decoded;
                }
                Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::Unsupported(_))
                    if decode_errors < MAX_VIRTUAL_BODY_SKIP_PACKETS =>
                {
                    decode_errors += 1;
                    decoder.reset();
                }
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                }
                Err(err)
                    if skipped < MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES
                        && splice_skip_retryable(&err) =>
                {
                    // 其它瞬时错误也跳
                    if skipped <= 4 {
                        log::warn!(
                            target: "remote-audio",
                            "splice skip decode err: {}",
                            err,
                        );
                    }
                    decoder.reset();
                }
                Err(err) => return Err(err),
            }
            if skipped >= MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES {
                return Err(SymphoniaError::DecodeError(
                    "virtual-body splice: could not find decodable moof sample",
                ));
            }
        };

        let spec = decoded.spec().to_owned();
        let buffer = Self::copy_buffer(decoded, &spec);
        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            track_id,
            total_duration,
            buffer,
            spec,
            retry_virtual_body_edges: true,
            source_eof: None,
            exhausted: false,
        })
    }

    /// 已有好包后继续跳过坏包（当前未使用；拼接路径在 init 内已处理）
    #[allow(dead_code)]
    fn skip_to_fragment_samples(&mut self, max_packets: usize) -> usize {
        let mut skipped = 0usize;
        while skipped < max_packets {
            // 已有 buffer 里是好包就直接返回
            if !self.buffer.is_empty() && self.current_frame_offset < self.buffer.len() {
                break;
            }
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(_) => break,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    decoded.spec().clone_into(&mut self.spec);
                    self.buffer = Self::copy_buffer(decoded, &self.spec);
                    self.current_frame_offset = 0;
                    break;
                }
                Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::Unsupported(_)) => {
                    skipped += 1;
                    self.decoder.reset();
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                }
                Err(_) => break,
            }
        }
        skipped
    }

    fn copy_buffer(decoded: AudioBufferRef<'_>, spec: &SignalSpec) -> SampleBuffer<f32> {
        let duration = units::Duration::from(decoded.capacity() as u64);
        let mut buffer = SampleBuffer::<f32>::new(duration, *spec);
        buffer.copy_interleaved_ref(decoded);
        buffer
    }

    fn refine_position(&mut self, seek_res: SeekedTo) -> Result<(), PcmSeekError> {
        let mut samples_to_pass = seek_res.required_ts.saturating_sub(seek_res.actual_ts);
        let packet = loop {
            let candidate = self
                .format
                .next_packet()
                .map_err(|err| seek_error(format!("Could not refine seek: {}", err)))?;
            if candidate.dur() > samples_to_pass {
                break candidate;
            }
            samples_to_pass = samples_to_pass.saturating_sub(candidate.dur());
        };

        let mut decoded = self.decoder.decode(&packet);
        for _ in 0..MAX_DECODE_RETRIES {
            if decoded.is_ok() {
                break;
            }
            let packet = self
                .format
                .next_packet()
                .map_err(|err| seek_error(format!("Could not retry after seek: {}", err)))?;
            decoded = self.decoder.decode(&packet);
        }
        let decoded =
            decoded.map_err(|err| seek_error(format!("Could not decode after seek: {}", err)))?;
        decoded.spec().clone_into(&mut self.spec);
        self.buffer = Self::copy_buffer(decoded, &self.spec);
        self.current_frame_offset = samples_to_pass as usize * self.channels() as usize;
        Ok(())
    }
}

// skip_decode_errors 已移除：init_remote 内已处理坏包；外部再跳会让 buffer 与 format 失步

impl Iterator for SymphoniaAudioDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.exhausted {
            return None;
        }
        if self.current_frame_offset >= self.buffer.len() {
            // 中段流允许跳过更多坏包；真 EOF 才返回 None
            let mut decode_errors = 0usize;
            let mut consecutive_io = 0usize;
            let decoded = loop {
                let packet = match self.format.next_packet() {
                    Ok(packet) => {
                        consecutive_io = 0;
                        packet
                    }
                    Err(SymphoniaError::ResetRequired) => {
                        self.decoder.reset();
                        continue;
                    }
                    Err(SymphoniaError::IoError(err))
                        if self.retry_virtual_body_edges
                            && consecutive_io < MAX_VIRTUAL_BODY_SKIP_PACKETS
                            && err.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // 网络重试由按需读取负责，这里只等待明确的暂不可读状态
                        consecutive_io += 1;
                        if consecutive_io <= 3 {
                            log::warn!(
                                target: "remote-audio",
                                "virtual-body next_packet io retry={} kind={} msg={}",
                                consecutive_io,
                                err.kind(),
                                err,
                            );
                        }
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(SymphoniaError::IoError(error))
                        if error.kind() == io::ErrorKind::UnexpectedEof =>
                    {
                        if self.retry_virtual_body_edges
                            && !self
                                .source_eof
                                .as_ref()
                                .is_some_and(|eof| eof.load(Ordering::Acquire))
                        {
                            log::warn!(target: "remote-audio", "virtual-body stream ended before source EOF: {error}");
                        }
                        self.exhausted = true;
                        return None;
                    }
                    Err(err) => {
                        log::warn!(
                            target: "remote-audio",
                            "virtual-body next_packet fatal: {}",
                            err,
                        );
                        self.exhausted = true;
                        return None;
                    }
                };
                if packet.track_id() != self.track_id {
                    continue;
                }

                match self.decoder.decode(&packet) {
                    Ok(decoded) => break decoded,
                    // DecodeError + Unsupported（如 aac coupling）都可跳，不能当 fatal 杀 worker
                    Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::Unsupported(_))
                        if decode_errors < MAX_VIRTUAL_BODY_SKIP_PACKETS =>
                    {
                        decode_errors += 1;
                        if decode_errors <= 4 || decode_errors.is_multiple_of(16) {
                            log::warn!(
                                target: "remote-audio",
                                "virtual-body stream decode skip count={}",
                                decode_errors,
                            );
                        }
                        self.decoder.reset();
                    }
                    Err(SymphoniaError::ResetRequired) => {
                        self.decoder.reset();
                    }
                    Err(err) => {
                        log::warn!(
                            target: "remote-audio",
                            "virtual-body stream decode fatal: {}",
                            err,
                        );
                        self.exhausted = true;
                        return None;
                    }
                }
            };
            decoded.spec().clone_into(&mut self.spec);
            self.buffer = Self::copy_buffer(decoded, &self.spec);
            self.current_frame_offset = 0;
        }

        let Some(sample) = self
            .buffer
            .samples()
            .get(self.current_frame_offset)
            .copied()
        else {
            self.exhausted = true;
            return None;
        };
        self.current_frame_offset += 1;
        Some(sample)
    }
}

impl PcmSource for SymphoniaAudioDecoder {
    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }

    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration.map(time_to_duration)
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), PcmSeekError> {
        let time = seek_target_time(self.total_duration, pos);

        let channel_offset = self.current_frame_offset % self.channels() as usize;
        let seek_res = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: None,
                },
            )
            .map_err(|err| seek_error(format!("Format seek failed: {}", err)))?;
        self.decoder.reset();
        self.refine_position(seek_res)?;
        self.current_frame_offset += channel_offset;
        self.exhausted = false;
        Ok(())
    }
}

fn time_to_duration(time: Time) -> Duration {
    Duration::from_secs(time.seconds)
        + Duration::from_nanos((time.frac * 1_000_000_000.0).max(0.0) as u64)
}

pub(super) fn track_duration(time_base: Option<TimeBase>, n_frames: Option<u64>) -> Option<Time> {
    let frames = n_frames.filter(|frames| *frames > 0)?;
    time_base.map(|base| base.calc_time(frames))
}

pub(super) fn seek_target_time(total_duration: Option<Time>, position: Duration) -> Time {
    match total_duration.filter(|duration| !time_to_duration(*duration).is_zero()) {
        Some(duration)
            if time_to_duration(duration)
                .saturating_sub(position)
                .as_millis()
                < 1 =>
        {
            skip_back_tiny_bit(duration)
        }
        _ => position.as_secs_f64().into(),
    }
}

fn skip_back_tiny_bit(mut time: Time) -> Time {
    time.frac -= 0.0001;
    if time.frac < 0.0 {
        time.seconds = time.seconds.saturating_sub(1);
        time.frac += 1.0;
    }
    time
}

fn seek_error(message: String) -> PcmSeekError {
    PcmSeekError::new(message)
}
