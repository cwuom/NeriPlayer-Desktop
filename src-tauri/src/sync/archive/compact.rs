use std::collections::{BTreeMap, HashMap, VecDeque};

use super::wire::{self, invalid, Cursor, Field};
use crate::error::AppResult;

const BLOCK_BYTES: usize = 2 * 1024 * 1024;
const MAX_PART_BYTES: usize = 16 * 1024 * 1024;
const MAX_FIELDS: usize = 262_144;
const MAX_ROWS: usize = 32_768;

fn unzigzag(value: u64) -> u64 {
    (value >> 1) ^ (0_u64.wrapping_sub(value & 1))
}

fn nested(context: &str, tag: u64) -> Option<&'static str> {
    match (context, tag) {
        ("song", 27) | ("kind10", 7) | ("kind11", 18) | ("kind16", 2) => Some("token"),
        ("kind5", 2) => Some("song"),
        ("kind8", 16) | ("kind9", 17) | ("kind11", 12) | ("kind12", 6) | ("kind13", 7) => {
            Some("shard")
        }
        _ => None,
    }
}

fn root(kind: u8) -> String {
    if matches!(kind, 2 | 4 | 15) {
        "song".into()
    } else {
        format!("kind{kind}")
    }
}

type Table = Vec<Vec<Field>>;
enum Dictionary {
    Literal(Vec<u8>),
    Lyric(String, usize),
}

pub(super) fn unpack(
    main: &[u8],
    legacy: &[u8],
    pool: &[u8],
    expected_main: usize,
    expected_legacy: usize,
) -> AppResult<(Vec<u8>, Vec<u8>)> {
    let bodies = decode_pool(
        pool,
        expected_main
            .checked_add(expected_legacy)
            .ok_or_else(|| invalid("original size overflow"))?,
    )?;
    let mut main = Cursor::new(main);
    let mut legacy = Cursor::new(legacy);
    if main.bytes(8)? != b"NPCOMP01" || legacy.bytes(8)? != b"NPORDR01" {
        return Err(invalid("unknown compact version"));
    }
    let mut main_output = Vec::new();
    let mut legacy_output = Vec::new();
    while main.remaining() > 0 {
        match main.byte()? {
            0 => {
                let encoded = main.part(MAX_PART_BYTES)?;
                let order = legacy.part(MAX_PART_BYTES)?;
                let sections = wire::fields(encoded)?;
                if sections.iter().map(|field| field.key).collect::<Vec<_>>() != [10, 18, 26, 34] {
                    return Err(invalid("invalid compact sections"));
                }
                let dictionary = read_dictionary(&sections[3].body)?;
                let tables = read_tables(&sections[1].body, &sections[2].body)?;
                restore_order(
                    &sections[0].body,
                    &tables,
                    &dictionary,
                    &bodies,
                    &mut main_output,
                    expected_main,
                )?;
                restore_order(
                    order,
                    &tables,
                    &dictionary,
                    &bodies,
                    &mut legacy_output,
                    expected_legacy,
                )?;
            }
            mode @ (1 | 2) => {
                let kind = main.byte()?;
                wire::context(kind)?;
                let raw = main.part(wire::MAX_RECORD_BYTES)?;
                let (output, maximum) = if mode == 1 {
                    (&mut main_output, expected_main)
                } else {
                    (&mut legacy_output, expected_legacy)
                };
                append_frame(output, maximum, kind, raw)?;
            }
            _ => return Err(invalid("unknown compact block mode")),
        }
    }
    legacy.finish()?;
    if main_output.len() != expected_main || legacy_output.len() != expected_legacy {
        return Err(invalid("restored stream size mismatch"));
    }
    Ok((main_output, legacy_output))
}

