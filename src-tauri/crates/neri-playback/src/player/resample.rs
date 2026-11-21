//! 声道下混与帧重采样

use super::*;

pub(super) struct FrameResampler {
    source: Box<dyn PcmSource>,
    source_rate: u32,
    current: Vec<f32>,
    next: Vec<f32>,
    phase: f64,
    initialized: bool,
    source_ended: bool,
    finished: bool,
    downmix: Option<Downmix>,
    /// 下混系数是按几路输出算的；0 表示还没算过
    downmix_output: usize,
}

/// 源声道多于输出时的下混系数（按 WAVE 标准声道顺序推断布局）
///
/// 旧做法只取前两个声道：5.1 的中置（对白）、环绕和低频全部丢失。
/// 这里按 ITU-R BS.775 取系数（低频不计入），再整体缩放到每路输出系数和不超过 1。
pub(super) struct Downmix {
    /// 按输出声道排列，每路一组源声道系数
    weights: Vec<f32>,
}

impl Downmix {
    pub(super) fn new(source_channels: usize, output_channels: usize) -> Option<Self> {
        if source_channels <= output_channels.max(2) || !(1..=2).contains(&output_channels) {
            return None;
        }
        const SIDE: f32 = std::f32::consts::FRAC_1_SQRT_2;
        // (左, 右) 系数；顺序与 WAVE/FLAC 的默认声道布局一致
        let roles: &[(f32, f32)] = match source_channels {
            3 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, SIDE)],
            4 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, 0.0), (0.0, SIDE)],
            5 => &[
                (1.0, 0.0),
                (0.0, 1.0),
                (SIDE, SIDE),
                (SIDE, 0.0),
                (0.0, SIDE),
            ],
            6 => &[
                (1.0, 0.0),
                (0.0, 1.0),
                (SIDE, SIDE),
                (0.0, 0.0),
                (SIDE, 0.0),
                (0.0, SIDE),
            ],
            7 => &[
                (1.0, 0.0),
                (0.0, 1.0),
                (SIDE, SIDE),
                (0.0, 0.0),
                (0.5, 0.5),
                (SIDE, 0.0),
                (0.0, SIDE),
            ],
            _ => &[
                (1.0, 0.0),
                (0.0, 1.0),
                (SIDE, SIDE),
                (0.0, 0.0),
                (SIDE, 0.0),
                (0.0, SIDE),
                (SIDE, 0.0),
                (0.0, SIDE),
            ],
        };
        let role = |channel: usize| roles.get(channel).copied().unwrap_or((0.5, 0.5));
        let mut weights = Vec::with_capacity(source_channels * output_channels);
        for output in 0..output_channels {
            for channel in 0..source_channels {
                let (left, right) = role(channel);
                // 单声道取立体声下混的平均，与立体声转单声道的处理一致
                weights.push(match (output_channels, output) {
                    (1, _) => (left + right) * 0.5,
                    (_, 0) => left,
                    _ => right,
                });
            }
        }
        let largest = weights
            .chunks(source_channels)
            .map(|row| row.iter().map(|weight| weight.abs()).sum::<f32>())
            .fold(0.0f32, f32::max);
        if largest > 1.0 {
            weights.iter_mut().for_each(|weight| *weight /= largest);
        }
        Some(Self { weights })
    }

    pub(super) fn sample(&self, frame: &[f32], output_channel: usize) -> f32 {
        let source_channels = frame.len();
        self.weights
            .chunks(source_channels)
            .nth(output_channel)
            .map_or(0.0, |row| {
                row.iter()
                    .zip(frame)
                    .map(|(weight, sample)| weight * sample)
                    .sum()
            })
    }
}

impl FrameResampler {
    pub(super) fn new(source: Box<dyn PcmSource>) -> Self {
        let source_channels = usize::from(source.channels().max(1));
        let source_rate = source.sample_rate().max(1);
        Self {
            source,
            source_rate,
            current: vec![0.0; source_channels],
            next: vec![0.0; source_channels],
            phase: 0.0,
            initialized: false,
            source_ended: false,
            finished: false,
            downmix: None,
            downmix_output: 0,
        }
    }

    pub(super) fn source_channels(&self) -> usize {
        self.current.len()
    }

    pub(super) fn source_mut(&mut self) -> &mut dyn PcmSource {
        self.source.as_mut()
    }

    /// 源被 seek 之后丢掉插值状态，从新位置重新开始
    pub(super) fn restart(&mut self) {
        self.phase = 0.0;
        self.initialized = false;
        self.source_ended = false;
        self.finished = false;
    }

    fn mixed_sample(&self, frame: &[f32], channel: usize, output_channels: usize) -> f32 {
        match &self.downmix {
            Some(downmix) => downmix.sample(frame, channel),
            None => channel_sample(frame, channel, output_channels),
        }
    }

    fn read_source_frame(source: &mut dyn PcmSource, frame: &mut [f32]) -> bool {
        for sample in frame {
            let Some(value) = source.next() else {
                return false;
            };
            *sample = value;
        }
        true
    }

    fn initialize(&mut self) -> bool {
        if self.initialized {
            return !self.finished;
        }
        if !Self::read_source_frame(self.source.as_mut(), &mut self.current) {
            self.finished = true;
            return false;
        }
        if !Self::read_source_frame(self.source.as_mut(), &mut self.next) {
            self.next.copy_from_slice(&self.current);
            self.source_ended = true;
        }
        self.initialized = true;
        true
    }

    /// 只做采样率转换；倍速在输出端由变速器完成，这样 ring 里的内容与倍速无关
    pub(super) fn next_frame(&mut self, output_rate: u32, output: &mut [f32]) -> bool {
        if self.finished || !self.initialize() {
            return false;
        }

        let phase = self.phase as f32;
        let output_channels = output.len();
        if self.downmix_output != output_channels {
            self.downmix_output = output_channels;
            self.downmix = Downmix::new(self.current.len(), output_channels);
        }
        for (channel, sample) in output.iter_mut().enumerate() {
            let current = self.mixed_sample(&self.current, channel, output_channels);
            let next = self.mixed_sample(&self.next, channel, output_channels);
            *sample = current + (next - current) * phase;
        }

        self.phase += f64::from(self.source_rate) / f64::from(output_rate.max(1));
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            if self.source_ended {
                self.finished = true;
                break;
            }
            self.current.copy_from_slice(&self.next);
            if !Self::read_source_frame(self.source.as_mut(), &mut self.next) {
                self.next.copy_from_slice(&self.current);
                self.source_ended = true;
            }
        }
        true
    }
}

/// 输出帧里承载节目内容的前几个声道，响度只按它们统计
///
/// 声道少的源接到声道多的设备上（立体声接 7.1），多出来的声道补的是零；
/// 把它们算进去会把 RMS 拉低，音量均衡就会把每首歌都多抬几 dB。
/// 单声道源复制到了每个声道，统计其中一个就够。
pub(super) fn loudness_channels(source_channels: usize, output_channels: usize) -> usize {
    source_channels.clamp(1, output_channels.max(1))
}

pub(super) fn channel_sample(frame: &[f32], output_channel: usize, output_channels: usize) -> f32 {
    match (frame.len(), output_channels) {
        (1, _) => frame[0],
        (_, 1) => frame.iter().copied().sum::<f32>() / frame.len() as f32,
        _ => frame.get(output_channel).copied().unwrap_or(0.0),
    }
}
