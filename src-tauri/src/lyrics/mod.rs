pub mod parser;
pub mod manager;
mod external;
pub mod matcher;
pub mod sanitize;
mod ttml;

use parser::LyricLine;

/// 歌词实际取自哪里；前端按它选默认偏移（对齐 Android LyricSourcePreference）
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricSource {
    /// 音频旁的 .lrc/.ttml 文件或内嵌歌词
    Local,
    Netease,
    Qq,
    Lrclib,
    Youtube,
    AmllTtml,
    Kugou,
}

/// 一次歌词获取的结果；没找到时来源为空、没有歌词行
#[derive(Debug, Default, serde::Serialize)]
pub struct FetchedLyrics {
    pub source: Option<LyricSource>,
    pub lines: Vec<LyricLine>,
}

impl FetchedLyrics {
    pub fn from(source: LyricSource, lines: Vec<LyricLine>) -> Self {
        Self { source: (!lines.is_empty()).then_some(source), lines }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetched_lyrics_report_their_source_only_when_found() {
        let lines = parser::parse_lrc("[00:01.00]Hello");
        let json = serde_json::to_value(FetchedLyrics::from(LyricSource::AmllTtml, lines)).unwrap();
        assert_eq!(json["source"], "amll_ttml");
        assert_eq!(json["lines"].as_array().unwrap().len(), 1);
        let empty = serde_json::to_value(FetchedLyrics::from(LyricSource::Kugou, Vec::new())).unwrap();
        assert!(empty["source"].is_null(), "没找到歌词时不报来源");
    }

    #[tokio::test]
    async fn a_sidecar_next_to_the_audio_is_reported_as_local() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("Song.mp3");
        std::fs::write(&audio, b"").unwrap();
        std::fs::write(dir.path().join("Song.mp3.lrc"), "[00:01.00]Hello\n[00:05.00]World").unwrap();
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let fetched = manager::LyricsManager::new(&http)
            .fetch_lyrics("Song", "Artist", 0, audio.to_str(), None, None, None)
            .await
            .unwrap();
        assert_eq!(fetched.source, Some(LyricSource::Local));
        assert_eq!(fetched.lines.len(), 2);
    }
}
