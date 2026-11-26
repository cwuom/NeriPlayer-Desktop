//! 缩略图工具栏的按钮模型：与平台无关，Win32 层与单测共用

use super::glyph::Glyph;

/// 系统 THUMBBUTTON::szTip 的容量（含结尾 0）
pub const TOOLTIP_CAPACITY: usize = 260;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Previous,
    Toggle,
    Next,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Previous, Action::Toggle, Action::Next];

    /// 按钮 id，WM_COMMAND 的低位字带回来
    pub fn id(self) -> u32 {
        match self {
            Action::Previous => 1,
            Action::Toggle => 2,
            Action::Next => 3,
        }
    }

    pub fn from_id(id: u32) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.id() == id)
    }

    /// 与托盘菜单相同的事件，主窗口里已有监听
    pub fn event(self) -> &'static str {
        match self {
            Action::Previous => "media:previous",
            Action::Toggle => "media:toggle",
            Action::Next => "media:next",
        }
    }
}

/// 托盘那边已经按当前语言翻译好的按钮文案与播放状态
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThumbbarState {
    pub has_track: bool,
    pub is_playing: bool,
    pub previous: String,
    pub toggle: String,
    pub next: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonSpec {
    pub action: Action,
    pub glyph: Glyph,
    pub tooltip: String,
    pub enabled: bool,
}

/// 三个按钮的外观：播放中显示暂停图标；没有曲目时全部置灰，免得点了没反应
pub fn buttons(state: &ThumbbarState) -> [ButtonSpec; 3] {
    let spec = |action, glyph, tooltip: &str| ButtonSpec {
        action,
        glyph,
        tooltip: tooltip.to_string(),
        enabled: state.has_track,
    };
    [
        spec(Action::Previous, Glyph::Previous, &state.previous),
        spec(
            Action::Toggle,
            if state.is_playing {
                Glyph::Pause
            } else {
                Glyph::Play
            },
            &state.toggle,
        ),
        spec(Action::Next, Glyph::Next, &state.next),
    ]
}

/// 编码进定长 UTF-16 缓冲：超长时在字符边界截断，不留下半个代理对，末尾保证有 0
pub fn encode_tooltip(text: &str) -> [u16; TOOLTIP_CAPACITY] {
    let mut buffer = [0_u16; TOOLTIP_CAPACITY];
    let mut length = 0;
    for character in text.chars() {
        let mut units = [0_u16; 2];
        let encoded = character.encode_utf16(&mut units);
        if length + encoded.len() > TOOLTIP_CAPACITY - 1 {
            break;
        }
        buffer[length..length + encoded.len()].copy_from_slice(encoded);
        length += encoded.len();
    }
    buffer
}

/// 任务栏是浅色时用深色图标，深色任务栏用浅色图标（与系统自带按钮一致）
pub fn glyph_color(light_taskbar: bool) -> [u8; 3] {
    if light_taskbar {
        [31, 31, 31]
    } else {
        [255, 255, 255]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(has_track: bool, is_playing: bool) -> ThumbbarState {
        ThumbbarState {
            has_track,
            is_playing,
            previous: "上一首".into(),
            toggle: if is_playing { "暂停" } else { "播放" }.into(),
            next: "下一首".into(),
        }
    }

    #[test]
    fn round_trips_button_ids_and_maps_them_to_tray_events() {
        for action in Action::ALL {
            assert_eq!(Action::from_id(action.id()), Some(action));
        }
        assert_eq!(Action::from_id(0), None);
        assert_eq!(Action::from_id(4), None);
        let events: Vec<_> = Action::ALL.into_iter().map(Action::event).collect();
        assert_eq!(events, ["media:previous", "media:toggle", "media:next"]);
    }

    #[test]
    fn shows_pause_while_playing_and_play_while_paused() {
        assert_eq!(buttons(&state(true, true))[1].glyph, Glyph::Pause);
        assert_eq!(buttons(&state(true, false))[1].glyph, Glyph::Play);
        assert_eq!(buttons(&state(true, true))[1].tooltip, "暂停");
    }

    #[test]
    fn keeps_previous_and_next_glyphs_and_order_fixed() {
        let specs = buttons(&state(true, false));
        assert_eq!(specs.clone().map(|spec| spec.action), Action::ALL);
        assert_eq!(specs[0].glyph, Glyph::Previous);
        assert_eq!(specs[2].glyph, Glyph::Next);
    }

    #[test]
    fn disables_every_button_without_a_track() {
        assert!(buttons(&state(false, false))
            .iter()
            .all(|spec| !spec.enabled));
        assert!(buttons(&state(true, false)).iter().all(|spec| spec.enabled));
    }

    #[test]
    fn encodes_tooltips_as_nul_terminated_utf16() {
        let encoded = encode_tooltip("下一首");
        assert_eq!(&encoded[..4], &[0x4e0b, 0x4e00, 0x9996, 0]);
    }

    #[test]
    fn truncates_long_tooltips_without_splitting_surrogate_pairs() {
        let encoded = encode_tooltip(&"🎵".repeat(200));
        let used = encoded.iter().position(|&unit| unit == 0).unwrap();
        assert_eq!(used, 258, "129 emoji fit before the terminator");
        assert!(String::from_utf16(&encoded[..used]).is_ok());
        assert_eq!(encoded[TOOLTIP_CAPACITY - 1], 0);
    }

    #[test]
    fn uses_dark_ink_on_a_light_taskbar() {
        assert_eq!(glyph_color(true), [31, 31, 31]);
        assert_eq!(glyph_color(false), [255, 255, 255]);
    }
}
