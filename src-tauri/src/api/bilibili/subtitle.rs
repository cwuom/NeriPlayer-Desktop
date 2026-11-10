use crate::error::{AppError, AppResult};
use reqwest::Url;

/// Subtitle URLs originate in player metadata. Credentials may only reach Bilibili's CDN.
pub fn subtitle_url(raw: &str) -> AppResult<Url> {
    let raw = raw.trim();
    let normalized = if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let invalid = || AppError::Api("Invalid Bilibili subtitle URL".into());
    let mut url = Url::parse(&normalized).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty() || url.password().is_some()
        || url.port().is_some()
        || !url.host_str().is_some_and(|host| {
            ["hdslb.com", "bilibili.com"].iter().any(|domain| {
                host == *domain || host.ends_with(&format!(".{domain}"))
            })
        })
    {
        return Err(invalid());
    }
    url.set_scheme("https").map_err(|_| invalid())?;
    Ok(url)
}

pub fn subtitle_referer(bvid: &str) -> AppResult<String> {
    if !bvid.starts_with("BV") || bvid.len() > 32 || !bvid.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::Api("Invalid Bilibili video ID".into()));
    }
    Ok(format!("https://www.bilibili.com/video/{bvid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_official_cdn_urls_and_preserves_signatures() {
        for raw in ["//aisubtitle.hdslb.com/a.json?auth_key=test", "http://aisubtitle.hdslb.com/a.json?auth_key=test", "aisubtitle.hdslb.com/a.json?auth_key=test"] {
            assert_eq!(subtitle_url(raw).unwrap().as_str(), "https://aisubtitle.hdslb.com/a.json?auth_key=test");
        }
        assert!(subtitle_url("https://i0.hdslb.com/a.json").is_ok());
        assert!(subtitle_url("https://www.bilibili.com/a.json").is_ok());
    }

    #[test]
    fn rejects_external_hosts_credentials_and_non_https_transports() {
        for raw in ["", "https://hdslb.com.evil.test/a", "https://evilhdslb.com/a", "https://127.0.0.1/a", "file:///a", "ftp://i0.hdslb.com/a", "https://user:pass@i0.hdslb.com/a", "https://i0.hdslb.com:8080/a"] {
            assert!(subtitle_url(raw).is_err(), "accepted {raw}");
        }
        assert!(subtitle_referer("BV1xx411c7mD").is_ok());
        assert!(subtitle_referer("BV1\r\nCookie: secret").is_err());
    }
}
