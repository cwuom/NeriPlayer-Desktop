//! fMP4 识别与 sidx/moof seek 索引构建

use super::*;

impl RemoteSeekIndex {
    /// 返回 <= target_ms 的最近 segment 起点（moof 边界），供 virtual-body 跳转
    #[cfg(test)]
    pub(super) fn estimate_segment_start(&self, target_ms: u64) -> u64 {
        self.estimate_segment(target_ms).0
    }

    /// (分片起点字节, 分片起点时间)
    pub(super) fn estimate_segment(&self, target_ms: u64) -> (u64, u64) {
        if self.entries.is_empty() {
            return (0, 0);
        }
        // 最后一个锚点是媒体末端，不能拿它重建可播放的分片
        let segments = &self.entries[..self.entries.len().saturating_sub(1)];
        let mut best_idx = 0usize;
        for (idx, entry) in segments.iter().enumerate() {
            if entry.time_ms <= target_ms {
                best_idx = idx;
            } else {
                break;
            }
        }
        let entry = self.entries[best_idx];
        (entry.byte_offset, entry.time_ms)
    }

    #[allow(dead_code)]
    pub(super) fn estimate_byte(&self, target_ms: u64) -> u64 {
        if self.entries.is_empty() {
            return 0;
        }
        if target_ms == 0 {
            return self.entries[0].byte_offset;
        }
        // 找最后一个 time <= target 的 entry，再线性插值到下一个
        let mut lo = 0usize;
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.time_ms <= target_ms {
                lo = idx;
            } else {
                break;
            }
        }
        let left = self.entries[lo];
        if lo + 1 >= self.entries.len() {
            return left.byte_offset;
        }
        let right = self.entries[lo + 1];
        if right.time_ms <= left.time_ms {
            return left.byte_offset;
        }
        let span = right.time_ms - left.time_ms;
        let progress = target_ms.saturating_sub(left.time_ms).min(span);
        let byte_span = right.byte_offset.saturating_sub(left.byte_offset);
        left.byte_offset + (u128::from(byte_span) * u128::from(progress) / u128::from(span)) as u64
    }
}

pub(super) fn is_fragmented_mp4_url(value: &str) -> bool {
    url::Url::parse(value)
        .ok()
        .map(|url| url.path().to_ascii_lowercase().ends_with(".m4s"))
        .unwrap_or_else(|| {
            value
                .split(['?', '#'])
                .next()
                .is_some_and(|path| path.to_ascii_lowercase().ends_with(".m4s"))
        })
}

/// 从首包字节嗅探分片 MP4
///
/// 仅靠 URL 后缀不够：Bilibili DASH 用 `.m4s`，但 YouTube 的 googlevideo 直链
/// 形如 `/videoplayback?itag=140&...`，路径没有任何扩展名，却同样是分片 MP4
/// （`moov` 里 `stbl` 为空，时间轴全在 `moof` 中）。误判成普通 MP4 会让
/// demuxer 拿着空样本表做 seek，长内容上表现为 seek 无响应。
pub(super) fn detect_fragmented_mp4(data: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let Ok(size_bytes) = data[offset..offset + 4].try_into() else {
            return false;
        };
        let size = u32::from_be_bytes(size_bytes) as usize;
        let kind = &data[offset + 4..offset + 8];

        // styp/sidx/moof 是分片 MP4 的直接标志
        if kind == b"styp" || kind == b"sidx" || kind == b"moof" {
            return true;
        }
        // moov 内含 mvex 表示后续为 movie fragment
        if kind == b"moov" {
            let header_len = if size == 1 { 16 } else { 8 };
            let end = if size == 0 {
                data.len()
            } else {
                (offset + size).min(data.len())
            };
            let body_start = (offset + header_len).min(end);
            if data[body_start..end]
                .windows(4)
                .any(|window| window == b"mvex")
            {
                return true;
            }
        }

        let header_len = if size == 1 { 16 } else { 8 };
        let atom_size = if size == 1 {
            let Ok(size_bytes) = data[offset + 8..offset + 16].try_into() else {
                return false;
            };
            u64::from_be_bytes(size_bytes) as usize
        } else if size == 0 {
            return false;
        } else {
            size
        };
        if atom_size < header_len {
            return false;
        }
        // mdat 之后不会再有头部信息，且首包通常放不下整个 mdat
        if kind == b"mdat" {
            return false;
        }
        offset += atom_size;
    }
    false
}

/// 哪些流需要在 open 阶段用首包 sidx 建 seek 索引
///
/// symphonia 的 isomp4 对分片流的 format.seek 是顺着 moof 链爬的
/// O(文件大小) 行为，所以凡是分片内容都必须自建索引走字节落点：
/// - 嗅探出的分片单文件（YouTube DASH 直链，URL 看不出分片）
/// - LongFormProgressive（长内容渐进，sidx/moov 定位）
/// - FragmentedProgressive（Bilibili .m4s 直链）——旧代码漏掉了它，
///   2 小时 118MB 的 fMP4 每次 seek 都要顺序爬 moof ~12s
pub(super) fn needs_head_seek_index(
    access_mode: RemoteAccessMode,
    sniffed_fragmented: bool,
) -> bool {
    sniffed_fragmented
        || matches!(
            access_mode,
            RemoteAccessMode::LongFormProgressive | RemoteAccessMode::FragmentedProgressive
        )
}

