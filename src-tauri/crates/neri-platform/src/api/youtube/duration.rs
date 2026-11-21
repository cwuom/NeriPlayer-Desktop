// YouTube InnerTube 时长提取
//
// 对齐前端 youtubePlaylistParse.ts 的 extractTrackDurationText 与
// Android YouTubeMusicClient.extractSearchDurationText 的多路径策略：
// fixedColumns 任意列 -> flexColumns 整列 -> flexColumns runs 反向扫描(跳过标题列)
// -> lengthText -> lengthSeconds
//
// 所有路径对缺失/畸形字段安全跳过，全部失败返回 0（前端显示 --:--）

use serde_json::Value;

/// 多路径提取曲目时长（毫秒），全部路径失败返回 0
pub fn extract_track_duration_ms(renderer: &Value) -> u64 {
    if let Some(text) = extract_track_duration_text(renderer) {
        let ms = parse_duration_text_to_ms(&text);
        if ms > 0 {
            return ms;
        }
    }
    length_seconds_to_ms(&renderer["lengthSeconds"])
}

/// 按前端验证过的优先级提取时长文本
fn extract_track_duration_text(renderer: &Value) -> Option<String> {
    // musicResponsiveListItemRenderer: fixedColumns 任意列
    if let Some(cols) = renderer["fixedColumns"].as_array() {
        for col in cols {
            let text = column_text(col, "musicResponsiveListItemFixedColumnRenderer");
            if looks_like_duration(&text) {
                return Some(text.trim().to_string());
            }
        }
    }

    if let Some(cols) = renderer["flexColumns"].as_array() {
        // 整列文本恰为时长
        for col in cols {
            let text = column_text(col, "musicResponsiveListItemFlexColumnRenderer");
            if looks_like_duration(&text) {
                return Some(text.trim().to_string());
            }
        }
        // 布局改版后时长常混在副标题 runs("Artist • Album • 3:45")里，
        // 整段拼接无法通过 looks_like_duration，需按 run 粒度反向扫描；
        // 跳过第 0 列标题，避免形如 "3:05" 的歌名被误判成时长
        for col in cols.iter().skip(1) {
            let text_node = &col["musicResponsiveListItemFlexColumnRenderer"]["text"];
            if let Some(text) = duration_from_runs(text_node) {
                return Some(text);
            }
        }
    }

    // 普通 YouTube 视频布局兜底(videoRenderer / playlistVideoRenderer 等)
    let length_text = extract_text(&renderer["lengthText"]);
    if looks_like_duration(&length_text) {
        return Some(length_text.trim().to_string());
    }

    None
}

/// 将 "3:45" / "1:02:33" 文本解析为毫秒，非法输入返回 0
pub fn parse_duration_text_to_ms(text: &str) -> u64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let parts: Vec<u64> = match trimmed
        .split(':')
        .map(|part| part.trim().parse::<u64>())
        .collect::<Result<_, _>>()
    {
        Ok(parts) => parts,
        Err(_) => return 0,
    };
    let seconds = match parts.as_slice() {
        [m, s] => m.saturating_mul(60).saturating_add(*s),
        [h, m, s] => h
            .saturating_mul(3600)
            .saturating_add(m.saturating_mul(60))
            .saturating_add(*s),
        _ => return 0,
    };
    seconds.saturating_mul(1000)
}

/// 判断文本是否形如 "3:45" / "1:02:33"：以冒号分隔且每段均为纯数字
fn looks_like_duration(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.contains(':') {
        return false;
    }
    let parts: Vec<&str> = trimmed.split(':').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            let part = part.trim();
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
        })
}

/// 在 runs 里从后往前找时长片段
fn duration_from_runs(text_node: &Value) -> Option<String> {
    let runs = text_node["runs"].as_array()?;
    runs.iter().rev().find_map(|run| {
        let text = run["text"].as_str()?.trim();
        looks_like_duration(text).then(|| text.to_string())
    })
}

/// 取列渲染器的整列文本
fn column_text(column: &Value, renderer_key: &str) -> String {
    extract_text(&column[renderer_key]["text"])
}

/// 提取 text 节点全文：runs 拼接 / simpleText / 裸字符串
fn extract_text(node: &Value) -> String {
    if let Some(text) = node.as_str() {
        return text.to_string();
    }
    if let Some(runs) = node["runs"].as_array() {
        return runs.iter().filter_map(|run| run["text"].as_str()).collect();
    }
    if let Some(text) = node["simpleText"].as_str() {
        return text.to_string();
    }
    String::new()
}

