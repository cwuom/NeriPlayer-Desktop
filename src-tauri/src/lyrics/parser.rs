// LRC / YRC 歌词解析器
use serde::Serialize;
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize)]
pub struct LyricLine {
    pub start_ms: u64,
    pub duration_ms: u64,
    pub text: String,
    pub translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roman: Option<String>,
    pub words: Vec<LyricWord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LyricWord {
    pub start_ms: u64,
    pub duration_ms: u64,
    pub text: String,
}

/// 自动检测格式并解析（对齐 Android parseNeteaseLyricsAuto）
pub fn parse_auto(content: &str) -> Vec<LyricLine> {
    if super::ttml::looks_like(content) {
        return super::ttml::parse(content).unwrap_or_default();
    }
    static YRC_DETECT: OnceLock<Regex> = OnceLock::new();
    let re = YRC_DETECT.get_or_init(|| Regex::new(r"\[\d+,\s*\d+\]\(\d+,").unwrap());
    if re.is_match(content) {
        parse_yrc(content)
    } else {
        parse_lrc(content)
    }
}

fn yrc_word_times_are_relative(
    line_start_ms: u64,
    words: &[LyricWord],
) -> bool {
    let Some(first_word_start) = words.iter().map(|w| w.start_ms).min() else {
        return false;
    };

    // 对齐 Android accompanist normalizeSyllableTimes: 仅以首词起点早于行起点判定相对
    // 时间轴（保留 250ms 容差防浮点抖动）。去掉 last_word_end<=duration+500 上限, 否则
    // 词轨略超行长的相对时间轴（YRC 拖尾常见）会被误判为绝对导致逐字整行跳回曲首（LY-5）
    first_word_start < line_start_ms.saturating_sub(250)
}

/// 归一化 YRC 逐字词间空格（对齐 Android accompanist normalizeSyllableSpacing，LY-4）
///
/// 网易 YRC 数据里部分英文词吞掉了词尾空格（如 `(..)in(..)the` 应为 "in the"），
/// 逐字渲染直接拼接会粘连成 "inthe"；相邻两词首尾均为 ASCII 字母数字且前词未以空白
/// 结尾时，给前词补一个尾空格。
fn normalize_yrc_syllable_spacing(words: &mut [LyricWord]) {
    if words.len() < 2 {
        return;
    }
    for i in 0..words.len() - 1 {
        let prev_last = words[i].text.chars().last();
        let next_first = words[i + 1].text.chars().next();
        if let (Some(a), Some(b)) = (prev_last, next_first) {
            if a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric() {
                words[i].text.push(' ');
            }
        }
    }
}

/// 解析网易云 YRC 逐字歌词
/// 格式：[startMs,durationMs](wordStartMs,wordDurationMs,0)文字...
pub fn parse_yrc(content: &str) -> Vec<LyricLine> {
    static LINE_RE: OnceLock<Regex> = OnceLock::new();
    static WORD_RE: OnceLock<Regex> = OnceLock::new();
    let line_re = LINE_RE.get_or_init(|| Regex::new(r"\[(\d+),\s*(\d+)\](.+)").unwrap());
    let word_re = WORD_RE.get_or_init(|| Regex::new(r"\((\d+),\s*(\d+),\s*[-\d]+\)([^()\n\r]+)").unwrap());

    let mut lines: Vec<LyricLine> = Vec::new();

    for line in content.lines() {
        if let Some(caps) = line_re.captures(line) {
            let start_ms: u64 = caps[1].parse().unwrap_or(0);
            let duration_ms: u64 = caps[2].parse().unwrap_or(0);
            let rest = &caps[3];

            let mut words = Vec::new();
            for wcap in word_re.captures_iter(rest) {
                let ws: u64 = wcap[1].parse().unwrap_or(0);
                let wd: u64 = wcap[2].parse().unwrap_or(0);
                let wt = wcap[3].to_string();
                words.push(LyricWord { start_ms: ws, duration_ms: wd, text: wt });
            }

            // 先补齐吞掉的词尾空格，再由归一化后的逐字拼出整行文本，避免英文粘连
            normalize_yrc_syllable_spacing(&mut words);
            let mut full_text: String = words.iter().map(|w| w.text.as_str()).collect();

            // 无逐字段时保留行文本, 对齐 Android parseNeteaseYrc (混排 YRC 导出走 [start,dur]text)
            if words.is_empty() {
                full_text = rest.trim().to_string();
            }

            if full_text.trim().is_empty() { continue; }

            if yrc_word_times_are_relative(start_ms, &words) {
                for word in &mut words {
                    word.start_ms = start_ms.saturating_add(word.start_ms);
                }
            }

            lines.push(LyricLine {
                start_ms,
                duration_ms,
                text: full_text,
                translation: None,
                roman: None,
                words,
            });
        }
    }

    // 按开始时间稳定排序，与 parse_lrc 及 Android 行为一致（LY-10）：
    // 在野 YRC 偶有乱序时间轴，消费端二分查找当前行依赖有序
    lines.sort_by_key(|line| line.start_ms);

    lines
}

/// 解析标准 LRC 格式
pub fn parse_lrc(content: &str) -> Vec<LyricLine> {
    static TAG_RE: OnceLock<Regex> = OnceLock::new();
    // 只匹配【行首】的单个时间标签：毫秒段可选，分隔符兼容 '.' 与 ':'（网易 legacy [mm:ss:ff]），
    // 分钟 1~2 位（对齐 Android LyricTimestampNormalizer，LY-1）。循环消费可支持压缩多标签行（LY-8）
    let tag_re = TAG_RE
        .get_or_init(|| Regex::new(r"^\[(\d{1,2}):(\d{2})(?:[.:](\d{2,3}))?\]").unwrap());
    // Enhanced LRC 行内音节标签 <mm:ss.xx>，在循环外只编译一次（LY-9）
    static WORD_TAG_RE: OnceLock<Regex> = OnceLock::new();
    let word_tag = WORD_TAG_RE
        .get_or_init(|| Regex::new(r"<\d{1,2}:\d{2}(?:[.:]\d{2,3})?>").unwrap());
    let mut lines: Vec<LyricLine> = Vec::new();

    for line in content.lines() {
        // 逐个吃掉行首连续时间标签，收集全部时间戳（压缩 LRC `[a][b]text` 一行多时间）
        let mut rest = line;
        let mut stamps: Vec<u64> = Vec::new();
        while let Some(caps) = tag_re.captures(rest) {
            let min: u64 = caps[1].parse().unwrap_or(0);
            let sec: u64 = caps[2].parse().unwrap_or(0);
            let ms: u64 = match caps.get(3).map(|m| m.as_str()) {
                Some(ms_str) if ms_str.len() == 2 => ms_str.parse::<u64>().unwrap_or(0) * 10,
                Some(ms_str) => ms_str.parse().unwrap_or(0),
                None => 0,
            };
            stamps.push(min * 60000 + sec * 1000 + ms);
            let consumed = caps.get(0).map(|m| m.end()).unwrap_or(0);
            if consumed == 0 { break; }
            rest = &rest[consumed..];
        }
        if stamps.is_empty() { continue; }
        // 去掉 Enhanced LRC 行内音节标签 <mm:ss.xx>，否则会作为字面文本显示（LY-9）
        let text = word_tag.replace_all(rest.trim(), "").trim().to_string();
        if text.is_empty() { continue; }
        for start_ms in stamps {
            lines.push(LyricLine {
                start_ms,
                duration_ms: 0,
                text: text.clone(),
                translation: None,
                roman: None,
                words: Vec::new(),
            });
        }
    }

    // 先按开始时间稳定排序：在野 LRC 存在乱序时间轴，直接按行序差分会 u64 下溢
    // （debug panic / release 得到天文数字时长）；同刻多行保持原文相对顺序
    lines.sort_by_key(|line| line.start_ms);

    // 计算每行持续时间；saturating_sub 兜底防御排序后仍可能出现的相等时间戳
    for i in 0..lines.len() {
        if i + 1 < lines.len() {
            lines[i].duration_ms = lines[i + 1].start_ms.saturating_sub(lines[i].start_ms);
        } else {
            lines[i].duration_ms = 5000;
        }
    }

    lines
}

/// 判断是否为制作信息行（作词/作曲等署名），翻译匹配时应跳过（对齐 Android isLyricCreditMetadataLine）
fn is_lyric_credit_metadata_line(text: &str) -> bool {
    const CREDIT_KEYWORDS: &[&str] = &[
        "作词", "作曲", "编曲", "制作", "混音", "母带", "和声", "录音",
        "出品", "监制", "配唱", "词：", "曲：",
        "Lyricist", "Composer", "Arranger", "Producer", "Mixing", "Mastering",
    ];
    let has_colon = text.contains('：') || text.contains(':');
    has_colon && CREDIT_KEYWORDS.iter().any(|k| text.contains(k))
}

/// 合并翻译到已有歌词行（对齐 Android LyricTranslationMatcher，450ms 容差）
///
/// 翻译必须向同时间戳组的【组尾正文行】对齐，并跳过制作信息行、一行只接收一条翻译；
/// 否则网易常见的"制作信息行与正文行同一时间戳"场景会让正文翻译被最靠前的元数据行
/// 窃取，导致整体翻译错位一行（LY-3）。
pub fn merge_translation(lines: &mut [LyricLine], translation_lrc: &str) {
    merge_secondary_text(lines, translation_lrc, |line, text| {
        line.translation = Some(text.to_string());
    });
}

/// 合并音译歌词到原文行，使用与翻译相同的时间容差和同刻向后对齐规则
pub fn merge_roman(lines: &mut [LyricLine], roman_lrc: &str) {
    merge_secondary_text(lines, roman_lrc, |line, text| {
        line.roman = Some(text.to_string());
    });
}

fn merge_secondary_text(
    lines: &mut [LyricLine],
    secondary_lrc: &str,
    mut assign: impl FnMut(&mut LyricLine, &str),
) {
    let trans = parse_lrc(secondary_lrc);
    let mut assigned = vec![false; lines.len()];
    for tl in &trans {
        // 候选：容差内、非制作信息行、未被占用；用 (Reverse(Δ), index) 取最大
        // => 时间最接近优先，并列时取 index 最大者（组尾），实现向下对齐
        let best = lines
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                !assigned[*i]
                    && !is_lyric_credit_metadata_line(&l.text)
                    && (l.start_ms as i64 - tl.start_ms as i64).unsigned_abs() < 450
            })
            .max_by_key(|(i, l)| {
                let delta = (l.start_ms as i64 - tl.start_ms as i64).unsigned_abs();
                (std::cmp::Reverse(delta), *i)
            })
            .map(|(i, _)| i);
        if let Some(i) = best {
            assign(&mut lines[i], &tl.text);
            assigned[i] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_roman, merge_translation, parse_auto, parse_lrc, parse_yrc};

