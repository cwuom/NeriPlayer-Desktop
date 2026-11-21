use std::time::Duration;

/// 解码后的交错 PCM 数据源，不绑定具体音频输出框架
///
/// 使用 f32 保留 24-bit PCM 的有效精度，避免在 DSP 和重采样前降为 16-bit
pub trait PcmSource: Iterator<Item = f32> + Send {
    fn channels(&self) -> u16;
    fn sample_rate(&self) -> u32;
    fn total_duration(&self) -> Option<Duration>;
    fn try_seek(&mut self, position: Duration) -> Result<(), PcmSeekError>;
}

impl<T> PcmSource for Box<T>
where
    T: PcmSource + ?Sized,
{
    fn channels(&self) -> u16 {
        (**self).channels()
    }

    fn sample_rate(&self) -> u32 {
        (**self).sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        (**self).total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), PcmSeekError> {
        (**self).try_seek(position)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmSeekError {
    message: String,
}

impl PcmSeekError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PcmSeekError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PcmSeekError {}
