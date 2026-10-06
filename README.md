[English](./README_EN.md) | [中文](./README.md)

<h1 align="center">NeriPlayer Desktop</h1>

<div align="center">

<h3>✨ 把多源在线播放、本地管理、歌词体验和自建同步带到 Windows / macOS / Linux 🎵</h3>

<p>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/cwuom/NeriPlayer-Desktop/total?style=social" />
  </a>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Release" src="https://img.shields.io/github/v/release/cwuom/NeriPlayer-Desktop?include_prereleases&label=Release" />
  </a>
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-desktop-4C8BF5" />
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml">
    <img alt="CI" src="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml/badge.svg" />
  </a>
  <a href="./LICENSE">
    <img alt="License" src="https://img.shields.io/badge/License-MIT-green" />
  </a>
  <a href="https://t.me/ouom_pub">
    <img alt="Telegram" src="https://img.shields.io/badge/Telegram-@ouom__pub-blue" />
  </a>
</p>

<p>
  <img src="app-icon.png" width="200" alt="NeriPlayer Desktop logo" />
</p>

<p>
本项目的名称及图标灵感来源于《星空鉄道とシロの旅》中的角色「风又音理」。
</p>

<p>
NeriPlayer Desktop 是
<a href="https://github.com/cwuom/NeriPlayer">NeriPlayer (Android)</a>
的桌面移植，基于 <strong>Tauri 2 + Vue 3 + Rust</strong> 构建，
与 Android 端共享同一套云同步协议与一起听协议，
围绕「多源探索、在线播放、本地可控、数据自持」持续打磨。
</p>

🚧 <strong>开发中 / Work in progress</strong>

</div>

> [!CAUTION]
> **项目与文档状态声明（请先阅读）**
>
> - 桌面端仍处于早期开发阶段，本文档中的相当一部分内容目前**仅为占位描述**：
>   对应功能可能尚未实现、仅部分实现，或与描述存在偏差。
> - 部分实现尚不完善，极端情况下**可能导致数据损坏或丢失**。
>   请勿把本应用当作唯一数据副本，重要歌单与配置请先导出备份。
> - 维护者精力有限，本仓库未来**可能进入无人维护状态**。
>   桌面端的理想是依靠社区共同推进——欢迎认领 Issue、提交 PR，
>   或在维护停滞时进行 fork 延续。
> - 文档与实际行为不一致时，请以源码为准，并欢迎顺手修正文档。

---

> [!WARNING]
> 本项目仅供学习与研究使用，请勿将其用于任何非法用途。
>
> 本项目及维护者不接受任何形式的赞助、捐赠或商业资助。

---

> [!NOTE]
> NeriPlayer Desktop 不提供公共云端曲库或媒体分发服务。
> 在线音频能力依赖用户在第三方平台上的账号授权，
> 会员或受限内容仍需遵循原平台规则。

---

## 快速定位 / Start here