/// 只用首包的 sidx 建索引，成功返回 true；失败不写入，交给调用方兜底
pub(super) fn build_seek_index_from_head(inner: &RemoteAudioInner, duration_hint_ms: u64) -> bool {
    // hint=0（时长未知）原样传入：parse 层只在 >0 时做 clamp。
    // 旧的 .max(1) 会把索引里所有 time_ms 夹到 ≤1ms，等于毁掉整张索引
    let Some(index) = parse_sidx_seek_index(inner, duration_hint_ms) else {
        return false;
    };
    if index.entries.is_empty() {
        return false;
    }
    log::info!(
        target: "remote-audio",
        "seek index from head sidx duration_ms={}, entries={}",
        index.duration_ms,
        index.entries.len()
    );
    let Ok(mut slot) = inner.seek_index.lock() else {
        return false;
    };
    *slot = Some(index);
    true
}

pub(super) fn build_seek_index_for_long_form(
    inner: &RemoteAudioInner,
    duration_hint_ms: u64,
    allow_linear_fallback: bool,
) {
    // duration_hint_ms=0 表示时长未知：sidx 解析不受影响（自带 timescale，
    // clamp 在 parse 层按 >0 守卫跳过），但线性估算没有时长就无从落点，禁用
    let mut index = parse_sidx_seek_index(inner, duration_hint_ms)
        .or_else(|| parse_sidx_seek_index_from_tail(inner, duration_hint_ms));
    let allow_linear_fallback = allow_linear_fallback && duration_hint_ms > 0;
    if index.is_none() && !allow_linear_fallback {
        // 分片内容没解析到真实 sidx 时不启用虚拟 body：线性估算落点不可靠，
        // 保持原有的 format.seek 行为，避免把可用路径换成更差的路径
        log::info!(
            target: "remote-audio",
            "seek index skipped: no sidx found and linear fallback disabled"
        );
        return;
    }
    if index.is_none() {
        // 线性估算兜底：足够让字节跳转落到大致区域（仅时长已知时启用）
        let total = inner.total_len.max(1);
        let steps = 32u64;
        let mut entries = Vec::with_capacity(steps as usize + 1);
        for i in 0..=steps {
            let time_ms = duration_hint_ms.saturating_mul(i) / steps;
            let byte_offset = total.saturating_mul(i) / steps;
            entries.push(SeekIndexEntry {
                time_ms,
                byte_offset,
            });
        }
        index = Some(RemoteSeekIndex {
            duration_ms: duration_hint_ms,
            entries,
            exact_times: false,
        });
        log::info!(
            target: "remote-audio",
            "seek index fallback linear duration_ms={}, entries={}",
            duration_hint_ms,
            steps + 1
        );
    } else if let Some(ref built) = index {
        log::info!(
            target: "remote-audio",
            "seek index from sidx duration_ms={}, entries={}",
            built.duration_ms,
            built.entries.len()
        );
    }
    if let Ok(mut slot) = inner.seek_index.lock() {
        *slot = index;
    }
}

fn parse_sidx_seek_index(inner: &RemoteAudioInner, duration_ms: u64) -> Option<RemoteSeekIndex> {
    // 从头缓存里找 sidx
    let head = {
        let cache = inner.cache.lock().ok()?;
        cache
            .iter()
            .find(|segment| segment.start == 0)
            .map(|segment| segment.data.clone())
    }?;
    parse_sidx_from_bytes(&head, 0, duration_ms, inner.total_len)
}

fn parse_sidx_seek_index_from_tail(
    inner: &RemoteAudioInner,
    duration_ms: u64,
) -> Option<RemoteSeekIndex> {
    let cache = inner.cache.lock().ok()?;
    let tail = cache
        .iter()
        .filter(|segment| segment.start > 0)
        .max_by_key(|segment| segment.start)?;
    parse_sidx_from_bytes(&tail.data, tail.start, duration_ms, inner.total_len)
}

pub(super) fn detect_mp4_header_end(data: &[u8]) -> Option<u64> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        let header_len = if size == 1 { 16 } else { 8 };
        let atom_size = if size == 1 {
            if offset + 16 > data.len() {
                break;
            }
            u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?) as usize
        } else if size == 0 {
            // 到文件尾：对 header 探测无意义
            break;
        } else {
            size
        };
        if atom_size < header_len {
            break;
        }
        // 首个 mdat/moof 即媒体起点
        if kind == b"mdat" || kind == b"moof" {
            return Some(offset as u64);
        }
        if offset + atom_size > data.len() {
            // 不完整 atom：若不是 mdat/moof，停止
            break;
        }
        offset += atom_size;
    }
    None
}

