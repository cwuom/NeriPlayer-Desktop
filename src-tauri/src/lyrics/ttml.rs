use super::parser::{LyricLine, LyricWord};
use crate::error::{AppError, AppResult};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use regex::Regex;
use std::{collections::HashMap, sync::OnceLock};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_TIME_MS: u64 = 24 * 60 * 60 * 1000;

enum Part {
    Text(String),
    Child(Node),
}
struct Node {
    name: String,
    ttml: bool,
    attrs: HashMap<String, String>,
    parts: Vec<Part>,
}
impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key.rsplit(':').next() == Some(name))
            .map(|(_, value)| value.as_str())
    }
    fn role(&self, role: &str) -> bool {
        self.attr("role") == Some(role)
    }
    fn children(&self) -> impl Iterator<Item = &Node> {
        self.parts.iter().filter_map(|part| match part {
            Part::Child(node) => Some(node),
            _ => None,
        })
    }
}

fn invalid(message: &str) -> AppError {
    AppError::Other(format!("TTML: {message}"))
}

pub(super) fn looks_like(content: &str) -> bool {
    static ROOT: OnceLock<Regex> = OnceLock::new();
    content.contains("http://www.w3.org/ns/ttml")
        || ROOT
            .get_or_init(|| Regex::new(r"<(?:[A-Za-z_][A-Za-z0-9_.-]*:)?tt(?:\s|/?>)").unwrap())
            .is_match(content)
}

pub(super) fn parse(content: &str) -> AppResult<Vec<LyricLine>> {
    if content.len() > MAX_BYTES {
        return Err(invalid("document exceeds size budget"));
    }
    let mut reader = NsReader::from_reader(content.trim_start_matches('\u{feff}').as_bytes());
    reader.config_mut().expand_empty_elements = true;
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut count = 0;
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|_| invalid("malformed XML"))?;
        match event {
            Event::Start(element) => {
                count += 1;
                if count > 100_000 || stack.len() >= 64 {
                    return Err(invalid("document exceeds structural budget"));
                }
                let ttml = match namespace {
                    ResolveResult::Bound(value) => value.as_ref() == b"http://www.w3.org/ns/ttml",
                    ResolveResult::Unbound => true,
                    ResolveResult::Unknown(_) => return Err(invalid("unknown namespace prefix")),
                };
                let mut attrs = HashMap::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|_| invalid("invalid attribute"))?;
                    let name = std::str::from_utf8(attribute.key.as_ref())
                        .map_err(|_| invalid("invalid attribute name"))?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| invalid("invalid attribute entity"))?;
                    attrs.insert(name.to_owned(), value.into_owned());
                }
                stack.push(Node {
                    name: std::str::from_utf8(element.local_name().as_ref())
                        .map_err(|_| invalid("invalid element name"))?
                        .to_owned(),
                    ttml,
                    attrs,
                    parts: Vec::new(),
                });
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| invalid("unexpected end tag"))?;
                if let Some(parent) = stack.last_mut() {
                    parent.parts.push(Part::Child(node));
                } else if root.replace(node).is_some() {
                    return Err(invalid("multiple roots"));
                }
            }
            Event::Text(text) => {
                let value = text
                    .decode()
                    .map_err(|_| invalid("invalid text encoding"))?;
                let value = quick_xml::escape::unescape(&value)
                    .map_err(|_| invalid("invalid text entity"))?;
                append_text(&mut stack, value.into_owned())?;
            }
            Event::CData(text) => append_text(
                &mut stack,
                text.decode()
                    .map_err(|_| invalid("invalid CDATA"))?
                    .into_owned(),
            )?,
            Event::GeneralRef(reference) => {
                let reference = std::str::from_utf8(reference.as_ref())
                    .map_err(|_| invalid("invalid entity"))?;
                let escaped = format!("&{reference};");
                append_text(
                    &mut stack,
                    quick_xml::escape::unescape(&escaped)
                        .map_err(|_| invalid("unknown entity"))?
                        .into_owned(),
                )?;
            }
            Event::DocType(_) => return Err(invalid("document types are not allowed")),
            Event::Eof => break,
            Event::Decl(declaration) => {
                if declaration
                    .encoding()
                    .transpose()
                    .map_err(|_| invalid("invalid declaration"))?
                    .is_some_and(|encoding| !encoding.eq_ignore_ascii_case(b"UTF-8"))
                {
                    return Err(invalid("only UTF-8 is supported"));
                }
            }
            Event::Comment(_) | Event::PI(_) => {}
            _ => return Err(invalid("unsupported XML event")),
        }
    }
    if !stack.is_empty() {
        return Err(invalid("truncated XML"));
    }
    let root = root.ok_or_else(|| invalid("missing root"))?;
    if root.name != "tt" || !root.ttml {
        return Err(invalid("invalid TTML root"));
    }
    let mut translation = HashMap::new();
    let mut roman = HashMap::new();
    collect_itunes(&root, &mut translation, &mut roman);
    let mut lines = Vec::new();
    collect_lines(&root, false, &translation, &roman, &mut lines)?;
    lines.sort_by_key(|line| line.start_ms);
    Ok(lines)
}

