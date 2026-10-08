//! 保持音调的变速（WSOLA，波形相似叠加）
//!
//! 设备回调按实时速度消费解码好的 PCM。倍速为 1 时逐样本直通；偏离 1 时每个合成跳（半窗）
//! 从输入里挑一段与上一段自然延续最相似的窗口做汉宁叠加，时长按倍速伸缩而音调不变。
//! 改倍速在下一跳生效；进出变速各经过一次叠加过渡，没有咔哒声，也不需要重建解码会话。
//! 缓冲全部在构造时分配，`render` 不分配、不加锁，可以在设备回调线程里调用。

/// 分析窗长：音乐用 30 ms 左右，短了相位感重，长了瞬态发糊
const FRAME_SECONDS: f64 = 0.030;
const UNITY_TOLERANCE: f32 = 1e-4;
const MIN_SPEED: f32 = 0.25;
const MAX_SPEED: f32 = 3.0;
/// 粗搜索的步长与相关计算的抽样间隔；之后在最佳点附近逐样本细搜
const COARSE_STEP: usize = 8;
const FINE_RADIUS: usize = COARSE_STEP - 1;
/// 连续这么多轮没有产出帧就退出：防御未预料的状态组合让回调线程空转
const MAX_IDLE_ROUNDS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rendered {
    /// 写进输出的帧数；少于请求说明输入断了
    pub frames: usize,
    /// 这些输出对应的媒体帧数（直通 1:1，变速按倍速折算），用来推进播放时钟
    pub media_frames: f64,
}

pub struct Stretcher {
    channels: usize,
    frame: usize,
    hop: usize,
    search: usize,
    window: Vec<f32>,
    input: Vec<f32>,
    capacity: usize,
    filled: usize,
    /// 直通模式下一帧的输出位置
    read: usize,
    active: bool,
    /// 上一段分析窗的起点；它的后半段还在 `overlap` 里等着叠加
    previous: usize,
    /// 下一段分析窗的名义起点，每跳前进 hop × 倍速
    nominal: f64,
    /// 上一跳实际选中的窗口起点减去它的名义起点（±search）。变速期间按名义位置报媒体
    /// 时间，回到直通时实际位置差着这么多，要补进媒体时间，否则每段变速都留下一个误差
    drift: f64,
    overlap: Vec<f32>,
    pending: Vec<f32>,
    pending_frames: usize,
    pending_read: usize,
    pending_speed: f64,
    /// 防空转保护触发的次数，正常应始终为 0
    stalls: u64,
}

