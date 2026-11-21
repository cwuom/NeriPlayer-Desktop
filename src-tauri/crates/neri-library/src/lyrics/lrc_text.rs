//! 把歌词行写成 LRC 文本（下载旁挂 .lrc 与元数据共用）

use super::parser::LyricLine;

pub fn make_lrc_timestamp(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let centis = (ms % 1000) / 10;
    format!("{:02}:{:02}.{:02}", minutes, seconds, centis)
}

pub fn build_lrc_text(lines: &[LyricLine]) -> Option<String> {
    let mut out = String::new();
    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        out.push('[');
        out.push_str(&make_lrc_timestamp(line.start_ms));
        out.push(']');
        out.push_str(text);
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn build_translation_lrc_text(lines: &[LyricLine]) -> Option<String> {
    let mut out = String::new();
    for line in lines {
        let Some(translated) = line.translation.as_deref() else {
            continue;
        };
        let text = translated.trim();
        if text.is_empty() {
            continue;
        }
        out.push('[');
        out.push_str(&make_lrc_timestamp(line.start_ms));
        out.push(']');
        out.push_str(text);
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}
