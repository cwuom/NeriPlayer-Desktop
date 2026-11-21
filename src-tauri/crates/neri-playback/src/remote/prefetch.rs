//! 顺序预取窗口规划、预取流水线与在途请求登记

use super::*;

pub(super) fn prefetch_window(total_len: u64, duration_hint_ms: u64) -> PrefetchWindow {
    let target_bytes = estimated_buffer_bytes(
        total_len,
        duration_hint_ms,
        REMOTE_PREFETCH_TARGET_MS,
        REMOTE_PREFETCH_FALLBACK_TARGET_BYTES,
        REMOTE_PREFETCH_MIN_TARGET_BYTES,
        REMOTE_PREFETCH_MAX_TARGET_BYTES,
    );
    let low_water_bytes = estimated_buffer_bytes(
        total_len,
        duration_hint_ms,
        REMOTE_PREFETCH_LOW_WATER_MS,
        REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES,
        REMOTE_PREFETCH_MIN_LOW_WATER_BYTES,
        REMOTE_PREFETCH_MAX_LOW_WATER_BYTES,
    )
    .min((target_bytes / 2).max(1));

    PrefetchWindow {
        low_water_bytes,
        target_bytes,
    }
}

fn estimated_buffer_bytes(
    total_len: u64,
    duration_hint_ms: u64,
    buffer_duration_ms: u64,
    fallback_bytes: u64,
    min_bytes: u64,
    max_bytes: u64,
) -> u64 {
    if total_len == 0 {
        return 0;
    }
    if duration_hint_ms == 0 {
        return fallback_bytes.clamp(min_bytes, max_bytes).min(total_len);
    }

    let estimated = (u128::from(total_len) * u128::from(buffer_duration_ms))
        .div_ceil(u128::from(duration_hint_ms))
        .min(u128::from(u64::MAX)) as u64;
    estimated.clamp(min_bytes, max_bytes).min(total_len)
}

pub(super) fn prefetch_plan(
    read_pos: u64,
    cached_end: u64,
    total_len: u64,
    window: PrefetchWindow,
) -> Option<PrefetchPlan> {
    let read_pos = read_pos.min(total_len);
    if read_pos >= total_len {
        return None;
    }

    let cached_end = cached_end.clamp(read_pos, total_len);
    let buffered_bytes = cached_end.saturating_sub(read_pos);
    if buffered_bytes > window.low_water_bytes {
        return None;
    }

    let target_end = read_pos.saturating_add(window.target_bytes).min(total_len);
    (cached_end < target_end).then_some(PrefetchPlan {
        start: cached_end,
        target_end,
    })
}

pub(super) fn replenish_prefetch_window(inner: &Arc<RemoteAudioInner>, read_pos: u64) {
    if inner.playback_cancelled() {
        return;
    }
    // 远端已 403/401：直链对新 range 永久拒绝，不再孵化任何预取
    if inner.remote_forbidden.load(Ordering::Acquire) {
        return;
    }
    let cached_end = contiguous_cached_end(inner, read_pos);
    if cached_end >= inner.total_len {
        try_mark_disk_cache_ready(inner);
        return;
    }

    let Some(plan) = prefetch_plan(read_pos, cached_end, inner.total_len, inner.prefetch_window)
    else {
        return;
    };
    spawn_prefetch_sequence(inner.clone(), plan);
}

fn spawn_prefetch_sequence(inner: Arc<RemoteAudioInner>, plan: PrefetchPlan) {
    // 先占首块：既是 0 号 lane 的首个任务，也天然拒绝重复提交的整段预取
    let Some((first_start, first_end, first_entry)) = claim_prefetch_sequence_start(&inner, plan)
    else {
        return;
    };
    let epoch = inner.prefetch_epoch.load(Ordering::Acquire);
    // 共享游标：各 lane 从这里领取下一块起点，构成并发流水线，
    // 掩盖单请求往返延迟（串行 256KB 块的有效吞吐在慢链路上低于码率）
    let cursor = Arc::new(AtomicU64::new(first_end.saturating_add(1)));
    let stop = Arc::new(AtomicBool::new(false));
    // 慢网降并发：EWMA 低于码率时单流都喂不饱，多 lane 只会挤占需求读带宽
    let lanes = prefetch_lane_count(inner.throughput_bps(), inner.bitrate_bps);
    for lane in 0..lanes {
        let preclaimed = (lane == 0).then(|| (first_start, first_end, Arc::clone(&first_entry)));
        let inner = Arc::clone(&inner);
        let cursor = Arc::clone(&cursor);
        let stop = Arc::clone(&stop);
        crate::runtime::spawn(async move {
            prefetch_lane(inner, plan, epoch, cursor, stop, preclaimed).await;
        });
    }
}

