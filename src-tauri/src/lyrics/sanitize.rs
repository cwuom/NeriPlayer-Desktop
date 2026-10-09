// 去掉匹配来的歌词里的「牛皮藓」：制作信息行、歌名歌手标题行等非歌词内容
// 规则对齐 Android EditableLyricSanitizer（sanitizeMatchedEditableLyrics），按已解析的歌词行处理
use regex::Regex;
use std::sync::OnceLock;

use super::matcher::normalize_text;
use super::parser::LyricLine;

const LEADING_METADATA_SCAN_LINES: usize = 8;
const TRAILING_METADATA_SCAN_LINES: usize = 4;
const SHORT_IDENTITY_LINE_DURATION_MS: u64 = 2_500;
const EDGE_IDENTITY_LINE_START_MS: u64 = 12_000;

const EDGE_CREDIT_PREFIXES: &[&str] = &[
    "lyrics by", "lyric by", "written by", "composed by", "arranged by", "produced by",
    "performed by", "music by", "words by",
    "作词", "作詞", "填词", "填詞", "作曲", "编曲", "編曲", "制作", "制作人", "演唱", "歌手", "词曲", "詞曲",
];

fn english_credit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:lyrics?|words?|written|composed|arranged|produced|performed|sung|music)\s+by\s*[:：-]?\s*.+$|^\s*(?:lyrics?|composer|lyricist|producer|arranger|vocal(?:s)?|artist|singer|title|album)\s*[:：-]\s*.+$",
        )
        .unwrap()
    })
}

fn chinese_credit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:作词|作詞|填词|填詞|作曲|编曲|編曲|制作人?|演唱|歌手|词曲|詞曲|词|詞|曲|和声|和聲|混音|母带|母帶|录音|錄音|监制|監製|原唱|翻唱|出品|OP|SP)\s*[:：/／-]\s*.+$",
        )
        .unwrap()
    })
}

fn artist_separator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)[/,，、&+]|\b(?:feat\.?|ft\.?|featuring)\b|\s+[xX]\s+").unwrap())
}

struct SanitizeContext {
    title: String,
    artist_terms: Vec<String>,
    album: String,
}

fn artist_terms(artist: &str) -> Vec<String> {
    let mut terms = vec![normalize_text(artist)];
    terms.extend(artist_separator_re().split(artist).map(normalize_text));
    let mut unique = Vec::new();
    for term in terms.into_iter().filter(|term| !term.is_empty()) {
        if !unique.contains(&term) {
            unique.push(term);
        }
    }
    unique
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    if text.is_empty() || phrase.is_empty() {
        return false;
    }
    if text == phrase
        || text.starts_with(&format!("{phrase} "))
        || text.ends_with(&format!(" {phrase}"))
        || text.contains(&format!(" {phrase} "))
    {
        return true;
    }
    !phrase.is_ascii() && text.contains(phrase)
}

fn is_credit_line(text: &str, is_edge: bool) -> bool {
    let trimmed = text.trim();
    if english_credit_re().is_match(trimmed) || chinese_credit_re().is_match(trimmed) {
        return true;
    }
    if !is_edge {
        return false;
    }
    let normalized = normalize_text(trimmed);
    EDGE_CREDIT_PREFIXES.iter().any(|prefix| {
        let prefix = normalize_text(prefix);
        normalized == prefix || normalized.starts_with(&format!("{prefix} "))
    })
}

fn is_song_identity_line(line: &LyricLine, context: &SanitizeContext) -> bool {
    let normalized = normalize_text(&line.text);
    if normalized.is_empty() {
        return false;
    }
    let has_title = contains_phrase(&normalized, &context.title);
    let has_artist = context.artist_terms.iter().any(|term| contains_phrase(&normalized, term));
    let has_album = contains_phrase(&normalized, &context.album);
    if has_title && (has_artist || has_album) {
        return true;
    }
    line.duration_ms <= SHORT_IDENTITY_LINE_DURATION_MS
        && line.start_ms <= EDGE_IDENTITY_LINE_START_MS
        && normalized == context.title
}

fn should_remove(line: &LyricLine, index: usize, count: usize, context: &SanitizeContext) -> bool {
    if line.text.trim().is_empty() {
        return false;
    }
    let is_edge = index < LEADING_METADATA_SCAN_LINES || index + TRAILING_METADATA_SCAN_LINES >= count;
    if is_credit_line(&line.text, is_edge) {
        return true;
    }
    is_edge && is_song_identity_line(line, context)
}

