use neri_model::{RepeatMode, TrackInfo};
use rand::seq::SliceRandom;

pub struct PlayQueue {
    pub tracks: Vec<TrackInfo>,
    pub current_index: Option<usize>,
    pub repeat_mode: RepeatMode,
    pub shuffle: bool,
    /// 随机播放时的索引序列
    shuffle_order: Vec<usize>,
    shuffle_pos: usize,
}

impl Default for PlayQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayQueue {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            current_index: None,
            repeat_mode: RepeatMode::Off,
            shuffle: false,
            shuffle_order: Vec::new(),
            shuffle_pos: 0,
        }
    }

    pub fn set_tracks(&mut self, tracks: Vec<TrackInfo>, start_index: usize) {
        self.tracks = tracks;
        self.current_index = Some(start_index);
        if self.shuffle {
            self.rebuild_shuffle();
        }
    }

    pub fn current(&self) -> Option<&TrackInfo> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }

    /// 前进到下一首并返回它
    ///
    /// 名字容易和 Iterator::next 混淆，但这里带播放模式语义（顺序/随机/
    /// 单曲循环），不是迭代器；调用点遍布播放器，改名收益不抵风险。
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&TrackInfo> {
        if self.tracks.is_empty() {
            return None;
        }

        let len = self.tracks.len();
        let idx = self.current_index.unwrap_or(0);

        let next_idx = match self.repeat_mode {
            RepeatMode::One => idx,
            RepeatMode::All => {
                if self.shuffle {
                    self.shuffle_pos = (self.shuffle_pos + 1) % len;
                    self.shuffle_order[self.shuffle_pos]
                } else {
                    (idx + 1) % len
                }
            }
            RepeatMode::Off => {
                if self.shuffle {
                    if self.shuffle_pos + 1 >= len {
                        return None;
                    }
                    self.shuffle_pos += 1;
                    self.shuffle_order[self.shuffle_pos]
                } else if idx + 1 >= len {
                    return None;
                } else {
                    idx + 1
                }
            }
        };

        self.current_index = Some(next_idx);
        self.tracks.get(next_idx)
    }

    pub fn prev(&mut self) -> Option<&TrackInfo> {
        if self.tracks.is_empty() {
            return None;
        }

        let len = self.tracks.len();
        let idx = self.current_index.unwrap_or(0);

        let prev_idx = if self.shuffle {
            if self.shuffle_pos == 0 {
                self.shuffle_order[0]
            } else {
                self.shuffle_pos -= 1;
                self.shuffle_order[self.shuffle_pos]
            }
        } else if idx == 0 {
            if self.repeat_mode == RepeatMode::All {
                len - 1
            } else {
                0
            }
        } else {
            idx - 1
        };

        self.current_index = Some(prev_idx);
        self.tracks.get(prev_idx)
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.rebuild_shuffle();
        }
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat_mode = match self.repeat_mode {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        self.repeat_mode
    }

    fn rebuild_shuffle(&mut self) {
        let mut rng = rand::thread_rng();
        self.shuffle_order = (0..self.tracks.len()).collect();
        self.shuffle_order.shuffle(&mut rng);
        // 把当前曲目放到首位
        if let Some(idx) = self.current_index {
            if let Some(pos) = self.shuffle_order.iter().position(|&i| i == idx) {
                self.shuffle_order.swap(0, pos);
            }
        }
        self.shuffle_pos = 0;
    }
}