fn append_text(stack: &mut [Node], value: String) -> AppResult<()> {
    if let Some(node) = stack.last_mut() {
        node.parts.push(Part::Text(value));
    } else if !value.trim().is_empty() {
        return Err(invalid("text outside root"));
    }
    Ok(())
}

fn text(node: &Node, exclude_roles: bool) -> String {
    let mut result = String::new();
    for part in &node.parts {
        match part {
            Part::Text(value) => result.push_str(&layout_text(value)),
            Part::Child(child) if exclude_roles && special_role(child) => {}
            Part::Child(child) if child.name == "br" => result.push('\n'),
            Part::Child(child) => result.push_str(&text(child, exclude_roles)),
        }
    }
    result
}

fn layout_text(value: &str) -> String {
    static LAYOUT: OnceLock<Regex> = OnceLock::new();
    LAYOUT
        .get_or_init(|| Regex::new(r"[\r\n]+[ \t]*").unwrap())
        .replace_all(value, "")
        .into_owned()
}

fn special_role(node: &Node) -> bool {
    ["x-translation", "x-roman", "x-bg"]
        .iter()
        .any(|role| node.role(role))
}

fn collect_itunes(
    node: &Node,
    translations: &mut HashMap<String, String>,
    romans: &mut HashMap<String, String>,
) {
    if node.name == "translation" || node.name == "transliteration" {
        for entry in node.children().filter(|child| child.name == "text") {
            let Some(key) = entry.attr("for") else {
                continue;
            };
            let value = if node.name == "transliteration" {
                entry
                    .children()
                    .filter(|child| child.name == "span")
                    .map(|child| text(child, false).trim().to_owned())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                text(entry, false).trim().to_owned()
            };
            if !value.is_empty() {
                if node.name == "translation" {
                    translations.insert(key.to_owned(), value);
                } else {
                    romans.insert(key.to_owned(), value);
                }
            }
        }
    }
    for child in node.children() {
        collect_itunes(child, translations, romans);
    }
}

fn collect_lines(
    node: &Node,
    in_body: bool,
    translations: &HashMap<String, String>,
    romans: &HashMap<String, String>,
    lines: &mut Vec<LyricLine>,
) -> AppResult<()> {
    let in_body = in_body || (node.ttml && node.name == "body");
    if in_body && node.ttml && node.name == "p" {
        let parent_key = node.attr("key");
        if let Some(line) = make_line(node, None, parent_key, false, translations, romans)? {
            lines.push(line);
        }
        let mut backgrounds = Vec::new();
        find_backgrounds(node, &mut backgrounds);
        let inherited = bounds(node, None)?;
        for background in backgrounds {
            if let Some(line) = make_line(
                background,
                Some(inherited),
                parent_key,
                true,
                translations,
                romans,
            )? {
                lines.push(line);
            }
        }
        if lines.len() > 20_000 {
            return Err(invalid("too many lyric lines"));
        }
    } else {
        for child in node.children() {
            collect_lines(child, in_body, translations, romans, lines)?;
        }
    }
    Ok(())
}

fn find_backgrounds<'a>(node: &'a Node, result: &mut Vec<&'a Node>) {
    for child in node.children() {
        if child.role("x-bg") {
            result.push(child);
        } else {
            find_backgrounds(child, result);
        }
    }
}

fn find_role(node: &Node, role: &str) -> Option<String> {
    for child in node.children() {
        if child.role(role) {
            return Some(text(child, false).trim().to_owned()).filter(|value| !value.is_empty());
        }
        if !child.role("x-bg") {
            if let Some(value) = find_role(child, role) {
                return Some(value);
            }
        }
    }
    None
}

type Bounds = (Option<u64>, Option<u64>);
fn bounds(node: &Node, inherited: Option<Bounds>) -> AppResult<Bounds> {
    let inherited = inherited.unwrap_or((None, None));
    let begin = node
        .attr("begin")
        .map(parse_time)
        .transpose()?
        .or(inherited.0);
    let mut end = node.attr("end").map(parse_time).transpose()?;
    if let Some(duration) = node.attr("dur") {
        let duration = parse_time(duration)?;
        let duration_end = begin
            .unwrap_or(0)
            .checked_add(duration)
            .filter(|value| *value <= MAX_TIME_MS)
            .ok_or_else(|| invalid("time overflow"))?;
        end = Some(end.map_or(duration_end, |value| value.min(duration_end)));
    }
    let end = end.or(inherited.1);
    if begin.zip(end).is_some_and(|(begin, end)| end < begin) {
        return Err(invalid("reversed time interval"));
    }
    Ok((begin, end))
}

