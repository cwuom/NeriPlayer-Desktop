//! 保持音调的变速（WSOLA，波形相似叠加）
//!
//! 设备回调按实时速度消费解码好的 PCM。倍速为 1 时逐样本直通；偏离 1 时每个合成跳（半窗）
//! 从输入里挑一段与上一段自然延续最相似的窗口做汉宁叠加，时长按倍速伸缩而音调不变。
//! 改倍速在下一跳生效；进出变速各经过一次叠加过渡，没有咔哒声，也不需要重建解码会话。
//! 缓冲全部在构造时分配，`render` 不分配、不加锁，可以在设备回调线程里调用。

/// 分析窗长：音乐用 30 ms 左右，短了相位感重，长了瞬态发糊
const FRAME_SECONDS: f64 = 0.030;
const UNITY_TOLERANCE: f32 = 1e-4;
// 独立音调先按 speed / pitch 伸缩，再重采样，内部范围覆盖两者的全部组合
const MIN_SPEED: f32 = 0.025;
const MAX_SPEED: f32 = 40.0;
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
    /// 下一段输出对应的媒体位置；短曲也从第一帧按倍速推进
    media_position: f64,
    source_ended: bool,
    first_hop: bool,
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
        // 高倍速的一跳也要预读到媒体落点，EOF 才能限制尾部的实际输出时长
        let capacity = 4 * (frame + (MAX_SPEED as usize + 1) * hop + 2 * search);
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
            media_position: 0.0,
            source_ended: false,
            first_hop: false,
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
            self.filled
                .saturating_sub(self.media_position.ceil().max(0.0) as usize)
        } else {
            self.filled - self.read
        };
        pending + queued
    }

    /// 生成 `out.len() / channels` 帧交错输出
    ///
    /// `pull` 往给定缓冲里填尽可能多的整帧并返回帧数，0 表示暂时没有数据。
    /// `draining` 为真表示上游已经结束，尾部按剩余媒体帧限制输出，不退回原速
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
                let media = take as f64 * self.pending_speed;
                rendered.media_frames += media;
                self.media_position += media;
                continue;
            }
            self.compact();
            if self.active {
                if self.synthesize_hop(speed, unity, draining, pull) {
                    continue;
                }
                break;
            }
            if !unity {
                let end = self.read + self.frame;
                if self.ensure(end, draining, pull) || self.source_ended && self.read < self.filled
                {
                    self.activate();
                    continue;
                }
                break;
            }
            if self.read >= self.filled {
                let need = (wanted - rendered.frames).min(self.capacity - self.filled);
                if need == 0
                    || !self.ensure(self.filled + need, draining, pull) && self.read >= self.filled
                {
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
            self.media_position = self.read as f64;
            rendered.frames += take;
            rendered.media_frames += take as f64;
        }
        rendered
    }

    fn input_sample(&self, frame: usize, channel: usize) -> f32 {
        if frame < self.filled {
            self.input[frame * self.channels + channel]
        } else {
            0.0
        }
    }

    fn activate(&mut self) {
        let channels = self.channels;
        self.previous = self.read;
        for index in 0..self.hop {
            let weight = self.window[self.hop + index];
            for channel in 0..channels {
                let sample = self.input_sample(self.read + index, channel);
                self.overlap[index * channels + channel] = sample * weight;
            }
        }
        self.media_position = self.read as f64;
        self.first_hop = true;
        self.active = true;
    }

    fn synthesize_hop(
        &mut self,
        speed: f32,
        unity: bool,
        draining: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
    ) -> bool {
        let channels = self.channels;
        let reference = if self.first_hop {
            self.previous
        } else {
            self.previous + self.hop
        };
        let target = self.media_position.round().max(0.0) as usize;
        let (low, high) = if unity || self.first_hop {
            (target, target)
        } else {
            (target.saturating_sub(self.search), target + self.search)
        };
        let media_end = (self.media_position + self.hop as f64 * f64::from(speed)).ceil() as usize;
        let need = (high + self.frame).max(reference + self.hop).max(media_end);
        if !self.ensure(need, draining, pull) && !self.source_ended {
            return false;
        }
        let remaining = (self.filled as f64 - self.media_position).max(0.0);
        if self.source_ended && remaining <= 1e-7 {
            self.active = false;
            self.read = self.filled;
            return false;
        }
        let start = if unity || self.first_hop {
            target.min(self.filled.saturating_sub(1))
        } else {
            self.best_match(low, high, reference)
        };
        let frames = if self.source_ended {
            (remaining / f64::from(speed)).ceil().max(1.0) as usize
        } else {
            self.hop
        }
        .min(self.hop);
        let frames = if unity {
            frames.min(self.filled.saturating_sub(start))
        } else {
            frames
        };
        for index in 0..frames {
            let rising = self.window[index];
            let falling = self.window[self.hop + index];
            for channel in 0..channels {
                let slot = index * channels + channel;
                let sample = self.input_sample(start + index, channel);
                let next = self.input_sample(start + self.hop + index, channel);
                self.pending[slot] = self.overlap[slot] + sample * rising;
                self.overlap[slot] = next * falling;
            }
        }
        self.pending_frames = frames;
        self.pending_read = 0;
        self.previous = start;
        self.first_hop = false;
        if unity {
            // 叠加过渡后恢复逐样本直通，四舍五入造成的位置差也计入媒体时间
            self.pending_speed =
                (start as f64 + frames as f64 - self.media_position) / frames.max(1) as f64;
            self.active = false;
            self.read = start + frames;
        } else {
            let media = frames as f64 * f64::from(speed);
            self.pending_speed = if self.source_ended {
                media.min(remaining)
            } else {
                media
            } / frames.max(1) as f64;
        }
        true
    }

    /// 在 [low, high] 里找与 reference 起的自然延续最相似的窗口起点（归一化互相关）
    fn best_match(&self, low: usize, high: usize, reference: usize) -> usize {
        let nominal = low + (high - low) / 2;
        let mut best = nominal;
        let mut best_score = self.similarity(nominal, reference, COARSE_STEP);
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
        best_score = self.similarity(best, reference, 2);
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
            for channel in 0..channels {
                // 逐声道比较，避免反相立体声相加成静音
                let candidate = self.input_sample(start + index, channel);
                let natural = self.input_sample(reference + index, channel);
                dot += candidate * natural;
                energy += candidate * candidate;
            }
            index += stride;
        }
        dot / energy.sqrt()
    }

    /// 补足输入直到有 `end` 帧；只取需要的量，避免把上游缓冲提前抽空
    fn ensure(
        &mut self,
        end: usize,
        draining: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
    ) -> bool {
        let end = end.min(self.capacity);
        if self.source_ended {
            return self.filled >= end;
        }
        while self.filled < end {
            let channels = self.channels;
            let got = pull(&mut self.input[self.filled * channels..end * channels]);
            if got == 0 {
                self.source_ended |= draining;
                return false;
            }
            self.filled += got;
        }
        true
    }

    fn keep_from(&self) -> usize {
        if self.active {
            let floor = (self.media_position.floor().max(0.0) as usize).saturating_sub(self.search);
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
        self.input
            .copy_within(shift * channels..self.filled * channels, 0);
        self.filled -= shift;
        self.read = self.read.saturating_sub(shift);
        self.previous = self.previous.saturating_sub(shift);
        self.media_position -= shift as f64;
    }
}

