//! 首包探测与预热：提前拉取首块并在后台继续补齐

use super::*;

pub(super) fn initial_block_bytes(duration_hint_ms: u64) -> u64 {
    if duration_hint_ms >= REMOTE_LONG_FORM_DURATION_MS {
        REMOTE_LONG_INITIAL_BLOCK_BYTES
    } else {
        REMOTE_INITIAL_BLOCK_BYTES
    }
}

/// 预先拿到的首包：总长与开头一块。经代理时冷连接拿首包要 3 秒左右，热连接不到 1 秒
pub(super) struct WarmProbe {
    url: String,
    pub(super) total_len: u64,
    pub(super) data: Vec<u8>,
    /// 当时拉首包的耗时，正式打开时据此估初始吞吐
    pub(super) elapsed_ms: u64,
    pub(super) fetched_at: Instant,
}

/// 直链签名本身也会过期，留太久的首包没用
const WARM_PROBE_TTL: Duration = Duration::from_secs(10 * 60);
/// 预开只拉 64KB：够解码起步（Opus 约 3 秒），冷连接上少收一半正文就能早一秒就绪，
/// 后面的数据走预开留下的热连接
const PREWARM_BLOCK_BYTES: u64 = 64 * 1024;
/// 下一首、鼠标停着的那首，外加切换途中的余量
const WARM_PROBE_LIMIT: usize = 4;
static WARM_PROBES: Mutex<Vec<WarmProbe>> = Mutex::new(Vec::new());

/// 正在预开的地址：正式打开撞上时等它，别对同一台冷主机再建一条连接互相抢带宽
static WARMING: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// 等预开的上限：超过就自己拉，预开卡住不能拖住起播
const WARMING_JOIN_TIMEOUT: Duration = Duration::from_secs(6);

/// 同一地址的预开还在路上就等它出结果；返回时预开已结束或等不及了
pub(super) async fn join_warming_probe(url: &str) {
    let Some(notify) = WARMING
        .lock()
        .ok()
        .and_then(|warming| warming.get(url).cloned())
    else {
        return;
    };
    // Notified 一创建就能收到 notify_waiters，先建再复查，不会漏掉中间的完成通知
    let notified = notify.notified();
    if !WARMING
        .lock()
        .is_ok_and(|warming| warming.contains_key(url))
    {
        return;
    }
    let _ = tokio::time::timeout(WARMING_JOIN_TIMEOUT, notified).await;
}

pub(super) fn take_warm_probe(url: &str) -> Option<WarmProbe> {
    let mut probes = WARM_PROBES.lock().ok()?;
    probes.retain(|probe| probe.fetched_at.elapsed() < WARM_PROBE_TTL);
    let index = probes.iter().position(|probe| probe.url == url)?;
    Some(probes.swap_remove(index))
}

/// 预开即将播放的直链：先把总长和首包拿到手，连接也留在池里。之后正式打开同一个地址时
/// 跳过首包请求，切到下一首几乎立即出声。已经预开过的地址不重复请求
pub async fn prewarm(
    client: &reqwest::Client,
    url: &str,
    referer: &str,
    duration_hint_ms: u64,
) -> AppResult<bool> {
    let already_warm = WARM_PROBES.lock().is_ok_and(|probes| {
        probes
            .iter()
            .any(|probe| probe.url == url && probe.fetched_at.elapsed() < WARM_PROBE_TTL)
    });
    if already_warm {
        return Ok(false);
    }
    let notify = {
        let Ok(mut warming) = WARMING.lock() else {
            return Ok(false);
        };
        if warming.contains_key(url) {
            return Ok(false);
        }
        let notify = Arc::new(tokio::sync::Notify::new());
        warming.insert(url.to_owned(), Arc::clone(&notify));
        notify
    };
    // 不管成败都要摘掉登记并唤醒等待方，否则正式打开会白等到超时
    struct WarmingGuard<'a> {
        url: &'a str,
        notify: Arc<tokio::sync::Notify>,
    }
    impl Drop for WarmingGuard<'_> {
        fn drop(&mut self) {
            if let Ok(mut warming) = WARMING.lock() {
                warming.remove(self.url);
            }
            self.notify.notify_waiters();
        }
    }
    let _guard = WarmingGuard { url, notify };
    let started = Instant::now();
    let full_block = initial_block_bytes(duration_hint_ms);
    let block = if full_block > REMOTE_INITIAL_BLOCK_BYTES {
        full_block
    } else {
        PREWARM_BLOCK_BYTES
    };
    let (mut total_len, mut data) = probe_range_len(client, url, referer, block).await?;
    if block < full_block.min(total_len) && !probe_head_is_self_contained(&data) {
        // MP4 文件头比预开块大：补拉完整首包，打开阶段建索引要用
        (total_len, data) = probe_range_len(client, url, referer, full_block).await?;
    }
    let probe = WarmProbe {
        url: url.to_owned(),
        total_len,
        data,
        elapsed_ms: started.elapsed().as_millis() as u64,
        fetched_at: Instant::now(),
    };
    if let Ok(mut probes) = WARM_PROBES.lock() {
        probes.retain(|existing| {
            existing.url != url && existing.fetched_at.elapsed() < WARM_PROBE_TTL
        });
        if probes.len() >= WARM_PROBE_LIMIT {
            probes.remove(0);
        }
        probes.push(probe);
    }
    Ok(true)
}