/// 在 [start, start+window) 内找第一个 moof atom 相对偏移；用于 sidx 落点校正
pub(super) fn find_moof_offset_near(data: &[u8], start: usize, window: u64) -> Option<u64> {
    if start >= data.len() {
        return None;
    }
    let end = (start + window as usize).min(data.len());
    let mut offset = start;
    while offset + 8 <= end {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        if kind == b"moof" {
            return Some((offset - start) as u64);
        }
        let atom_size = if size == 1 {
            if offset + 16 > data.len() {
                break;
            }
            u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?) as usize
        } else if size == 0 {
            break;
        } else {
            size
        };
        if atom_size < 8 {
            // 坏 size：逐字节扫描找 "moof" fourcc
            offset += 1;
            continue;
        }
        if offset + atom_size > data.len() {
            // 不完整：在剩余窗口里暴力找 moof fourcc
            let search = &data[offset..end];
            if let Some(pos) = search.windows(4).position(|w| w == b"moof") {
                // fourcc 在 size 之后 4 字节，atom 起点 = pos - 4
                if pos >= 4 {
                    return Some((offset + pos - 4 - start) as u64);
                }
            }
            break;
        }
        offset += atom_size;
    }
    // 兜底：窗口内暴力找 moof fourcc
    let search = &data[start..end];
    if let Some(pos) = search.windows(4).position(|w| w == b"moof") {
        if pos >= 4 {
            return Some((pos - 4) as u64);
        }
    }
    None
}

pub(super) fn parse_sidx_from_bytes(
    data: &[u8],
    base_offset: u64,
    duration_ms: u64,
    total_len: u64,
) -> Option<RemoteSeekIndex> {
    // 在缓冲里扫描 sidx box
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        let atom_size = if size == 1 {
            if offset + 16 > data.len() {
                break;
            }
            u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?) as usize
        } else if size == 0 {
            data.len().saturating_sub(offset)
        } else {
            size
        };
        if atom_size < 8 || offset + atom_size > data.len() {
            break;
        }
        if kind == b"sidx" {
            let header_len = if size == 1 { 16 } else { 8 };
            let body = &data[offset + header_len..offset + atom_size];
            if let Some(index) = parse_sidx_body(
                body,
                base_offset + offset as u64 + atom_size as u64,
                duration_ms,
                total_len,
            ) {
                return Some(index);
            }
        }
        if atom_size == 0 {
            break;
        }
        offset += atom_size;
    }
    None
}

pub(super) fn parse_sidx_body(
    body: &[u8],
    first_offset_anchor: u64,
    duration_ms: u64,
    total_len: u64,
) -> Option<RemoteSeekIndex> {
    if body.len() < 20 {
        return None;
    }
    let version = body[0];
    let mut o = 4usize; // skip version+flags
    o += 4; // reference_id
    if o + 4 > body.len() {
        return None;
    }
    let timescale = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
    o += 4;
    if timescale == 0 {
        return None;
    }
    let (earliest_pts, first_offset) = if version == 0 {
        if o + 8 > body.len() {
            return None;
        }
        let pts = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
        o += 4;
        let off = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
        o += 4;
        (pts, first_offset_anchor + off)
    } else {
        if o + 16 > body.len() {
            return None;
        }
        let pts = u64::from_be_bytes(body[o..o + 8].try_into().ok()?);
        o += 8;
        let off = u64::from_be_bytes(body[o..o + 8].try_into().ok()?);
        o += 8;
        (pts, first_offset_anchor + off)
    };
    if o + 4 > body.len() {
        return None;
    }
    o += 2; // reserved
    let reference_count = u16::from_be_bytes(body[o..o + 2].try_into().ok()?) as usize;
    o += 2;

    let mut entries = Vec::with_capacity(reference_count.saturating_add(1));
    let mut time_ticks = earliest_pts;
    let mut byte_pos = first_offset.min(total_len);
    entries.push(SeekIndexEntry {
        time_ms: time_ticks.saturating_mul(1000) / timescale,
        byte_offset: byte_pos,
    });
    for _ in 0..reference_count {
        if o + 12 > body.len() {
            break;
        }
        let reference = u32::from_be_bytes(body[o..o + 4].try_into().ok()?);
        o += 4;
        let subsegment_duration = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
        o += 4;
        o += 4; // SAP
        let reference_type = (reference & 0x8000_0000) != 0;
        let reference_size = (reference & 0x7fff_ffff) as u64;
        if reference_type {
            // 嵌套 sidx，跳过大小但仍推进偏移
            byte_pos = byte_pos.saturating_add(reference_size).min(total_len);
            continue;
        }
        byte_pos = byte_pos.saturating_add(reference_size).min(total_len);
        time_ticks = time_ticks.saturating_add(subsegment_duration);
        entries.push(SeekIndexEntry {
            time_ms: time_ticks.saturating_mul(1000) / timescale,
            byte_offset: byte_pos,
        });
    }
    if entries.len() < 2 {
        return None;
    }
    // 夹到声明时长
    for entry in &mut entries {
        if duration_ms > 0 {
            entry.time_ms = entry.time_ms.min(duration_ms);
        }
        entry.byte_offset = entry.byte_offset.min(total_len);
    }
    Some(RemoteSeekIndex {
        duration_ms: duration_ms.max(entries.last().map(|e| e.time_ms).unwrap_or(0)),
        entries,
        exact_times: true,
    })
}