/// 当前预取块大小：按近期吞吐自适应（8s 可完成），无样本时用 512KB 默认
fn current_prefetch_block_bytes(inner: &RemoteAudioInner) -> u64 {
    adaptive_block_bytes(
        inner.throughput_bps(),
        REMOTE_PREFETCH_BLOCK_BYTES,
        REMOTE_ADAPTIVE_MAX_BLOCK_BYTES,
    )
}

/// 预取块长度：夹到窗口/文件边界
fn prefetch_block_len(block: u64, start: u64, limit: u64) -> u64 {
    block.min(limit.saturating_sub(start))
}

/// 从共享游标领取下一块 [start, end]。块大小逐次按最新吞吐估计计算，
/// CAS 推进游标保证变长块之间无空洞、无重叠
fn take_prefetch_block(cursor: &AtomicU64, limit: u64, block: u64) -> Option<(u64, u64)> {
    loop {
        let start = cursor.load(Ordering::Acquire);
        if start >= limit {
            return None;
        }
        let len = prefetch_block_len(block, start, limit);
        if len == 0 {
            return None;
        }
        if cursor
            .compare_exchange(start, start + len, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some((start, start + len - 1));
        }
    }
}

/// 单条预取 lane：从共享游标领块 → 登记在途 → 拉取 → 入缓存 → 发布。
/// 出错/短响应/换代即停整条流水线，空洞由下一轮 replenish 从
/// contiguous_cached_end 重新规划补齐
async fn prefetch_lane(
    inner: Arc<RemoteAudioInner>,
    plan: PrefetchPlan,
    epoch: u64,
    cursor: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    preclaimed: Option<(u64, u64, Arc<InflightFetch>)>,
) {
    let limit = plan.target_end.min(inner.total_len);
    let mut pending = preclaimed;
    loop {
        let lane_cancelled = || {
            stop.load(Ordering::Acquire)
                || inner.playback_cancelled()
                || inner.prefetch_epoch.load(Ordering::Acquire) != epoch
        };
        let (start, end, entry) = match pending.take() {
            Some(next) => next,
            None => {
                // 需求读优先：解码线程正亲自拉块时暂缓开新请求，
                // 已在途的请求继续跑完，只是不再抢新带宽
                while inner.demand_fetches.load(Ordering::Acquire) > 0 && !lane_cancelled() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                if lane_cancelled() {
                    break;
                }
                // 每块领取时都按最新吞吐估计选块：慢网自动缩小、快网恢复大块
                let block = current_prefetch_block_bytes(&inner);
                let Some((start, end)) = take_prefetch_block(&cursor, limit, block) else {
                    break;
                };
                match claim_prefetch_start(&inner, start, end) {
                    Some(entry) => (start, end, entry),
                    // 已缓存或已被别人（需求读）拉着：跳过本块领下一块
                    None => continue,
                }
            }
        };
        if lane_cancelled() {
            release_prefetch_start(&inner, start, &entry);
            break;
        }
        // seek 换代时立刻中止在途预取请求，不让旧位置继续吃带宽
        let fetch = fetch_range_block_async(&inner, start, end, None);
        tokio::pin!(fetch);
        let result = tokio::select! {
            result = &mut fetch => result,
            () = wait_for_generation_change(&inner.prefetch_epoch, epoch) => Err(
                io::Error::new(io::ErrorKind::Interrupted, "prefetch epoch invalidated"),
            ),
        };

        let requested = end - start + 1;
        match result {
            Ok(data) if !data.is_empty() => {
                let fetched_len = data.len() as u64;
                // epoch 变了：丢弃本块，避免旧位置继续污染缓存中心
                if inner.prefetch_epoch.load(Ordering::Acquire) == epoch {
                    if let Ok(mut cache) = inner.cache.lock() {
                        if !cache_contains_position(&cache, start) {
                            cache.push(CachedSegment { start, data });
                            cache.sort_by_key(|segment| segment.start);
                            trim_cache(&inner, &mut cache, cache_center(&inner, start));
                        }
                    }
                } else {
                    stop.store(true, Ordering::Release);
                }
                release_prefetch_start(&inner, start, &entry);
                // 短响应在块尾留下空洞：停流水线交给 replenish 补齐
                if fetched_len < requested {
                    stop.store(true, Ordering::Release);
                    break;
                }
            }
            Ok(_) => {
                release_prefetch_start(&inner, start, &entry);
                stop.store(true, Ordering::Release);
                break;
            }
            Err(err) => {
                release_prefetch_start(&inner, start, &entry);
                if err.kind() != io::ErrorKind::Interrupted && !inner.playback_cancelled() {
                    log::warn!(
                        target: "remote-audio",
                        "prefetch stopped at {}: {}",
                        start, err
                    );
                }
                stop.store(true, Ordering::Release);
                break;
            }
        }
    }
    // 走到文件尾时触发磁盘缓存收尾检查（内部校验覆盖完整性，幂等）
    if cursor.load(Ordering::Acquire) >= inner.total_len {
        try_mark_disk_cache_ready(&inner);
    }
}