fn gather_words(
    node: &Node,
    inherited: Bounds,
    output: &mut String,
    words: &mut Vec<LyricWord>,
) -> AppResult<()> {
    for part in &node.parts {
        match part {
            Part::Text(value) => {
                let value = layout_text(value);
                output.push_str(&value);
                if let Some(word) = words.last_mut() {
                    word.text.push_str(&value);
                }
            }
            Part::Child(child) if special_role(child) => {}
            Part::Child(child) => {
                let interval = bounds(child, Some(inherited))?;
                let nested_timing = child
                    .children()
                    .any(|nested| !special_role(nested) && nested.attr("begin").is_some());
                let explicitly_timed = child.ttml
                    && child.name == "span"
                    && child.attr("begin").is_some()
                    && (child.attr("end").is_some() || child.attr("dur").is_some());
                if explicitly_timed && !nested_timing {
                    let value = text(child, true);
                    output.push_str(&value);
                    if let Some((start, end)) = interval
                        .0
                        .zip(interval.1)
                        .filter(|(start, end)| end > start)
                    {
                        if !value.trim().is_empty() {
                            words.push(LyricWord {
                                start_ms: start,
                                duration_ms: end - start,
                                text: value,
                            });
                        }
                    }
                } else {
                    gather_words(child, interval, output, words)?;
                }
            }
        }
    }
    Ok(())
}

fn split_translation(value: &str, background: bool) -> String {
    if value.ends_with('）') {
        if let Some((main, bg)) = value.rsplit_once('（') {
            return if background {
                bg.trim_end_matches('）').trim().to_owned()
            } else {
                main.trim().to_owned()
            };
        }
    }
    value.to_owned()
}

fn make_line(
    node: &Node,
    inherited: Option<Bounds>,
    parent_key: Option<&str>,
    background: bool,
    translations: &HashMap<String, String>,
    romans: &HashMap<String, String>,
) -> AppResult<Option<LyricLine>> {
    let interval = bounds(node, inherited)?;
    let mut text = String::new();
    let mut words = Vec::new();
    gather_words(node, interval, &mut text, &mut words)?;
    if let Some(last) = words.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Ok(None);
    }
    let Some(start) = interval
        .0
        .or_else(|| words.iter().map(|word| word.start_ms).min())
    else {
        return Ok(None);
    };
    let Some(end) = interval.1.or_else(|| {
        words
            .iter()
            .map(|word| word.start_ms + word.duration_ms)
            .max()
    }) else {
        return Ok(None);
    };
    let duration = end
        .checked_sub(start)
        .ok_or_else(|| invalid("reversed derived interval"))?;
    let key = node.attr("key").or(parent_key);
    let translation = find_role(node, "x-translation")
        .or_else(|| {
            key.and_then(|key| translations.get(key))
                .map(|value| split_translation(value, background))
        })
        .filter(|value| !value.is_empty());
    let roman = find_role(node, "x-roman").or_else(|| key.and_then(|key| romans.get(key)).cloned());
    Ok(Some(LyricLine {
        start_ms: start,
        duration_ms: duration,
        text,
        translation,
        roman,
        words,
    }))
}

