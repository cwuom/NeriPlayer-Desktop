//! RemoteAudioSource：按需读取、seek 与缓存命中

use super::*;

impl RemoteAudioSource {
    pub async fn open(
        client: reqwest::Client,
        url: String,
        referer: String,
        disk_cache: Option<RemoteAudioCache>,
        duration_hint_ms: u64,
        playback_generation: Arc<AtomicU64>,
        expected_generation: u64,
    ) -> AppResult<Self> {
        let open_started = Instant::now();
        let host = url::Url::parse(&url)
            .ok()
            .and_then(|value| value.host_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        let access_mode = RemoteAccessMode::for_url(&url, duration_hint_ms);
        log::info!(
            target: "remote-audio",
            "open begin host={}, mode={:?}, generation={}, duration_hint_ms={}",
            host,
            access_mode,
            expected_generation,
            duration_hint_ms,
        );
        if playback_generation.load(Ordering::Acquire) != expected_generation {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
        let initial_block = initial_block_bytes(duration_hint_ms);
        let probe_started = Instant::now();
        tokio::select! {
            () = join_warming_probe(&url) => {}
            () = wait_for_generation_change(&playback_generation, expected_generation) => {
                return Err(AppError::Audio("Playback request superseded".into()));
            }
        }
        let warm = take_warm_probe(&url);
        let probe_elapsed_ms = warm.as_ref().map(|probe| probe.elapsed_ms);
        let range_result = if let Some(probe) = warm {
            log::info!(
                target: "remote-audio",
                "range probe warm hit host={}, age_ms={}, bytes={}",
                host,
                probe.fetched_at.elapsed().as_millis(),
                probe.data.len(),
            );
            Ok(ProbeHead {
                total_len: probe.total_len,
                data: probe.data,
                rest: None,
            })
        } else {
            // 连接层失败（重置、超时）只说明这条链路一时不通，换条连接再试一次；
            // 状态码或 Range 不受支持则直接交给调用方走兜底
            let range_probe = async {
                match probe_range_head(&client, &url, &referer, initial_block, true).await {
                    Err(AppError::Network(error)) if error.is_connect() || error.is_timeout() => {
                        log::warn!(
                            target: "remote-audio",
                            "range probe transport failure host={}, elapsed_ms={}, retrying: {}",
                            host,
                            probe_started.elapsed().as_millis(),
                            error.without_url(),
                        );
                        probe_range_head(&client, &url, &referer, initial_block, true).await
                    }
                    result => result,
                }
            };
            tokio::pin!(range_probe);
            tokio::select! {
                result = &mut range_probe => result,
                () = wait_for_generation_change(
                    &playback_generation,
                    expected_generation,
                ) => {
                    return Err(AppError::Audio("Playback request superseded".into()));
                }
            }
        };
        log::info!(
            target: "remote-audio",
            "range probe finished host={}, ok={}, elapsed_ms={}, total_ms={}",
            host,
            range_result.is_ok(),
            probe_started.elapsed().as_millis(),
            open_started.elapsed().as_millis(),
        );
        let (total_len, initial_segment, probe_rest) = match range_result {
            Ok(ProbeHead {
                total_len: len,
                data,
                rest,
            }) => {
                log::info!(
                    target: "remote-audio",
                    "range probe data host={}, total_len={}, initial_bytes={}, streaming_rest={}",
                    host,
                    len,
                    data.len(),
                    rest.as_ref().map_or(0, |(end, _)| end.saturating_add(1).saturating_sub(data.len() as u64)),
                );
                (len, Some(CachedSegment { start: 0, data }), rest)
            }
            Err(range_error) => {
                if playback_generation.load(Ordering::Acquire) != expected_generation {
                    return Err(AppError::Audio("Playback request superseded".into()));
                }
                log::warn!(
                    target: "remote-audio",
                    "range probe failed host={}, elapsed_ms={}, error={}",
                    host,
                    probe_started.elapsed().as_millis(),
                    summarize_remote_error(&range_error),
                );
                return Err(AppError::Audio(
                    "Remote source does not support seekable HTTP Range".into(),
                ));
            }
        };

        // 用首包探测播种吞吐 EWMA：冷启动第一个自适应块/超时预算就有依据。
        // 探测耗时含建连与 TTFB，样本偏保守（慢网首块更小），符合预期
        let initial_throughput_bps = throughput_sample_bps(
            initial_segment
                .as_ref()
                .map(|segment| segment.data.len() as u64)
                .unwrap_or(0),
            probe_elapsed_ms
                .unwrap_or(probe_started.elapsed().as_millis() as u64)
                .max(1),
        );

        if let Some(cache) = disk_cache.as_ref().cloned() {
            let cache_started = Instant::now();
            let queued_at = Instant::now();
            tokio::task::spawn_blocking(move || {
                log::info!(
                    target: "remote-cache",
                    "prepare worker started queued_ms={}",
                    queued_at.elapsed().as_millis(),
                );
                cache.prepare(total_len)
            })
            .await
            .map_err(|error| AppError::Other(error.to_string()))?
            .map_err(|error| AppError::Other(error.to_string()))?;
            log::info!(
                target: "remote-cache",
                "prepare host={}, total_len={}, elapsed_ms={}",
                host,
                total_len,
                cache_started.elapsed().as_millis(),
            );
        }

        let head_len = initial_segment
            .as_ref()
            .map_or(0, |segment| segment.data.len() as u64);
        let initial_disk_segment = initial_segment
            .as_ref()
            .map(|segment| (segment.start, segment.data.clone()));
        let cache = initial_segment.into_iter().collect();
        let prefetch_window = prefetch_window(total_len, duration_hint_ms);
        let mut protected_ranges = Vec::new();
        if let Some((start, data)) = &initial_disk_segment {
            if !data.is_empty() {
                protected_ranges.push(CachedRange {
                    start: *start,
                    end: start.saturating_add(data.len() as u64),
                });
            }
        }
        let head_bytes = initial_disk_segment
            .as_ref()
            .map(|(_, data)| data.as_slice())
            .unwrap_or(&[]);
        let header_end = detect_mp4_header_end(head_bytes).unwrap_or(0);
        // URL 后缀嗅探不到 googlevideo 的分片直链，必须按首包内容再判一次。
        //
        // 嗅探结果只用于「打开阶段顺序读」这一件事，不改 access_mode：
        // 分片模式的按需块是 8MB，那是给 Bilibili 独立 .m4s 分片文件调的，
        // 套到 YouTube 的单文件上反而拖慢 seek。
        // 索引构建不看嗅探结果：所有分片流（嗅探出的 + .m4s 直链）
        // 统一在下方 needs_seek_index 里按首包 sidx 建索引
        let sniffed_fragmented = !matches!(access_mode, RemoteAccessMode::FragmentedProgressive)
            && detect_fragmented_mp4(head_bytes);
        let inner = Arc::new(RemoteAudioInner {
            client,
            url,
            referer,
            total_len,
            header_end: AtomicU64::new(header_end),
            prefetch_window,
            cache: Mutex::new(cache),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(protected_ranges),
            disk_cache,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(initial_throughput_bps),
            bitrate_bps: total_len
                .saturating_mul(1000)
                .checked_div(duration_hint_ms)
                .unwrap_or(0),
            playback_generation,
            expected_generation,
        });
        if header_end > 0 {
            log::info!(
                target: "remote-audio",
                "mp4 header_end detected host={}, header_end={}",
                host,
                header_end,
            );
        }
        if sniffed_fragmented {
            // 打开阶段隐藏 seekable，避免 symphonia 顺着 moof 链把整个文件读完；
            // sidx 位于 header_end 之前，顺序读同样能拿到，不影响后续按 sidx seek。
            // finish_demuxer_open() 会在打开完成后恢复可 seek。
            inner.demuxer_open_sequential.store(true, Ordering::Release);
            log::info!(
                target: "remote-audio",
                "fragmented mp4 sniffed from payload host={}, mode={:?}, opening sequentially",
                host,
                access_mode,
            );
        }
        inner
            .ensure_playback_current()
            .map_err(|err| AppError::Audio(err.to_string()))?;
        if let Some((start, data)) = initial_disk_segment {
            write_disk_range(&inner, start, &data);
        }
        // 分片流必须建索引
        //
        // symphonia 0.5.5 的 isomp4 解析完 sidx 就丢掉（demuxer.rs 里只剩一句
        // "stream is segmented with a segment index" 日志），seek 时只能靠
        // try_read_more_segments 一个个 moof 往前爬，代价是 O(文件大小)：
        // 实测 68MB 的 70 分钟音频跳到 37 分钟要顺序读满 30MB。
        // 所以 sidx 必须由我们自己解析成索引，seek 时直接按字节落点开 demuxer。
        //
        // FragmentedProgressive（Bilibili .m4s 直链）同样必须建：它的 sidx
        // 一样在首包（header_end 之前），旧代码只认嗅探结果与 LongForm，
        // 导致 2 小时 118MB 的 fMP4 每次 seek 都顺序爬 moof ~12s
        let needs_seek_index = needs_head_seek_index(access_mode, sniffed_fragmented);
        // 分片 MP4 的 sidx 紧跟 moov 位于首包，先用首包建索引；
        // 建成了就不必再为长内容拉 1MB 尾部，直接省掉起播路径上的一次往返
        //
        // 不要求 duration_hint_ms > 0：sidx 自带 timescale 与分段时长，
        // 时长未知（YouTube 前端常传 hint=0）同样能建出精确索引；
        // 否则这类流会退化到 format.seek 顺序爬 moof，O(文件大小)——
        // 实测 58MB 一次 seek 要顺序下完全文件耗时 16.6s
        let head_index_ready = needs_seek_index
            && total_len > 0
            && build_seek_index_from_head(&inner, duration_hint_ms);

        // 长内容：尾部 moov/sidx 预取进缓存，seek 升级为 seekable 后 demuxer 能直接跳转
        if matches!(access_mode, RemoteAccessMode::LongFormProgressive)
            && total_len > 0
            && !head_index_ready
            // 分片流的 moov/sidx 都在首包，拉尾部只会白白拖慢起播
            && !sniffed_fragmented
        {
            let tail_len = REMOTE_LONG_FORM_TAIL_BYTES.min(total_len);
            let tail_start = total_len.saturating_sub(tail_len);
            // 与首包重叠时跳过（极短文件）
            if tail_start > 0 {
                let tail_end = total_len.saturating_sub(1);
                match fetch_range_block_async(&inner, tail_start, tail_end, None).await {
                    Ok(data) if !data.is_empty() => {
                        log::info!(
                            target: "remote-audio",
                            "long-form tail cached host={}, start={}, bytes={}",
                            host,
                            tail_start,
                            data.len(),
                        );
                        protect_cached_range(&inner, tail_start, data.len() as u64);
                        if let Ok(mut cache) = inner.cache.lock() {
                            if !cache_contains_position(&cache, tail_start) {
                                cache.push(CachedSegment {
                                    start: tail_start,
                                    data: data.clone(),
                                });
                                cache.sort_by_key(|segment| segment.start);
                                trim_cache(&inner, &mut cache, cache_center(&inner, 0));
                            }
                        }
                        write_disk_range(&inner, tail_start, &data);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!(
                            target: "remote-audio",
                            "long-form tail prefetch failed host={}, error={}",
                            host,
                            summarize_remote_error(&err),
                        );
                    }
                }
            }
        }
        // 首包没建成索引时再兜底：长内容可退到尾部 sidx / 线性估算，
        // 分片内容（嗅探出的 + .m4s 直链）只接受真实 sidx，解析不到就保持
        // 原 format.seek 顺序行为（函数内部会打 "seek index skipped" 日志）。
        // 线性估算依赖声明时长，hint=0（时长未知）时在函数内部自动禁用
        if needs_seek_index && total_len > 0 && !head_index_ready {
            let allow_linear_fallback =
                matches!(access_mode, RemoteAccessMode::LongFormProgressive);
            build_seek_index_for_long_form(&inner, duration_hint_ms, allow_linear_fallback);
        }
        if let Some((rest_end, response)) = probe_rest {
            continue_probe_in_background(&inner, head_len, rest_end, response, probe_started);
        }
        replenish_prefetch_window(&inner, 0);

        log::info!(
            target: "remote-audio",
            "open ready host={}, total_len={}, prefetch_low={}, prefetch_target={}, total_ms={}",
            host,
            total_len,
            prefetch_window.low_water_bytes,
            prefetch_window.target_bytes,
            open_started.elapsed().as_millis(),
        );

        Ok(Self {
            inner,
            pos: 0,
            access_mode,
            read_cancellation: None,
        })
    }

    pub fn byte_len(&self) -> u64 {
        self.inner.total_len
    }

    pub fn seekable_clone(&self) -> Self {
        // 分片 m4s → FragmentedSeekable（大块按需）；
        // 长内容渐进 m4a → StandardSeekable（用 moov 表跳转，绝不能走分片 8MB 逻辑）
        let (access_mode, sequential_demuxer_open) = match self.access_mode {
            RemoteAccessMode::FragmentedProgressive => (RemoteAccessMode::FragmentedSeekable, true),
            RemoteAccessMode::LongFormProgressive => (RemoteAccessMode::StandardSeekable, true),
            mode => (mode, false),
        };
        // 作废旧位置的预取，避免 seek 重建 demuxer 时和旧 in_flight 撞车
        invalidate_prefetch_epoch(&self.inner);
        // 长内容/分片升级：demuxer 打开必须假装不可 seek，否则会扫完整 mdat
        // 短内容本身就是 StandardSeekable，moov 可能在尾部，不能强制顺序打开
        if sequential_demuxer_open {
            self.inner
                .demuxer_open_sequential
                .store(true, Ordering::Release);
        }
        Self {
            inner: Arc::clone(&self.inner),
            pos: 0,
            access_mode,
            read_cancellation: self.read_cancellation.clone(),
        }
    }

    pub fn with_read_cancellation(mut self, cancellation: RemoteReadCancellation) -> Self {
        self.read_cancellation = Some(cancellation);
        self
    }

    /// demuxer 打开完成：允许真实时间 seek 的字节跳转
    pub fn finish_demuxer_open(&self) {
        self.inner
            .demuxer_open_sequential
            .store(false, Ordering::Release);
    }

    fn is_demuxer_open_sequential(&self) -> bool {
        self.inner.demuxer_open_sequential.load(Ordering::Acquire)
    }

    /// 长内容 seek：用「虚拟 body」打开 demuxer，而不是 format.seek 扫全文件
    ///
    /// LongFormProgressive 也算：seek 时会先 seekable_clone 升级，再走虚拟 body。
    /// 判断必须在 clone 前后都能成立，否则会误走 format.seek 扫整文件 mdat。
    pub fn prefers_virtual_body_seek(&self) -> bool {
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if header_end == 0 {
            return false;
        }
        let mode_ok = matches!(
            self.access_mode,
            RemoteAccessMode::LongFormProgressive
                | RemoteAccessMode::StandardSeekable
                | RemoteAccessMode::FragmentedProgressive
                | RemoteAccessMode::FragmentedSeekable
        );
        if !mode_ok {
            return false;
        }
        self.inner
            .seek_index
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|index| !index.entries.is_empty()))
            .unwrap_or(false)
    }