pub(super) async fn probe_range_len(
    client: &reqwest::Client,
    url: &str,
    referer: &str,
    initial_block_bytes: u64,
) -> AppResult<(u64, Vec<u8>)> {
    let head = probe_range_head(client, url, referer, initial_block_bytes, false).await?;
    Ok((head.total_len, head.data))
}

/// 首包收到这么多、并且文件头已经完整，就可以开始解码；同一响应剩下的部分在后台接着收
const PROBE_EARLY_RETURN_BYTES: usize = 32 * 1024;

pub(super) struct ProbeHead {
    pub(super) total_len: u64,
    pub(super) data: Vec<u8>,
    /// 提前返回时还没收完的部分：(物理尾偏移, 仍在传输的响应)
    pub(super) rest: Option<(u64, reqwest::Response)>,
}

/// 打开阶段只从首包解析 MP4 文件头（moov、sidx 都在第一个 moof/mdat 之前）。
/// 其它容器不靠首包建任何状态，解码器读到哪拉到哪
fn probe_head_is_self_contained(head: &[u8]) -> bool {
    let is_iso_bmff = head.len() >= 8
        && matches!(
            &head[4..8],
            b"ftyp" | b"styp" | b"moov" | b"sidx" | b"free" | b"skip" | b"wide" | b"moof" | b"mdat"
        );
    !is_iso_bmff || detect_mp4_header_end(head).is_some()
}

/// 慢链路上新连接的拥塞窗口要好几个往返才撑得开，128KB 首包光收完正文就要 1 秒左右；
/// allow_early 时收够文件头和开头一小段就返回，让解码先跑起来
pub(super) async fn probe_range_head(
    client: &reqwest::Client,
    url: &str,
    referer: &str,
    initial_block_bytes: u64,
    allow_early: bool,
) -> AppResult<ProbeHead> {
    let range_end = initial_block_bytes.max(1).saturating_sub(1);
    let mut response = client
        .get(url)
        .header(REFERER, referer)
        .header(USER_AGENT, playback_user_agent(url))
        .header(RANGE, format!("bytes=0-{}", range_end))
        .timeout(REMOTE_PROBE_TIMEOUT)
        .send()
        .await
        .map_err(AppError::Network)?;

    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(AppError::Audio(format!(
            "Remote source does not support HTTP Range: {}",
            response.status()
        )));
    }

    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(parse_content_range)
        .ok_or_else(|| {
            AppError::Audio("Remote Range response did not include a valid byte range".into())
        })?;
    if content_range.start != 0 || content_range.end > range_end {
        return Err(AppError::Audio(format!(
            "unexpected Content-Range {}-{}/{} for request 0-{range_end}",
            content_range.start, content_range.end, content_range.total,
        )));
    }
    let expected_len = (content_range.end - content_range.start).saturating_add(1) as usize;
    let mut data = Vec::with_capacity(expected_len);
    while data.len() < expected_len {
        let Some(chunk) = response.chunk().await.map_err(AppError::Network)? else {
            break;
        };
        data.extend_from_slice(&chunk);
        if allow_early
            && data.len() >= PROBE_EARLY_RETURN_BYTES
            && data.len() < expected_len
            && probe_head_is_self_contained(&data)
        {
            return Ok(ProbeHead {
                total_len: content_range.total,
                data,
                rest: Some((content_range.end, response)),
            });
        }
    }
    validate_http_range_data(content_range, 0, range_end, &data)
        .map_err(|err| AppError::Audio(err.to_string()))?;

    Ok(ProbeHead {
        total_len: content_range.total,
        data,
        rest: None,
    })
}

/// 提前返回的首包剩下的部分登记成在途请求：解码线程读到这里会等它，不会另发一次请求。
/// 收完入缓存后接着按正常节奏预取；中途失败只是撤销登记，读的人自己去拉
pub(super) fn continue_probe_in_background(
    inner: &Arc<RemoteAudioInner>,
    start: u64,
    end: u64,
    mut response: reqwest::Response,
    started: Instant,
) {
    let Some(entry) = claim_prefetch_start(inner, start, end) else {
        return;
    };
    let inner = Arc::clone(inner);
    crate::runtime::spawn(async move {
        let expected_len = (end - start).saturating_add(1) as usize;
        let mut data = Vec::with_capacity(expected_len);
        let complete = loop {
            if inner.playback_cancelled() {
                break false;
            }
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    data.extend_from_slice(&chunk);
                    if data.len() >= expected_len {
                        break data.len() == expected_len;
                    }
                }
                Ok(None) => break data.len() == expected_len,
                Err(error) => {
                    log::warn!(target: "remote-range", "probe tail interrupted at {}: {}", start + data.len() as u64, error.without_url());
                    break false;
                }
            }
        };
        if complete {
            inner.record_fetch_success(end.saturating_add(1), started.elapsed().as_millis() as u64);
            if let Ok(mut cache) = inner.cache.lock() {
                if !cache_contains_position(&cache, start) {
                    cache.push(CachedSegment {
                        start,
                        data: data.clone(),
                    });
                    cache.sort_by_key(|segment| segment.start);
                    trim_cache(&inner, &mut cache, cache_center(&inner, start));
                }
            }
            write_disk_range(&inner, start, &data);
        }
        release_prefetch_start(&inner, start, &entry);
        if complete {
            replenish_prefetch_window(&inner, start);
        }
    });
}
