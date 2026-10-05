use crate::audio::remote::RemoteReadCancellation;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::{Duration, Instant};
use symphonia::core::io::MediaSource;

#[derive(Clone)]
pub struct GrowingAudioBuffer {
    inner: Arc<GrowingAudioInner>,
}

struct GrowingAudioInner {
    state: Mutex<GrowingAudioState>,
    spool: Option<Mutex<std::fs::File>>,
    cv: Condvar,
    aborted: AtomicBool,
}

struct GrowingAudioState {
    data: Vec<u8>,
    buffered_len: u64,
    complete: bool,
    error: Option<String>,
    total_len: Option<u64>,
}

const MEMORY_FALLBACK_LIMIT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct GrowingAudioReader {
    inner: Arc<GrowingAudioInner>,
    pos: u64,
    // prepare 路径注入的外部取消标志：symphonia probe 阻塞等数据时，
    // 播放请求超时可通过它打断 read，避免音频控制线程无限期卡死
    cancel: Option<Arc<AtomicBool>>,
    read_cancellation: Option<RemoteReadCancellation>,
}

impl Default for GrowingAudioBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl GrowingAudioBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(GrowingAudioInner {
                state: Mutex::new(GrowingAudioState {
                    data: Vec::new(),
                    buffered_len: 0,
                    complete: false,
                    error: None,
                    total_len: None,
                }),
                // 流式回退可能是数百 MB 的长音频，匿名临时文件让内存占用
                // 与曲目长度脱钩；磁盘不可用时仍有有限内存兜底
                spool: tempfile::tempfile().ok().map(Mutex::new),
                cv: Condvar::new(),
                aborted: AtomicBool::new(false),
            }),
        }
    }

    pub fn reader(&self) -> GrowingAudioReader {
        GrowingAudioReader {
            inner: self.inner.clone(),
            pos: 0,
            cancel: None,
            read_cancellation: None,
        }
    }

    pub fn set_total_len(&self, total_len: Option<u64>) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.total_len = total_len;
        }
    }

    pub fn append(&self, bytes: &[u8]) {
        if bytes.is_empty() || self.is_aborted() {
            return;
        }
        if let Ok(mut state) = self.inner.state.lock() {
            if let Some(spool) = &self.inner.spool {
                let write_result = spool
                    .lock()
                    .map_err(|_| io::Error::other("stream spool lock poisoned"))
                    .and_then(|mut file| {
                        file.seek(SeekFrom::End(0))?;
                        file.write_all(bytes)
                    });
                if let Err(error) = write_result {
                    state.error = Some(format!("stream spool write failed: {error}"));
                    state.complete = true;
                    self.inner.cv.notify_all();
                    return;
                }
            } else if state.data.len().saturating_add(bytes.len()) <= MEMORY_FALLBACK_LIMIT_BYTES {
                state.data.extend_from_slice(bytes);
            } else {
                state.error = Some(format!(
                    "stream spool unavailable and memory fallback exceeded {} bytes",
                    MEMORY_FALLBACK_LIMIT_BYTES
                ));
                state.complete = true;
                self.inner.cv.notify_all();
                return;
            }
            state.buffered_len = state.buffered_len.saturating_add(bytes.len() as u64);
            self.inner.cv.notify_all();
        }
    }

    pub fn finish(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.complete = true;
            self.inner.cv.notify_all();
        }
    }

    pub fn fail(&self, message: String) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.error = Some(message);
            state.complete = true;
            self.inner.cv.notify_all();
        }
    }

    pub fn abort(&self) {
        self.inner.aborted.store(true, Ordering::SeqCst);
        if let Ok(mut state) = self.inner.state.lock() {
            state.data.clear();
            state.buffered_len = 0;
            state.complete = true;
            if let Some(spool) = &self.inner.spool {
                if let Ok(file) = spool.lock() {
                    let _ = file.set_len(0);
                }
            }
            self.inner.cv.notify_all();
        }
    }

    pub fn is_aborted(&self) -> bool {
        self.inner.aborted.load(Ordering::SeqCst)
    }

    pub fn wait_for_buffer(&self, min_bytes: usize, timeout: Duration) -> Result<usize, String> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "stream lock poisoned".to_string())?;
        loop {
            if let Some(err) = &state.error {
                return Err(err.clone());
            }
            if self.is_aborted() {
                return Err("stream aborted".to_string());
            }
            let buffered = usize::try_from(state.buffered_len).unwrap_or(usize::MAX);
            if buffered >= min_bytes || (state.complete && buffered > 0) {
                return Ok(buffered);
            }
            if state.complete {
                return Err("empty audio stream".to_string());
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "stream startup timeout: buffered {} bytes, need {} bytes",
                    state.buffered_len, min_bytes
                ));
            }
            let wait_for = deadline.saturating_duration_since(now);
            let (next_state, timeout_result) = self
                .inner
                .cv
                .wait_timeout(state, wait_for)
                .map_err(|_| "stream lock poisoned".to_string())?;
            state = next_state;
            if timeout_result.timed_out() {
                return Err(format!(
                    "stream startup timeout: buffered {} bytes, need {} bytes",
                    state.buffered_len, min_bytes
                ));
            }
        }
    }
}

