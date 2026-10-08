[English](./CONTRIBUTING_EN.md) | [中文](./CONTRIBUTING.md)

## Contributing to NeriPlayer Desktop / 贡献指南

感谢你愿意为 NeriPlayer Desktop 做出贡献。
本文描述**当前桌面客户端的真实实现**，
请以源码和运行行为为准同步维护文档。

> [!CAUTION]
> 桌面端仍处于早期开发阶段：本文与 README 的部分内容为占位描述，
> 实际实现可能缺失、不完善，极端情况下可能损坏数据；
> 仓库未来可能进入无人维护状态，理想形态是由社区共同推进。
> 发现文档与实现不符时，请以源码为准并顺手修正文档。

---

### 项目定位 / Scope

- NeriPlayer Desktop 是 NeriPlayer (Android) 的 **Tauri 2 桌面移植**，
  不是公共云端曲库服务。
- 在线内容能力来自 **网易云音乐**、**QQ 音乐**、**Bilibili** 与
  **YouTube Music**；歌词补全额外接入 **LRCLIB**。
- 数据默认保存在本地；GitHub / WebDAV 同步是**可选能力**，
  同步对象是歌单、收藏、最近播放、播放统计等元数据，
  不是媒体文件本身。
- 云同步数据模型与一起听协议**必须与 Android 端保持兼容**，
  这是本仓库最重要的外部约束。
- 一起听服务端在 Android 仓库的 `np-submodule/NeriPlayer-LTW`，
  基于 Cloudflare Workers 与 Durable Objects；桌面端只实现客户端。

---

### 文档地图 / Documentation Map

- `README.md` / `README_EN.md`
  - 面向用户和新贡献者，说明项目定位、能力边界、安装构建、
    同步与隐私。
- `CONTRIBUTING.md` / `CONTRIBUTING_EN.md`
  - 面向开发者，说明真实模块边界、扩展路径、测试和提交要求。
- `scripts/test-sync-android-interop.ps1`
  - 使用只读 Android 源码和已有 Gradle 缓存验证桌面归档的 Kotlin 反向解码。
- `CODE_OF_CONDUCT.md`
  - 社区行为准则。
- `CLAUDE.md` / `AGENTS.md`
  - 面向 AI 编码代理的仓库说明，人类贡献者可当作架构速览。
- `.github/ISSUE_TEMPLATE/`、`.github/PULL_REQUEST_TEMPLATE.md`
  - Issue 与 PR 模板。

行为变更如果影响用户理解，请同步更新 README；
如果影响扩展方式、测试方式或模块边界，请同步更新 CONTRIBUTING。

---

### 开发环境 / Development Environment

- **Node.js**：20+（CI 使用 20）
- **pnpm**：`9.15.9`（pinned，`corepack enable` 即可）
- **Rust**：`1.95.0`（`rust-toolchain.toml` 自动固定）
- **Tauri**：2.x；**Vue**：3.5；**Vite**：6；**TypeScript**：5.8
- **版本名格式**：`<git短哈希>.<MMddHHmm>`（Asia/Taipei）

平台依赖：

- **Windows**：Visual Studio C++ Build Tools、WebView2 运行时
- **macOS**：Xcode Command Line Tools
- **Linux**（Debian/Ubuntu）：
  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
    librsvg2-dev patchelf libasound2-dev
  ```

补充说明：

- 仓库依赖 Git 子模块 `vendor/applemusic-like-lyrics`（AMLL 歌词组件），
  首次克隆请使用 `--recursive`，或手动执行
  `git submodule update --init --recursive`。
- 构建会读取 Git 短提交生成版本名，本地请确保已安装 Git。
- `pnpm tauri` 实际经由 `scripts/run-tauri.mjs` 转发，
  该脚本负责注入 `NERI_BUILD_EPOCH` 供 `src-tauri/build.rs`
  生成可复现的构建时间戳。

---

### 快速开始 / Quick Start

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install

pnpm tauri dev      # 完整应用（Vite :1420，被占用时自动换用下一个空闲端口 + Rust shell）
pnpm dev            # 仅前端（无 Tauri 后端，IPC 调用会失败）
pnpm build          # vue-tsc 类型检查 + vite build -> dist/
pnpm tauri build    # 生产打包 -> src-tauri/target/release/bundle/
```

