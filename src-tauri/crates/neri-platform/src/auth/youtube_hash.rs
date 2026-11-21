// YouTube SAPISIDHASH 认证头计算
// 算法: SHA1(timestamp + " " + SAPISID + " " + origin)
// 头格式: SAPISIDHASH {timestamp}_{sha1hex}

use sha1::{Digest, Sha1};

/// 构建 YouTube Web 使用的完整 SID Authorization 头
pub fn build_youtube_authorization(
    sapisid: Option<&str>,
    sapisid_1p: Option<&str>,
    sapisid_3p: Option<&str>,
    apisid: Option<&str>,
    origin: &str,
    user_session_id: &str,
) -> Option<String> {
    let timestamp = chrono::Utc::now().timestamp();
    build_youtube_authorization_at(
        sapisid,
        sapisid_1p,
        sapisid_3p,
        apisid,
        origin,
        user_session_id,
        timestamp,
    )
}

fn build_youtube_authorization_at(
    sapisid: Option<&str>,
    sapisid_1p: Option<&str>,
    sapisid_3p: Option<&str>,
    apisid: Option<&str>,
    origin: &str,
    user_session_id: &str,
    timestamp: i64,
) -> Option<String> {
    let primary_sapisid = sapisid.or(sapisid_3p).or(sapisid_1p).or(apisid);
    let mut headers = Vec::with_capacity(3);
    if let Some(sid) = primary_sapisid.filter(|value| !value.is_empty()) {
        headers.push(build_sid_authorization(
            "SAPISIDHASH",
            sid,
            origin,
            timestamp,
            user_session_id,
        ));
    }
    if let Some(sid) = sapisid_1p.filter(|value| !value.is_empty()) {
        headers.push(build_sid_authorization(
            "SAPISID1PHASH",
            sid,
            origin,
            timestamp,
            user_session_id,
        ));
    }
    if let Some(sid) = sapisid_3p.filter(|value| !value.is_empty()) {
        headers.push(build_sid_authorization(
            "SAPISID3PHASH",
            sid,
            origin,
            timestamp,
            user_session_id,
        ));
    }

    (!headers.is_empty()).then(|| headers.join(" "))
}

fn build_sid_authorization(
    scheme: &str,
    sid: &str,
    origin: &str,
    timestamp: i64,
    user_session_id: &str,
) -> String {
    let input = if user_session_id.is_empty() {
        format!("{timestamp} {sid} {origin}")
    } else {
        format!("{user_session_id} {timestamp} {sid} {origin}")
    };
    let hash = hex::encode(Sha1::digest(input.as_bytes()));
    if user_session_id.is_empty() {
        format!("{scheme} {timestamp}_{hash}")
    } else {
        format!("{scheme} {timestamp}_{hash}_u")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sapisidhash_format() {
        let result = build_sid_authorization(
            "SAPISIDHASH",
            "test_sapisid",
            "https://music.youtube.com",
            1_700_000_000,
            "",
        );
        assert!(result.starts_with("SAPISIDHASH "));
        let parts: Vec<&str> = result["SAPISIDHASH ".len()..].split('_').collect();
        assert_eq!(parts.len(), 2);
        // timestamp 部分是数字
        assert!(parts[0].parse::<i64>().is_ok());
        // hash 部分是 40 字符 hex
        assert_eq!(parts[1].len(), 40);
    }

    #[test]
    fn builds_all_available_sid_hashes() {
        let result = build_youtube_authorization_at(
            Some("main"),
            Some("one"),
            Some("three"),
            None,
            "https://music.youtube.com",
            "session",
            1_700_000_000,
        )
        .unwrap();

        assert!(result.contains("SAPISIDHASH 1700000000_"));
        assert!(result.contains("SAPISID1PHASH 1700000000_"));
        assert!(result.contains("SAPISID3PHASH 1700000000_"));
        assert_eq!(result.matches("_u").count(), 3);
    }
}