struct LowPassSection {
    coefficients: [f64; 5],
    target_coefficients: [f64; 5],
    coefficient_steps: [f64; 5],
    remaining: usize,
    states: Vec<[f64; 2]>,
}

impl LowPassSection {
    fn new(channels: usize) -> Self {
        Self {
            coefficients: [1.0, 0.0, 0.0, 0.0, 0.0],
            target_coefficients: [1.0, 0.0, 0.0, 0.0, 0.0],
            coefficient_steps: [0.0; 5],
            remaining: 0,
            states: vec![[0.0; 2]; channels],
        }
    }

    fn tune(&mut self, normalized_cutoff: f64, q: f64, ramp_frames: usize) {
        let omega = std::f64::consts::TAU * normalized_cutoff;
        let cosine = omega.cos();
        let alpha = omega.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b0 = (1.0 - cosine) / (2.0 * a0);
        let target = [b0, 2.0 * b0, b0, -2.0 * cosine / a0, (1.0 - alpha) / a0];
        if target
            .iter()
            .zip(self.target_coefficients)
            .any(|(new, old)| (new - old).abs() > 1e-9)
        {
            self.target_coefficients = target;
            self.remaining = ramp_frames;
            for ((step, target), value) in self
                .coefficient_steps
                .iter_mut()
                .zip(target)
                .zip(self.coefficients)
            {
                *step = (target - value) / ramp_frames as f64;
            }
        }
    }