impl GrowingAudioReader {
    pub fn abort(&self) {
        self.inner.aborted.store(true, Ordering::SeqCst);
        // 必须持锁置 complete 后再 notify：read 的临界区存在「已检查
        // aborted、尚未 wait」窗口，无锁 notify 会命中该窗口而丢失，
        // 之后 append 因 aborted 提前返回不再唤醒 —— 解码线程带着整首歌
        // 的缓冲永久阻塞。与 GrowingAudioBuffer::abort 保持一致
        if let Ok(mut state) = self.inner.state.lock() {
            state.data.clear();
            state.buffered_len = 0;
            state.complete = true;
            if let Some(spool) = &self.inner.spool {
                if let Ok(file) = spool.lock() {
                    let _ = file.set_len(0);
                }
            }
            self.inner.cv.notify_all();
        }
    }

    /// 注入外部取消标志（prepare 超时路径使用）。置位后阻塞中的 read
    /// 会在一个轮询周期内返回错误，使 probe 得以失败退出
    pub fn set_prepare_cancel(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = Some(cancel);
    }

    pub(crate) fn set_read_cancellation(&mut self, cancellation: RemoteReadCancellation) {
        self.read_cancellation = Some(cancellation);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
            || self
                .read_cancellation
                .as_ref()
                .is_some_and(RemoteReadCancellation::is_cancelled)
    }

    fn known_byte_len(&self) -> Option<u64> {
        let state = self.inner.state.lock().ok()?;
        if !state.complete || state.error.is_some() || self.inner.aborted.load(Ordering::Acquire) {
            return None;
        }
        state.total_len.or(Some(state.buffered_len))
    }
}

impl Read for GrowingAudioReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| io::Error::other("stream lock poisoned"))?;
        loop {
            if self.inner.aborted.load(Ordering::SeqCst) {
                return Ok(0);
            }
            if self.is_cancelled() {
                // prepare 超时取消：返回错误让 symphonia probe 立即失败，
                // 不能返回 Ok(0)——那会被误判为正常 EOF
                return Err(io::Error::other("stream prepare cancelled"));
            }
            if let Some(err) = &state.error {
                return Err(io::Error::other(err.clone()));
            }

            let available = state.buffered_len;
            if self.pos < available {
                let len = out.len().min((available - self.pos) as usize);
                if let Some(spool) = &self.inner.spool {
                    let mut file = spool
                        .lock()
                        .map_err(|_| io::Error::other("stream spool lock poisoned"))?;
                    file.seek(SeekFrom::Start(self.pos))?;
                    file.read_exact(&mut out[..len])?;
                } else {
                    let start = self.pos as usize;
                    out[..len].copy_from_slice(&state.data[start..start + len]);
                }
                self.pos += len as u64;
                return Ok(len);
            }

            if state.complete {
                return Ok(0);
            }

            // 有界等待而非无限 wait：外部取消标志没有唤醒通道，
            // 需要按周期醒来轮询；200ms 对 prepare 取消延迟足够小
            state = self
                .inner
                .cv
                .wait_timeout(state, Duration::from_millis(200))
                .map_err(|_| io::Error::other("stream lock poisoned"))?
                .0;
        }
    }
}