    /// 取出文件头缓存（ftyp/moov/sidx），供 virtual-body 拼接 demux 使用
    pub fn header_bytes(&self) -> Option<Vec<u8>> {
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if header_end == 0 {
            return None;
        }
        let need = header_end as usize;
        let mut out = vec![0u8; need];
        // 优先内存缓存 start=0
        if let Ok(cache) = self.inner.cache.lock() {
            if let Some(seg) = cache.iter().find(|s| s.start == 0) {
                let n = seg.data.len().min(need);
                out[..n].copy_from_slice(&seg.data[..n]);
                if n == need {
                    return Some(out);
                }
            }
        }
        // 磁盘缓存兜底
        if let Some(disk) = &self.inner.disk_cache {
            if disk.read_cached(0, &mut out).unwrap_or(0) == need {
                return Some(out);
            }
        }
        None
    }

    /// 配置虚拟 body：逻辑 [0, header_end) = 文件头；逻辑 header_end.. = 目标 moof 起
    ///
    /// 解码会从目标所在分片的开头出声。返回该分片的起点时间（只有落点确实是 sidx
    /// 记录的那个分片时才有），调用方据此丢掉分片开头到目标之间的音频
    pub fn configure_virtual_body_for_time(&self, position_ms: u64) -> io::Result<Option<u64>> {
        self.ensure_read_current()?;
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if header_end == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mp4 header_end unknown",
            ));
        }
        let index = self
            .inner
            .seek_index
            .lock()
            .map_err(|_| io::Error::other("seek index lock poisoned"))?
            .clone();
        let Some(index) = index else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "seek index missing",
            ));
        };
        // 只算 sidx 落点；真正 body 拉取交给 splice 分块（避免这里一次拉过大超时）
        let (seg_start, seg_start_ms) = index.estimate_segment(position_ms);
        let mut target = seg_start
            .max(header_end)
            .min(self.inner.total_len.saturating_sub(1));
        let mut exact_start = index.exact_times && target == seg_start;

        // 轻量探测：最多拉 256KB 校验 moof 并对齐；失败也不致命，splice 会再 snap
        let probe_end = target
            .saturating_add(REMOTE_FETCH_BLOCK_BYTES)
            .saturating_sub(1)
            .min(self.inner.total_len.saturating_sub(1));
        let probe = if position_is_cached_range(&self.inner, target, probe_end) {
            None
        } else {
            // seek 关键路径同样属于需求读：探测期间预取让路
            let _demand_guard = DemandFetchGuard::new(&self.inner);
            match fetch_range_block(
                &self.inner,
                target,
                probe_end,
                self.read_cancellation.as_ref(),
            ) {
                Ok(data) if !data.is_empty() => Some(data),
                Ok(_) => None,
                // 403/401 对本直链是永久错误：继续配置虚拟 body 只会让
                // demuxer 打开阶段对同一 range 无限重试刷 403，
                // 立即终止本次 seek 让上层回滚/换直链
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                    log::warn!(
                        target: "remote-audio",
                        "virtual-body probe forbidden at {}: {}",
                        target,
                        err,
                    );
                    return Err(err);
                }
                Err(err) => {
                    log::warn!(
                        target: "remote-audio",
                        "virtual-body probe fetch failed at {}: {}",
                        target,
                        err,
                    );
                    None
                }
            }
        };
        if let Some(data) = probe {
            if let Some(rel) = find_moof_offset_near(&data, 0, 64 * 1024) {
                if rel > 0 {
                    log::warn!(
                        target: "remote-audio",
                        "virtual-body sidx miss-aligned by {} bytes, snapping to moof",
                        rel,
                    );
                    target = target
                        .saturating_add(rel)
                        .min(self.inner.total_len.saturating_sub(1));
                    exact_start = false;
                }
                let store = data[rel as usize..].to_vec();
                if !store.is_empty() {
                    if let Ok(mut cache) = self.inner.cache.lock() {
                        if !cache_contains_position(&cache, target) {
                            cache.push(CachedSegment {
                                start: target,
                                data: store.clone(),
                            });
                            cache.sort_by_key(|segment| segment.start);
                            trim_cache(&self.inner, &mut cache, target);
                        }
                    }
                    write_disk_range(&self.inner, target, &store);
                    protect_cached_range(&self.inner, target, store.len() as u64);
                }
            } else {
                log::warn!(
                    target: "remote-audio",
                    "virtual-body probe {:#x} has no moof in first 64KiB (head={:02x?})",
                    target,
                    &data[..data.len().min(16)],
                );
            }
        }

        self.inner
            .virtual_body_origin
            .store(target, Ordering::Release);
        self.inner
            .demuxer_open_sequential
            .store(true, Ordering::Release);
        invalidate_prefetch_epoch(&self.inner);
        let logical_len = header_end.saturating_add(self.inner.total_len.saturating_sub(target));
        log::info!(
            target: "remote-audio",
            "virtual-body configured target_ms={}, segment_start_ms={:?}, header_end={}, body_origin={}, total_len={}, logical_len={}",
            position_ms,
            exact_start.then_some(seg_start_ms),
            header_end,
            target,
            self.inner.total_len,
            logical_len,
        );
        Ok(exact_start.then_some(seg_start_ms))
    }

    pub fn clear_virtual_body(&self) {
        self.inner.virtual_body_origin.store(0, Ordering::Release);
    }

    /// virtual-body **纯逻辑连续**坐标（AtomIterator / moof_base_pos 共用）：
    /// - [0, header_end) → 物理 header
    /// - [header_end + k) → 物理 body_origin + k
    ///
    /// 绝对 base_data_offset 的 sample Seek 在 `seek()` 里先折算成逻辑坐标，
    /// 禁止在 map 里对 pos≥origin 做恒等：否则 MSS pos 跳到物理 55MB，
    /// AtomIterator 仍在逻辑 ~header_end，下一 atom 直接 overread / EOF。
    pub(super) fn map_logical_to_physical(&self, pos: u64) -> u64 {
        let origin = self.inner.virtual_body_origin.load(Ordering::Acquire);
        if origin == 0 {
            return pos.min(self.inner.total_len);
        }
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if pos < header_end {
            pos.min(self.inner.total_len)
        } else {
            origin
                .saturating_add(pos - header_end)
                .min(self.inner.total_len)
        }
    }

    pub(super) fn logical_len(&self) -> u64 {
        let origin = self.inner.virtual_body_origin.load(Ordering::Acquire);
        if origin == 0 {
            return self.inner.total_len;
        }
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        // 纯连续逻辑长度；绝对 sample 偏移在 seek 时折算，不靠拉长 logical_len
        header_end.saturating_add(self.inner.total_len.saturating_sub(origin))
    }

    /// 将 demuxer 的 Seek 目标折成逻辑坐标。
    /// - 相对 moof：目标已在 [0, logical_len)
    /// - 绝对 base_data_offset：目标 ≥ body_origin，折成 header_end + (phys - origin)
    pub(super) fn normalize_seek_offset(&self, offset: u64) -> u64 {
        let origin = self.inner.virtual_body_origin.load(Ordering::Acquire);
        if origin == 0 {
            return offset.min(self.inner.total_len);
        }
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        let logical_len = self.logical_len();
        if offset >= origin {
            // 绝对文件偏移 → 逻辑 body
            let logical = header_end.saturating_add(offset - origin);
            logical.min(logical_len)
        } else {
            offset.min(logical_len)
        }
    }

    fn read_cancelled(&self) -> bool {
        self.inner.playback_cancelled()
            || self
                .read_cancellation
                .as_ref()
                .is_some_and(RemoteReadCancellation::is_cancelled)
    }

    fn ensure_read_current(&self) -> io::Result<()> {
        if self.read_cancelled() {
            // 取消是永久态；read_exact 和 Symphonia 会对 Interrupted 无界重试
            Err(io::Error::other("remote read superseded"))
        } else {
            Ok(())
        }
    }

    /// 按物理偏移读内存缓存
    fn read_cached_at(&self, physical: u64, out: &mut [u8]) -> usize {
        let cache = match self.inner.cache.lock() {
            Ok(cache) => cache,
            Err(_) => return 0,
        };

        for segment in cache.iter() {
            let segment_end = segment.start + segment.data.len() as u64;
            if physical < segment.start || physical >= segment_end {
                continue;
            }

            let offset = (physical - segment.start) as usize;
            let available = segment.data.len().saturating_sub(offset);
            let len = available.min(out.len());
            out[..len].copy_from_slice(&segment.data[offset..offset + len]);
            return len;
        }

        0
    }

    fn read_disk_cached_at(&self, physical: u64, out: &mut [u8]) -> usize {
        let Some(cache) = &self.inner.disk_cache else {
            return 0;
        };
        let Some(len) = disk_cached_len(&self.inner, physical, out.len()) else {
            return 0;
        };
        cache.read_cached(physical, &mut out[..len]).unwrap_or(0)
    }

    fn has_cached_physical(&self, physical: u64) -> bool {
        let cache = match self.inner.cache.lock() {
            Ok(cache) => cache,
            Err(_) => return false,
        };

        cache.iter().any(|segment| {
            let segment_end = segment.start + segment.data.len() as u64;
            physical >= segment.start && physical < segment_end
        })
    }

    /// 按物理偏移拉取；全程不改 self.pos（self.pos 只存逻辑坐标）
    fn fetch_physical_range(&self, physical: u64, wanted_len: usize) -> io::Result<()> {
        self.ensure_read_current()?;
        if physical >= self.inner.total_len {
            return Ok(());
        }

        // 虚拟 body 生效时改用 seek 专用块，避免 8MB 拖垮跳转延迟
        let mode_block = if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0 {
            self.access_mode.seek_fetch_block_bytes()
        } else {
            self.access_mode.demand_fetch_block_bytes()
        };
        // 慢网自适应：按近期吞吐缩块（8s 可完成），mode 块只作上限；
        // 无样本时保持 mode 默认块，冷启动行为不变
        let block = adaptive_block_bytes(self.inner.throughput_bps(), mode_block, mode_block);
        let wanted = wanted_len.max(block as usize) as u64;
        let end = physical
            .saturating_add(wanted)
            .saturating_sub(1)
            .min(self.inner.total_len.saturating_sub(1));

        // 「查缓存 → 等在途共享 → claim 亲拉」最多三轮：
        // 同一块已有请求在途时等待并共享其结果，绝不重复发起（重复只会把
        // 带宽劈半，数据不会更早到达）；claim 竞态失败则下一轮转入等待
        let mut claimed = None;
        for _ in 0..3 {
            self.ensure_read_current()?;
            if self.has_cached_physical(physical)
                || disk_cached_len(&self.inner, physical, 1).is_some()
            {
                return Ok(());
            }
            if self.wait_for_inflight(physical) {
                return Ok(());
            }
            self.ensure_read_current()?;
            if let Some(entry) = claim_prefetch_start(&self.inner, physical, end) {
                claimed = Some(entry);
                break;
            }
        }
        let Some(entry) = claimed else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "remote range is already being fetched",
            ));
        };

        // 需求读在网标记：预取 lane 看到后暂缓开新请求，带宽让给解码急需的数据
        let demand_guard = DemandFetchGuard::new(&self.inner);
        // 超时/传输中断绝不直接 fatal：对同一起点折半块大小重试（短退避），
        // 连续 REMOTE_DEMAND_MAX_ATTEMPTS 次失败才把错误抛给解码层。
        // 播放侧表现为 rebuffering（read 阻塞在重试循环里）而非会话死亡
        let mut attempt_end = end;
        let mut attempt = 0usize;
        let fetched = loop {
            attempt += 1;
            match fetch_range_block(
                &self.inner,
                physical,
                attempt_end,
                self.read_cancellation.as_ref(),
            ) {
                Ok(data) => break Ok(data),
                Err(err)
                    if attempt < REMOTE_DEMAND_MAX_ATTEMPTS && is_transient_fetch_error(&err) =>
                {
                    let current_len = attempt_end.saturating_sub(physical).saturating_add(1);
                    let next_len = halved_fetch_len(current_len);
                    attempt_end = physical
                        .saturating_add(next_len)
                        .saturating_sub(1)
                        .min(self.inner.total_len.saturating_sub(1));
                    // 收缩在途登记边界并唤醒等待方：落在收缩区外的等待方
                    // 立即改走自己的 claim，不悬挂在本重试循环上
                    entry.end.store(attempt_end, Ordering::Release);
                    self.inner.range_available.notify_all();
                    log::warn!(
                        target: "remote-range",
                        "demand fetch retry attempt={}/{} range={}..{} next_len={} error={}",
                        attempt,
                        REMOTE_DEMAND_MAX_ATTEMPTS,
                        physical,
                        attempt_end,
                        next_len,
                        summarize_remote_error(&err),
                    );
                    // 短退避（按次数放大），期间保持取消即时性
                    let backoff =
                        Duration::from_millis(REMOTE_DEMAND_RETRY_BACKOFF_MS * attempt as u64);
                    let backoff_started = Instant::now();
                    while backoff_started.elapsed() < backoff {
                        if self.ensure_read_current().is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    if let Err(cancelled) = self.ensure_read_current() {
                        break Err(cancelled);
                    }
                }
                Err(err) => break Err(err),
            }
        };
        drop(demand_guard);
        let data = match fetched {
            Ok(data) => data,
            Err(err) => {
                release_prefetch_start(&self.inner, physical, &entry);
                return Err(err);
            }
        };

        if data.is_empty() {
            release_prefetch_start(&self.inner, physical, &entry);
            return Ok(());
        }

        // 先入缓存再发布 done：共享等待方醒来即可命中，不会空等一场
        let insert_result = (|| -> io::Result<()> {
            let mut cache = self
                .inner
                .cache
                .lock()
                .map_err(|_| io::Error::other("remote cache lock poisoned"))?;
            if !cache_contains_position(&cache, physical) {
                cache.push(CachedSegment {
                    start: physical,
                    data,
                });
                cache.sort_by_key(|segment| segment.start);
                trim_cache(&self.inner, &mut cache, cache_center(&self.inner, physical));
            }
            Ok(())
        })();
        release_prefetch_start(&self.inner, physical, &entry);
        insert_result?;
        replenish_prefetch_window(&self.inner, physical);
        Ok(())
    }

    /// 等待覆盖 start 的在途请求完成并共享其缓存结果。
    ///
    /// 旧实现只等 800ms 就强抢重发同一 range，慢链路上必然双请求并发、
    /// 带宽减半；这里改为等到发布方置 done。seek 换代时
    /// invalidate_prefetch_epoch 会立刻置 done 解除等待，低延迟语义不变。
    fn wait_for_inflight(&self, start: u64) -> bool {
        let entry = {
            let Ok(in_flight) = self.inner.in_flight.lock() else {
                return false;
            };
            in_flight
                .iter()
                .find(|(entry_start, entry)| {
                    **entry_start <= start && start <= entry.end.load(Ordering::Acquire)
                })
                .map(|(_, entry)| Arc::clone(entry))
        };
        let Some(entry) = entry else {
            return false;
        };

        let started = Instant::now();
        // 防御上限：发布方的请求超时上限为 REMOTE_REQUEST_TIMEOUT_CEILING，
        // 正常路径必在此之前发布结果
        let wait_budget = REMOTE_REQUEST_TIMEOUT_CEILING + Duration::from_secs(5);
        while !entry.done.load(Ordering::Acquire) && started.elapsed() < wait_budget {
            if self.read_cancelled() {
                return false;
            }
            // 发布方折半重试收缩了区间、本偏移已不被覆盖：立即退出等待，
            // 走自己的 claim 拉取，不陪发布方的重试循环空等
            if start > entry.end.load(Ordering::Acquire) {
                break;
            }
            let Ok(guard) = self.inner.in_flight.lock() else {
                return false;
            };
            // 拿锁后复查，避免错过 done 置位与 notify 之间的竞态窗口
            if entry.done.load(Ordering::Acquire) {
                break;
            }
            match self
                .inner
                .range_available
                .wait_timeout(guard, Duration::from_millis(40))
            {
                Ok(_) | Err(_) => {}
            }
        }

        self.has_cached_physical(start) || disk_cached_len(&self.inner, start, 1).is_some()
    }
}