如果你只是想体验应用，请看 [快速体验](#快速体验--getting-started)。
如果你想了解项目能力，请看 [项目亮点](#项目亮点--why-it-stands-out)
和 [核心特性](#核心特性--key-features)。
如果你准备贡献代码，请直接阅读 [CONTRIBUTING.md](./CONTRIBUTING.md)。
如果你关心桌面端和 Android 端的关系，请看
[与 Android 端的关系](#与-android-端的关系--relationship-with-the-android-app)。

```text
NeriPlayer Desktop
├── 多源在线播放：网易云 / QQ 音乐 / Bilibili / YouTube Music + 本地文件
├── 本地优先数据：歌单、收藏、最近播放、播放统计、下载、设置
├── 可选自有同步：GitHub / WebDAV 元数据同步（与 Android 端互通）
├── 丰富播放体验：Rust 音频引擎、AMLL 逐字歌词、流体着色器背景、系统媒体键
└── 一起听：与 Android 端同协议的实时同步房间
```

---

## 项目简介 / About

NeriPlayer Desktop 是一个 **Tauri 2** 桌面应用：**Vue 3** 前端与 **Rust**
后端两个进程通过 IPC 桥通信。Rust 侧独占音频播放、全部平台网络请求、
请求签名/加密、文件系统与云同步；前端从不直接访问音乐平台。

当前定位：

- **账号即能力**：通过第三方平台授权启用搜索、播放、歌单和收藏夹访问。
  登录使用内置 WebView 窗口自动捕获 Cookie（含 HttpOnly），也支持手动导入。
- **本地优先**：歌单、收藏、最近播放、播放统计、下载文件、设置与授权信息
  默认保存在本机。
- **可选同步**：可将歌单、收藏、最近播放和播放统计同步到用户自己的
  GitHub 仓库或 WebDAV 远端文件，数据格式与 Android 端完全互通。
- **尊重隐私与账号安全**：同步策略刻意保持去中心化，数据写入用户自己
  控制的远端；不向第三方音乐平台回传本地播放历史与统计，
  避免触发平台风控。
- **对齐 Android 端**：功能集、交互习惯、同步数据模型、一起听协议
  与版本号格式都以 Android 端为基准持续对齐。

---

## 项目亮点 / Why it stands out

- **Rust 自研播放引擎，不是包一层系统播放器**：
  `PlayerEngine` 基于 **cpal** 输出 + **symphonia** 解码
  （MP3 / AAC / FLAC / OGG Vorbis / WAV / PCM / ADPCM / MP4 容器），
  支持本地文件、内存字节与网络流三种播放路径；网络流使用渐进式缓冲
  （growing buffer）与自适应 Range 拉流，弱网下仍可持续播放，
  分片音频支持快速 seek 与暂停时静默拖动。
- **听感可细调**：暂停/恢复淡入淡出、切歌交叉淡入淡出（时长可调）、
  倍速、响度增益、按歌曲实时响度均衡，以及 5 频段均衡器
  （60 / 230 / 910 / 3600 / 14000 Hz，预设 + 手动，±15 dB）。
- **仿 Apple Music 的歌词体验**：
  歌词渲染基于 [applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics)
  （以子模块内嵌），支持逐字/逐词高亮、翻译歌词、歌词模糊、字号调节、
  全局逐源偏移与单曲偏移；网易云 YRC 逐字歌词端到端接入。
- **歌词补全不是单一来源**：
  按 QQ 音乐 → 网易云 → LRCLIB 逐级匹配，优先使用平台曲目 ID，
  无 ID 时按曲名、歌手和时长搜索匹配，时长误差参与打分。
- **GLSL 流体动态背景**：
  播放页由 WebGL `HyperBackground` 着色器逐帧渲染流体背景，
  基于封面取色并接入实时音频电平做音频响应；
  也支持封面模糊背景与自定义背景图（模糊度/透明度可调）。
- **跨端同步是字段级互通，不是"都能连 WebDAV"**：
  Rust 侧的 ProtoBuf 模型字段号与 Android 端 `SyncDataModels.kt` 的
  `@ProtoNumber` 逐一对齐，三路合并对照基准快照裁决冲突，
  当前写入 Android V4 的 ProtoBuf 清单和 Zstd 分块归档，兼容读取旧 JSON、GZIP 和 V3 归档。
- **一起听与 Android 端同协议**：
  桌面客户端连接同一套 Cloudflare Workers 服务端，
  支持房间、角色权限、队列同步、循环/随机模式同步、
  成员控制请求与直链共享开关。
- **桌面化不是"网页装壳"**：
  无边框自绘标题栏（macOS 使用原生 unified toolbar 居中红绿灯）、
  全键盘快捷键、指针拖拽排序、多选工具栏、右键上下文菜单、
  系统媒体键与 SMTC / MPRIS 媒体会话集成、单实例保护。
- **凭据安全按桌面威胁模型设计**：
  Release 构建使用应用侧加密文件存储平台 Cookie、GitHub Token 与
  WebDAV 密码（对标 Android 的 EncryptedSharedPreferences），
  避免未签名应用反复触发系统钥匙串弹窗；日志经过脱敏处理。
- **诊断闭环**：
  Debug 页内置平台连通性探针、实时日志查看器（可暂停）、
  崩溃报告管理与调试报告导出；文件日志与日志级别可在设置中开启。

---

## 快速体验 / Getting Started

### a. 下载 Release 版本（推荐）

1. 前往 [GitHub Releases](https://github.com/cwuom/NeriPlayer-Desktop/releases)
2. 如何选择安装包？

| 平台 | 产物 | 说明 |
| --- | --- | --- |
| Windows | `.msi` / `.exe` (NSIS) | x64，需要 WebView2 运行时（Win10/11 通常已内置） |
| macOS (Apple Silicon) | `*-adhoc.dmg` (arm64) | macOS 11.0+ |
| macOS (Intel) | `*-adhoc.dmg` (x64) | macOS 10.15+ |
| Linux | `.deb` / `.rpm` / `.AppImage.tar.gz` | x64，依赖 WebKitGTK 4.1 |

> [!IMPORTANT]
> macOS 安装包为 ad-hoc 签名（未经公证）。首次打开如被 Gatekeeper 拦截，
> 请在访达中右键 → 打开，或执行
> `xattr -cr /Applications/NeriPlayer.app` 后再启动。
> Windows 安装包未做代码签名，SmartScreen 提示属预期现象。

### b. 下载 CI 版本

前往 [GitHub Actions](https://github.com/cwuom/NeriPlayer-Desktop/actions)
的 `Artifacts` 工作流，下载最近一次成功构建的四平台产物
（`NeriPlayer-Windows-x64 / macOS-arm64 / macOS-x64 / Linux-x64`）。

### c. 本地构建

前置要求：**Node.js 20+**、**pnpm 9.15.9**（`corepack enable` 即可）、
**Rust 1.95.0**（`rust-toolchain.toml` 会自动固定），以及平台依赖：

- **Windows**：Visual Studio C++ Build Tools、WebView2 运行时
- **macOS**：Xcode Command Line Tools
- **Linux**（Debian/Ubuntu 示例）：
  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
    librsvg2-dev patchelf libasound2-dev
  ```

构建步骤：

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install
pnpm tauri dev      # 开发运行（Vite :1420 + Rust shell）
pnpm tauri build    # 生产打包，产物在 src-tauri/target/release/bundle/
```

仓库依赖 Git 子模块（`vendor/applemusic-like-lyrics`），
克隆时请务必带 `--recursive`，或补执行
`git submodule update --init --recursive`。

首次启动后，在「设置」中登录所需平台账号即可解锁在线能力；
连续点击版本号 **7 次** 可解锁开发者模式，侧栏会出现 `Debug` 页。

---

## 核心特性 / Key Features

- 🎧 **多源探索与播放**：
  支持网易云音乐、QQ 音乐、Bilibili、YouTube Music 与本地音频播放。
- 🏠 **首页推荐与继续播放**：
  最近播放、网易云每日推荐歌单/歌曲、热门与私人雷达歌曲栏目、
  跨平台歌单入口；国际化模式下且已登录 YouTube 时，
  优先展示 YouTube Music 首页货架。
- 🔍 **分层搜索能力**：
  `探索` 页按平台独立搜索（网易云 / Bilibili / YouTube Music），
  并提供 Bilibili 与 YouTube 发现货架；
  播放页元数据与歌词补全使用网易云 + QQ 音乐，并接入 LRCLIB。
- 🗂️ **媒体库分类浏览**：
  本地 / 收藏（歌单 + 已关注艺术家）/ 下载 / 网易云（歌单 + 专辑）/
  Bilibili 收藏夹 / YouTube Music 歌单，每个分类独立搜索，
  关键词切换分类后仍保留。
  收藏歌手按网易云 / B站 / YouTube 分平台展示，同步完成后自动刷新；
  支持站内歌手详情、作品分页与关注，网易云和 YouTube 可拉取远端关注。
- 🧠 **播放核心**：
  队列管理、随机/循环模式、播放请求代际防串扰、失败恢复、
  进度与播放模式记忆（可关）。
  `设置 → 播放源` 可选择 YouTube 首选客户端，失败时继续尝试其它来源；
  网易云仅有试听或无播放权限时，可独立开启匹配本地音频或 B站兜底，
  本地匹配优先，原歌曲的队列与同步身份保留，下载不使用替代音源。
- 🌊 **流式播放**：
  渐进式缓冲、自适应 Range 拉流、分片音频快速 seek、
  暂停时静默拖动、在途请求去重与预取。
- 🎚️ **播放音效**：
  倍速、响度增益、按歌曲实时响度均衡、5 频段均衡器（预设 + 手动）。
- ⬇️ **应用内下载**：
  多平台音频下载、歌词/翻译歌词/封面 sidecar 落盘、
  文件名模板、自定义下载目录、进度事件、批量取消、
  损坏校验与失效清理、在文件管理器中显示。
- 🩷 **本地歌单与收藏**：
  创建/重命名/删除/排序歌单、多选批量操作、指针拖拽排序、
  网易云歌曲喜欢/取消喜欢，收藏歌单按来源平台跳转详情。
- 🧑‍🎤 **网易云艺术家**：
  艺术家详情、热门歌曲与专辑分页浏览，收藏页提供艺术家分类入口。
- 📊 **播放统计**：
  按歌曲稳定身份记录播放次数、收听时长与每日统计桶，
  提供概览视图，并参与 GitHub / WebDAV 同步。
- 🕘 **最近播放**：
  独立页面浏览与管理，删除记录参与云同步（跨端删除不复活）。
- ☁️ **GitHub / WebDAV 同步**：
  同步本地歌单、收藏、最近播放与播放统计，三路合并，
  使用与 Android 对齐的 V4 分块归档；另有歌单 JSON 与完整配置的导入/导出。
- 🎧 **一起听**：
  创建/加入房间，WebSocket 实时同步播放状态与队列，
  支持成员控制开关、成员进出自动暂停、循环/随机模式同步、
  直链共享开关与自定义服务端地址。
- 🌈 **个性化与主题**：
  主题色与动态取色、浅色/深色、主题色平滑过渡、自定义背景图
  （模糊/透明度可调）、界面显示项开关（封面角标、工具栏、
  音质切换、音频规格等）、默认启动页；
  界面语言支持简体中文、繁體中文、English、日本語。
- ✨ **播放页动效与歌词**：
  WebGL 流体动态背景（音频响应）、封面模糊背景、AMLL 逐字歌词、
  翻译歌词、歌词模糊与字号、逐源歌词偏移 + 单曲歌词偏移。
- 🪟 **桌面系统集成**：
  系统媒体键与 SMTC / MPRIS 媒体会话、单实例保护、
  无边框自绘标题栏（macOS 原生红绿灯布局）。
- ⌨️ **全键盘快捷键**：
  见 [键盘快捷键](#键盘快捷键--keyboard-shortcuts)。
- 🧾 **登录友好**：
  内置 WebView 登录窗口自动捕获 Cookie（含 HttpOnly），
  支持手动 Cookie 导入与登录态校验。
- 🧯 **存储管理**：
  存储占用分组统计与缓存清理，不影响用户主动下载的内容。
- 🛠️ **开发者模式与调试**：
  平台连通性探针、实时日志、崩溃报告查看/清理、调试报告导出。

---

## 平台现状 / Platform Status

- **网易云音乐**：
  WebView 登录、搜索、每日推荐/精选/高质量歌单、用户歌单与订阅专辑、
  歌单/专辑详情、艺术家详情（热门歌曲/专辑分页）、喜欢/取消喜欢、
  多音质取流、歌词（含 YRC 逐字与翻译）、下载。
- **QQ 音乐**：
  搜索、多音质取流、歌词与播放页元数据补全；
  当前无账号登录态，能力受游客接口限制。
- **Bilibili**：
  WebView 登录、视频搜索、创建/订阅收藏夹浏览、DASH 音频取流、
  封面代理加载（Referer 处理）、下载、发现货架。
- **YouTube Music**：
  WebView 登录、首页 Feed、歌单浏览与详情、搜索、取流、下载、
  账号资料刷新。
  取流仅处理音频签名，网页令牌短等待后尝试其它客户端，快速切歌会取消旧解析。
- **本地音频**：
  目录扫描导入、本地歌单管理、本地艺术家聚合与详情页。
- **LRCLIB**：
  外部歌词来源，带时长精确匹配。

---

## 与 Android 端的关系 / Relationship with the Android app

桌面端刻意保持与 [NeriPlayer (Android)](https://github.com/cwuom/NeriPlayer)
的行为一致：

- **同步协议互通**：ProtoBuf 字段号与 Android `SyncDataModels.kt` 的
  `@ProtoNumber` 逐一对齐，同一个 GitHub 仓库 / WebDAV 远端可被
  两端交替读写，三路合并语义一致（含删除记录与成员 token）。
- **一起听互通**：桌面客户端与 Android 客户端连接同一套
  Cloudflare Workers 服务端，可同处一个房间。
- **交互对齐**：媒体库分类、设置项、调试页、歌词行为等
  以 Android 端为基准移植。
- **版本号格式一致**：`<git短哈希>.<MMddHHmm>`。

Android 端独有的移动平台能力（USB 独占播放、悬浮/状态栏歌词、
SAF 目录、安全模式等）不在桌面端范围内；桌面端仍在持续补齐
其余能力，遇到两端行为不一致的场景欢迎提 Issue。

---

## 实现概览 / Implementation Notes

### 构建与版本

- 前端：Vue 3.5 + Pinia + Vue Router + vue-i18n + Vite 6 + TypeScript 5.8
- 后端：Rust 2021（工具链固定 `1.95.0`）+ Tauri 2
- 包管理：pnpm `9.15.9`（pinned），CI 使用 Node 20
- 版本名格式：`<git短哈希>.<MMddHHmm>`（Asia/Taipei 时区），
  由 `src-tauri/build.rs` 注入 `BUILD_UUID / BUILD_TIMESTAMP / BUILD_VERSION`，
  应用内通过 `get_build_info` 命令展示
- CI（GitHub Actions）：
  - `CI`：前端 `vue-tsc + vite build`；后端 `cargo check --locked`
    覆盖 Windows / macOS(arm64+x64) / Linux 四个 target
  - `Artifacts`：main 分支推送即构建四平台产物
  - `Release`：推送 `v*` 标签后构建并发布 GitHub Release
- 打包目标：Windows MSI + NSIS；macOS DMG（Intel 10.15+ / arm64 11.0+，
  ad-hoc 签名）；Linux deb + rpm + AppImage

### 进程与 IPC

- 前端从不直接访问音乐平台；每个平台请求都经由 Rust
  `#[tauri::command]`，命令统一注册在 `src-tauri/src/main.rs` 的
  `generate_handler![...]` 中（约 140 个命令，按
  player / library / search / lyrics / settings / auth / recommend /
  sync / download / listen_together / stats / storage / debug 分域）。
- 后端通过事件向前端推送状态：`player:position`、`player:audio-level`、
  `player:track-ended`、`download-progress`、`playlists-changed`、
  `playback-stats-changed`、`lt:*`、`media:*` 等；
  后台 ticker 线程轮询播放器并发出进度与结束事件。
- `AppState` 是唯一被 `.manage()` 的共享状态：播放引擎、播放队列、
  可重建的 `reqwest` 客户端（用于切换代理直连）、共享 Cookie jar、
  各平台登录态、一起听会话与下载任务注册表。

### 音频引擎

- `cpal` 输出 + `symphonia` 解码，`lofty` 读取音频标签；
  播放路径覆盖本地文件、内存字节与网络流（渐进缓冲 + 自适应 Range）。
- 淡入淡出与交叉淡入淡出在引擎层实现；EQ、响度均衡、响度增益、
  倍速在解码管线上生效。
- 播放请求带代际（generation）防止旧请求覆盖新请求；
  seek 结果按代际采纳，避免拖动竞态。
- 系统媒体会话经 `souvlaki` 接入 SMTC（Windows）/ MPRIS（Linux）/
  macOS Now Playing，媒体键动作通过通道回传前端。

### 平台 API 与请求签名

- 每个平台一个模块（`api/netease`、`api/qq`、`api/bilibili`、
  `api/youtube`）加 `api/lrclib`；签名/加密逻辑隔离存放：
  - `netease/crypto.rs` — WEAPI / EAPI / linuxapi 的 AES + RSA 加密
  - `bilibili/wbi.rs` — WBI mixin-key 参数签名
  - `auth/youtube_hash.rs` — SAPISIDHASH 授权头生成
- 登录通过内置 WebView 窗口加载平台登录页，轮询捕获 Cookie
  （含 HttpOnly），检测到会话 Cookie 后自动关窗保存。

### 云同步

- 同步对象：本地歌单、收藏歌单、最近播放（含删除记录）、
  播放统计（含每日桶与清空标记）、歌单歌曲删除记录与同步日志。
- `merge.rs` 对照基准快照做三路合并（不是 last-write-wins）；
  歌曲成员携带因果 token，跨端删除/恢复不会互相覆盖。
- 当前云端写入 V4 清单和 Zstd 分块归档，兼容读取旧 JSON、GZIP 和 V3；
  迁移已有旧归档前需要在设置页确认升级，并先更新其它设备的客户端。
- GitHub Token 与 WebDAV 密码保存在应用侧加密存储中
  （见下节），不落明文配置。

### 凭据与本地数据

- Release 构建使用应用侧加密文件存储凭据（Cookie、GitHub Token、
  WebDAV 密码），与 Android 端 EncryptedSharedPreferences 的
  威胁模型一致：防「文件被拷走后可读」；
  旧版系统钥匙串中的凭据会在首次读取时自动迁移。
- 歌单等本地数据以 JSON 落盘，统一走原子写入工具（临时文件 + rename），
  避免断电/崩溃导致文件损坏。
- 日志经脱敏处理后写入文件（可配置级别与开关），
  崩溃报告独立落盘。

---

## 键盘快捷键 / Keyboard Shortcuts

| 快捷键 | 动作 |
| --- | --- |
| `Space` | 播放 / 暂停 |
| `Ctrl/Cmd + →` / `Ctrl/Cmd + ←` | 下一首 / 上一首 |
| `→` / `←` | 快进 / 快退（`Shift` 加大步长） |
| `↑` / `↓` | 音量加 / 减 |
| `M` | 静音 |
| `S` | 随机播放 |
| `R` | 循环模式 |
| `Ctrl/Cmd + P` | 打开 / 收起正在播放 |
| `Ctrl/Cmd + F` | 搜索 |
| `Esc` | 分层关闭浮层（歌词 → 播放页 → 面板） |

---

## GitHub / WebDAV 同步 / Cloud Sync

NeriPlayer Desktop 支持将本地元数据同步到 **用户自己的 GitHub 仓库**
或 WebDAV 远端文件，配置入口在「设置 → 备份与同步」。

当前同步对象：

- 本地歌单（含歌曲自定义名称/歌手/封面、匹配歌词与单曲歌词偏移）
- 收藏歌单
- 最近播放记录及其删除记录
- 播放统计（含每日统计桶）

技术细节：

- 🔒 **本地安全存储**：GitHub Token 与 WebDAV 密码保存在
  应用侧加密存储中。
- 🧩 **冲突处理**：三路合并对照基准快照裁决歌单、收藏、历史、
  删除记录与播放统计；歌曲成员携带因果 token，
  从备份恢复的内容不会被旧删除记录再次删掉。
- 🪶 **分块归档**：使用 Android V4 的 `neriplayer-sync-v3.manifest` 清单和 Zstd 对象；
  旧格式可读取，升级确认分别绑定 GitHub 或 WebDAV 的当前目标与内容。
- 🔄 **跨端互通**：与 Android 端共用同一数据模型，
  同一远端可被两端交替读写。
- 🚫 **同步边界**：不会上传音频文件、下载内容、Cookie 或播放 Token。
- 🧪 **对齐验证**：Kotlin 反向解码脚本为 `scripts/test-sync-android-interop.ps1`；
  自动编解码测试不代替真实账号、provider 与跨设备交替同步验收。
- 📦 **远端格式**：GitHub 仓库 / WebDAV 文件不是端到端加密备份，
  远端文件由用户自行保管。

使用方法（GitHub 为例）：

1. 创建 GitHub Personal Access Token（需要 `repo` 权限）。
2. 在应用内完成 Token 校验，选择创建默认私有仓库或接入已有仓库。
3. 手动点击立即同步，或让本地变更触发同步。

另有 **歌单 JSON 导入/导出** 与 **完整配置导入/导出**
（含设置、授权与同步配置，适合自用迁移，不应公开分享）。

---

## 一起听 / Listen Together

- 支持创建房间或加入他人房间，通过 WebSocket 实时同步播放、暂停、
  进度、切歌与循环/随机模式；支持成员控制开关、成员进出自动暂停、
  直链共享开关与自定义服务端地址。
- 服务端与 Android 端共用，基于 **Cloudflare Workers + Durable Objects**：
  - 仓库内源码：Android 仓库的
    [np-submodule/NeriPlayer-LTW](https://github.com/cwuom/NeriPlayer/tree/master/np-submodule/NeriPlayer-LTW)
  - 公开部署模板：
    [TheSmallHanCat/NeriPlayer-LTW](https://github.com/TheSmallHanCat/NeriPlayer-LTW)
- 房间号固定为 6 位可读字符；昵称长度 1-24，
  允许中文、英文字母和数字（与服务端约束一致）。

---

## 发展规划 / Roadmap

### 方向探索

这些方向会根据维护精力、平台可用性和社区反馈调整，不承诺固定周期。

- [ ] QQ 音乐账号登录与库页能力
- [ ] 系统托盘与系统级全局快捷键
- [ ] 应用内自动更新通道
- [ ] 下载断点续传与队列恢复
- [ ] 持续对齐 Android 端新能力

### 近期已落地

- [x] 网易云艺术家详情页与收藏页艺术家分类
- [x] 国际化模式下 YouTube Music 首页货架优先
- [x] 分层 ESC 关闭、光标锚定菜单与菜单细节打磨
- [x] 指针拖拽排序、多选工具栏与稳定拖放指示器
- [x] 自适应 Range 流式拉流、分片快速 seek 与静默拖动
- [x] 主题色平滑过渡与设置页动效
- [x] macOS 原生 unified toolbar 居中红绿灯标题栏
- [x] 三路合并语义与 Android 对齐，修复数据丢失路径
- [x] 原子文件写入与存储损坏防护
- [x] 凭据加密存储与日志脱敏
- [x] 调试台对齐 Android（探针 / 日志 / 崩溃报告）
- [x] 单实例保护与 Linux 依赖修复
- [x] AMLL 逐字歌词、YRC 接入与歌词右键菜单
- [x] 一起听循环/随机模式同步与会话防护

---

## 问题反馈 / Bug Report

- 反馈前建议先开启开发者模式（设置页连续点击 **版本号** 7 次），
  侧栏会出现 `Debug` 页。
- Debug 页可查看实时日志、崩溃报告，并一键导出调试报告；
  「设置 → 日志」可开启持久文件日志并调整级别。
- 前往 [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues)
  按模板提供：操作系统与版本、应用版本（设置页可复制构建信息）、
  复现步骤与关键日志。

---

## 已知问题 / Known Issues

### 安装与签名

- macOS 安装包为 ad-hoc 签名，首次启动可能被 Gatekeeper 拦截，
  处理方式见 [快速体验](#快速体验--getting-started)。
- Windows 安装包未做代码签名，SmartScreen 提示属预期现象。
- Linux 需要 WebKitGTK 4.1 运行时；deb 依赖已在包内声明。

### 网络

- 请合理配置代理规则；全局代理可能导致部分第三方接口返回异常数据。
  设置中提供「绕过系统代理」直连开关。

### 能力边界

- QQ 音乐当前无账号登录态，能力受游客接口限制。
- 下载暂不支持断点续传；取消即清理。
- Android 端独有的移动能力（USB 独占、悬浮/状态栏歌词、SAF、
  安全模式等）不在桌面端范围。
- GitHub / WebDAV 同步不是端到端加密；完整配置导出文件包含授权信息，
  请自行妥善保管。

---

## 隐私与数据 / Privacy

- NeriPlayer Desktop 不提供公共云端媒体分发服务，也不接入广告 SDK、
  第三方统计或崩溃分析 SDK。
- 歌单、收藏、最近播放、播放统计、下载文件、设置与授权信息
  默认保存在本机。
- 如用户主动开启 GitHub / WebDAV 同步，仅同步歌单、收藏、
  历史和播放统计等元数据，远端由用户自行选择和保管。
- 不会将音频文件、下载内容、Cookie 或播放 Token 上传给开发者。
- 出于账号安全考虑，应用不会把本地播放历史或播放统计
  回写到第三方音乐平台。
- 完整配置导出文件包含设置、授权信息和同步配置，适合自用迁移，
  不应公开分享。
- 第三方平台侧的访问日志与风控策略，由对应平台按照其自身
  隐私政策处理。

---

## 鸣谢 / Reference

<table>
<tr>
  <td><a href="https://github.com/cwuom/NeriPlayer">NeriPlayer (Android)</a></td>
  <td>✨ 一个把多源在线播放、本地管理、歌词体验和自建同步做进原生 Android 的音频播放器 🎵</td>
</tr>
<tr>
  <td><a href="https://github.com/amll-dev/applemusic-like-lyrics">applemusic-like-lyrics</a></td>
  <td>An Apple Music style lyric player component, with React &amp; Vue support. 一个类 Apple Music 歌词显示组件</td>
</tr>
<tr>
  <td><a href="https://github.com/chaunsin/netease-cloud-music">netease-cloud-music</a></td>
  <td>✨ 网易云音乐 Golang 实现 🎵</td>
</tr>
<tr>
  <td><a href="https://github.com/SocialSisterYi/bilibili-API-collect">bilibili-API-collect</a></td>
  <td>哔哩哔哩 API 收集整理</td>
</tr>
<tr>
  <td><a href="https://lrclib.net">LRCLIB</a></td>
  <td>开放的歌词数据库服务</td>
</tr>
</table>

---

## 更新周期 / Update Cycle

- 项目处于开发中，Release 通常按功能批次手动发布。
- 播放、本地数据与同步链路会优先维护。
- 第三方平台能力可能受平台策略影响，欢迎提交 Issue、PR 或复现日志。

---

## 支持方式 / Support

- 由于项目特殊性，暂不接受任何形式的捐赠。
- 欢迎通过提交 Issue、PR 或分享使用体验来支持项目发展。

---

## 许可证 / License

NeriPlayer Desktop 使用 **MIT** 开源许可证发布，
详细条款请参阅 [LICENSE](./LICENSE)。

> Android 端使用 GPL-3.0，两个仓库的许可证相互独立；
> 子模块 `vendor/applemusic-like-lyrics` 遵循其自身许可证。

---

# Contributing to NeriPlayer Desktop / 贡献指南

贡献前请先阅读完整的 [CONTRIBUTING.md](./CONTRIBUTING.md)
与 [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)。