impl Seek for GrowingAudioReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| io::Error::other("stream lock poisoned"))?;

        let next = match pos {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::Current(offset) => self.pos as i128 + offset as i128,
            SeekFrom::End(offset) => {
                let len = state
                    .total_len
                    .or_else(|| state.complete.then_some(state.buffered_len));
                match len {
                    Some(len) => len as i128 + offset as i128,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "stream length is not known yet",
                        ));
                    }
                }
            }
        };

        if next < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek before start",
            ));
        }
        self.pos = next as u64;
        Ok(self.pos)
    }
}

impl MediaSource for GrowingAudioReader {
    fn is_seekable(&self) -> bool {
        self.known_byte_len().is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.known_byte_len()
    }
}

#[cfg(test)]
mod tests {
    use super::GrowingAudioBuffer;
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use symphonia::core::io::MediaSource;

    // B1 回归：abort 必须能唤醒已经阻塞在 read 中的解码线程
    #[test]
    fn reader_abort_wakes_blocked_read() {
        let buffer = GrowingAudioBuffer::new();
        buffer.append(&[1, 2, 3]);
        let mut reader = buffer.reader();
        let mut scratch = [0u8; 8];
        assert_eq!(reader.read(&mut scratch).expect("initial read"), 3);

        let blocked_reader = reader.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut reader = blocked_reader;
            let mut buf = [0u8; 8];
            // 数据已耗尽且未 complete，此 read 将阻塞等待
            let _ = result_tx.send(reader.read(&mut buf));
        });
        // 给 read 足够时间进入阻塞等待
        std::thread::sleep(Duration::from_millis(100));
        reader.abort();

        let result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("blocked read was not woken by abort");
        assert_eq!(result.expect("aborted read result"), 0);
        handle.join().expect("reader thread join");
    }

    // B3 回归：prepare 取消标志置位后，阻塞中的 read 必须在轮询周期内报错返回
    #[test]
    fn prepare_cancel_unblocks_pending_read() {
        let buffer = GrowingAudioBuffer::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reader = buffer.reader();
        reader.set_prepare_cancel(Arc::clone(&cancel));

        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8];
            let _ = result_tx.send(reader.read(&mut buf));
        });
        std::thread::sleep(Duration::from_millis(50));
        cancel.store(true, Ordering::Release);

        let result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("blocked read was not woken by prepare cancel");
        assert!(result.is_err(), "cancelled read must fail, not fake EOF");
        handle.join().expect("reader thread join");
    }

    #[test]
    fn in_progress_download_is_progressive_until_complete() {
        let buffer = GrowingAudioBuffer::new();
        buffer.set_total_len(Some(4));
        buffer.append(&[1, 2]);
        let reader = buffer.reader();

        assert!(!reader.is_seekable());
        assert_eq!(reader.byte_len(), None);

        buffer.append(&[3, 4]);
        buffer.finish();

        assert!(reader.is_seekable());
        assert_eq!(reader.byte_len(), Some(4));
    }

    #[test]
    fn completed_download_reports_buffered_length_without_total_length() {
        let buffer = GrowingAudioBuffer::new();
        buffer.append(&[1, 2, 3, 4]);
        buffer.finish();

        let reader = buffer.reader();
        assert!(reader.is_seekable());
        assert_eq!(reader.byte_len(), Some(4));
    }

    #[test]
    fn aborted_reader_is_not_seekable() {
        let buffer = GrowingAudioBuffer::new();
        buffer.append(&[1, 2, 3, 4]);
        let reader = buffer.reader();
        reader.abort();

        assert!(!reader.is_seekable());
        assert_eq!(reader.byte_len(), None);
    }
}