Rust 后端（在 `src-tauri/` 下执行）：

```bash
cargo check          # 快速类型检查
cargo build          # 调试构建
cargo clippy         # lint（交付要求零警告）
```

首次启动后在设置页登录平台账号；连续点击版本号 **7 次**
解锁开发者模式，侧栏出现 `Debug` 页
（平台探针 / 实时日志 / 崩溃报告 / 构建信息）。

---

### 发布构建 / Release Build

- 本地：`pnpm tauri build`，产物在
  `src-tauri/target/release/bundle/`
  （Windows：`msi/` + `nsis/`；macOS：`dmg/`；
  Linux：`deb/` + `rpm/` + `appimage/`）。
- 安装包随附精简 FFmpeg（Opus、E-AC-3 等格式靠它解码）：
  先 `bash scripts/ffmpeg/build-ffmpeg.sh <target>`（Windows 版在 Linux / WSL 里交叉编译），
  再 `node scripts/ffmpeg/bundle-config.mjs <target>`，
  打包时加 `--config .cache/ffmpeg-build/<target>/tauri.bundle.json`。
  不加这一步打出的包没有 FFmpeg，那些格式会被前端避开。
  详见 `src-tauri/native/ffmpeg/README.md`。
- CI：推送 `v*` 标签触发 `.github/workflows/release.yml`，
  在 Windows x64 / macOS arm64 / macOS x64 / Linux x64
  四个矩阵上构建并发布 GitHub Release（同时附上 FFmpeg 源码包）；
  macOS 产物为 ad-hoc 签名（文件名带 `-adhoc` 后缀）。
- main 分支推送会触发 `Artifacts` 工作流产出同矩阵的测试包。

安全提醒：

- 不要提交 Cookie、Token、密码或其他敏感信息。
- 不要在 Issue / PR 中粘贴完整授权信息。
- 完整配置导出文件包含平台授权和同步凭据，
  不能作为公开测试附件。

---

### 项目结构与当前实现 / Project Layout

两个进程通过 Tauri IPC 桥通信；前端从不直接访问音乐平台。

#### 前端（`src/`）

- `main.ts`
  - 应用入口与路由表（页面全部懒加载）；主窗口初始隐藏，
    Vue 挂载后再显示以避免闪白。
- `stores/`
  - Pinia store 是 IPC 的唯一入口。`player.ts` 是播放中枢
    （播放控制、淡入淡出/交叉淡入淡出编排、队列、进度）；
    `settings.ts` 持久化全部设置项；
    `listenTogether/` 内含协议/映射子模块，镜像 Rust 侧协议；
    其余 store 覆盖 auth / library / likedSongs / history /
    playbackStats / download / recommend / search / sync / toast /
    lyricOffset。
- `views/`
  - 路由级页面：Home / Explore / Library / Settings / Downloads /
    Recent / PlaybackStats / Debug，以及各平台歌单、专辑、
    艺术家详情页。
- `components/`
  - `NowPlaying`、`LyricsView`（AMLL）、`MiniPlayer`、`QueuePanel`、
    `SideNav`、`TitleBar`（自绘窗口控制）、`HyperBackground`
    （WebGL 流体背景）、`CoverBlurBackground`、`ListenTogetherPanel`、
    `TrackSelectionToolbar`（多选）、`LocateTrackFab`、
    `WaveformSlider`、`StorageManagementDialog` 等；
    `ui/` 存放 Material 3 风格基础组件
    （M3Dialog / M3Input / ContextMenu / CustomSelect 等）。