fn parse_time(value: &str) -> AppResult<u64> {
    fn decimal(value: &str, unit: u64) -> Option<u64> {
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let whole = whole.parse::<u64>().ok()?.checked_mul(unit)?;
        let fraction = if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u64>().ok()?.checked_mul(unit)?
                / 10u64.checked_pow(fraction.len() as u32)?
        };
        whole.checked_add(fraction)
    }
    let value = value.trim();
    let parsed = if value.contains(':') {
        let parts = value.split(':').collect::<Vec<_>>();
        let (hours, minutes, seconds) = match parts.as_slice() {
            [minutes, seconds] => (0, minutes.parse::<u64>().ok(), decimal(seconds, 1000)),
            [hours, minutes, seconds] => (
                hours.parse::<u64>().ok().unwrap_or(u64::MAX),
                minutes.parse::<u64>().ok().filter(|value| *value < 60),
                decimal(seconds, 1000),
            ),
            _ => return Err(invalid("invalid timestamp")),
        };
        minutes
            .zip(seconds.filter(|value| *value < 60_000))
            .and_then(|(minutes, seconds)| {
                hours
                    .checked_mul(3_600_000)?
                    .checked_add(minutes.checked_mul(60_000)?)?
                    .checked_add(seconds)
            })
    } else if let Some(value) = value.strip_suffix("ms") {
        decimal(value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        decimal(value, 1000)
    } else if let Some(value) = value.strip_suffix('m') {
        decimal(value, 60_000)
    } else if let Some(value) = value.strip_suffix('h') {
        decimal(value, 3_600_000)
    } else {
        decimal(value, 1000)
    };
    parsed
        .filter(|value| *value <= MAX_TIME_MS)
        .ok_or_else(|| invalid("invalid or excessive timestamp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDS: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><div>
      <p begin="00:00:01.000" end="00:00:03.000"><span begin="1s" end="1.5s">Hello </span><span begin="1.5s" end="3s">&amp; world&#33;</span><span ttm:role="x-translation">你好</span><span ttm:role="x-roman">ni hao</span><span ttm:role="x-bg" begin="2s" end="3s"><span begin="2s" end="3s">echo</span></span></p>
      <p begin="4s" end="6s">plain line</p>
    </div></body></tt>"#;

    #[test]
    fn ttml_preserves_words_entities_translation_and_background() {
        let lines = parse(WORDS).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "Hello & world!");
        assert_eq!(lines[0].translation.as_deref(), Some("你好"));
        assert_eq!(lines[0].roman.as_deref(), Some("ni hao"));
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(
            (lines[0].words[0].start_ms, lines[0].words[0].duration_ms),
            (1000, 500)
        );
        assert_eq!(lines[1].text, "echo");
        assert_eq!(lines[1].start_ms, 2000);
        assert!(lines[2].words.is_empty());
    }

    #[test]
    fn ttml_handles_prefix_cdata_duration_and_inline_spacing() {
        let lines = parse(r#"<t:tt xmlns:t="http://www.w3.org/ns/ttml"><t:body><t:div><t:p begin="1m" dur="2s"><t:span begin="60s" dur="1s"><![CDATA[A < B]]></t:span> <t:span begin="61s" end="62s">C</t:span></t:p></t:div></t:body></t:tt>"#).unwrap();
        assert_eq!(lines[0].text, "A < B C");
        assert_eq!(lines[0].duration_ms, 2000);
        assert_eq!(lines[0].words[0].text, "A < B ");
        assert_eq!(lines[0].words[1].start_ms, 61_000);
    }

    #[test]
    fn ttml_rejects_dtd_unknown_entities_truncation_and_invalid_times() {
        for xml in [
            "<!DOCTYPE tt [<!ENTITY x 'unsafe'>]><tt><body><p>&x;</p></body></tt>",
            "<tt><body><p>&unknown;</p></body></tt>",
            "<tt><body><p begin='1s'>unfinished",
            "<tt><body><p begin='-1s' end='2s'>bad</p></body></tt>",
            "<tt><body><p begin='2s' end='1s'>bad</p></body></tt>",
            "<tt><body><p begin='999999999999999999s'>bad</p></body></tt>",
            "<tt/><tt/>",
        ] {
            assert!(parse(xml).is_err(), "accepted invalid XML");
        }
    }

    #[test]
    fn ttml_auto_detection_uses_real_parser() {
        let lines = super::super::parser::parse_auto(WORDS);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].text, "Hello & world!");
    }

    #[test]
    fn ttml_reads_itunes_translation_and_phonetic_by_line_key() {
        let lines = parse(r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal"><head><metadata><iTunesMetadata xmlns="http://music.apple.com/lyric-ttml-internal"><translations><translation><text for="L1">你好世界</text></translation></translations><transliterations><transliteration><text for="L1"><span>Halo</span><span>waludo</span></text></transliteration></transliterations></iTunesMetadata></metadata></head><body><p begin="1s" end="3s" itunes:key="L1"><span begin="1s" end="2s">Hello </span><span begin="2s" end="3s">world</span></p></body></tt>"#).unwrap();
        assert_eq!(lines[0].translation.as_deref(), Some("你好世界"));
        assert_eq!(lines[0].roman.as_deref(), Some("Halo waludo"));
    }

    #[test]
    fn ttml_bounds_xml_size_and_depth() {
        let depth = format!(
            "<tt>{}text{}</tt>",
            "<span>".repeat(66),
            "</span>".repeat(66)
        );
        assert!(parse(&depth).is_err());
        let bytes = format!(
            "<tt><body><p>{}</p></body></tt>",
            "x".repeat(2 * 1024 * 1024)
        );
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn ttml_rejects_derived_line_start_after_explicit_end() {
        assert!(parse(
            r#"<tt><body><p end="1s"><span begin="2s" end="3s">late</span></p></body></tt>"#
        )
        .is_err());
    }
}
