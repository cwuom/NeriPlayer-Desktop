[English](#english) | [中文](#中文)

# neri-platform

## 中文

在线平台的协议层：请求、签名/加密、响应解析与登录态模型。依赖 `neri-common`，不依赖 Tauri。

| 模块 | 职责 |
| --- | --- |
| `api::netease` | 网易云客户端与 weapi/eapi/linuxapi 加密 |
| `api::qq` | QQ 音乐搜索、歌词与取流 |
| `api::bilibili` | B 站客户端（WBI 签名、合集/分 P、字幕） |
| `api::youtube` | YouTube Music：InnerTube 目录、取流客户端选择、`player.js` 的 sig/n 求解（受限 QuickJS） |
| `api::lrclib` | LRCLIB 歌词 |
| `api::transport` | 代理兜底通道与统一的 JSON 响应解析 |
| `auth::state` | 各平台登录态与 Cookie 条目 |
| `auth::cookie_jar` | reqwest Cookie Jar 的注入、回读、过期与白名单解析（持久化在宿主） |
| `auth::coordination` | 登录/登出操作的并发协调 |

宿主注入：YouTube 取流需要的「是否绕过系统代理」和隐藏 WebView 里的 PoToken 铸造通过 `api::youtube::host::YouTubeHost` 注入；桌面端实现在 `src-tauri/src/platform_webview/`。网易云评论令牌同样依赖 WebView，整体放在宿主。

约束：

- 签名与加密方案必须与平台现有实现逐字节一致，改动前先对照 Android 端。
- `api::youtube::challenge` 在 QuickJS 里执行远端脚本，不要向脚本暴露网络、文件或 IPC 能力。

```bash
cargo test -p neri-platform
```

## English

Protocol layer for online platforms: requests, signing/encryption, response parsing, and login state models. Depends on `neri-common`, not on Tauri.

| Module | Responsibility |
| --- | --- |
| `api::netease` | NetEase client and weapi/eapi/linuxapi encryption |
| `api::qq` | QQ Music search, lyrics, and stream URLs |
| `api::bilibili` | Bilibili client (WBI signing, collections/parts, subtitles) |
| `api::youtube` | YouTube Music: InnerTube catalog, playback client selection, sig/n solving for `player.js` in a restricted QuickJS |
| `api::lrclib` | LRCLIB lyrics |
| `api::transport` | Proxy fallback transport and shared JSON response parsing |
| `auth::state` | Per-platform login state and cookie entries |
| `auth::cookie_jar` | Injecting, reading back, expiring, and allow-list parsing of cookies in the reqwest jar (persistence lives in the host) |
| `auth::coordination` | Concurrency coordination of login/logout operations |

Host injection: YouTube playback needs "bypass the system proxy" and PoToken minting in a hidden WebView; both come through `api::youtube::host::YouTubeHost`, implemented in `src-tauri/src/platform_webview/`. The NetEase comment token also needs a WebView and lives entirely in the host.

Constraints:

- Signing and encryption must match the platforms byte for byte; compare with the Android client before changing them.
- `api::youtube::challenge` runs remote scripts in QuickJS; never expose network, file, or IPC capabilities to them.

```bash
cargo test -p neri-platform
```
