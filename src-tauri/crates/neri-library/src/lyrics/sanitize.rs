// 去掉匹配来的歌词里的「牛皮藓」：制作信息行、歌名歌手标题行等非歌词内容
// 按已解析的歌词行处理，正文过滤与副轨对齐共用规则
use regex::Regex;
use std::sync::OnceLock;

use super::matcher::normalize_text;
use super::parser::LyricLine;

const LEADING_METADATA_SCAN_LINES: usize = 8;
const TRAILING_METADATA_SCAN_LINES: usize = 4;

const CREDIT_ROLES: &[&str] = &[
    "作词",
    "作詞",
    "填词",
    "填詞",
    "作曲",
    "词曲",
    "詞曲",
    "编曲",
    "編曲",
    "制作人",
    "製作人",
    "制作",
    "製作",
    "出品",
    "出品人",
    "联合出品",
    "聯合出品",
    "营销",
    "營銷",
    "策划",
    "策劃",
    "企划",
    "企劃",
    "监制",
    "監製",
    "统筹",
    "統籌",
    "发行",
    "發行",
    "宣传",
    "宣傳",
    "推广",
    "推廣",
    "宣传推广",
    "宣傳推廣",
    "宣发",
    "宣發",
    "混音",
    "母带",
    "母帶",
    "录音",
    "錄音",
    "和声",
    "和聲",
    "和音",
    "配唱",
    "演唱",
    "歌手",
    "原唱",
    "翻唱",
    "词",
    "詞",
    "曲",
    "吉他",
    "电吉他",
    "電吉他",
    "木吉他",
    "贝斯",
    "貝斯",
    "鼓",
    "键盘",
    "鍵盤",
    "钢琴",
    "鋼琴",
    "弦乐",
    "弦樂",
    "弦乐编写",
    "弦樂編寫",
    "弦乐编曲",
    "弦樂編曲",
    "和声编写",
    "和聲編寫",
    "和声编唱",
    "和聲編唱",
    "和声演唱",
    "和聲演唱",
    "和声设计",
    "和聲設計",
    "和声编配",
    "和聲編配",
    "口哨",
    "midi程序",
    "midi编程",
    "midi編程",
    "录音师",
    "錄音師",
    "混音师",
    "混音師",
    "母带工程师",
    "母帶工程師",
    "录音室",
    "錄音室",
    "录音棚",
    "錄音棚",
    "混音室",
    "母带后期",
    "母帶後期",
    "人声编辑",
    "人聲編輯",
    "音频编辑",
    "音頻編輯",
    "配唱制作人",
    "配唱製作人",
    "音乐制作",
    "音樂製作",
    "音乐总监",
    "音樂總監",
    "制作公司",
    "製作公司",
    "版权",
    "版權",
    "版权所有",
    "版權所有",
    "鸣谢",
    "鳴謝",
    "特别鸣谢",
    "特別鳴謝",
    "封面设计",
    "封面設計",
    "文案",
    "op",
    "sp",
    "lyric",
    "lyrics",
    "word",
    "words",
    "music",
    "lyricist",
    "composer",
    "arranger",
    "producer",
    "co-producer",
    "mix",
    "mixing",
    "mixengineer",
    "mixingengineer",
    "mastering",
    "masteringengineer",
    "recording",
    "recordingengineer",
    "vocal",
    "vocals",
    "vocalist",
    "vocalproduction",
    "vocalproducer",
    "vocalarrangement",
    "vocalediting",
    "backingvocals",
    "backgroundvocals",
    "harmony",
    "harmonies",
    "guitar",
    "guitars",
    "acousticguitar",
    "electricguitar",
    "bass",
    "drum",
    "drums",
    "keyboard",
    "keyboards",
    "piano",
    "strings",
    "stringarrangement",
    "midiprogramming",
    "programming",
    "artist",
    "singer",
    "title",
    "album",
    "copyright",
    "publisher",
    "label",
    "distribution",
    "promotion",
    "credits",
    "by",
    "lyricby",
    "lyricsby",
    "wordby",
    "wordsby",
    "writtenby",
    "composedby",
    "arrangedby",
    "producedby",
    "performedby",
    "sungby",
    "musicby",
    "mixedby",
    "masteredby",
    "recordedby",
    "publishedby",
    "distributedby",
];

