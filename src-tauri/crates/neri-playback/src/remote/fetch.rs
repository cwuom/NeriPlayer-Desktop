//! HTTP Range 分块拉取、吞吐估计与自适应块大小

use super::*;

pub(super) fn fetch_range_block(
    inner: &RemoteAudioInner,
    start: u64,
    end: u64,
    read_cancellation: Option<&RemoteReadCancellation>,
) -> io::Result<Vec<u8>> {
    crate::runtime::block_on(fetch_range_block_async(
        inner,
        start,
        end,
        read_cancellation,
    ))
}

pub(super) async fn fetch_range_block_async(
    inner: &RemoteAudioInner,
    start: u64,
    end: u64,
    read_cancellation: Option<&RemoteReadCancellation>,
) -> io::Result<Vec<u8>> {
    let request_started = Instant::now();
    inner.ensure_playback_current()?;
    if read_cancellation.is_some_and(RemoteReadCancellation::is_cancelled) {
        return Err(io::Error::other("remote read superseded"));
    }
    let requested_len = end.saturating_sub(start).saturating_add(1);
    // 超时预算随块大小/近期吞吐缩放：块大或网慢时给足传输时间，
    // 防止固定 15s 在慢链路上"块越大越必超时"
    let request_timeout = request_timeout_for(requested_len, inner.throughput_bps());
    let request = async {
        let response = inner
            .client
            .get(&inner.url)
            .header(REFERER, inner.referer.as_str())
            .header(USER_AGENT, playback_user_agent(&inner.url))
            .header(RANGE, format!("bytes={}-{}", start, end))
            .timeout(request_timeout)
            .send()
            .await?;
        let status = response.status();
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(parse_content_range);
        // 只有 206 的响应体才会进入缓存。对 200/4xx 直接丢弃 response，
        // 避免 Range 被网关忽略时把整首音频读进内存后再报错（SR-04）
        let bytes = if status == StatusCode::PARTIAL_CONTENT {
            response.bytes().await?.to_vec()
        } else {
            Vec::new()
        };
        Ok::<_, reqwest::Error>((status, content_range, bytes))
    };
    tokio::pin!(request);
    let result = tokio::select! {
        result = &mut request => {
            match result {
                Ok(result) => result,
                Err(error) => {
                    let elapsed_ms = request_started.elapsed().as_millis() as u64;
                    let kind = classify_reqwest_error(&error);
                    // 超时/中断说明链路吞吐撑不起当前块：向下修正估计，
                    // 让下一个块（含折半重试）自动缩小
                    if matches!(
                        kind,
                        io::ErrorKind::TimedOut | io::ErrorKind::ConnectionAborted
                    ) {
                        inner.record_fetch_failure(requested_len, elapsed_ms);
                    }
                    log::warn!(
                        target: "remote-range",
                        "request failed generation={}, range={}..{}, elapsed_ms={}, timeout_ms={}, kind={:?}, error={}",
                        inner.expected_generation,
                        start,
                        end,
                        elapsed_ms,
                        request_timeout.as_millis(),
                        kind,
                        summarize_remote_error(&error),
                    );
                    return Err(io::Error::new(kind, error.to_string()));
                }
            }
        }
        () = wait_for_remote_read_cancellation(inner, read_cancellation) => {
            log::info!(
                target: "remote-range",
                "request cancelled generation={}, range={}..{}, elapsed_ms={}",
                inner.expected_generation,
                start,
                end,
                request_started.elapsed().as_millis(),
            );
            return Err(io::Error::other("remote read superseded"));
        }
    };

    let (status, content_range, data) = result;
    let elapsed_ms = request_started.elapsed().as_millis();
    if start == 0 || elapsed_ms >= 200 {
        log::info!(
            target: "remote-range",
            "response generation={}, range={}..{}, status={}, bytes={}, elapsed_ms={}",
            inner.expected_generation,
            start,
            end,
            status,
            data.len(),
            elapsed_ms,
        );
    } else {
        log::debug!(
            target: "remote-range",
            "response generation={}, range={}..{}, status={}, bytes={}, elapsed_ms={}",
            inner.expected_generation,
            start,
            end,
            status,
            data.len(),
            elapsed_ms,
        );
    }
    if status == StatusCode::PARTIAL_CONTENT {
        let content_range = content_range.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "partial response omitted Content-Range",
            )
        })?;
        if content_range.total != inner.total_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "remote length changed from {} to {}",
                    inner.total_len, content_range.total
                ),
            ));
        }
        validate_http_range_data(content_range, start, end, &data)?;
        inner.record_fetch_success(data.len() as u64, elapsed_ms.max(1) as u64);
        write_disk_range(inner, start, &data);
        return Ok(data);
    }

    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        // 远端内容长度发生变化时，空 Vec 会被 Read 当作 EOF，导致无声截断
        // 并错误推进到下一首。把永久范围错误交给上层的 URL 刷新/恢复路径
        // 处理，不能把它伪装成正常结束（SR-03）
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "remote range no longer satisfiable: 416",
        ));
    }

    if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
        // 对齐 Android YouTubeGoogleVideoRangeSupport.shouldRetryChunkError：
        // 403 是 CDN 拒绝访问（直链过期/远端 range 风控），重试与缩块都无解，
        // 必须按永久错误上抛让上层回滚或换直链；同时停掉本源后续预取孵化，
        // 否则每次读都会重新发起注定 403 的后台请求
        inner.remote_forbidden.store(true, Ordering::Release);
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("remote range request forbidden: {}", status),
        ));
    }

    Err(io::Error::other(format!(
        "unexpected remote range status: {}",
        status
    )))
}