impl Stretcher {
    pub fn new(channels: usize, sample_rate: u32) -> Self {
        let channels = channels.max(1);
        let mut frame = ((f64::from(sample_rate.max(8_000)) * FRAME_SECONDS) as usize).max(64);
        frame -= frame % 2;
        let hop = frame / 2;
        let search = hop / 2;
        // 周期汉宁窗：w[i] + w[i + hop] = 1，前后两段叠加后幅度不变
        let window = (0..frame)
            .map(|index| {
                (0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / frame as f64).cos()) as f32
            })
            .collect();
        // 历史 (hop + search) + 前瞻 (frame + search + 3 × hop)，再留出压缩前的余量
        let capacity = 4 * (frame + 4 * hop + 2 * search);
        Self {
            channels,
            frame,
            hop,
            search,
            window,
            input: vec![0.0; capacity * channels],
            capacity,
            filled: 0,
            read: 0,
            active: false,
            previous: 0,
            nominal: 0.0,
            drift: 0.0,
            overlap: vec![0.0; hop * channels],
            pending: vec![0.0; hop * channels],
            pending_frames: 0,
            pending_read: 0,
            pending_speed: 1.0,
            stalls: 0,
        }
    }

    pub fn stalls(&self) -> u64 {
        self.stalls
    }

    /// 已经从上游取出、还没交给输出的帧数（变速时含前瞻）
    pub fn buffered_frames(&self) -> usize {
        let pending = self.pending_frames - self.pending_read;
        let queued = if self.active {
            self.filled.saturating_sub(self.previous + self.hop)
        } else {
            self.filled - self.read
        };
        pending + queued
    }

    /// 生成 `out.len() / channels` 帧交错输出
    ///
    /// `pull` 往给定缓冲里填尽可能多的整帧并返回帧数，0 表示暂时没有数据。
    /// `draining` 为真表示上游已经结束：凑不齐一跳时把剩下的输入原样送出，不丢尾巴。
    pub fn render(
        &mut self,
        out: &mut [f32],
        speed: f32,
        draining: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
    ) -> Rendered {
        let channels = self.channels;
        let wanted = out.len() / channels;
        let speed = speed.clamp(MIN_SPEED, MAX_SPEED);
        let unity = (speed - 1.0).abs() < UNITY_TOLERANCE;
        let mut rendered = Rendered::default();
        let mut idle_rounds = 0;
        while rendered.frames < wanted {
            // 正常情况下连续几轮（压缩、激活、合成）就会产出帧；回调线程绝不能空转
            idle_rounds += 1;
            if idle_rounds > MAX_IDLE_ROUNDS {
                self.stalls += 1;
                break;
            }
            if self.pending_read < self.pending_frames {
                idle_rounds = 0;
                let take = (self.pending_frames - self.pending_read).min(wanted - rendered.frames);
                let source = self.pending_read * channels;
                let target = rendered.frames * channels;
                out[target..target + take * channels]
                    .copy_from_slice(&self.pending[source..source + take * channels]);
                self.pending_read += take;
                rendered.frames += take;
                rendered.media_frames += take as f64 * self.pending_speed;
                continue;
            }
            self.compact();
            if self.active {
                if self.synthesize_hop(speed, unity, pull) {
                    continue;
                }
                if !draining {
                    break;
                }
                // 上游已经结束、凑不齐一跳：从自然延续处直通剩下的输入
                self.active = false;
                self.read = (self.previous + self.hop).min(self.filled);
                rendered.media_frames += std::mem::take(&mut self.drift);
                continue;
            }
            // 激活前就要备齐第一跳的前瞻：只够半窗时激活会立刻凑不齐一跳，
            // 上游结束时又退回直通，两边来回打转
            if !unity
                && self.read >= self.hop + self.search
                && self.ensure(self.activation_end(speed), pull)
            {
                self.activate(speed);
                continue;
            }
            if self.read >= self.filled {
                let need = (wanted - rendered.frames).min(self.capacity - self.filled);
                if need == 0 || !self.ensure(self.filled + need, pull) && self.read >= self.filled {
                    break;
                }
            }
            let take = (self.filled - self.read).min(wanted - rendered.frames);
            if take > 0 {
                idle_rounds = 0;
            }
            let source = self.read * channels;
            let target = rendered.frames * channels;
            out[target..target + take * channels]
                .copy_from_slice(&self.input[source..source + take * channels]);
            self.read += take;
            rendered.frames += take;
            rendered.media_frames += take as f64;
        }
        rendered
    }

    /// 激活后第一跳需要的输入终点，与 `synthesize_hop` 的前瞻一致
    fn activation_end(&self, speed: f32) -> usize {
        let target = (self.read - self.hop) as f64 + self.hop as f64 * f64::from(speed);
        let lookahead = target.round() as usize + self.search + self.frame;
        lookahead.max(self.read + self.hop)
    }

    /// 把直通位置当作上一段窗口后半的起点：叠加缓冲取 read 之后半窗乘下降半窗
    fn activate(&mut self, speed: f32) {
        let channels = self.channels;
        self.previous = self.read - self.hop;
        for index in 0..self.hop {
            let weight = self.window[self.hop + index];
            for channel in 0..channels {
                self.overlap[index * channels + channel] =
                    self.input[(self.read + index) * channels + channel] * weight;
            }
        }
        self.nominal = self.previous as f64 + self.hop as f64 * f64::from(speed);
        self.drift = 0.0;
        self.active = true;
    }

    fn synthesize_hop(
        &mut self,
        speed: f32,
        unity: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
    ) -> bool {
        let channels = self.channels;
        let reference = self.previous + self.hop;
        let target = if unity {
            reference
        } else {
            self.nominal.round().max(0.0) as usize
        };
        let (low, high) = if unity {
            (target, target)
        } else {
            (target.saturating_sub(self.search), target + self.search)
        };
        if !self.ensure((high + self.frame).max(reference + self.hop), pull) {
            return false;
        }
        let start = if unity { reference } else { self.best_match(low, high, reference) };
        for index in 0..self.hop {
            let rising = self.window[index];
            let falling = self.window[self.hop + index];
            for channel in 0..channels {
                let slot = index * channels + channel;
                self.pending[slot] =
                    self.overlap[slot] + self.input[(start + index) * channels + channel] * rising;
                self.overlap[slot] =
                    self.input[(start + self.hop + index) * channels + channel] * falling;
            }
        }
        self.pending_frames = self.hop;
        self.pending_read = 0;
        self.previous = start;
        if unity {
            // 这一跳已经接回原始信号的自然延续，此后从 start + hop 起原样直通；
            // 补上最后一个变速窗与名义位置的差，直通后的媒体时间与实际读到的位置一致
            let hop = self.hop as f64;
            self.pending_speed = (hop + std::mem::take(&mut self.drift)) / hop;
            self.active = false;
            self.read = start + self.hop;
        } else {
            self.drift = start as f64 - self.nominal;
            self.pending_speed = f64::from(speed);
            self.nominal += self.hop as f64 * f64::from(speed);
        }
        true
    }

    /// 在 [low, high] 里找与 reference 起的自然延续最相似的窗口起点（归一化互相关）
    fn best_match(&self, low: usize, high: usize, reference: usize) -> usize {
        let mut best = low;
        let mut best_score = f32::MIN;
        let mut start = low;
        while start <= high {
            let score = self.similarity(start, reference, COARSE_STEP);
            if score > best_score {
                best_score = score;
                best = start;
            }
            start += COARSE_STEP;
        }
        let from = best.saturating_sub(FINE_RADIUS).max(low);
        let to = (best + FINE_RADIUS).min(high);
        best_score = f32::MIN;
        for start in from..=to {
            let score = self.similarity(start, reference, 2);
            if score > best_score {
                best_score = score;
                best = start;
            }
        }
        best
    }

    fn similarity(&self, start: usize, reference: usize, stride: usize) -> f32 {
        let channels = self.channels;
        let mut dot = 0.0f32;
        let mut energy = 1e-9f32;
        let mut index = 0;
        while index < self.hop {
            let candidate_at = (start + index) * channels;
            let reference_at = (reference + index) * channels;
            let mut candidate = 0.0f32;
            let mut natural = 0.0f32;
            for channel in 0..channels {
                candidate += self.input[candidate_at + channel];
                natural += self.input[reference_at + channel];
            }
            dot += candidate * natural;
            energy += candidate * candidate;
            index += stride;
        }
        dot / energy.sqrt()
    }

    /// 补足输入直到有 `end` 帧；只取需要的量，避免把上游缓冲提前抽空
    fn ensure(&mut self, end: usize, pull: &mut dyn FnMut(&mut [f32]) -> usize) -> bool {
        let end = end.min(self.capacity);
        while self.filled < end {
            let channels = self.channels;
            let got = pull(&mut self.input[self.filled * channels..end * channels]);
            if got == 0 {
                return false;
            }
            self.filled += got;
        }
        true
    }

    fn keep_from(&self) -> usize {
        if self.active {
            let floor = (self.nominal.floor() as usize).saturating_sub(self.search);
            self.previous.min(floor)
        } else {
            self.read.saturating_sub(self.hop + self.search)
        }
    }

    fn compact(&mut self) {
        let shift = self.keep_from();
        if shift == 0 || shift < self.capacity / 2 {
            return;
        }
        let channels = self.channels;
        self.input.copy_within(shift * channels..self.filled * channels, 0);
        self.filled -= shift;
        self.read = self.read.saturating_sub(shift);
        self.previous = self.previous.saturating_sub(shift);
        self.nominal -= shift as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::Stretcher;

    const RATE: u32 = 48_000;

    struct Source {
        samples: Vec<f32>,
        cursor: usize,
        channels: usize,
    }

    impl Source {
        fn sine(seconds: f64, frequency: f64) -> Self {
            let frames = (f64::from(RATE) * seconds) as usize;
            Self {
                samples: (0..frames)
                    .map(|index| (0.5 * (std::f64::consts::TAU * frequency * index as f64 / f64::from(RATE)).sin()) as f32)
                    .collect(),
                cursor: 0,
                channels: 1,
            }
        }

        fn pull(&mut self, out: &mut [f32]) -> usize {
            let frames = (out.len() / self.channels).min((self.samples.len() - self.cursor) / self.channels);
            let count = frames * self.channels;
            out[..count].copy_from_slice(&self.samples[self.cursor..self.cursor + count]);
            self.cursor += count;
            frames
        }
    }

    fn render(stretcher: &mut Stretcher, source: &mut Source, frames: usize, speed: f32, chunk: usize) -> (Vec<f32>, f64) {
        let mut output = Vec::with_capacity(frames);
        let mut media = 0.0;
        let mut block = vec![0.0f32; chunk];
        while output.len() < frames {
            let wanted = chunk.min(frames - output.len());
            let rendered = stretcher.render(&mut block[..wanted], speed, false, &mut |out| source.pull(out));
            output.extend_from_slice(&block[..rendered.frames]);
            media += rendered.media_frames;
            if rendered.frames < wanted {
                break;
            }
        }
        (output, media)
    }

    fn rising_crossings(samples: &[f32]) -> usize {
        samples.windows(2).filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0).count()
    }

    #[test]
    fn unity_speed_passes_samples_through_unchanged() {
        let mut source = Source {
            samples: (0..20_000).map(|index| index as f32).collect(),
            cursor: 0,
            channels: 2,
        };
        let mut stretcher = Stretcher::new(2, RATE);
        let mut output = Vec::new();
        let mut media = 0.0;
        for chunk in [37usize, 480, 1_000, 3] .iter().cycle().take(40) {
            let mut block = vec![0.0f32; chunk * 2];
            let rendered = stretcher.render(&mut block, 1.0, false, &mut |out| source.pull(out));
            output.extend_from_slice(&block[..rendered.frames * 2]);
            media += rendered.media_frames;
        }
        assert_eq!(output, source.samples[..output.len()].to_vec(), "1 倍速必须逐样本直通");
        assert_eq!(media, (output.len() / 2) as f64);
        assert_eq!(stretcher.buffered_frames(), 0, "直通时不该多抽上游数据");
    }

    #[test]
    fn speeding_up_and_slowing_down_keep_the_pitch() {
        for speed in [1.25f32, 0.8, 1.05, 0.95] {
            let mut source = Source::sine(4.0, 440.0);
            let mut stretcher = Stretcher::new(1, RATE);
            let (output, media) = render(&mut stretcher, &mut source, 96_000, speed, 480);
            assert_eq!(output.len(), 96_000, "speed {speed}: 输入足够时应输出满额");
            let tail = &output[24_000..];
            let frequency = rising_crossings(tail) as f64 / (tail.len() as f64 / f64::from(RATE));
            assert!((frequency - 440.0).abs() < 4.0, "speed {speed}: 音调应保持 440 Hz，实测 {frequency:.1} Hz");
            let consumed = source.cursor as f64;
            // 开始变速前先直通约 22 ms 攒历史，这段按 1:1 计
            assert!((media - 96_000.0 * f64::from(speed)).abs() < 400.0, "speed {speed}: 媒体时间按倍速折算: {media}");
            assert!((consumed - media).abs() < 4_000.0, "speed {speed}: 消耗的输入 {consumed} 应接近媒体时间 {media}");
        }
    }

    #[test]
    fn rate_changes_are_click_free_and_return_to_exact_passthrough() {
        let mut source = Source::sine(8.0, 440.0);
        let mut stretcher = Stretcher::new(1, RATE);
        let mut output = Vec::new();
        for (frames, speed) in [(24_000usize, 1.0f32), (48_000, 1.05), (24_000, 0.97), (48_000, 1.0)] {
            output.extend(render(&mut stretcher, &mut source, frames, speed, 480).0);
        }
        // 0.5 振幅 440 Hz 正弦的相邻样本差最多约 0.029；叠加过渡允许略大，但不能出现跳变
        let largest_step = output.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0f32, f32::max);
        assert!(largest_step < 0.06, "变速切换出现跳变: {largest_step}");
        // 回到 1 倍速后应当原样直通：最后一段输出必须是输入里连续的一段
        let tail = &output[output.len() - 12_000..];
        let found = source.samples.windows(tail.len()).any(|window| window == tail);
        assert!(found, "回到 1 倍速后应恢复逐样本直通");
    }

    /// 一起听的软同步会反复进出变速：每段变速选中的窗口与名义位置差着 ±search，
    /// 回到直通时要补回来，否则媒体时间和实际读到的位置每段都多差一点
    #[test]
    fn media_time_matches_the_input_position_after_many_speed_episodes() {
        let mut source = Source {
            samples: (0..400_000).map(|index| index as f32).collect(),
            cursor: 0,
            channels: 1,
        };
        let mut stretcher = Stretcher::new(1, RATE);
        let mut media = 0.0;
        let mut last = 0.0f32;
        for _ in 0..20 {
            for (frames, speed) in [(4_800usize, 1.05f32), (9_600, 1.0)] {
                let (output, advanced) = render(&mut stretcher, &mut source, frames, speed, 480);
                assert_eq!(output.len(), frames);
                media += advanced;
                last = *output.last().unwrap();
            }
        }
        // 直通时输出就是输入，斜坡信号最后一个样本的值就是它在输入里的下标
        let position = f64::from(last) + 1.0;
        assert!((media - position).abs() < 2.0, "媒体时间 {media} 应等于实际读到的位置 {position}");
    }

    /// 上游结束时从变速直接退出到直通：同样要把窗口偏差补进媒体时间
    #[test]
    fn draining_out_of_a_stretch_reports_the_exact_input_length() {
        let mut source = Source {
            samples: (0..48_000).map(|index| index as f32).collect(),
            cursor: 0,
            channels: 1,
        };
        let mut stretcher = Stretcher::new(1, RATE);
        let mut block = vec![0.0f32; 480];
        let mut media = 0.0;
        for _ in 0..400 {
            let rendered = stretcher.render(&mut block, 1.05, true, &mut |out| source.pull(out));
            media += rendered.media_frames;
            if rendered.frames < block.len() {
                break;
            }
        }
        assert_eq!(stretcher.buffered_frames(), 0);
        assert!((media - 48_000.0).abs() < 2.0, "整段媒体时间应等于输入长度: {media}");
    }

    #[test]
    fn running_out_of_input_returns_a_short_render_instead_of_blocking() {
        let mut source = Source::sine(0.1, 440.0);
        let mut stretcher = Stretcher::new(1, RATE);
        let mut block = vec![0.0f32; 9_600];
        let rendered = stretcher.render(&mut block, 1.2, false, &mut |out| source.pull(out));
        assert!(rendered.frames < 9_600 && rendered.frames > 0, "输入不足时返回已经生成的部分: {}", rendered.frames);
    }

    #[test]
    fn draining_flushes_the_tail_once_the_source_has_ended() {
        let mut source = Source::sine(1.0, 440.0);
        let mut stretcher = Stretcher::new(1, RATE);
        let mut block = vec![0.0f32; 480];
        let mut media = 0.0;
        for _ in 0..400 {
            let rendered = stretcher.render(&mut block, 1.25, true, &mut |out| source.pull(out));
            media += rendered.media_frames;
            if rendered.frames < block.len() {
                break;
            }
        }
        assert_eq!(source.cursor, 48_000, "上游的全部输入都应被消费");
        assert_eq!(stretcher.buffered_frames(), 0, "结束时不能有残留");
        assert_eq!(stretcher.stalls(), 0, "不应触发防空转保护");
        assert!((media - 48_000.0).abs() < 2_000.0, "整段媒体时间应接近输入长度: {media}");
    }
}