const AMBIGUOUS_CREDIT_ROLES: &[&str] = &[
    "词",
    "詞",
    "曲",
    "鼓",
    "吉他",
    "贝斯",
    "貝斯",
    "键盘",
    "鍵盤",
    "弦乐",
    "弦樂",
    "和声",
    "和聲",
    "和音",
    "口哨",
    "lyric",
    "lyrics",
    "word",
    "words",
    "music",
    "vocal",
    "vocals",
    "guitar",
    "guitars",
    "bass",
    "drum",
    "drums",
    "keyboard",
    "keyboards",
    "strings",
];

fn contributor_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"^(?:[\p{Han}]{2,4}|@\S+|[\p{Lu}\p{N}][\p{L}\p{N}.'_-]*(?:\s+(?:[\p{Lu}\p{N}][\p{L}\p{N}.'_-]*|de|van|von|da|del)){0,5})$",
    ).unwrap())
}

fn contributor_separator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\s+and\s+|[/／&、,，+;；]").unwrap())
}

fn is_contributor_list(value: &str) -> bool {
    static SENTENCE_PREFIX: OnceLock<Regex> = OnceLock::new();
    let sentence_prefix = SENTENCE_PREFIX.get_or_init(|| Regex::new(
        r"(?i)^(?:i|you|he|she|it|we|they|me|us|my|your|his|her|its|our|their|a|an|the|this|that|these|those)\b|^(?:我|你|他|她|它|想|爱|愛|在|是|不|别|別|没|沒)",
    ).unwrap());
    let value = value.trim_end_matches(|c: char| c.is_whitespace() || "/／&、,，+;；".contains(c));
    contributor_separator_re().split(value).all(|name| {
        let name = name.trim();
        let name = name.split(['(', '（']).next().unwrap_or("").trim();
        let name = if name.starts_with('@') {
            name
        } else {
            name.split('@').next().unwrap_or("").trim()
        };
        !sentence_prefix.is_match(name) && contributor_name_re().is_match(name)
    })
}

fn normalize_role(role: &str) -> String {
    role.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn is_labelled_credit(role: &str, value: &str, has_context: bool) -> bool {
    if value.trim().is_empty() {
        return false;
    }
    role.split(['/', '／', '&', '、', ',', '，', '+'])
        .all(|role| {
            let role = normalize_role(role);
            CREDIT_ROLES.contains(&role.as_str())
                && (has_context || !["词", "詞", "曲"].contains(&role.as_str()))
                && (!(AMBIGUOUS_CREDIT_ROLES.contains(&role.as_str()) || role.ends_with("by"))
                    || is_contributor_list(value))
        })
}

/// 正文过滤和副轨对齐共用角色规则，含糊的短标签还需符合署名格式
pub(super) fn is_lyric_credit_metadata_line(text: &str) -> bool {
    let text = text.trim();
    if let Some((role, value)) = text.split_once([':', '：']) {
        return is_labelled_credit(role, value, false);
    }
    static SEPARATED: OnceLock<Regex> = OnceLock::new();
    let separated = SEPARATED.get_or_init(|| Regex::new(
        r"(?i)^\s*(作词|作詞|填词|填詞|作曲|编曲|編曲|制作人|製作人|producer|composer|arranger|lyricist)\s*[-/／]\s*(\S.*?)\s*$",
    ).unwrap());
    if separated.is_match(text) {
        return true;
    }
    false
}

fn is_possible_contextual_credit(text: &str) -> bool {
    let text = text.trim();
    if let Some((role, value)) = text.split_once([':', '：']) {
        return is_labelled_credit(role, value, true);
    }
    static ATTRIBUTION: OnceLock<Regex> = OnceLock::new();
    let attribution = ATTRIBUTION.get_or_init(|| Regex::new(
        r"(?i)^\s*((?:lyrics?|words?|written|composed|arranged|produced|performed|sung|music|mixed|mastered|recorded|published|distributed)\s+by)\s+(\S.*?)\s*$",
    ).unwrap());
    attribution.captures(text).is_some_and(|caps| {
        let name = caps[2].trim();
        is_contributor_list(name)
    })
}

/// 短标签和无分隔符的署名只在连续的制作信息块内过滤
pub(super) fn lyric_credit_metadata_mask(texts: &[&str]) -> Vec<bool> {
    let mut credits: Vec<bool> = texts
        .iter()
        .map(|text| is_lyric_credit_metadata_line(text))
        .collect();
    let possible: Vec<bool> = texts
        .iter()
        .zip(&credits)
        .map(|(text, credit)| *credit || is_possible_contextual_credit(text))
        .collect();
    let mut start = 0;
    while start < texts.len() {
        if !possible[start] {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < texts.len() && possible[end] {
            end += 1;
        }
        if credits[start..end].iter().any(|credit| *credit) {
            credits[start..end].fill(true);
        }
        start = end;
    }
    credits
}

fn artist_separator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[/,，、&+]|\b(?:feat\.?|ft\.?|featuring)\b|\s+[xX]\s+").unwrap()
    })
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