/// lengthSeconds 兜底：InnerTube 常给字符串 "212"，个别客户端给数字
fn length_seconds_to_ms(node: &Value) -> u64 {
    let seconds = match node {
        Value::String(text) => text.trim().parse::<u64>().ok(),
        Value::Number(num) => num.as_u64().or_else(|| {
            num.as_f64()
                .filter(|f| f.is_finite() && *f >= 0.0)
                .map(|f| f as u64)
        }),
        _ => None,
    };
    seconds.unwrap_or(0).saturating_mul(1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_from_fixed_columns() {
        // 经典布局：时长在 fixedColumns 的 simpleText
        let renderer = json!({
            "fixedColumns": [{
                "musicResponsiveListItemFixedColumnRenderer": {
                    "text": { "simpleText": "3:45" }
                }
            }]
        });
        assert_eq!(extract_track_duration_ms(&renderer), 225_000);

        // fixedColumns 用 runs 且带小时段
        let renderer = json!({
            "fixedColumns": [{
                "musicResponsiveListItemFixedColumnRenderer": {
                    "text": { "runs": [{ "text": "1:02:33" }] }
                }
            }]
        });
        assert_eq!(extract_track_duration_ms(&renderer), 3_753_000);
    }

    #[test]
    fn extracts_from_flex_column_whole_text() {
        let renderer = json!({
            "flexColumns": [
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Song Title" }] }
                    }
                },
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "4:12" }] }
                    }
                }
            ]
        });
        assert_eq!(extract_track_duration_ms(&renderer), 252_000);
    }

    #[test]
    fn extracts_from_subtitle_runs_tail_skipping_title() {
        // 改版布局：时长混在副标题 runs 尾段；标题列含 "3:05" 但整列
        // 不像时长，runs 扫描又跳过第 0 列，不会被歌名误导
        let renderer = json!({
            "flexColumns": [
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Song 3:05 (Remix)" }] }
                    }
                },
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [
                            { "text": "Artist" },
                            { "text": " • " },
                            { "text": "Album" },
                            { "text": " • " },
                            { "text": "3:45" }
                        ] }
                    }
                }
            ]
        });
        assert_eq!(extract_track_duration_ms(&renderer), 225_000);
    }

    #[test]
    fn falls_back_to_video_renderer_length_text() {
        let renderer = json!({
            "videoId": "abc",
            "lengthText": { "simpleText": "10:07" }
        });
        assert_eq!(extract_track_duration_ms(&renderer), 607_000);

        // lengthText 也可能是 runs 形式
        let renderer = json!({
            "lengthText": { "runs": [{ "text": "0:59" }] }
        });
        assert_eq!(extract_track_duration_ms(&renderer), 59_000);
    }

    #[test]
    fn falls_back_to_length_seconds() {
        let renderer = json!({ "lengthSeconds": "212" });
        assert_eq!(extract_track_duration_ms(&renderer), 212_000);

        let renderer = json!({ "lengthSeconds": 212 });
        assert_eq!(extract_track_duration_ms(&renderer), 212_000);
    }

    #[test]
    fn missing_or_malformed_fields_return_zero() {
        assert_eq!(extract_track_duration_ms(&json!({})), 0);
        assert_eq!(extract_track_duration_ms(&Value::Null), 0);
        // 字段类型畸形：columns 不是数组、runs 是数字、lengthSeconds 非数值
        let renderer = json!({
            "fixedColumns": "not-an-array",
            "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": 42 } } }],
            "lengthText": { "simpleText": "Live" },
            "lengthSeconds": { "nested": true }
        });
        assert_eq!(extract_track_duration_ms(&renderer), 0);
        // 副标题只有文字没有时长片段
        let renderer = json!({
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Title" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Artist" }] } } }
            ]
        });
        assert_eq!(extract_track_duration_ms(&renderer), 0);
    }

    #[test]
    fn parse_duration_text_edge_cases() {
        assert_eq!(parse_duration_text_to_ms("3:45"), 225_000);
        assert_eq!(parse_duration_text_to_ms(" 03:5 "), 185_000);
        assert_eq!(parse_duration_text_to_ms("1:02:33"), 3_753_000);
        // 单段 / 四段 / 非数字 / 空串均回退 0
        assert_eq!(parse_duration_text_to_ms("45"), 0);
        assert_eq!(parse_duration_text_to_ms("1:2:3:4"), 0);
        assert_eq!(parse_duration_text_to_ms("abc:def"), 0);
        assert_eq!(parse_duration_text_to_ms(""), 0);
        // 超出 u64 的恶意数字不 panic
        assert_eq!(parse_duration_text_to_ms("99999999999999999999:00"), 0);
        // u64 范围内的极端值走饱和运算不溢出
        assert_eq!(
            parse_duration_text_to_ms("18446744073709551615:59"),
            u64::MAX
        );
    }

    #[test]
    fn duration_like_artist_text_is_not_matched() {
        // "12:34AM" 含字母不算时长
        assert!(!looks_like_duration("12:34AM"));
        assert!(!looks_like_duration("Artist • 3:45"));
        assert!(!looks_like_duration(":45"));
        assert!(looks_like_duration(" 3:45 "));
    }
}