- `modules/`
  - 按域拆分的纯逻辑模块：`playback/`（队列、请求、策略、预取、
    状态、统计采集）、`lyrics/`（格式、缓存、偏移、逐字行、
    同步载荷）、`library/`、`shortcuts/`（全局快捷键）、`youtube/`。
    这些模块大多有对应的 `scripts/test-*.mjs` 纯逻辑测试。
- `shaders/`
  - `hyperBackground.vert/.frag` 以字符串导入（见 `shaders.d.ts`）。
- `i18n/`
  - `zh-CN`（回退语言）/ `zh-TW` / `en` / `ja`。
- `utils/`、`composables/`
  - 封面缓存、取色、日志（含脱敏）、持久化缓存、弹簧动画、
    多选/拖拽排序/定位当前曲目等组合式函数。

#### 后端（`src-tauri/src/`）

- `main.rs`
  - 唯一的命令注册点 `generate_handler![...]`、事件 ticker、
    媒体键通道、单实例插件、运行时关闭窗口装饰
    （macOS 保留原生 Overlay 标题栏）。
- `state.rs`
  - `AppState`：播放引擎、播放队列、可重建的共享 `reqwest::Client`
    （代理直连切换）、共享 cookie jar、各平台登录态、
    一起听会话、下载任务注册表；`TrackInfo` / `TrackSource`
    是跨 IPC 边界的规范曲目类型。
- `commands/*_cmd.rs`
  - 按域分组的命令实现：player / library / search / lyrics /
    settings / auth / recommend / sync / download / listen_together /
    stats / storage / image / debug。
- `commands/download_cmd.rs`、`commands/download_metadata.rs`
  - 前端 `stores/download.ts` 与 `modules/download/downloadQueue.ts` 调度任务，
    `DownloadsView.vue` 在下载 Tab 展示排队/解析/传输/元数据处理进度、取消与失败重试。
    并发设置限制为 1-8，默认 6；默认跟随播放音质，也可使用平台独立下载音质。
  - 音频位于选定根目录，`Lyrics/` 存歌词、翻译与罗马字，`Covers/` 存封面，
    `.tmp/` 存临时文件；`<音频文件名>.npmeta.json` 保存 NP 元数据与关联资源引用。
    元数据补齐默认开启，标准化歌词嵌入默认关闭；标准化不覆盖 sidecar/NP 中的原文。
    写标签后更新 manifest 的最终文件大小，避免将标签造成的大小变化误判为损坏。
- `commands/local_files_cmd.rs`、`library/local_file_tags.rs`
  - `LocalFilesView.vue` 与 `stores/library.ts` 提供可取消的扫描预览，搜索、多选与
    已入歌单/元数据重复筛选；扫描不写歌单，由用户手动导入现有或新歌单。
    标题/歌手/专辑先在副本写入，读回验证原有歌词、图片和其他标签后原子替换，
    保留 NP sidecar 未修改字段；已下载文件的标签修改同时更新下载清单并发送
    `downloads-changed`，提交失败时回滚。
  - 针对性测试：`node scripts/test-local-scan-preview.mjs`、
    `node scripts/test-local-scan-store.mjs` 与
    `cargo test --manifest-path src-tauri/Cargo.toml --lib library::local_file_tags::tests`。
- `audio/`
  - `player.rs`（`PlayerEngine`，播放/seek/淡入淡出/交叉淡入淡出）、
    `queue.rs`（随机/循环）、`effects.rs`（5 频段 EQ、响度均衡、
    响度增益）、`growing.rs`（渐进缓冲）、`remote.rs`（Range 流）、
    `analyzer.rs`（音频电平）、`media_session.rs`（SMTC/MPRIS）。
- `api/`
  - `netease/`（`crypto.rs`：WEAPI/EAPI/linuxapi）、`qq/`、
    `bilibili/`（`wbi.rs`：WBI 签名）、`youtube/`（会话/取流/刷新）、
    `lrclib.rs`、`transport.rs`。
- `auth/`
  - Cookie 存取、登录态模型、`youtube_hash.rs`（SAPISIDHASH）。
