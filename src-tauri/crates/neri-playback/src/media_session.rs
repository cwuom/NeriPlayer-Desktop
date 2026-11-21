// 系统媒体会话集成：Windows SMTC / Linux MPRIS
// 通过 souvlaki 库实现跨平台媒体键控制和系统通知中心展示

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use std::sync::mpsc;
use std::time::Duration;

/// 媒体会话控制器，管理 SMTC/MPRIS 状态
pub struct MediaSessionController {
    cmd_tx: mpsc::Sender<MediaSessionCmd>,
}

/// 将封面标识规整为系统媒体会话可用的 URI：
/// http(s)/file/data 原样保留，裸本地路径转 file://，无法转换则丢弃
/// （传裸路径给 souvlaki 在 MPRIS/SMTC 上都不会显示）
fn normalize_cover_uri(raw: String) -> Option<String> {
    let is_uri = raw.starts_with("http://")
        || raw.starts_with("https://")
        || raw.starts_with("file://")
        || raw.starts_with("data:");
    if is_uri {
        return Some(raw);
    }
    url::Url::from_file_path(&raw)
        .ok()
        .map(|uri| uri.to_string())
}

enum MediaSessionCmd {
    UpdateMetadata {
        title: String,
        artist: String,
        album: String,
        cover_url: Option<String>,
        duration_ms: u64,
    },
    UpdatePlayback {
        is_playing: bool,
        position_ms: u64,
    },
    Stop,
}

/// 媒体事件回调类型
#[derive(Debug, Clone)]
pub enum MediaAction {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    SeekTo(u64), // position_ms
}

impl MediaSessionController {
    /// 创建媒体会话控制器
    /// - `hwnd`: Windows 窗口句柄（Windows 必需）
    /// - `action_tx`: 媒体事件回调通道
    pub fn new(
        hwnd: Option<*mut std::ffi::c_void>,
        action_tx: mpsc::Sender<MediaAction>,
    ) -> Option<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<MediaSessionCmd>();
        // 将 *mut c_void 转为 usize 以满足 Send 要求（线程安全由调用方保证）
        let hwnd_usize: Option<usize> = hwnd.map(|p| p as usize);

        // 在独立线程中运行 MediaControls（Windows SMTC 需要）
        std::thread::Builder::new()
            .name("media-session".into())
            .spawn(move || {
                let config = PlatformConfig {
                    dbus_name: "neri_player",
                    display_name: "Neri Player",
                    hwnd: hwnd_usize.map(|u| u as *mut std::ffi::c_void),
                };

                let mut controls = match MediaControls::new(config) {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!(target: "media-session", "Failed to create MediaControls: {:?}", e);
                        return;
                    }
                };

                // 注册事件回调
                let tx = action_tx;
                if let Err(e) = controls.attach(move |event: MediaControlEvent| {
                    let action = match event {
                        MediaControlEvent::Play => Some(MediaAction::Play),
                        MediaControlEvent::Pause => Some(MediaAction::Pause),
                        MediaControlEvent::Toggle => Some(MediaAction::Toggle),
                        MediaControlEvent::Next => Some(MediaAction::Next),
                        MediaControlEvent::Previous => Some(MediaAction::Previous),
                        MediaControlEvent::SetPosition(pos) => {
                            Some(MediaAction::SeekTo(pos.0.as_millis() as u64))
                        }
                        _ => None,
                    };
                    if let Some(a) = action {
                        let _ = tx.send(a);
                    }
                }) {
                    log::error!(target: "media-session", "Failed to attach handler: {:?}", e);
                    return;
                }

                log::info!(target: "media-session", "MediaControls initialized");

                // 媒体事件由 souvlaki 分派，命令通道无需定时轮询
                loop {
                    match cmd_rx.recv() {
                        Ok(MediaSessionCmd::UpdateMetadata {
                            title,
                            artist,
                            album,
                            cover_url,
                            duration_ms,
                        }) => {
                            let dur = if duration_ms > 0 {
                                Some(Duration::from_millis(duration_ms))
                            } else {
                                None
                            };
                            // 本地封面是裸文件路径，而 MPRIS mpris:artUrl /
                            // SMTC 缩略图都要求 URI，必须转成 file:// 才能显示
                            let cover_url = cover_url.and_then(normalize_cover_uri);
                            let _ = controls.set_metadata(MediaMetadata {
                                title: Some(&title),
                                artist: Some(&artist),
                                album: Some(&album),
                                cover_url: cover_url.as_deref(),
                                duration: dur,
                            });
                        }
                        Ok(MediaSessionCmd::UpdatePlayback {
                            is_playing,
                            position_ms,
                        }) => {
                            let progress =
                                Some(MediaPosition(Duration::from_millis(position_ms)));
                            let playback = if is_playing {
                                MediaPlayback::Playing { progress }
                            } else {
                                MediaPlayback::Paused { progress }
                            };
                            let _ = controls.set_playback(playback);
                        }
                        Ok(MediaSessionCmd::Stop) => {
                            let _ = controls.set_playback(MediaPlayback::Stopped);
                        }
                        Err(mpsc::RecvError) => {
                            log::warn!(target: "media-session", "channel disconnected, exiting");
                            break;
                        }
                    }
                }
            })
            .ok()?;

        Some(Self { cmd_tx })
    }

    /// 更新曲目元数据
    pub fn update_metadata(
        &self,
        title: &str,
        artist: &str,
        album: &str,
        cover_url: Option<&str>,
        duration_ms: u64,
    ) {
        let _ = self.cmd_tx.send(MediaSessionCmd::UpdateMetadata {
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            cover_url: cover_url.map(|s| s.to_string()),
            duration_ms,
        });
    }

    /// 更新播放状态和位置
    pub fn update_playback(&self, is_playing: bool, position_ms: u64) {
        let _ = self.cmd_tx.send(MediaSessionCmd::UpdatePlayback {
            is_playing,
            position_ms,
        });
    }

    /// 停止媒体会话
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(MediaSessionCmd::Stop);
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_cover_uri;

    #[test]
    fn remote_and_uri_covers_pass_through() {
        assert_eq!(
            normalize_cover_uri("https://example.com/a.jpg".into()),
            Some("https://example.com/a.jpg".into())
        );
        assert_eq!(
            normalize_cover_uri("file:///tmp/a.jpg".into()),
            Some("file:///tmp/a.jpg".into())
        );
        assert_eq!(
            normalize_cover_uri("data:image/png;base64,AAAA".into()),
            Some("data:image/png;base64,AAAA".into())
        );
    }

    #[test]
    fn bare_local_path_becomes_file_uri() {
        let dir = std::env::temp_dir().join("neri cover.jpg");
        let raw = dir.to_string_lossy().to_string();
        let uri = normalize_cover_uri(raw).expect("absolute path converts");
        assert!(uri.starts_with("file://"), "got {uri}");
        // 空格等必须百分号转义
        assert!(!uri.contains(' '));
    }

    #[test]
    fn relative_path_is_dropped() {
        assert_eq!(normalize_cover_uri("covers/a.jpg".into()), None);
    }
}