fn append_frame(output: &mut Vec<u8>, maximum: usize, kind: u8, bytes: &[u8]) -> AppResult<()> {
    if bytes.len().saturating_add(5) > maximum.saturating_sub(output.len()) {
        return Err(invalid("restored stream exceeds budget"));
    }
    output.push(kind);
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

fn read_dictionary(raw: &[u8]) -> AppResult<Vec<Dictionary>> {
    let mut input = Cursor::new(raw);
    let count = input.count(MAX_FIELDS)?;
    let mut output = Vec::new();
    let mut bytes = 0;
    for _ in 0..count {
        let mode = input.byte()?;
        let hash = if mode == 1 {
            Some(hex::encode(input.bytes(32)?))
        } else if mode == 0 {
            None
        } else {
            return Err(invalid("invalid dictionary mode"));
        };
        let length = input.count(BLOCK_BYTES)?;
        bytes += if hash.is_some() { 32 } else { length };
        if bytes > BLOCK_BYTES {
            return Err(invalid("dictionary metadata exceeds budget"));
        }
        output.push(match hash {
            Some(hash) => Dictionary::Lyric(hash, length),
            None => Dictionary::Literal(input.bytes(length)?.to_vec()),
        });
    }
    input.finish()?;
    Ok(output)
}

fn read_tables(shape_raw: &[u8], values_raw: &[u8]) -> AppResult<HashMap<String, Table>> {
    let mut shape = Cursor::new(shape_raw);
    let mut values = Cursor::new(values_raw);
    let mut shared = Vec::new();
    let mut previous = 0_u64;
    for _ in 0..values.count(MAX_FIELDS)? {
        previous = previous.wrapping_add(unzigzag(values.number()?));
        shared.push(previous);
    }
    let mut tables = HashMap::new();
    let mut rows_used = 0;
    let mut fields_used = 0;
    for _ in 0..shape.count(259)? {
        let context = std::str::from_utf8(shape.part(16)?)
            .map_err(|_| invalid("invalid context UTF-8"))?
            .to_string();
        let valid = matches!(context.as_str(), "song" | "token" | "shard")
            || context
                .strip_prefix("kind")
                .is_some_and(|value| value.parse::<u8>().is_ok());
        if !valid || tables.contains_key(&context) {
            return Err(invalid("invalid or duplicate compact context"));
        }
        let count = shape.count(MAX_FIELDS)?;
        rows_used += count;
        if rows_used > MAX_FIELDS + MAX_ROWS {
            return Err(invalid("compact row budget exceeded"));
        }
        let mut rows = Vec::new();
        let mut expected = BTreeMap::<u64, usize>::new();
        for _ in 0..count {
            let fields = shape.count(MAX_FIELDS)?;
            fields_used += fields;
            if fields_used > MAX_FIELDS {
                return Err(invalid("compact field budget exceeded"));
            }
            let mut row = Vec::new();
            for _ in 0..fields {
                let key = shape.number()?;
                if key != 0
                    && (key >> 3 == 0
                        || key >> 3 > 536_870_911
                        || !matches!(key & 7, 0 | 1 | 2 | 5))
                {
                    return Err(invalid("invalid compact column key"));
                }
                *expected.entry(key).or_default() += 1;
                row.push(Field {
                    key,
                    body: Vec::new(),
                });
            }
            rows.push(row);
        }
        let mut columns = HashMap::<u64, VecDeque<Vec<u8>>>::new();
        if shape.count(expected.len())? != expected.len() {
            return Err(invalid("missing compact column"));
        }
        for _ in 0..expected.len() {
            let key = shape.number()?;
            let count = shape.count(MAX_FIELDS)?;
            if expected.get(&key) != Some(&count) || columns.contains_key(&key) {
                return Err(invalid("compact column count mismatch"));
            }
            let mode = shape.byte()?;
            if mode != 0 && !(mode == 2 && key != 0 && key & 7 == 0) {
                return Err(invalid("invalid compact numeric mode"));
            }
            let length = shape.count(MAX_PART_BYTES)?;
            let mut input = Cursor::new(values.bytes(length)?);
            let mut column = VecDeque::new();
            let mut previous = 0_u64;
            for _ in 0..count {
                let body = if mode == 2 {
                    let reference = input.count(shared.len())?;
                    previous = if reference == 0 {
                        previous.wrapping_add(unzigzag(input.number()?))
                    } else {
                        shared[reference - 1]
                    };
                    let mut output = Vec::new();
                    wire::number(&mut output, previous);
                    output
                } else {
                    input.part(BLOCK_BYTES)?.to_vec()
                };
                column.push_back(body);
            }
            input.finish()?;
            columns.insert(key, column);
        }
        for row in &mut rows {
            for field in row {
                field.body = columns
                    .get_mut(&field.key)
                    .and_then(VecDeque::pop_front)
                    .ok_or_else(|| invalid("missing compact value"))?;
            }
        }
        tables.insert(context, rows);
    }
    shape.finish()?;
    values.finish()?;
    Ok(tables)
}

fn restore_order(
    raw: &[u8],
    tables: &HashMap<String, Table>,
    dictionary: &[Dictionary],
    pool: &HashMap<String, Vec<u8>>,
    output: &mut Vec<u8>,
    maximum: usize,
) -> AppResult<()> {
    let mut input = Cursor::new(raw);
    let mut previous = [0_u64; 256];
    for _ in 0..input.count(MAX_ROWS)? {
        let kind = input.byte()?;
        wire::context(kind)?;
        let row = previous[usize::from(kind)].wrapping_add(unzigzag(input.number()?));
        if row > i32::MAX as u64 {
            return Err(invalid("invalid compact row index"));
        }
        previous[usize::from(kind)] = row;
        let body = restore_message(&root(kind), row as usize, tables, dictionary, pool, 0)?;
        append_frame(output, maximum, kind, &body)?;
    }
    input.finish()
}

fn restore_message(
    context: &str,
    row: usize,
    tables: &HashMap<String, Table>,
    dictionary: &[Dictionary],
    pool: &HashMap<String, Vec<u8>>,
    depth: usize,
) -> AppResult<Vec<u8>> {
    if depth > 8 {
        return Err(invalid("compact nesting exceeds budget"));
    }
    let row = tables
        .get(context)
        .and_then(|table| table.get(row))
        .ok_or_else(|| invalid("missing compact context or row"))?;
    let mut output = Vec::new();
    for token in row {
        if token.key == 0 {
            output.extend_from_slice(&token.body);
        } else if token.key & 7 != 2 {
            wire::field(&mut output, token.key, &token.body);
        } else {
            let mut input = Cursor::new(&token.body);
            let reference = input.number()?;
            input.finish()?;
            let body = if let Some(context) = nested(context, token.key >> 3) {
                if reference > i32::MAX as u64 {
                    return Err(invalid("invalid child reference"));
                }
                restore_message(
                    context,
                    reference as usize,
                    tables,
                    dictionary,
                    pool,
                    depth + 1,
                )?
            } else {
                if reference & 1 != 0 || reference >> 1 == 0 {
                    return Err(invalid("invalid dictionary reference"));
                }
                match dictionary
                    .get((reference >> 1) as usize - 1)
                    .ok_or_else(|| invalid("missing dictionary reference"))?
                {
                    Dictionary::Literal(bytes) => bytes.clone(),
                    Dictionary::Lyric(hash, length) => {
                        let bytes = pool
                            .get(hash)
                            .ok_or_else(|| invalid("missing shared lyric body"))?;
                        if bytes.len() != *length {
                            return Err(invalid("lyric reference size mismatch"));
                        }
                        bytes.clone()
                    }
                }
            };
            wire::field(&mut output, token.key, &body);
        }
        if output.len() > BLOCK_BYTES {
            return Err(invalid("restored compact record exceeds budget"));
        }
    }
    Ok(output)
}

fn decode_pool(raw: &[u8], maximum: usize) -> AppResult<HashMap<String, Vec<u8>>> {
    let mut input = Cursor::new(raw);
    if input.bytes(8)? != b"NPBODY01" {
        return Err(invalid("unknown lyric pool version"));
    }
    let mut pool = HashMap::new();
    let mut previous = String::new();
    let mut total = 0;
    while input.remaining() > 0 {
        for body in decode_bodies(input.part(MAX_PART_BYTES)?)? {
            total += body.len();
            if total > maximum || pool.len() >= 131_072 {
                return Err(invalid("lyric pool exceeds original budget"));
            }
            let hash = super::digest(&body);
            if !previous.is_empty() && hash <= previous {
                return Err(invalid("duplicate or unordered lyric body"));
            }
            previous = hash.clone();
            pool.insert(hash, body);
        }
    }
    Ok(pool)
}

fn parts(input: &mut Cursor<'_>, expected: usize) -> AppResult<Vec<Vec<u8>>> {
    if input.count(expected)? != expected {
        return Err(invalid("invalid lyric part count"));
    }
    let mut result = Vec::new();
    let mut total = 0;
    for _ in 0..expected {
        let bytes = input.part(8 * 1024 * 1024 + 512 * 1024)?;
        total += bytes.len();
        if total > 20 * 1024 * 1024 {
            return Err(invalid("lyric encoded group exceeds budget"));
        }
        result.push(bytes.to_vec());
    }
    Ok(result)
}

fn decode_bodies(raw: &[u8]) -> AppResult<Vec<Vec<u8>>> {
    let mut input = Cursor::new(raw);
    let mode = input.byte()?;
    if mode > 1 {
        return Err(invalid("invalid lyric body mode"));
    }
    let parts = parts(&mut input, if mode == 0 { 3 } else { 5 })?;
    input.finish()?;
    let literals = if mode == 0 {
        parts[2].clone()
    } else {
        let mut palette_input = Cursor::new(&parts[2]);
        let count = palette_input.count(65_536)?;
        if count == 0 {
            return Err(invalid("empty lyric palette"));
        }
        let mut palette = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..count {
            let raw = palette_input.part(4)?;
            let text =
                std::str::from_utf8(raw).map_err(|_| invalid("invalid lyric palette UTF-8"))?;
            if text.chars().count() != 1 || !seen.insert(text.to_string()) {
                return Err(invalid("invalid or duplicate palette entry"));
            }
            palette.push(raw.to_vec());
        }
        palette_input.finish()?;
        let mut lengths = Cursor::new(&parts[3]);
        if lengths.number()? != 1 {
            return Err(invalid("invalid literal count"));
        }
        let count = lengths.count(4 * 1024 * 1024)?;
        lengths.finish()?;
        if parts[4].len() != count * 2 {
            return Err(invalid("invalid lyric character count"));
        }
        let mut output = Vec::new();
        for raw in parts[4].chunks_exact(2) {
            let index = u16::from_le_bytes([raw[0], raw[1]]) as usize;
            output.extend_from_slice(
                palette
                    .get(index)
                    .ok_or_else(|| invalid("invalid palette index"))?,
            );
            if output.len() > 4 * 1024 * 1024 {
                return Err(invalid("lyric literal group exceeds budget"));
            }
        }
        output
    };
    if literals.len() > 4 * 1024 * 1024 {
        return Err(invalid("lyric literal group exceeds budget"));
    }
    let mut layout = Cursor::new(&parts[0]);
    let mut numbers = Cursor::new(&parts[1]);
    let number_parts = self::parts(&mut numbers, 8)?;
    numbers.finish()?;
    let mut columns: Vec<_> = number_parts.iter().map(|raw| Cursor::new(raw)).collect();
    let mut text = Cursor::new(&literals);
    let count = layout.count(4096)?;
    let mut result = Vec::new();
    let mut total = 0;
    for _ in 0..count {
        let mut body = Vec::new();
        let mut previous = [0_u64; 8];
        for _ in 0..layout.count(BLOCK_BYTES)? {
            let count = layout.count(BLOCK_BYTES)?;
            body.extend_from_slice(text.bytes(count)?);
            let kind = layout.byte()?;
            let (offset, pieces, open, close, separators) = match kind {
                0 => {
                    let separator = layout.byte()?;
                    if separator > 1 {
                        return Err(invalid("invalid timestamp separator"));
                    }
                    (0, 3, '[', ']', if separator == 1 { ":." } else { "::" })
                }
                1 => (3, 3, '(', ')', ",,"),
                2 => (6, 2, '[', ']', ","),
                _ => return Err(invalid("invalid timestamp kind")),
            };
            let mut token = String::new();
            token.push(open);
            for index in 0..pieces {
                let width = layout.count(18)?;
                if width == 0 {
                    return Err(invalid("invalid numeric width"));
                }
                let slot = offset + index;
                let encoded = columns[slot].number()?;
                let value = if index == 0 {
                    previous[slot].wrapping_add(unzigzag(encoded))
                } else {
                    encoded
                };
                if value > 999_999_999_999_999_999 {
                    return Err(invalid("invalid lyric numeric value"));
                }
                previous[slot] = value;
                let decimal = value.to_string();
                if decimal.len() > width {
                    return Err(invalid("lyric numeric width overflow"));
                }
                if index > 0 {
                    token.push(separators.as_bytes()[index - 1] as char);
                }
                token.extend(std::iter::repeat_n('0', width - decimal.len()));
                token.push_str(&decimal);
            }
            token.push(close);
            body.extend_from_slice(token.as_bytes());
            if body.len() > BLOCK_BYTES {
                return Err(invalid("restored lyric body exceeds budget"));
            }
        }
        let count = layout.count(BLOCK_BYTES)?;
        body.extend_from_slice(text.bytes(count)?);
        total += body.len();
        if body.len() > BLOCK_BYTES || total > 4 * 1024 * 1024 {
            return Err(invalid("restored lyric group exceeds budget"));
        }
        result.push(body);
    }
    layout.finish()?;
    text.finish()?;
    for column in columns {
        column.finish()?;
    }
    Ok(result)
}

pub(super) fn pack_literal(main: &[u8], legacy: &[u8]) -> AppResult<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut output = b"NPCOMP01".to_vec();
    for (stream, mode) in [(main, 1), (legacy, 2)] {
        let mut input = Cursor::new(stream);
        while input.remaining() > 0 {
            let kind = input.byte()?;
            wire::context(kind)?;
            let raw_size = u32::from_be_bytes(input.bytes(4)?.try_into().unwrap()) as usize;
            if raw_size > wire::MAX_RECORD_BYTES {
                return Err(invalid("record exceeds budget"));
            }
            let raw = input.bytes(raw_size)?;
            output.push(mode);
            output.push(kind);
            wire::part(&mut output, raw);
        }
    }
    Ok((output, b"NPORDR01".to_vec(), b"NPBODY01".to_vec()))
}