pub(super) fn claim_prefetch_sequence_start(
    inner: &RemoteAudioInner,
    plan: PrefetchPlan,
) -> Option<(u64, u64, Arc<InflightFetch>)> {
    let limit = plan.target_end.min(inner.total_len);
    let start = plan.start;
    if start >= limit {
        return None;
    }
    let block = current_prefetch_block_bytes(inner);
    let end = start + prefetch_block_len(block, start, limit) - 1;
    claim_prefetch_start(inner, start, end).map(|entry| (start, end, entry))
}

/// 登记 [start, end] 的在途请求；起点已被在途请求覆盖或已被连续缓存覆盖时
/// 返回 None（调用方应转入等待/命中缓存，而不是重复发起请求）
pub(super) fn claim_prefetch_start(
    inner: &RemoteAudioInner,
    start: u64,
    end: u64,
) -> Option<Arc<InflightFetch>> {
    if contiguous_cached_end(inner, start) > start {
        return None;
    }
    let Ok(mut in_flight) = inner.in_flight.lock() else {
        return None;
    };
    // 只按「起点被覆盖」判重：部分尾部重叠的少量浪费可接受，
    // 若按整段重叠判重，需求读会在等不到对应登记项时陷入空转
    if in_flight.iter().any(|(entry_start, entry)| {
        *entry_start <= start && start <= entry.end.load(Ordering::Acquire)
    }) {
        return None;
    }
    let entry = Arc::new(InflightFetch {
        end: AtomicU64::new(end),
        done: AtomicBool::new(false),
    });
    in_flight.insert(start, Arc::clone(&entry));
    Some(entry)
}

/// 发布在途请求结果：置 done 唤醒共享等待方，并摘除登记项。
/// 摘除前校验仍是同一个 Arc——代际作废会清空注册表，同一起点可能已被
/// 新请求重新占用，不能误删别人的登记
pub(super) fn release_prefetch_start(
    inner: &RemoteAudioInner,
    start: u64,
    entry: &Arc<InflightFetch>,
) {
    entry.done.store(true, Ordering::Release);
    if let Ok(mut in_flight) = inner.in_flight.lock() {
        if in_flight
            .get(&start)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            in_flight.remove(&start);
        }
    }
    inner.range_available.notify_all();
}

/// seek 时作废旧预取：抬 epoch + 置 done 解除所有共享等待方 + 清空注册表，
/// 让需求读立刻能 claim（seek 到未缓存位置保持低延迟）
pub(super) fn invalidate_prefetch_epoch(inner: &RemoteAudioInner) {
    inner.prefetch_epoch.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut in_flight) = inner.in_flight.lock() {
        for entry in in_flight.values() {
            entry.done.store(true, Ordering::Release);
        }
        in_flight.clear();
    }
    inner.range_available.notify_all();
}