- `sync/`
  - `models.rs`（同步载荷）、`proto_models.rs`（ProtoBuf 模型，
    字段号对齐 Android）、`merge.rs`（三路合并）、`serializer.rs`
    （旧 JSON / GZIP）、`archive/`（V4 清单与分块编码）、
    `cloud.rs`（归档发布）、`github_api.rs`、`webdav_archive.rs`、
    `webdav_gc.rs` 与 `manager.rs`。
- `listen_together/`
  - `protocol.rs`（事件与模型）、`session.rs`、`ws_client.rs`。
- `library/`（本地扫描、歌单存储）、`lyrics/`（多源歌词管理与解析）、
  `settings/`、`stats/`、`logging/`。
- `security.rs`
  - 凭据存储：Release 用应用侧加密文件，Debug 用随机路径明文；
    旧钥匙串凭据首次读取时自动迁移。
- `fsutil.rs`
  - 原子写入工具（临时文件 + rename），所有本地数据落盘必须走这里。

---

### IPC 契约 / IPC Contract

- 新增命令必须同时改三处：`commands/*_cmd.rs` 实现、
  `main.rs` 的 `generate_handler![...]` 注册、前端调用方。
  注册列表是 IPC 面的单一事实来源。
- 前端 `invoke('command_name', { camelCaseArgs })`；
  Tauri 自动把 camelCase JS 参数映射到 snake_case Rust 参数。
  载荷字段命名要和现有约定一致（结构体多用
  `#[serde(rename_all = "camelCase")]`），
  不一致会静默丢字段——历史上播放统计就因 payload 命名踩过坑。
- 后端到前端只用事件推送（`app.emit(...)`），不要让前端轮询；
  现有事件见 README「进程与 IPC」。

---

### 质量护栏 / Quality Guardrails

提交前请优先保护这些链路：

- **播放命令不得阻塞 IPC 线程**：
  `player_cmd.rs` 中所有触碰 `PlayerEngine` 锁的操作必须经
  `run_player_blocking` 派发到阻塞线程，
  直接在异步命令里拿锁曾造成 30 秒级 UI 卡死。
- **播放代际与 seek 采纳**：
  播放请求与 seek 结果都带 generation，改动播放链路时
  不能让旧请求的结果覆盖新请求。
  YouTube 等待令牌的任务必须随旧解析取消；首选客户端应进入解析和预取缓存键。
  网易云兜底保留队列身份，但缓存、格式和时长按实际候选记录，下载禁止使用兜底。
- **签名/加密逻辑精确匹配**：
  WEAPI / EAPI / linuxapi、WBI、SAPISIDHASH 是平台请求的
  脆弱点，修改平台请求时必须与现有 scheme 完全一致。
- **同步兼容性是硬约束**：
  `proto_models.rs` 的字段号与 Android `SyncDataModels.kt` 的
  `@ProtoNumber` 逐一对齐，**改字段号 = 破坏跨端同步**；
  `merge.rs` 是三路合并，不得退化为 last-write-wins；
  读远端快照要兼容缺字段旧数据。
- **一起听协议兼容**：
  协议字段变更必须同时兼容 Android 客户端与 Worker 服务端；
  昵称允许中文、英文字母和数字（**不含连字符**），
  服务端会 sanitize 非法字符；`shareAudioLinks=false` 时
  不得暴露直链。
- **本地数据原子写**：
  所有 JSON 落盘走 `fsutil.rs` 的原子写入，
  不要直接 `fs::write` 覆盖用户数据。
  收藏歌手的修改通过 `manager::update_favorite_playlists` 串行读改写，
  保留删除记录并推进同步 epoch，远端关注拉取不可覆盖拉取期间的取消关注。
- **UI 状态变化必须有过渡**：
  本项目的硬性体验要求——任何可见状态切换（页面、面板、封面、
  主题色）都要有过渡动画，不能闪变；
  封面加载采用 peek-first（先探缓存再渲染），避免占位图闪现。