    fn advance(&mut self) {
        if self.remaining > 0 {
            for (coefficient, step) in self.coefficients.iter_mut().zip(self.coefficient_steps) {
                *coefficient += step;
            }
            self.remaining -= 1;
            if self.remaining == 0 {
                self.coefficients = self.target_coefficients;
            }
        }
    }

    fn process(&mut self, input: f64, channel: usize) -> f64 {
        let [b0, b1, b2, a1, a2] = self.coefficients;
        let state = &mut self.states[channel];
        let output = b0 * input + state[0];
        state[0] = b1 * input - a1 * output + state[1];
        state[1] = b2 * input - a2 * output;
        for value in state {
            if value.abs() < 1e-30 {
                *value = 0.0;
            }
        }
        output
    }
}

struct AntialiasFilter {
    sections: [LowPassSection; 2],
    mix: f64,
    target_mix: f64,
    mix_step: f64,
    remaining: usize,
    ramp_frames: usize,
}

impl AntialiasFilter {
    fn new(channels: usize, sample_rate: u32, pitch: f64) -> Self {
        let mix = if pitch > 1.0 { 1.0 } else { 0.0 };
        let mut filter = Self {
            sections: std::array::from_fn(|_| LowPassSection::new(channels)),
            mix,
            target_mix: mix,
            mix_step: 0.0,
            remaining: 0,
            ramp_frames: (f64::from(sample_rate.max(1)) * 0.03).round().max(1.0) as usize,
        };
        filter.prepare(pitch);
        for section in &mut filter.sections {
            section.coefficients = section.target_coefficients;
            section.remaining = 0;
        }
        filter
    }

    fn prepare(&mut self, pitch: f64) {
        let target = if pitch > 1.0 + 1e-6 { 1.0 } else { 0.0 };
        if target != self.target_mix {
            self.target_mix = target;
            self.remaining = self.ramp_frames;
            self.mix_step = (target - self.mix) / self.ramp_frames as f64;
        }
        if pitch > 1.0 {
            // 四阶 Butterworth 在重采样后的 Nyquist 之前留出过渡带
            let cutoff = 0.45 / pitch;
            self.sections[0].tune(cutoff, 0.541_196_1, self.ramp_frames);
            self.sections[1].tune(cutoff, 1.306_563, self.ramp_frames);
        }
    }

    fn is_idle(&self) -> bool {
        self.mix == 0.0 && self.remaining == 0
    }

    fn process(&mut self, frame: &mut [f32]) {
        if self.is_idle() {
            return;
        }
        if self.remaining > 0 {
            self.mix += self.mix_step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.mix = self.target_mix;
            }
        }
        for section in &mut self.sections {
            section.advance();
        }
        for (channel, sample) in frame.iter_mut().enumerate() {
            let input = if sample.is_finite() {
                f64::from(*sample)
            } else {
                0.0
            };
            let mut filtered = input;
            for section in &mut self.sections {
                filtered = section.process(filtered, channel);
            }
            *sample = (input + (filtered - input) * self.mix) as f32;
        }
        if self.is_idle() {
            for section in &mut self.sections {
                section.states.fill([0.0; 2]);
            }
        }
    }
}

/// 先保持音高地伸缩，再按音调比例采样，最终媒体速度只由 speed 决定
pub struct PitchShifter {
    channels: usize,
    stretcher: Stretcher,
    antialias: AntialiasFilter,
    current: Vec<f32>,
    next: Vec<f32>,
    current_media: f64,
    next_media: f64,
    has_current: bool,
    has_next: bool,
    phase: f64,
    pending_step: f64,
    pitch: f64,
    target_pitch: f64,
    pitch_step: f64,
    ramp_remaining: usize,
    ramp_frames: usize,
}