fn title_is_missing_internal_function_word(title: &str, candidate: &str) -> bool {
    let title_words: Vec<&str> = title.split_whitespace().collect();
    let candidate_words: Vec<&str> = candidate.split_whitespace().collect();
    if title_words.len() < 4
        || title_words.len() != candidate_words.len() + 1
        || !title_words
            .iter()
            .all(|word| word.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return false;
    }
    (1..title_words.len() - 1).any(|index| {
        matches!(
            title_words[index],
            "a" | "an" | "the" | "in" | "on" | "at" | "of" | "to" | "for" | "and"
        ) && title_words
            .iter()
            .enumerate()
            .filter_map(|(word_index, word)| (word_index != index).then_some(*word))
            .eq(candidate_words.iter().copied())
    })
}

fn is_song_identity_line(line: &LyricLine, context: &SanitizeContext) -> bool {
    if context.title.is_empty() || !line.text.contains(['-', '－', '–', '—', '/', '／', '|'])
    {
        return false;
    }
    // 平台标题偶尔漏掉介词，仅在分隔符另一侧精确匹配歌手时放宽
    line.text
        .char_indices()
        .filter(|(_, c)| matches!(c, '-' | '－' | '–' | '—' | '/' | '／' | '|'))
        .any(|(index, c)| {
            let left = normalize_text(&line.text[..index]);
            let right = normalize_text(&line.text[index + c.len_utf8()..]);
            if context
                .artist_terms
                .iter()
                .chain(std::iter::once(&context.album))
                .filter(|term| !term.is_empty() && **term != context.title)
                .any(|term| {
                    (left == context.title && right == *term)
                        || (right == context.title && left == *term)
                })
            {
                return true;
            }
            context
                .artist_terms
                .iter()
                .filter(|term| !term.is_empty() && **term != context.title)
                .any(|term| {
                    (left == *term
                        && title_is_missing_internal_function_word(&context.title, &right))
                        || (right == *term
                            && title_is_missing_internal_function_word(&context.title, &left))
                })
        })
}

fn should_remove(line: &LyricLine, index: usize, count: usize, context: &SanitizeContext) -> bool {
    if line.text.trim().is_empty() {
        return false;
    }
    let is_edge =
        index < LEADING_METADATA_SCAN_LINES || index + TRAILING_METADATA_SCAN_LINES >= count;
    is_edge && is_song_identity_line(line, context)
}