    #[test]
    fn parse_lrc_expands_compressed_multi_timestamp_lines() {
        // 压缩 LRC：一行多时间戳共享同一文本，应展开成多行而非把第二个标签当字面文本
        let lines = parse_lrc("[00:01.00][00:05.00]chorus");
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.text == "chorus"));
        assert_eq!(lines[0].start_ms, 1000);
        assert_eq!(lines[1].start_ms, 5000);
    }

    #[test]
    fn translation_aligns_to_body_not_credit_metadata_line() {
        // 制作信息行与正文行同一时间戳：翻译必须落到正文行，而非被最靠前的元数据行窃取
        let mut lines = parse_lrc("[00:01.00]作词：someone\n[00:01.00]Hello world");
        merge_translation(&mut lines, "[00:01.00]你好世界");
        let body = lines.iter().find(|l| l.text == "Hello world").unwrap();
        assert_eq!(body.translation.as_deref(), Some("你好世界"));
        let credit = lines.iter().find(|l| l.text.contains("作词")).unwrap();
        assert_eq!(credit.translation, None);
    }

    #[test]
    fn roman_lyrics_align_to_body_lines() {
        let mut lines = parse_lrc("[00:01.00]作词：someone\n[00:01.00]Hello world");
        merge_roman(&mut lines, "[00:01.02]annai");
        let body = lines.iter().find(|l| l.text == "Hello world").unwrap();
        assert_eq!(body.roman.as_deref(), Some("annai"));
        let credit = lines.iter().find(|l| l.text.contains("作词")).unwrap();
        assert_eq!(credit.roman, None);
    }

    #[test]
    fn parse_lrc_sorts_out_of_order_timestamps_without_underflow() {
        // 乱序时间轴：第二行时间早于第一行，旧实现差分会 u64 下溢
        let lines = parse_lrc("[00:10.00]later\n[00:05.00]earlier\n[00:12.00]last");

        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            vec!["earlier", "later", "last"]
        );
        assert_eq!(lines[0].start_ms, 5000);
        assert_eq!(lines[0].duration_ms, 5000);
        assert_eq!(lines[1].duration_ms, 2000);
        assert_eq!(lines[2].duration_ms, 5000);
    }

    #[test]
    fn parse_lrc_equal_timestamps_do_not_underflow() {
        let lines = parse_lrc("[00:05.00]a\n[00:05.00]b");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].duration_ms, 0);
        assert_eq!(lines[1].duration_ms, 5000);
    }

    #[test]
    fn parse_yrc_normalizes_relative_word_times() {
        let lines = parse_yrc("[10000,2000](0,500,0)你(500,500,0)好");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words[0].start_ms, 10000);
        assert_eq!(lines[0].words[1].start_ms, 10500);
    }

    #[test]
    fn parse_yrc_keeps_absolute_word_times() {
        let lines = parse_yrc("[10000,2000](10000,500,0)你(10500,500,0)好");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words[0].start_ms, 10000);
        assert_eq!(lines[0].words[1].start_ms, 10500);
    }

    #[test]
    fn parse_yrc_keeps_text_only_lines_without_word_segments() {
        let lines = parse_yrc("[12000,3000]世界");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "世界");
        assert!(lines[0].words.is_empty());
        assert_eq!(lines[0].start_ms, 12000);
        assert_eq!(lines[0].duration_ms, 3000);
    }

    #[test]
    fn parse_auto_detects_mixed_yrc_block() {
        let content = "[10000,2000](10000,500,0)你(10500,500,0)好\n[12000,3000]世界";
        let lines = parse_auto(content);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[1].text, "世界");
    }
}