impl PitchShifter {
    pub fn new(channels: usize, sample_rate: u32, pitch: f32) -> Self {
        let channels = channels.max(1);
        let pitch = f64::from(pitch.clamp(0.1, 4.0));
        Self {
            channels,
            stretcher: Stretcher::new(channels, sample_rate),
            antialias: AntialiasFilter::new(channels, sample_rate, pitch),
            current: vec![0.0; channels],
            next: vec![0.0; channels],
            current_media: 0.0,
            next_media: 0.0,
            has_current: false,
            has_next: false,
            phase: 0.0,
            pending_step: 0.0,
            pitch,
            target_pitch: pitch,
            pitch_step: 0.0,
            ramp_remaining: 0,
            ramp_frames: (f64::from(sample_rate.max(1)) * 0.03).round().max(1.0) as usize,
        }
    }

    pub fn buffered_frames(&self) -> usize {
        self.stretcher.buffered_frames()
            + usize::from(self.has_current)
            + usize::from(self.has_next)
    }

    fn read_frame(
        &mut self,
        next: bool,
        speed: f32,
        draining: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
        media_frames: &mut f64,
    ) -> bool {
        let target = if next {
            &mut self.next
        } else {
            &mut self.current
        };
        let rendered = self
            .stretcher
            .render(target, speed / self.pitch as f32, draining, pull);
        if rendered.frames == 0 {
            // 退出伸缩时可能只剩媒体位置校正，仍要记入时钟
            *media_frames += rendered.media_frames;
            return false;
        }
        self.antialias.process(target);
        if next {
            self.next_media = rendered.media_frames;
            self.has_next = true;
        } else {
            self.current_media = rendered.media_frames;
            self.has_current = true;
        }
        true
    }

    fn advance(
        &mut self,
        speed: f32,
        draining: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
        media_frames: &mut f64,
    ) -> bool {
        while self.pending_step > 1e-9 {
            if !self.has_current && !self.read_frame(false, speed, draining, pull, media_frames) {
                if draining {
                    self.pending_step = 0.0;
                }
                return false;
            }
            let step = self.pending_step.min(1.0 - self.phase);
            *media_frames += step * self.current_media;
            self.phase += step;
            self.pending_step -= step;
            if self.phase >= 1.0 - 1e-9 {
                self.phase = 0.0;
                self.has_current = self.has_next;
                if self.has_next {
                    std::mem::swap(&mut self.current, &mut self.next);
                    self.current_media = self.next_media;
                    self.has_next = false;
                }
            }
        }
        self.pending_step = 0.0;
        true
    }

    /// 全部状态预分配；保留欠载时的插值与前瞻，恢复后继续同一段音频
    pub fn render(
        &mut self,
        out: &mut [f32],
        speed: f32,
        pitch: f32,
        draining: bool,
        pull: &mut dyn FnMut(&mut [f32]) -> usize,
    ) -> Rendered {
        let target = f64::from(pitch.clamp(0.1, 4.0));
        if (target - self.target_pitch).abs() > 1e-6 {
            self.target_pitch = target;
            self.ramp_remaining = self.ramp_frames;
            self.pitch_step = (target - self.pitch) / self.ramp_frames as f64;
        }
        self.antialias.prepare(self.pitch.max(target));
        let channels = self.channels;
        let wanted = out.len() / channels;
        let mut rendered = Rendered::default();
        if !self.advance(speed, draining, pull, &mut rendered.media_frames) {
            return rendered;
        }
        while rendered.frames < wanted {
            if self.ramp_remaining > 0 {
                self.pitch += self.pitch_step;
                self.ramp_remaining -= 1;
                if self.ramp_remaining == 0 {
                    self.pitch = self.target_pitch;
                    if self.pitch == 1.0 && self.has_current {
                        // 回到原音调后接回整帧，半帧位置差也交给媒体时钟处理
                        rendered.media_frames -= self.phase * self.current_media;
                        self.phase = 0.0;
                    }
                }
            }
            if self.pitch == 1.0
                && self.ramp_remaining == 0
                && !self.has_current
                && !self.has_next
                && self.antialias.is_idle()
            {
                let rest = self.stretcher.render(
                    &mut out[rendered.frames * channels..],
                    speed,
                    draining,
                    pull,
                );
                rendered.frames += rest.frames;
                rendered.media_frames += rest.media_frames;
                break;
            }
            if !self.has_current
                && !self.read_frame(false, speed, draining, pull, &mut rendered.media_frames)
            {
                break;
            }
            if self.phase > 1e-9
                && !self.has_next
                && !self.read_frame(true, speed, draining, pull, &mut rendered.media_frames)
                && !draining
            {
                break;
            }
            let target = &mut out[rendered.frames * channels..(rendered.frames + 1) * channels];
            for (channel, sample) in target.iter_mut().enumerate() {
                let current = self.current[channel];
                let next = if self.has_next {
                    self.next[channel]
                } else {
                    current
                };
                *sample = current + (next - current) * self.phase as f32;
            }
            rendered.frames += 1;
            self.pending_step = self.pitch;
            if !self.advance(speed, draining, pull, &mut rendered.media_frames) {
                break;
            }
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::{PitchShifter, Stretcher};

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
                    .map(|index| {
                        (0.5 * (std::f64::consts::TAU * frequency * index as f64 / f64::from(RATE))
                            .sin()) as f32
                    })
                    .collect(),
                cursor: 0,
                channels: 1,
            }
        }