impl Read for RemoteAudioSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.ensure_read_current()?;
        let logical_len = self.logical_len();
        if out.is_empty() || self.pos >= logical_len {
            return Ok(0);
        }

        let mut total_read = 0usize;
        while total_read < out.len() && self.pos < logical_len {
            self.ensure_read_current()?;
            let logical_pos = self.pos;
            let physical = self.map_logical_to_physical(logical_pos);

            // 单次读绝不许跨过 header_end 的拼接边界
            //
            // 虚拟 body 的逻辑地址在 header_end 处不连续，而缓存段在物理空间
            // 是连续的：从头部起读时若不封顶，read_cached_at 会顺着物理地址
            // 一路读进「文件开头的第一个 moof」——它也是合法 moof，demuxer
            // 便按它的 trun 去取样本，解出来的就是真假混杂的 AAC。
            // 封顶后本轮只读到边界，循环下一轮重新映射，body 从正确落点续读。
            let mut limit = out.len() - total_read;
            let header_end = self.inner.header_end.load(Ordering::Acquire);
            if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0
                && logical_pos < header_end
            {
                limit = limit.min((header_end - logical_pos) as usize);
            }
            let window = total_read + limit;

            // 全程不改 self.pos 为物理坐标：缓存按 physical 显式寻址
            let mut read = self.read_cached_at(physical, &mut out[total_read..window]);
            if read == 0 {
                read = self.read_disk_cached_at(physical, &mut out[total_read..window]);
            }
            // 已经读到数据、下一段还在路上时先交出去：FFmpeg 一次要 64KB，凑满要等慢链路上的下一块，
            // 起播就被这一块卡住。短读是 Read 的正常语义，调用方会接着读
            if read == 0 && total_read > 0 {
                break;
            }
            if read == 0 {
                match self.fetch_physical_range(physical, limit) {
                    Ok(()) => {
                        read = self.read_cached_at(physical, &mut out[total_read..window]);
                        if read == 0 {
                            read = self.read_disk_cached_at(physical, &mut out[total_read..window]);
                        }
                    }
                    Err(err) if total_read > 0 && can_return_partial_read(&err) => {
                        return Ok(total_read)
                    }
                    Err(err) => return Err(err),
                }
            }
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "remote range is not cached yet",
                ));
            }
            self.pos = logical_pos.saturating_add(read as u64);
            total_read += read;
            remember_read_pos(&self.inner, physical.saturating_add(read as u64));
            if !self.is_demuxer_open_sequential() {
                replenish_prefetch_window(&self.inner, physical.saturating_add(read as u64));
            }
        }

        Ok(total_read)
    }
}