async fn wait_for_remote_read_cancellation(
    inner: &RemoteAudioInner,
    read_cancellation: Option<&RemoteReadCancellation>,
) {
    while !inner.playback_cancelled()
        && !read_cancellation.is_some_and(RemoteReadCancellation::is_cancelled)
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn wait_for_generation_change(generation: &AtomicU64, expected: u64) {
    while generation.load(Ordering::Acquire) == expected {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 单请求实测吞吐样本（bytes/s）
pub(super) fn throughput_sample_bps(bytes: u64, elapsed_ms: u64) -> u64 {
    if bytes == 0 || elapsed_ms == 0 {
        return 0;
    }
    (u128::from(bytes) * 1000 / u128::from(elapsed_ms)) as u64
}

/// 成功样本按固定权重混入 EWMA；首个样本直接采纳
pub(super) fn ewma_bps_after_success(old_bps: u64, sample_bps: u64) -> u64 {
    if sample_bps == 0 {
        return old_bps;
    }
    if old_bps == 0 {
        return sample_bps;
    }
    let keep = 100 - THROUGHPUT_EWMA_NEW_WEIGHT_PERCENT;
    ((u128::from(old_bps) * u128::from(keep)
        + u128::from(sample_bps) * u128::from(THROUGHPUT_EWMA_NEW_WEIGHT_PERCENT))
        / 100) as u64
}

/// 失败样本（超时/传输中断）：requested/elapsed 只是真实吞吐的上界，
/// 只允许把估计往下拉；跑得太短的失败（连接秒断）不计入
pub(super) fn ewma_bps_after_failure(old_bps: u64, requested_bytes: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms < THROUGHPUT_FAILURE_MIN_ELAPSED_MS {
        return old_bps;
    }
    let upper_bound = throughput_sample_bps(requested_bytes, elapsed_ms);
    if upper_bound == 0 {
        return old_bps;
    }
    if old_bps == 0 {
        return upper_bound;
    }
    if upper_bound >= old_bps {
        return old_bps;
    }
    ewma_bps_after_success(old_bps, upper_bound)
}

/// 按近期吞吐选块：目标 8s 内可完成，64KB 对齐，夹在 [64KB, max_block]。
/// 无样本时用调用方默认块（保持冷启动行为不变）
pub(super) fn adaptive_block_bytes(ewma_bps: u64, default_block: u64, max_block: u64) -> u64 {
    if ewma_bps == 0 {
        return default_block.clamp(
            REMOTE_MIN_BLOCK_BYTES,
            max_block.max(REMOTE_MIN_BLOCK_BYTES),
        );
    }
    let raw = ewma_bps.saturating_mul(REMOTE_BLOCK_FILL_TARGET_SECS);
    let aligned = (raw / REMOTE_MIN_BLOCK_BYTES) * REMOTE_MIN_BLOCK_BYTES;
    aligned.clamp(
        REMOTE_MIN_BLOCK_BYTES,
        max_block.max(REMOTE_MIN_BLOCK_BYTES),
    )
}

/// per-request 超时预算：预计传输时长 ×1.5 + 5s 固定裕量，夹在 [20s, 60s]。
/// 无吞吐样本时用 20s 下限（冷启动块都不大，20s 足够 40KB/s 拉完 512KB）
pub(super) fn request_timeout_for(len: u64, ewma_bps: u64) -> Duration {
    if ewma_bps == 0 {
        return REMOTE_REQUEST_TIMEOUT_FLOOR;
    }
    let expected_ms =
        (u128::from(len) * 1000 / u128::from(ewma_bps)).min(u128::from(u64::MAX)) as u64;
    let budget = Duration::from_millis(expected_ms.saturating_mul(3) / 2 + 5_000);
    budget.clamp(REMOTE_REQUEST_TIMEOUT_FLOOR, REMOTE_REQUEST_TIMEOUT_CEILING)
}

/// 慢网降低预取并发：EWMA 低于码率时单流都喂不饱，多开只会挤占需求读；
/// 码率或吞吐未知时维持默认并发（冷启动行为不变）
pub(super) fn prefetch_lane_count(ewma_bps: u64, bitrate_bps: u64) -> usize {
    if ewma_bps == 0 || bitrate_bps == 0 {
        return REMOTE_PREFETCH_CONCURRENCY;
    }
    if ewma_bps < bitrate_bps {
        1
    } else if ewma_bps < bitrate_bps.saturating_mul(2) {
        2
    } else {
        REMOTE_PREFETCH_CONCURRENCY
    }
}

/// 需求读折半重试的下一个长度（下限 64KB）
pub(super) fn halved_fetch_len(len: u64) -> u64 {
    (len / 2).clamp(REMOTE_MIN_BLOCK_BYTES.min(len), len.max(1))
}

/// 超时/传输中断类错误：可通过折半重试恢复；
/// 4xx、长度变化和主动取消等永久失败不在此列
/// splice 打开期能否把 next_packet/decode 错误当「moov 假样本区」继续跳过：
/// 解码/格式类错误可跳；IO 错误里只有预取边缘的瞬时类可跳。
/// PermissionDenied（403 直链失效）、Other（读已被更新操作取消）等
/// 永久态必须立即上抛——每次重试都会对同一 range 重发注定失败的网络请求，
/// 4096 次上限等于无限刷 403（实机日志：seek 后每 ~600ms 一次 403 卡死）
pub(super) fn splice_skip_retryable(err: &SymphoniaError) -> bool {
    match err {
        SymphoniaError::IoError(io_err) => matches!(
            io_err.kind(),
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::WouldBlock
                | io::ErrorKind::TimedOut
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::BrokenPipe
        ),
        _ => true,
    }
}

pub(super) fn is_transient_fetch_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
    )
}

pub(super) fn can_return_partial_read(err: &io::Error) -> bool {
    is_transient_fetch_error(err) || err.kind() == io::ErrorKind::UnexpectedEof
}

/// reqwest 错误分类：超时（含 body 读取被 per-request 超时打断的
/// "error decoding response body"）与传输中断映射为可重试 kind
fn classify_reqwest_error(error: &reqwest::Error) -> io::ErrorKind {
    if error.is_timeout() {
        io::ErrorKind::TimedOut
    } else if error.is_connect() || error.is_body() || error.is_decode() || error.is_request() {
        io::ErrorKind::ConnectionAborted
    } else {
        io::ErrorKind::Other
    }
}

pub(super) fn parse_content_range(value: &reqwest::header::HeaderValue) -> Option<HttpByteRange> {
    let text = value.to_str().ok()?;
    let value = text.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    if total == "*" || range == "*" {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    let parsed = HttpByteRange {
        start: start.parse::<u64>().ok()?,
        end: end.parse::<u64>().ok()?,
        total: total.parse::<u64>().ok()?,
    };
    (parsed.start <= parsed.end && parsed.end < parsed.total).then_some(parsed)
}

pub(super) fn validate_http_range_data(
    actual: HttpByteRange,
    requested_start: u64,
    requested_end: u64,
    data: &[u8],
) -> io::Result<()> {
    if actual.start != requested_start || actual.end > requested_end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unexpected Content-Range {}-{}/{} for request {}-{}",
                actual.start, actual.end, actual.total, requested_start, requested_end
            ),
        ));
    }
    let expected_len = actual
        .end
        .checked_sub(actual.start)
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid byte range"))?;
    if data.len() as u64 != expected_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "range body length {} does not match Content-Range length {}",
                data.len(),
                expected_len
            ),
        ));
    }
    Ok(())
}