/// 去掉制作信息行和开头/结尾的歌名歌手标题行；去完什么都不剩时原样返回
pub fn sanitize_matched_lines(lines: Vec<LyricLine>, title: &str, artist: &str, album: &str) -> Vec<LyricLine> {
    let context = SanitizeContext {
        title: normalize_text(title),
        artist_terms: artist_terms(artist),
        album: normalize_text(album),
    };
    let count = lines.len();
    let keep: Vec<bool> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| !should_remove(line, index, count, &context))
        .collect();
    let removed = keep.iter().filter(|kept| !**kept).count();
    let has_lyric_left = lines
        .iter()
        .zip(&keep)
        .any(|(line, kept)| *kept && !line.text.trim().is_empty());
    if removed == 0 || !has_lyric_left {
        return lines;
    }
    lines.into_iter().zip(keep).filter_map(|(line, kept)| kept.then_some(line)).collect()
}

#[cfg(test)]
mod tests {
    use super::sanitize_matched_lines;
    use crate::lyrics::parser::parse_auto;

    fn texts(lines: &[crate::lyrics::parser::LyricLine]) -> Vec<String> {
        lines.iter().map(|line| line.text.clone()).collect()
    }

    #[test]
    fn credit_and_title_lines_are_removed_from_netease_yrc() {
        let yrc = "[0,7010](0,876,0)周(876,876,0)杰(1752,876,0)伦 (2628,876,0)- (3504,876,0)开(4380,876,0)不(5256,876,0)了(6132,876,0)口\n\
            [7010,7010](7010,1402,0)词(8412,1402,0)：(9814,1402,0)徐(11216,1402,0)若(12618,1402,0)瑄\n\
            [14020,7010](14020,1402,0)曲(15422,1402,0)：(16824,1402,0)周(18226,1402,0)杰(19628,1402,0)伦\n\
            [21030,7100](21030,1168,0)编(22198,1168,0)曲(23366,1168,0)：(24534,1168,0)洪(25702,1168,0)敬(26870,1260,0)尧\n\
            [28130,2957](28130,319,0)才(28449,392,0)离(28841,389,0)开\n\
            [31353,3041](31353,275,0)担(31628,215,0)心";
        let lines = sanitize_matched_lines(parse_auto(yrc), "开不了口", "周杰伦", "范特西");
        assert_eq!(texts(&lines), vec!["才离开", "担心"]);
    }

    #[test]
    fn credit_lines_in_android_synced_yrc_are_removed() {
        // Android 匹配后经同步存下的《Sincerely》：开头的制作信息被导出成零时长 YRC 行
        let yrc = "[0,0](0,0,0) 作词 : TRUE\n[0,0](0,0,0) 作曲 : 堀江晶太\n[0,0](0,0,0) 编曲 : 堀江晶太/Evan Call\n\
            [0,1150](0,115,0)编(115,115,0)曲 (230,115,0): (345,115,0)堀(460,115,0)江\n\
            [1150,6150](1150,60,0)知(1210,270,0)ら\n\
            [7460,6190](7460,180,0)お(7640,470,0)も";
        let lines = sanitize_matched_lines(parse_auto(yrc), "Sincerely", "TRUE", "");
        assert_eq!(texts(&lines), vec!["知ら", "おも"]);
    }

    #[test]
    fn lyric_lines_that_mention_the_title_mid_song_are_kept() {
        let lrc = (0..20)
            .map(|i| format!("[00:{:02}.00]{}", i * 2, if i == 10 { "开不了口 周杰伦" } else { "歌词" }))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = sanitize_matched_lines(parse_auto(&lrc), "开不了口", "周杰伦", "");
        assert_eq!(lines.len(), 20, "中段提到歌名歌手的是正文，不能删");
    }

    #[test]
    fn nothing_left_keeps_the_original() {
        let lines = sanitize_matched_lines(parse_auto("[00:01.00]作词：某人"), "x", "y", "");
        assert_eq!(texts(&lines), vec!["作词：某人"]);
    }

    #[test]
    fn english_credits_are_removed() {
        let lines = sanitize_matched_lines(
            parse_auto("[00:00.00]Lyrics by: Someone\n[00:03.00]Hello\n[00:05.00]Composer: Other\n[00:07.00]World"),
            "Song",
            "Artist",
            "",
        );
        assert_eq!(texts(&lines), vec!["Hello", "World"]);
    }
}