impl Seek for RemoteAudioSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.ensure_read_current()?;
        let logical_len = self.logical_len();
        let raw_next = match pos {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::Current(offset) => self.pos as i128 + offset as i128,
            SeekFrom::End(offset) => logical_len as i128 + offset as i128,
        };

        if raw_next < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek before start",
            ));
        }

        let previous = self.pos;
        // Start 可能带绝对 base_data_offset；先折成逻辑坐标再 clamp
        let next = match pos {
            SeekFrom::Start(offset) => self.normalize_seek_offset(offset),
            _ => (raw_next as u64).min(logical_len),
        };
        self.pos = next;
        if self.pos.abs_diff(previous) > REMOTE_FETCH_BLOCK_BYTES {
            invalidate_prefetch_epoch(&self.inner);
        }
        let physical = self.map_logical_to_physical(self.pos);
        if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0
            && self.pos.abs_diff(previous) > 64
        {
            log::debug!(
                target: "remote-audio",
                "virtual-body seek logical={} physical={} header_end={} body_origin={} prev={} raw={}",
                self.pos,
                physical,
                self.inner.header_end.load(Ordering::Acquire),
                self.inner.virtual_body_origin.load(Ordering::Acquire),
                previous,
                raw_next,
            );
        }
        remember_read_pos(&self.inner, physical);
        // 返回逻辑 pos（MSS / AtomIterator 只认连续逻辑坐标）
        Ok(self.pos)
    }
}

impl MediaSource for RemoteAudioSource {
    fn is_seekable(&self) -> bool {
        // 仅 try_new 扫描顶层 atom 时隐藏 seekable，避免扫完整 mdat
        if self.is_demuxer_open_sequential() {
            return false;
        }
        // virtual-body：打开后必须可 seek，样本要从 mdat 回跳
        if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0 {
            return true;
        }
        self.access_mode.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        if self.is_demuxer_open_sequential() {
            return None;
        }
        if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0 {
            return Some(self.logical_len());
        }
        self.access_mode
            .is_seekable()
            .then_some(self.inner.total_len)
    }
}

impl crate::ffmpeg::ByteInput for RemoteAudioSource {
    // FFmpeg 靠 sidx/Cues 定位，不会像 symphonia 那样扫描全部顶层 atom，可以直接给出总长
    fn byte_len(&mut self) -> Option<u64> {
        Some(self.logical_len())
    }

    fn interrupted(&self) -> bool {
        self.read_cancelled()
    }
}
