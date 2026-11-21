//! 在线平台协议层：网易云、QQ 音乐、Bilibili、YouTube、LRCLIB 等客户端，请求签名/加密，
//! 拉流请求头策略与平台登录态模型。YouTube PoToken 等需要浏览器环境的能力
//! 通过 [`api::youtube::host::YouTubeHost`] 由宿主注入，本 crate 不依赖 Tauri

pub mod api;
pub mod auth;
