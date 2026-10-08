use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 单生产者、单消费者 PCM 环形缓冲
///
/// 设备回调只执行固定次数的原子读写和内存复制，不分配内存、不加锁
pub struct PcmRing {
    samples: Box<[UnsafeCell<f32>]>,
    capacity: usize,
    read_position: AtomicUsize,
    write_position: AtomicUsize,
}

// 每个槽位只会由生产者写入、消费者读取，位置发布使用 Release/Acquire 配对
unsafe impl Send for PcmRing {}
unsafe impl Sync for PcmRing {}

impl PcmRing {
    pub fn new(capacity_samples: usize) -> Self {
        let capacity = capacity_samples.max(2);
        let samples = (0..capacity)
            .map(|_| UnsafeCell::new(0.0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            samples,
            capacity,
            read_position: AtomicUsize::new(0),
            write_position: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn readable_samples(&self) -> usize {
        let read = self.read_position.load(Ordering::Acquire);
        let write = self.write_position.load(Ordering::Acquire);
        write.wrapping_sub(read).min(self.capacity)
    }

    pub fn writable_samples(&self) -> usize {
        self.capacity.saturating_sub(self.readable_samples())
    }

    pub fn try_push_frame(&self, frame: &[f32]) -> bool {
        if frame.is_empty() || frame.len() > self.capacity {
            return false;
        }
        let write = self.write_position.load(Ordering::Relaxed);
        let read = self.read_position.load(Ordering::Acquire);
        if write.wrapping_sub(read) > self.capacity.saturating_sub(frame.len()) {
            return false;
        }

        for (offset, sample) in frame.iter().copied().enumerate() {
            let index = write.wrapping_add(offset) % self.capacity;
            // SAFETY: SPSC 模型下只有生产者写尚未发布的槽位
            unsafe { *self.samples[index].get() = sample };
        }
        self.write_position
            .store(write.wrapping_add(frame.len()), Ordering::Release);
        true
    }

    /// 一次取出尽可能多的整帧填进 `out`（交错），返回帧数
    ///
    /// 只发布一次读位置，设备回调里批量取比逐帧取少很多原子操作
    pub fn pop_frames(&self, out: &mut [f32], channels: usize) -> usize {
        let channels = channels.max(1);
        let wanted = out.len() / channels * channels;
        if wanted == 0 {
            return 0;
        }
        let read = self.read_position.load(Ordering::Relaxed);
        let write = self.write_position.load(Ordering::Acquire);
        let available = write.wrapping_sub(read).min(self.capacity) / channels * channels;
        let take = available.min(wanted);
        for (offset, sample) in out[..take].iter_mut().enumerate() {
            let index = read.wrapping_add(offset) % self.capacity;
            // SAFETY: 槽位已由生产者发布，消费者在推进 read_position 前独占读取
            *sample = unsafe { *self.samples[index].get() };
        }
        self.read_position
            .store(read.wrapping_add(take), Ordering::Release);
        take / channels
    }

    pub fn try_pop_frame(&self, frame: &mut [f32]) -> bool {
        if frame.is_empty() || frame.len() > self.capacity {
            return false;
        }
        let read = self.read_position.load(Ordering::Relaxed);
        let write = self.write_position.load(Ordering::Acquire);
        if write.wrapping_sub(read) < frame.len() {
            return false;
        }

        for (offset, sample) in frame.iter_mut().enumerate() {
            let index = read.wrapping_add(offset) % self.capacity;
            // SAFETY: 槽位已由生产者发布，消费者在推进 read_position 前独占读取
            *sample = unsafe { *self.samples[index].get() };
        }
        self.read_position
            .store(read.wrapping_add(frame.len()), Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::PcmRing;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn frames_are_published_and_consumed_atomically() {
        let ring = PcmRing::new(4);
        assert!(ring.try_push_frame(&[1.0, 2.0]));
        assert!(ring.try_push_frame(&[3.0, 4.0]));
        assert!(!ring.try_push_frame(&[5.0, 6.0]));

        let mut frame = [0.0; 2];
        assert!(ring.try_pop_frame(&mut frame));
        assert_eq!(frame, [1.0, 2.0]);
        assert!(ring.try_pop_frame(&mut frame));
        assert_eq!(frame, [3.0, 4.0]);
        assert!(!ring.try_pop_frame(&mut frame));
    }

    #[test]
    fn bulk_pop_takes_whole_frames_in_order_across_the_wrap() {
        let ring = PcmRing::new(8);
        assert!(ring.try_push_frame(&[1.0, 2.0]));
        assert!(ring.try_push_frame(&[3.0, 4.0]));
        let mut out = [0.0; 6];
        assert_eq!(ring.pop_frames(&mut out, 2), 2, "只取已发布的整帧");
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
        for value in [5.0, 7.0, 9.0] {
            assert!(ring.try_push_frame(&[value, value + 1.0]));
        }
        let mut wrapped = [0.0; 7];
        assert_eq!(ring.pop_frames(&mut wrapped, 2), 3, "输出缓冲按整帧截断");
        assert_eq!(&wrapped[..6], &[5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        assert_eq!(ring.pop_frames(&mut wrapped, 2), 0);
    }

    #[test]
    fn wrapped_positions_preserve_sample_order() {
        let ring = PcmRing::new(6);
        let mut frame = [0.0; 2];
        for index in 0..100 {
            let expected = [index as f32, index as f32 + 0.5];
            assert!(ring.try_push_frame(&expected));
            assert!(ring.try_pop_frame(&mut frame));
            assert_eq!(frame, expected);
        }
    }

    #[test]
    fn producer_and_consumer_can_run_without_locks() {
        let ring = Arc::new(PcmRing::new(256));
        let producer_ring = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            for index in 0..10_000u32 {
                let frame = [index as f32, -(index as f32)];
                while !producer_ring.try_push_frame(&frame) {
                    thread::yield_now();
                }
            }
        });

        let mut frame = [0.0; 2];
        for index in 0..10_000u32 {
            while !ring.try_pop_frame(&mut frame) {
                thread::yield_now();
            }
            assert_eq!(frame, [index as f32, -(index as f32)]);
        }
        assert!(producer.join().is_ok());
    }
}