/// 明确的制作信息可全部移除，身份行删完没有正文时保留以免误伤
pub fn sanitize_matched_lines(
    lines: Vec<LyricLine>,
    title: &str,
    artist: &str,
    album: &str,
) -> Vec<LyricLine> {
    let credits = lyric_credit_metadata_mask(
        &lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
    );
    let translation_credits = lyric_credit_metadata_mask(
        &lines
            .iter()
            .map(|line| line.translation.as_deref().unwrap_or(""))
            .collect::<Vec<_>>(),
    );
    let roman_credits = lyric_credit_metadata_mask(
        &lines
            .iter()
            .map(|line| line.roman.as_deref().unwrap_or(""))
            .collect::<Vec<_>>(),
    );
    let lines: Vec<LyricLine> = lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut line)| {
            if credits[index] {
                return None;
            }
            if translation_credits[index] {
                line.translation = None;
            }
            if roman_credits[index] {
                line.roman = None;
            }
            Some(line)
        })
        .collect();
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
    lines
        .into_iter()
        .zip(keep)
        .filter_map(|(line, kept)| kept.then_some(line))
        .collect()
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
            .map(|i| {
                format!(
                    "[00:{:02}.00]{}",
                    i * 2,
                    if i == 10 {
                        "开不了口 周杰伦"
                    } else {
                        "歌词"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let lines = sanitize_matched_lines(parse_auto(&lrc), "开不了口", "周杰伦", "");
        assert_eq!(lines.len(), 20, "中段提到歌名歌手的是正文，不能删");
    }

    #[test]
    fn explicit_credits_without_lyrics_leave_no_display_lines() {
        let lines = sanitize_matched_lines(parse_auto("[00:01.00]作词：某人"), "x", "y", "");
        assert!(lines.is_empty());
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

    #[test]
    fn head_in_the_clouds_screenshot_metadata_is_removed() {
        let mut lines = parse_auto(
            "[12000,3000](12000,1500,0)I miss the days when (13500,1500,0)life was so simple\n\
            [15000,2000](15000,1000,0)Felt like (16000,1000,0)the glass",
        );
        let mut metadata = parse_auto("[00:00.00]Hayd - Head The Clouds\n[00:04.00]by: Robert Hubers、\n[00:08.00]by: Robert Hubers");
        metadata.append(&mut lines);
        let verses = metadata[3..].to_vec();
        let result = sanitize_matched_lines(metadata, "Head In The Clouds", "Hayd", "Changes EP");
        assert_eq!(
            texts(&result),
            vec![
                "I miss the days when life was so simple",
                "Felt like the glass"
            ]
        );
        assert_eq!(result[0].start_ms, verses[0].start_ms);
        assert_eq!(result[0].duration_ms, verses[0].duration_ms);
        assert_eq!(result[0].words[1].start_ms, verses[0].words[1].start_ms);
        assert_eq!(
            result[0].words[1].duration_ms,
            verses[0].words[1].duration_ms
        );
    }

    #[test]
    fn an_internal_function_word_may_be_missing_from_an_artist_title_header() {
        for header in ["Hayd - Head The Clouds", "Head The Clouds / Hayd"] {
            let lrc = format!("[00:00.00]{header}\n[00:12.00]I miss the days");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Head In The Clouds",
                    "Hayd",
                    ""
                )),
                vec!["I miss the days"],
                "未过滤少介词的标题行：{header}"
            );
        }
    }

    #[test]
    fn exact_title_identity_requires_a_separator_between_title_and_artist() {
        for text in ["Hayd Head-In The Clouds", "Hayd Head In The Clouds-"] {
            let lrc = format!("[00:00.00]{text}\n[00:12.00]I miss the days");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Head In The Clouds",
                    "Hayd",
                    "Changes EP"
                )),
                vec![text, "I miss the days"],
                "歌名内部或行尾分隔符不能变成身份行：{text}"
            );
        }
        for header in [
            "Hayd - Head In The Clouds",
            "Head In The Clouds / Hayd",
            "Changes EP - Head In The Clouds",
            "Head In The Clouds | Changes EP",
        ] {
            let lrc = format!("[00:00.00]{header}\n[00:12.00]I miss the days");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Head In The Clouds",
                    "Hayd",
                    "Changes EP"
                )),
                vec!["I miss the days"],
                "明确歌名歌手或专辑标题行应过滤：{header}"
            );
        }
    }

    #[test]
    fn title_header_tolerance_requires_an_exact_artist_and_only_one_internal_function_word() {
        for header in [
            "Hayd - Head Other Clouds",
            "Hayd - Head Clouds",
            "Hayd - In The Clouds",
            "Someone - Head The Clouds",
            "Hayden - Head The Clouds",
            "Hayd - Head In The",
            "Head The Clouds - Changes EP",
            "Hayd Head The Clouds",
            "Hayd Head-The Clouds",
        ] {
            let lrc = format!("[00:00.00]{header}\n[00:12.00]I miss the days");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Head In The Clouds",
                    "Hayd",
                    "Changes EP"
                )),
                vec![header, "I miss the days"],
                "不应过滤不满足标题规则的正文：{header}"
            );
        }
        for (title, header) in [
            ("Head In Clouds", "Hayd - Head Clouds"),
            ("The Head Of Clouds", "Hayd - Head Of Clouds"),
            ("Head In Clouds Of", "Hayd - Head In Clouds"),
        ] {
            let lrc = format!("[00:00.00]{header}\n[00:12.00]I miss the days");
            assert_eq!(
                texts(&sanitize_matched_lines(parse_auto(&lrc), title, "Hayd", "")),
                vec![header, "I miss the days"],
                "短标题和首尾词不能放宽：{header}"
            );
        }
        let lrc = (0..20)
            .map(|index| {
                format!(
                    "[00:{:02}.00]{}",
                    index * 2,
                    if index == 10 {
                        "Hayd - Head The Clouds"
                    } else {
                        "正文"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            sanitize_matched_lines(parse_auto(&lrc), "Head In The Clouds", "Hayd", "").len(),
            20
        );
    }

    #[test]
    fn multilingual_credits_are_removed_without_damaging_verses() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/credit-lines.json")).unwrap();
        for value in fixtures["credits"].as_array().unwrap() {
            let text = value.as_str().unwrap();
            let lrc = format!("[00:01.00]{text}\n[00:03.00]真正的歌词");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Song",
                    "Artist",
                    ""
                )),
                vec!["真正的歌词"],
                "未过滤署名：{text}"
            );
        }
        for value in fixtures["lyrics"].as_array().unwrap() {
            let text = value.as_str().unwrap();
            if text.is_empty() {
                continue;
            }
            let lrc = format!("[00:01.00]{text}\n[00:03.00]真正的歌词");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Song",
                    "Artist",
                    ""
                )),
                vec![text, "真正的歌词"],
                "误伤歌词：{text}"
            );
        }
        for value in fixtures["contextualCredits"].as_array().unwrap() {
            let text = value.as_str().unwrap();
            let lrc = format!("[00:01.00]编曲：张三\n[00:02.00]{text}\n[00:03.00]真正的歌词");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&lrc),
                    "Song",
                    "Artist",
                    ""
                )),
                vec!["真正的歌词"],
                "制作信息块内漏掉署名：{text}"
            );
            let standalone = format!("[00:02.00]{text}\n[00:03.00]真正的歌词");
            assert_eq!(
                texts(&sanitize_matched_lines(
                    parse_auto(&standalone),
                    "Song",
                    "Artist",
                    ""
                )),
                vec![text, "真正的歌词"],
                "单独出现的含糊署名应保留：{text}"
            );
        }
    }

    #[test]
    fn title_words_and_short_title_verses_are_kept_at_song_edges() {
        let lrc = "[00:00.00]Love\n[00:02.00]Love will keep us alive\n[00:04.00]Song Artist forever\n[00:06.00]Love";
        assert_eq!(
            texts(&sanitize_matched_lines(
                parse_auto(lrc),
                "Love",
                "Artist",
                "Love"
            )),
            vec![
                "Love",
                "Love will keep us alive",
                "Song Artist forever",
                "Love"
            ]
        );
        assert_eq!(
            texts(&sanitize_matched_lines(
                parse_auto(lrc),
                "Song",
                "Artist",
                ""
            )),
            vec![
                "Love",
                "Love will keep us alive",
                "Song Artist forever",
                "Love"
            ]
        );
    }

    #[test]
    fn credits_in_the_middle_are_removed_and_word_timings_are_preserved() {
        let mut lines = parse_auto("[10000,1000](10000,500,0)你(10500,500,0)好");
        let verse = lines[0].clone();
        lines.extend(parse_auto("[00:11.00]MIDI程序：韦伟"));
        lines.push(verse.clone());
        let result = sanitize_matched_lines(lines, "Song", "Artist", "");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start_ms, verse.start_ms);
        assert_eq!(result[0].duration_ms, verse.duration_ms);
        assert_eq!(result[0].words[1].start_ms, verse.words[1].start_ms);
        assert_eq!(result[0].words[1].duration_ms, verse.words[1].duration_ms);
    }

    #[test]
    fn attached_secondary_credits_use_context_and_keep_missing_track_boundaries() {
        let mut lines =
            parse_auto("[00:01.00]Hello\n[00:02.00]World\n[00:03.00]Break\n[00:04.00]After");
        lines[0].translation = Some("编曲：张三".into());
        lines[0].roman = Some("Producer: John".into());
        for index in [1, 3] {
            lines[index].translation = Some("词：徐若瑄".into());
            lines[index].roman = Some("Lyrics by Alice".into());
        }
        let result = sanitize_matched_lines(lines, "Song", "Artist", "");
        for line in &result[..3] {
            assert_eq!(line.translation, None);
            assert_eq!(line.roman, None);
        }
        assert_eq!(result[3].translation.as_deref(), Some("词：徐若瑄"));
        assert_eq!(result[3].roman.as_deref(), Some("Lyrics by Alice"));
    }
}