- **凭据与日志**：
  凭据只走 `security.rs`；日志输出经 `logSanitizer` 脱敏，
  新增日志不要打印 Cookie / Token / URL 中的签名参数。

---

### 常见扩展路径 / Extension Paths

#### 1. 新增 IPC 命令

1. 在对应域的 `src-tauri/src/commands/*_cmd.rs` 实现
   `#[tauri::command]`（新域则新建文件并在 `commands/mod.rs` 导出）。
2. 在 `main.rs` 的 `generate_handler![...]` 注册。
3. 前端在对应 Pinia store 中封装 `invoke` 调用，
   不要在组件里直接 `invoke`。
4. 载荷类型两侧对齐（camelCase ↔ snake_case）。

#### 2. 新增取流/搜索平台

1. 参考 `api/bilibili/` 或 `api/youtube/` 建立
   `api/<platform>/client.rs`；签名/加密逻辑单独放一个文件。
2. 登录态在 `auth/state.rs` 扩展，Cookie 捕获复用
   `auth_cmd.rs` 的 WebView 登录流程。
3. 搜索接入 `search_cmd.rs` 的平台分发；
   取流接入 `settings_cmd.rs` 的 `get_*_url` 家族。
4. 前端在 `Explore` / `Library` 增加平台 tab 与路由页面。
5. 下载、歌词、封面与播放统计的映射保持边界清晰。

#### 3. 新增设置项

1. 在 `src/stores/settings.ts` 的类型与默认值中登记。
2. UI 入口放在 `SettingsView.vue` 对应分区。
3. 影响后端行为的设置通过既有命令传递
   （如 `set_bypass_proxy`），不要另起持久化通道。
4. 注意 `formatVersion` 迁移逻辑，旧配置必须能无损升级。

#### 4. 修改歌词链路

1. 来源优先级在 `src-tauri/src/lyrics/manager.rs`：
   QQ（有 song_mid 时）→ 网易云（有 id 时）→ LRCLIB →
   按曲名/歌手/时长搜索兜底；改动时保持时长匹配打分。
2. 前端格式解析与缓存在 `src/modules/lyrics/`；
   同步载荷中的匹配歌词（`syncPayload`）优先于重新请求。
3. YRC 逐字歌词与翻译行的结构要与 AMLL 渲染层约定一致。
4. 相关纯逻辑测试：`pnpm test:lyrics-format`、
   `pnpm test:lyric-offset`、`pnpm test:lyrics-request`。

#### 5. 修改 GitHub / WebDAV 同步

1. 先理解 `sync/models.rs` 与 `sync/proto_models.rs`；
   **ProtoBuf 字段号只增不改**，新增字段必须兼容
   旧 JSON 与 ProtoBuf 的缺字段载荷。
2. 合并逻辑在 `sync/merge.rs`（三路合并 + 因果 token）；
   修改删除/恢复语义时，必须用「两端交替同步」场景自测：
   桌面写 → Android 读 → Android 写 → 桌面读。
3. 凭据统一走 `security.rs`，不要放回明文配置。
4. V4 迁移授权必须绑定后端、目标、凭据指纹和当前远端内容。GitHub 使用分支 HEAD
   条件更新；WebDAV V4 清单使用强 ETag 或经过验证的有限排他集合租约，旧单文件
   升级必须取得有限集合租约。不要用重定向、
   无条件覆盖或本地对象缓存替代远端完整性校验。
5. 执行 `pnpm test:sync-protocol-upgrade` 与 Rust `sync` 模块测试；
   Android JVM 反向解码运行 `./scripts/test-sync-android-interop.ps1 -AndroidRoot <Android仓库> -ExportFixtures`。
   此检查不代替整仓 Gradle、provider 或设备互通验收。

#### 6. 修改一起听

1. 客户端逻辑在 `src-tauri/src/listen_together/` 与
   `src/stores/listenTogether/`；两侧协议模型必须同步修改。