        fn pull(&mut self, out: &mut [f32]) -> usize {
            let frames =
                (out.len() / self.channels).min((self.samples.len() - self.cursor) / self.channels);
            let count = frames * self.channels;
            out[..count].copy_from_slice(&self.samples[self.cursor..self.cursor + count]);
            self.cursor += count;
            frames
        }
    }

    fn render(
        stretcher: &mut Stretcher,
        source: &mut Source,
        frames: usize,
        speed: f32,
        chunk: usize,
    ) -> (Vec<f32>, f64) {
        let mut output = Vec::with_capacity(frames);
        let mut media = 0.0;
        let mut block = vec![0.0f32; chunk];
        while output.len() < frames {
            let wanted = chunk.min(frames - output.len());
            let rendered = stretcher.render(&mut block[..wanted], speed, false, &mut |out| {
                source.pull(out)
            });
            output.extend_from_slice(&block[..rendered.frames]);
            media += rendered.media_frames;
            if rendered.frames < wanted {
                break;
            }
        }
        (output, media)
    }

    fn rising_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count()
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
        for chunk in [37usize, 480, 1_000, 3].iter().cycle().take(40) {
            let mut block = vec![0.0f32; chunk * 2];
            let rendered = stretcher.render(&mut block, 1.0, false, &mut |out| source.pull(out));
            output.extend_from_slice(&block[..rendered.frames * 2]);
            media += rendered.media_frames;
        }
        assert_eq!(
            output,
            source.samples[..output.len()].to_vec(),
            "1 倍速必须逐样本直通"
        );
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
            assert!(
                (frequency - 440.0).abs() < 4.0,
                "speed {speed}: 音调应保持 440 Hz，实测 {frequency:.1} Hz"
            );
            let consumed = source.cursor as f64;
            // 固定速度的媒体推进不受 WSOLA 选中窗口的相位影响
            assert!(
                (media - 96_000.0 * f64::from(speed)).abs() < 400.0,
                "speed {speed}: 媒体时间按倍速折算: {media}"
            );
            assert!(
                (consumed - media).abs() < 4_000.0,
                "speed {speed}: 消耗的输入 {consumed} 应接近媒体时间 {media}"
            );
        }
    }

    #[test]
    fn android_speed_extremes_keep_the_pitch_and_advance_media_time() {
        for (speed, expected_media) in [(0.1f32, 9_600.0), (4.0, 384_000.0)] {
            let mut source = Source::sine(14.0, 440.0);
            let mut stretcher = Stretcher::new(1, RATE);
            render(&mut stretcher, &mut source, 4_800, speed, 480);
            let (output, media) = render(&mut stretcher, &mut source, 96_000, speed, 480);
            assert_eq!(output.len(), 96_000);
            assert!(
                (media - expected_media).abs() < 2.0,
                "speed {speed}: media {media}"
            );
            let tail = &output[48_000..];
            let frequency = rising_crossings(tail) as f64;
            assert!(
                (frequency - 440.0).abs() < 8.0,
                "speed {speed}: frequency {frequency}"
            );
            assert_eq!(stretcher.stalls(), 0);
        }
    }

    fn pitched_source(speed: f32, pitch: f32, seconds: f64) -> (Vec<f32>, f64) {
        let mut source = Source::sine(seconds, 440.0);
        let mut processor = PitchShifter::new(1, RATE, pitch);
        let mut output = Vec::new();
        let mut media = 0.0;
        let mut block = [0.0f32; 480];
        for _ in 0..10_000 {
            let rendered =
                processor.render(&mut block, speed, pitch, true, &mut |out| source.pull(out));
            output.extend_from_slice(&block[..rendered.frames]);
            media += rendered.media_frames;
            if rendered.frames < block.len() && processor.buffered_frames() == 0 {
                break;
            }
        }
        assert_eq!(source.cursor, source.samples.len());
        assert_eq!(processor.buffered_frames(), 0);
        assert_eq!(processor.stretcher.stalls(), 0);
        (output, media)
    }

    #[test]
    fn vinyl_pitch_covers_the_full_playback_speed_range() {
        for speed in [0.1f32, 0.95, 1.05, 4.0] {
            let (output, media) = pitched_source(speed, speed, 0.3);
            let middle = &output[output.len() / 3..output.len() * 2 / 3];
            let frequency =
                rising_crossings(middle) as f64 / (middle.len() as f64 / f64::from(RATE));
            assert!(
                (frequency - 440.0 * f64::from(speed)).abs() < 35.0,
                "speed {speed}: frequency {frequency}"
            );
            assert!(
                output
                    .len()
                    .abs_diff((14_400.0 / f64::from(speed)).round() as usize)
                    <= 3
            );
            assert!((media - 14_400.0).abs() < 2.0);
        }
    }

    #[test]
    fn independent_pitch_changes_frequency_without_changing_media_speed() {
        for (speed, pitch, expected_frequency, expected_frames) in [
            (1.0f32, 0.5f32, 220.0, 144_000usize),
            (1.0, 2.0, 880.0, 144_000),
            (2.0, 0.5, 220.0, 72_000),
            (0.5, 1.5, 660.0, 288_000),
        ] {
            let (output, media) = pitched_source(speed, pitch, 3.0);
            let middle = &output[output.len() / 3..output.len() * 2 / 3];
            let frequency = rising_crossings(middle) as f64 / (middle.len() as f64 / 48_000.0);
            assert!(
                (frequency - expected_frequency).abs() < 8.0,
                "speed {speed}, pitch {pitch}: frequency {frequency}"
            );
            assert!(
                output.len().abs_diff(expected_frames) < 5_000,
                "speed {speed}, pitch {pitch}: duration frames {}",
                output.len()
            );
            assert!(
                (media - 144_000.0).abs() < 2.0,
                "speed {speed}, pitch {pitch}: media {media}"
            );
        }
    }

    #[test]
    fn raising_pitch_rejects_above_nyquist_input_without_muffling_the_passband() {
        for (input_frequency, output_frequency, minimum, maximum) in [
            (18_000.0, 12_000.0, 0.0, 0.02),
            (4_000.0, 8_000.0, 0.45, 0.51),
        ] {
            let mut source = Source::sine(2.0, input_frequency);
            let mut processor = PitchShifter::new(1, RATE, 2.0);
            let mut block = [0.0f32; 480];
            let mut steady = Vec::new();
            for index in 0..100 {
                let rendered =
                    processor.render(&mut block, 1.0, 2.0, false, &mut |out| source.pull(out));
                assert_eq!(rendered.frames, block.len());
                if index >= 50 {
                    steady.extend_from_slice(&block);
                }
            }
            let (sine, cosine) =
                steady
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(sine, cosine), (index, sample)| {
                        let phase =
                            std::f64::consts::TAU * output_frequency * index as f64 / 48_000.0;
                        (
                            sine + f64::from(*sample) * phase.sin(),
                            cosine + f64::from(*sample) * phase.cos(),
                        )
                    });
            let amplitude = 2.0 * sine.hypot(cosine) / steady.len() as f64;
            assert!(
                (minimum..=maximum).contains(&amplitude),
                "input {input_frequency}: output {output_frequency} amplitude {amplitude}"
            );
        }
    }

    #[test]
    fn independent_pitch_drains_short_tracks_and_extreme_combinations() {
        for (speed, pitch) in [(0.1f32, 2.0f32), (4.0, 0.25), (1.0, 0.25), (1.0, 2.0)] {
            let (_, media) = pitched_source(speed, pitch, 0.3);
            assert!(
                (media - 14_400.0).abs() < 2.0,
                "speed {speed}, pitch {pitch}: media {media}"
            );
        }
    }

    #[test]
    fn pitch_and_speed_preserve_the_actual_output_duration_at_track_boundaries() {
        for (seconds, speed, pitch, expected_frames) in [
            (0.02, 1.0f32, 0.25f32, 960usize),
            (0.3, 1.0, 0.25, 14_400),
            (0.3, 4.0, 0.25, 3_600),
            (0.3, 0.1, 2.0, 144_000),
            (3.0, 4.0, 0.25, 36_000),
            (30.0, 4.0, 0.25, 360_000),
        ] {
            let (output, _) = pitched_source(speed, pitch, seconds);
            assert!(output.len().abs_diff(expected_frames) <= 3, "{seconds}s, speed {speed}, pitch {pitch}: {} output frames, expected {expected_frames}", output.len());
        }
    }

    #[test]
    fn silent_and_opposite_phase_stereo_tracks_finish_without_looping_the_tail() {
        for silent in [false, true] {
            let mut source = Source {
                samples: Vec::with_capacity(28_800),
                cursor: 0,
                channels: 2,
            };
            for index in 0..14_400 {
                let sample = if silent {
                    0.0
                } else {
                    (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32
                };
                source.samples.extend_from_slice(&[sample, -sample]);
            }
            let mut processor = PitchShifter::new(2, RATE, 2.0);
            let mut block = [0.0f32; 960];
            let mut media = 0.0;
            let mut frames = 0;
            for _ in 0..100 {
                let rendered =
                    processor.render(&mut block, 1.0, 2.0, true, &mut |out| source.pull(out));
                assert!(block[..rendered.frames * 2]
                    .chunks_exact(2)
                    .all(|frame| (frame[0] + frame[1]).abs() < 1e-6));
                media += rendered.media_frames;
                frames += rendered.frames;
                if rendered.frames < block.len() / 2 && processor.buffered_frames() == 0 {
                    break;
                }
            }
            assert_eq!(
                processor.buffered_frames(),
                0,
                "silent {silent}: tail must finish"
            );
            assert!(
                (media - 14_400.0).abs() < 2.0,
                "silent {silent}: media {media}"
            );
            assert!(
                frames.abs_diff(14_400) <= 3,
                "silent {silent}: frames {frames}"
            );
        }
    }

    #[test]
    fn original_pitch_and_speed_preserve_exact_stereo_samples() {
        let mut source = Source {
            samples: (0..20_000).map(|index| index as f32 / 20_000.0).collect(),
            channels: 2,
            cursor: 0,
        };
        let mut processor = PitchShifter::new(2, RATE, 1.0);
        let mut output = [0.0f32; 960];
        let rendered = processor.render(&mut output, 1.0, 1.0, false, &mut |out| source.pull(out));
        assert_eq!(output, source.samples[..960]);
        assert_eq!(rendered.media_frames, 480.0);
        assert_eq!(processor.buffered_frames(), 0);
    }

    #[test]
    fn changing_pitch_preserves_continuity_and_returns_to_exact_passthrough() {
        let mut source = Source::sine(4.0, 440.0);
        let mut processor = PitchShifter::new(1, RATE, 1.0);
        let mut block = [0.0f32; 480];
        let mut output = Vec::new();
        let mut media = 0.0;
        for pitch in [1.0f32, 1.5, 0.75, 2.0, 1.0] {
            for _ in 0..40 {
                let rendered =
                    processor.render(&mut block, 1.0, pitch, false, &mut |out| source.pull(out));
                assert_eq!(rendered.frames, block.len());
                output.extend_from_slice(&block);
                media += rendered.media_frames;
            }
        }
        let (index, largest_step) = output
            .windows(2)
            .enumerate()
            .map(|(index, pair)| (index, (pair[1] - pair[0]).abs()))
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .unwrap();
        assert!(
            largest_step < 0.13,
            "pitch change step {largest_step} at frame {index}"
        );
        let tail = &output[output.len() - 4_800..];
        assert!(source
            .samples
            .windows(tail.len())
            .any(|window| window == tail));
        for _ in 0..500 {
            let rendered =
                processor.render(&mut block, 1.0, 1.0, true, &mut |out| source.pull(out));
            media += rendered.media_frames;
            if rendered.frames < block.len() && processor.buffered_frames() == 0 {
                break;
            }
        }
        assert!(
            (media - 192_000.0).abs() < 2.0,
            "media after pitch changes {media}"
        );
    }

    #[test]
    fn pitch_underflow_retains_interpolation_and_drains_after_recovery() {
        let mut source = Source::sine(1.0, 440.0);
        let mut processor = PitchShifter::new(1, RATE, 1.5);
        let mut block = [0.0f32; 480];
        let mut media = 0.0;
        let mut frames = 0;
        for _ in 0..100 {
            let rendered = processor.render(&mut block, 1.0, 1.5, false, &mut |out| {
                let available = 9_600usize.saturating_sub(source.cursor);
                let take = out.len().min(available);
                source.pull(&mut out[..take])
            });
            media += rendered.media_frames;
            frames += rendered.frames;
            if rendered.frames < block.len() {
                break;
            }
        }
        assert!(frames > 0);
        assert!(processor.buffered_frames() > 0);
        for _ in 0..300 {
            let rendered =
                processor.render(&mut block, 1.0, 1.5, true, &mut |out| source.pull(out));
            media += rendered.media_frames;
            if rendered.frames < block.len() && processor.buffered_frames() == 0 {
                break;
            }
        }
        assert_eq!(processor.buffered_frames(), 0);
        assert!(
            (media - 48_000.0).abs() < 2.0,
            "media after underflow {media}"
        );
    }

    #[test]
    fn rate_changes_are_click_free_and_return_to_exact_passthrough() {
        let mut source = Source::sine(8.0, 440.0);
        let mut stretcher = Stretcher::new(1, RATE);
        let mut output = Vec::new();
        for (frames, speed) in [
            (24_000usize, 1.0f32),
            (48_000, 1.05),
            (24_000, 0.97),
            (48_000, 1.0),
        ] {
            output.extend(render(&mut stretcher, &mut source, frames, speed, 480).0);
        }
        // 0.5 振幅 440 Hz 正弦的相邻样本差最多约 0.029；叠加过渡允许略大，但不能出现跳变
        let largest_step = output
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(largest_step < 0.06, "变速切换出现跳变: {largest_step}");
        // 回到 1 倍速后应当原样直通：最后一段输出必须是输入里连续的一段
        let tail = &output[output.len() - 12_000..];
        let found = source
            .samples
            .windows(tail.len())
            .any(|window| window == tail);
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
        assert!(
            (media - position).abs() < 2.0,
            "媒体时间 {media} 应等于实际读到的位置 {position}"
        );
    }

    /// 上游结束时按剩余媒体帧排空，不能为了尾部完整而退回原速
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
        assert!(
            (media - 48_000.0).abs() < 2.0,
            "整段媒体时间应等于输入长度: {media}"
        );
    }

    #[test]
    fn running_out_of_input_returns_a_short_render_instead_of_blocking() {
        let mut source = Source::sine(0.1, 440.0);
        let mut stretcher = Stretcher::new(1, RATE);
        let mut block = vec![0.0f32; 9_600];
        let rendered = stretcher.render(&mut block, 1.2, false, &mut |out| source.pull(out));
        assert!(
            rendered.frames < 9_600 && rendered.frames > 0,
            "输入不足时返回已经生成的部分: {}",
            rendered.frames
        );
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
        assert!(
            (media - 48_000.0).abs() < 2_000.0,
            "整段媒体时间应接近输入长度: {media}"
        );
    }
}