2. 服务端约束（房间号 6 位、昵称 1-24 且不含连字符、
   事件语义）以 Worker 实现为准，不要只改 UI 校验。
3. 相关测试：`pnpm test:listen-together-mapper` 与
   `node scripts/test-listen-together-protocol.mjs`。
4. 当前对照 Android `3e1abcb7` 及其 Worker `31d55c60`：
   schemaVersion >= 2 使用带 `baseRoomVersion` 的队列操作，重复曲目以
   `stableKey` + `occurrence` 定位；旧服务端回退完整队列。
   `streamUrls` 按平台白名单过滤，并保留 `streamUrl` 供旧客户端读取。
5. 播放同步策略在 `playbackSync.ts`，队列操作在 `queue.ts`。
   修改后执行 `pnpm test:listen-together-queue`、
   `pnpm test:listen-together-sync`、`pnpm test:listen-together-store`，
   Rust 侧执行 `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib listen_together`。
   本地测试不代替 Android 与桌面端实际同房的联调。

---

### 测试与提交流程 / Testing & PR

本仓库的正确性门禁：

1. 前端类型检查 + 构建（必须）：
   ```bash
   pnpm build
   ```
2. 后端类型检查（必须）与 lint（建议，零警告交付）：
   ```bash
   cargo check --manifest-path src-tauri/Cargo.toml --locked
   cargo clippy --manifest-path src-tauri/Cargo.toml
   ```
3. 纯逻辑测试脚本（Node，无需构建，按改动范围执行）：
   ```bash
   pnpm test:player-state
   pnpm test:playback-request
   pnpm test:playback-source
   pnpm test:track-cover
   pnpm test:lyrics-format
   pnpm test:lyric-offset
   pnpm test:lyrics-request
   pnpm test:listen-together-mapper
   pnpm test:youtube-playlist-parse
   pnpm test:bilibili-cover-cache
   node scripts/playback-queue.test.mjs
   node scripts/test-listen-together-protocol.mjs
   node scripts/test-now-playing-background.mjs
   ```
4. 涉及播放、登录、下载或同步的行为变更，
   请在至少一个桌面平台上手动验证，
   并在 PR 中说明验证平台与场景。
5. 修改 `modules/` 纯逻辑时，优先为其补充/更新对应
   `scripts/test-*.mjs`；新增测试脚本请在 `package.json`
   注册 `test:*` 入口。

PR 建议包含（模板见 `.github/PULL_REQUEST_TEMPLATE.md`）：

- 变更动机与关键实现点
- 风险与兼容性影响（特别是跨端同步 / 一起听协议）
- 已执行的验证命令与结果
- 如涉及 UI，附截图或录屏

不要提交：

- 构建产物（`dist/`、`src-tauri/target/`）、IDE 本地配置
- 授权 Cookie、Token、完整配置备份、个人数据
- `.report/`、`docs-private/` 下的内容

Commit 信息遵循 Conventional Commits，
例如 `feat(player): ...`、`fix(sync): ...`、`docs: ...`。

---

### 法律与许可 / Legal & License

- 项目仅供学习与研究使用，请勿用于非法用途。
- 本项目使用 **GPL-3.0** 协议（GPL-3.0-or-later）；提交贡献即表示你同意
  以 GPL-3.0-or-later 分发你的修改。2026-10-07 之前以 MIT 提交的代码，
  原许可声明保留在 `LICENSES/MIT.txt`。
- 子模块 `vendor/applemusic-like-lyrics` 使用 AGPL-3.0，遵循其自身许可证。
- Android 端同样使用 GPL-3.0；移植 Android 端实现时请保留原有版权声明，
  并在提交正文中注明来源。

---

### 沟通方式 / Communication

- [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues)：
  缺陷、功能建议、讨论
- [README.md](./README.md)：功能与使用说明
- [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)：社区行为准则

如你准备提交较大的结构性改动（特别是涉及同步数据模型或
一起听协议的），建议先开 Issue 对齐方向。


