[English](./CONTRIBUTING_EN.md) | [中文](./CONTRIBUTING.md)

## Contributing to NeriPlayer Desktop / 贡献指南

感谢你愿意为 NeriPlayer Desktop 做出贡献。
本文描述**当前桌面客户端的真实实现**，请以源码和运行行为为准同步维护文档。
项目目前处于预览阶段，开发和测试请使用可恢复的数据副本。

---

### 项目定位 / Scope

- NeriPlayer Desktop 是基于 **Tauri 2、Vue 3 与 Rust** 的桌面音频播放器，
  面向 Windows、macOS 和 Linux。
- 在线内容接入网易云音乐、QQ 音乐、Bilibili 与 YouTube Music，
  各来源的账号、搜索、取流和元数据能力并不完全相同。
- Rust 原生后端负责音频和系统操作，Vue WebView 负责界面与状态编排；
  浏览器预览不能替代桌面运行验证。
- 数据默认保存在本地。GitHub / WebDAV 同步是可选能力，
  同步歌单、收藏、历史、统计等元数据，不上传音频文件。
- 云同步与一起听需要维护 Android 兼容性。一起听服务端位于
  [Android 仓库](https://github.com/cwuom/NeriPlayer)的 `np-submodule/NeriPlayer-LTW`，
  不包含在本仓库中。

---

### 文档地图 / Documentation Map

维护文档时建议按用途拆开看：

- [README.md](README.md) / [README_EN.md](README_EN.md)
  - 面向用户和新贡献者，说明项目定位、能力边界、安装、同步与隐私。
- [CONTRIBUTING.md](CONTRIBUTING.md) / [CONTRIBUTING_EN.md](CONTRIBUTING_EN.md)
  - 面向开发者，说明环境、模块、扩展方式、验证和提交要求。
- [src-tauri/native/ffmpeg/README.md](src-tauri/native/ffmpeg/README.md)
  - 说明共享库准备、运行时加载、安装包配置、解码测试与分发许可。
- [src-tauri/native/dsp/README.md](src-tauri/native/dsp/README.md)
  - 说明 Android DSP 来源、桌面桥接、参数兼容与原生测试。
- [src-tauri/src/sync/archive/fixtures/README.md](src-tauri/src/sync/archive/fixtures/README.md)
  - 说明同步归档夹具及其验证范围。
- [.github/workflows/](.github/workflows/)、[PR 模板](.github/PULL_REQUEST_TEMPLATE.md)
  - 说明实际 CI、测试包、发布入口和审查所需信息。
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)、[LICENSE](LICENSE)、[LICENSES/MIT.txt](LICENSES/MIT.txt)
  - 说明社区规范、当前许可和历史 MIT 声明。

前端版本和命令以 `package.json` / `pnpm-lock.yaml` 为准；Rust 版本和依赖以
`rust-toolchain.toml`、`src-tauri/Cargo.toml` / `Cargo.lock` 为准。
窗口、安全、资源与打包配置见 `src-tauri/tauri.conf.json` 和 `src-tauri/capabilities/`。

用户可感知的行为变化同步更新两份 README；开发方式、模块边界、协议、测试或发布方式变化
同步更新两份 CONTRIBUTING。不要把计划中的能力写成现有实现。

---

### 开发环境 / Development Environment

- **Git**：用于子模块和构建版本信息。
- **Node.js**：仓库 CI 使用 20，`package.json` 未声明 `engines`。
  复现 CI 时使用相同主版本；验证其他版本时记录环境。
- **pnpm**：9.15.9，由 `packageManager` 固定。
- **Rust**：1.95.0，由 `rust-toolchain.toml` 固定，使用 `minimal` profile。
- **前端**：Vue 3、Vite 6、TypeScript 5，通过项目依赖安装。
- **原生构建**：Tauri 2、C++17 编译器与各平台开发库。
  无需全局安装 Tauri CLI，也不要求为编译安装系统 FFmpeg 开发包。

平台前置条件参见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)：

- Windows 使用 MSVC Rust 工具链、Microsoft C++ Build Tools 的桌面 C++ 工作负载与 WebView2。
- macOS 桌面开发需要 Xcode Command Line Tools。
- Linux 需要 WebKitGTK 4.1、GTK、OpenSSL、托盘和 ALSA 开发依赖。
  Ubuntu 22.04 可参考以下命令，其他发行版需按官方文档映射包名：

  ```bash
  sudo apt-get update
  sudo apt-get install -y build-essential pkg-config curl wget file libssl-dev \
    libxdo-dev libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev \
    patchelf libasound2-dev
  ```

构建 RPM 另需 `rpm`；FFmpeg 共享库的额外工具和目标限制见其独立文档。
Windows 构建 MSI 还需要启用 VBScript 可选功能，排查步骤见上述 Tauri 文档。
如需 Clippy / rustfmt，安装 `minimal` profile 不自带的组件：

```bash
rustup component add clippy rustfmt
```

---

### 质量护栏 / Quality Guardrails

提交前请优先保护这些链路：

- **播放与异步结果**：播放请求、seek、预取、取流缓存和音频信息的 generation 校验。
  旧结果不能覆盖新曲目；需要持有音频引擎锁的重操作沿用阻塞线程派发。
- **下载与文件**：任务取消、重试、sidecar、最终文件大小、下载清单和恢复流程。
  标签修改保留副本写入、读回核验与替换，不丢失歌词、封面和 NP 元数据。
- **本地数据**：SQLite 事务、旧 JSON / localStorage 迁移、启动快照和串行写入。
  设置、凭据、媒体与文件夹索引仍有独立存储边界，不能按普通缓存清除。
- **平台 API**：保留各平台请求、签名、认证与客户端选择语义。
  mock 测试不能证明真实平台登录、搜索或取流可用。
- **歌词与界面**：保护匹配结果、翻译 / 罗马字、逐字时间、偏移、点击跳转和窗口生命周期。
  复用已有菜单、快捷键、动画、封面缓存和事件清理方式。
- **音频与系统集成**：保留解码回退、输出设备、DSP 参数和实时回调约束。
  托盘、媒体键、桌面歌词、多窗口与 IME 输入需要原生环境验收。
- **同步与一起听**：保留 ProtoBuf 字段号、三路合并、因果 token、删除记录、远端并发保护、
  房间版本和重连恢复；跨端兼容是单独的验收条件。
- **凭据与诊断**：凭据统一走 `security.rs`，日志不得包含 Cookie、Token、同步密码或签名 URL。
  不通过忽略错误、关闭规则或删除有效测试来掩盖问题。

前端回归在 `scripts/`，Rust 测试随模块维护。修改这些链路时先搜索相邻测试，
再补能复现问题的覆盖；验证范围应与变更风险相匹配。

---

### 快速开始 / Quick Start

1. 克隆并初始化子模块：

   ```bash
   git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
   cd NeriPlayer-Desktop
   ```

   已有克隆缺少子模块时，运行 `git submodule update --init --recursive`。
   Vite 直接引用 `vendor/applemusic-like-lyrics` 的 core 源码，缺少它会使构建失败。

2. 使用固定版本的 pnpm 安装依赖：

   ```bash
   pnpm install --frozen-lockfile
   ```

   已有 Corepack 的环境可先运行 `corepack enable`，否则按 pnpm 官方说明准备对应版本。

3. 启动桌面调试版：

   ```bash
   pnpm tauri dev
   ```

4. 在设置页连续点击**版本号** 7 次启用开发者模式，进入 `Debug` 页查看探针、日志和构建信息。

命令的用途需要区分：

| 命令 | 用途 |
| --- | --- |
| `pnpm tauri dev` | 同时启动 Vite 和 Rust 原生应用，验证完整桌面流程 |
| `pnpm dev` | 仅前端浏览器开发，无原生 IPC，不能验证原生播放、登录、文件或系统集成 |
| `pnpm build` | 运行 `vue-tsc --build` 与 Vite，输出 `dist/` |
| `pnpm preview` | 预览构建后的前端，仍不包含原生后端 |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | 检查 Rust，仍需平台依赖与 C++ 编译器 |

`pnpm tauri` 经过 `scripts/run-tauri.mjs`。开发端口默认 1420，被占用时在 1420–1469
中寻找可用端口，并同时更新 Vite 与 Tauri 地址。`NERI_DEV_PORT` 可指定端口，指定端口必须可用。
直接运行 `pnpm dev` 使用 Vite 严格端口配置，不自动顺延。

---

### 构建发布版 / Release Build

从仓库根目录运行：

```bash
pnpm tauri build -- --locked
```

该命令构建原生应用和安装包，**不会自动下载或打包 FFmpeg 共享库**。
构建 C++ 垫片只需要仓库头文件；实际 FFmpeg 解码依赖运行时的匹配共享库。
缺库时会尝试 Symphonia 回退，不能据此保证全部格式可播放。

分发完整解码包时：

1. 按 [FFmpeg 构建说明](src-tauri/native/ffmpeg/README.md)构建共享库，
   或从本仓库可信工作流取得匹配的 `ffmpeg-<目标>` artifact。
2. 将四个共享库、`LICENSE.txt` 与 `BUILDINFO.txt` 放入 `.cache/ffmpeg-build/<目标>/`。
   不混合不同架构或来源的库。
3. 生成资源配置并传给 Tauri。Windows x64 示例：

   ```powershell
   node scripts/ffmpeg/bundle-config.mjs windows-x86_64
   pnpm tauri build --config .cache/ffmpeg-build/windows-x86_64/tauri.bundle.json -- --locked
   ```

配置生成器会核对 `BUILDINFO.txt` 与库文件哈希。`NERI_FFMPEG_DIR` 只控制运行时查找，
不会把文件加入安装包。完整 Linux / macOS 命令见 FFmpeg 文档。

| 平台 | Rust 目标 | FFmpeg 目标 | Release 产物 |
| --- | --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` | `windows-x86_64` | NSIS `.exe`、`.msi` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `macos-aarch64` | `.dmg`，最低系统配置 11.0 |
| macOS Intel | `x86_64-apple-darwin` | `macos-x86_64` | `.dmg`，最低系统配置 10.15 |
| Linux x64 | `x86_64-unknown-linux-gnu` | `linux-x86_64` | `.deb`、`.rpm`、`.AppImage.tar.gz` |

未指定 `--target` 时，默认产物目录是 `src-tauri/target/release/bundle/`；
显式目标使用 `src-tauri/target/<Rust目标>/release/bundle/`，`CARGO_TARGET_DIR` 可改变根目录。
Linux 本地生成 `.AppImage`，工作流上传时压为 `.tar.gz`。

macOS 使用 ad-hoc 签名，未经 Apple 公证。Release 检查 dylib 架构和签名，DMG 带 `-adhoc` 后缀。
基础 Tauri 配置的系统下限是 10.15；本地 arm64 发布构建应与 Release 一致设置
`MACOSX_DEPLOYMENT_TARGET=11.0` 并覆盖 `bundle.macOS.minimumSystemVersion`，
不能把基础值当成 arm64 支持承诺。

设置页构建版本为 `<7位Git哈希>.<MMddHHmm>`，时间采用 UTC+8，另有构建 UUID 和时间戳。
`scripts/run-tauri.mjs` 注入 `NERI_BUILD_EPOCH`，CI 统一 Git SHA、epoch 与 UUID，
用于关联多平台产物；不代表二进制逐字节可复现。`package.json` / Cargo 的 `1.0.0`
是打包元数据，不表示项目已脱离预览阶段。

---

### 项目结构与当前实现 / Project Layout

#### 前端与原生边界

- `src/main.ts`、`src/App.vue`、`src/views/`
  - 应用启动、路由和页面。桌面歌词与托盘弹窗由独立窗口入口挂载。
- `src/stores/`、`src/modules/`
  - Pinia 状态与业务逻辑，覆盖播放、曲库、下载、歌词、设置、同步和一起听。
  - `modules/persistence/userData.ts` 在 store 创建前读取启动快照，并串行发送持久化写入。
- `src/components/`、`src/composables/`、`src/utils/`
  - 播放器、AMLL 歌词、菜单、共用交互与原生适配；语言资源在 `src/i18n/`。
- `src-tauri/src/main.rs`、`lib.rs`、`state.rs`
  - Tauri 应用与共享运行状态，命令注册、窗口、托盘、插件和原生事件。
- `src-tauri/src/commands/`
  - 按业务划分 IPC 入口，包括播放、账号、下载、本地文件、存储、同步和诊断。
- `src-tauri/src/audio/`
  - CPAL 输出、Symphonia / FFmpeg 解码、Range / HLS、缓冲、队列、DSP、变速与媒体会话。
- `src-tauri/src/api/`、`auth/`、`lyrics/`
  - 平台请求、授权、元数据和歌词解析 / 匹配；不在前端复制平台签名或凭据逻辑。
- `src-tauri/src/library/`、`stats/`、`db/`、`settings/`
  - 本地扫描与标签、歌单、统计、数据库、设置与迁移。
- `src-tauri/src/sync/`、`src/stores/listenTogether/`、`src-tauri/src/listen_together/`
  - 云同步的模型、传输和合并，以及一起听的协议、会话、队列与重连。
- `src-tauri/src/security.rs`、`logging/`、`fsutil.rs`
  - 凭据、统一日志和文件原子写入。
- `src-tauri/native/ffmpeg/`、`src-tauri/native/dsp/`
  - C++17 解码加载垫片与 DSP 核心；由 `src-tauri/build.rs` 编入原生应用。
- `scripts/`、`.github/workflows/`
  - 前端回归、开发启动、FFmpeg 构建、跨端辅助检查与 CI / 发布。

新增 IPC 时，需要实现 `commands/` 入口、在 `main.rs` 的 `generate_handler!` 注册，
并同步前端命令、参数和返回类型。JavaScript 参数与 Rust / serde 命名保持一致。
共享状态复用已有 store / module，事件监听随组件或作用域销毁时清理。

#### 持久化边界

`src-tauri/src/db/` 将歌单、收藏、历史、队列、统计、下载目录与同步元数据存入
`neri_user_data.db`，并维护 schema 和旧数据迁移。业务写入复用领域事务，
不为同一状态另建 JSON 副本；前端启动恢复与后续写入保持顺序。

本地音乐文件夹和曲目索引由 `library/local_index.rs` 另存为 `local-library.json`。
它按文件大小和修改时间复用未变曲目，合并 notify 原生事件触发增量扫描，并在启动时更新。
文件夹管理、直接播放与导入歌单是独立操作；移除文件夹不应删除源文件或隐式改写歌单。

设置、凭据、缓存、音频和导出文件不全属于 SQLite。修改存储或清理流程前，
先确认实际目录和所有者，文件写入复用 `fsutil.rs` 等既有原子替换方式。

---

### 当前能力边界 / Current Boundaries

- **桌面应用与浏览器不同**：前端构建通过只证明类型检查和资源构建，
  不证明原生 IPC、WebView、登录窗口、音频设备或安装包可用。
- **平台能力受外部服务影响**：来源平台的授权、接口、地区和会员限制各不相同，
  不能将某一来源的成功推广到所有来源，也不能用 UI mock 代替真实请求验证。
- **解码依赖分发内容**：FFmpeg 配置测试、共享库实际解码和安装后运行是不同层次。
  开发缓存可加载库，不表示 Release 已包含库；macOS Intel 的 CI 解码覆盖也不同于其他目标。
- **同步范围是元数据**：本地路径不等于另一台设备可访问的媒体地址，云同步不替代音频备份，
  完整配置导出也不是数据库与媒体的完整备份。
- **V4 与旧客户端存在升级边界**：当前归档使用 V4，发布点仍为 `neriplayer-sync-v3.manifest`。
  保留旧格式读取、迁移确认和完整对象校验，不能仅凭文件名判断协议版本。
  修改 ProtoBuf、记录类型、删除或顺序语义时必须对照 Android 实现。
- **一起听跨仓库维护**：本仓库只能覆盖桌面客户端；Worker 和 Android 的变化需要单独核对。
  本地协议回归不代表已完成双端同房或真实网络验收。
- **凭据存储区分构建类型**：Release 使用应用侧加密文件，Debug 使用随机路径明文文件。
  该机制不承诺抵御同机同用户的恶意进程。完整配置导出可能包含授权和同步凭据，不能公开。

---

### 常见扩展路径 / Extension Paths

#### 1. 新增平台或搜索来源

1. 先检查 `src-tauri/src/api/`、`auth/`、`commands/search_cmd.rs`、
   `commands/explore_cmd.rs` 与 `commands/settings_cmd.rs` 中已有请求和取流入口。
2. 明确新来源承担搜索、元数据补全还是实际播放，保留原始曲目身份与最终播放来源的区别。
3. 同步前端来源模型、搜索状态、路由、登录、封面、歌词、下载和统计身份。
4. 添加解析 / 签名回归，并单独记录真实平台请求验证；不要提交账号凭据或签名 URL。

#### 2. 修改播放、队列或音效

1. 从 `src/stores/player.ts`、`src/modules/playback/` 和
   `src-tauri/src/commands/player_cmd.rs` 追踪请求，再检查 `src-tauri/src/audio/`。
2. 同时检查取消、快速切歌、seek、预取、失败回退、续播与音频信息更新。
   保留 generation 校验和来源 / 客户端相关缓存身份。
3. DSP 变更还需检查 `src/modules/audioEffects/`、`audio/dsp.rs`、`audio/dsp_settings.rs`
   及 `native/dsp/` 的参数布局和来源说明；不在实时音频回调中引入配置、锁等待或堆分配。
4. 逻辑测试外，在原生应用验证实际播放、输出设备切换和参数变化，记录系统与设备环境。

#### 3. 修改下载、本地音乐或标签

1. 查看 `src/modules/download/`、`src/stores/download.ts` 与
   `commands/download_cmd.rs`、`download_catalog.rs`、`download_metadata.rs`、`download_recovery.rs`。
2. 修改扫描和标签时检查 `commands/local_files_cmd.rs`、`library/local_index.rs`
   与 `library/local_file_tags.rs`，区分索引、歌单记录和真实文件。
3. 保留任务取消、恢复、sidecar 与清单一致性；使用测试文件验证失败 / 取消不会破坏原文件。
4. 涉及写标签时覆盖歌词、封面和 NP 元数据保留，并在写回后读回检查。

#### 4. 新增设置项

1. 在 `src/stores/settings.ts` 的 `AppSettings`、默认值与迁移规则中登记。
2. 同步 `src/views/SettingsView.vue`、`src/modules/settings/` 的搜索映射和 `src/i18n/` 翻译。
3. 后端行为复用已有 IPC 与 `src-tauri/src/settings/`，保持 `formatVersion` 和旧配置导入兼容。
4. 覆盖默认值、持久化、旧值迁移与重启后的行为；设置只在启动生效时明确提示。

#### 5. 修改歌词或桌面歌词

1. 查看 `src-tauri/src/lyrics/manager.rs`、`src/modules/lyrics/`、`src/stores/currentLyrics.ts`
   与 `src/components/LyricsView.vue`，区分解析、匹配和渲染职责。
2. 保留已匹配结果、翻译 / 罗马字、逐字结构、用户偏移和请求代次；不要假设来源格式相同。
3. 桌面歌词同时检查 `src/modules/desktopLyrics/`、`src/views/DesktopLyricsView.vue`
   与 `commands/desktop_lyrics_cmd.rs`，验证窗口锁定、点击穿透、拖动和关闭恢复。
4. 时间逻辑可用回归脚本验证；多窗口行为仍需在受影响桌面系统验证。

#### 6. 修改 GitHub / WebDAV 同步

1. 先读 `sync/models.rs`、`proto_models.rs`、`serializer.rs`、`archive/`、`merge.rs`、
   `manager.rs` 和 `commands/sync_cmd.rs`，记录对照的 Android 提交。
2. 保持 ProtoBuf 字段号与 V4 记录类型；新字段需要缺省读取，旧字段号不能重新分配。
   保留三路合并、因果 token、删除记录和成员恢复语义。
3. Bilibili 分 P 跳过规则按 `bvid + cid` 绑定，使用 `biliVideoSkipRules` 和 V4 类型 14；
   清空时保留 tombstone，避免旧端恢复规则。
4. 迁移授权必须与当前后端、目标、凭据和远端内容匹配。GitHub 发布保留 HEAD 条件更新；
   WebDAV 同时维护有限排他集合租约和无锁兼容路径，保留适用的 ETag 条件写入、
   清单指纹检查、发布后回读及完整对象校验。旧单文件可在有租约或无租约时迁移；
   无租约迁移的每个对象和新清单均使用 `If-None-Match: *` 只创建不覆盖，
   发布前还会重读旧源并核对升级确认指纹。不要跳过这些并发与迁移保护。
5. 执行桌面写 → Android 读 → Android 写 → 桌面读，覆盖编辑、删除、恢复与冲突。
   固定夹具的编解码测试不能替代 provider 和双端运行验收。

#### 7. 修改一起听

1. 同时检查 `src/stores/listenTogether/`、`src-tauri/src/listen_together/`
   与 `commands/listen_together_cmd.rs`，再对照 Android 客户端和 Worker。
2. 保持字段、角色权限、房间版本和旧协议回退；增量队列使用 `baseRoomVersion`，
   重复曲目以 `stableKey + occurrence` 定位。
3. 分享链接遵守设置和平台过滤，保留重连、看门狗与候选资源处理。
4. 自动回归外，验证桌面与 Android 同房、加入退出、重连、切歌、seek 和队列冲突。

#### 8. 修改窗口、快捷键或系统集成

1. 从 `src/main.ts`、`src/components/TitleBar.vue`、`src/modules/shortcuts/`、
   `src/modules/tray/` 及对应 Rust 命令 / `main.rs` 追踪生命周期。
2. 保留单实例、窗口状态、事件清理、IME 输入与全局快捷键边界；不要扩大不必要的 Tauri 权限。
3. 在实际 WebView 环境检查主窗口、托盘弹窗、桌面歌词、媒体键和退出路径。
   涉及平台分支时，明确记录已验证平台与未验证平台。

---

### 调试与日志 / Debugging & Logs

- 设置页连续点击版本号 7 次启用开发者模式，`Debug` 页提供平台探针、最近日志、
  崩溃报告与诊断信息；入口见 `src/views/DebugView.vue` 和 `commands/debug_cmd.rs`。
- 前端 `src/utils/logger.ts` 与 Rust `log` 统一进入 `src-tauri/src/logging/`。
  最近日志保存在有界内存缓冲中，不依赖文件日志开关。
- 文件日志位于系统数据目录下的 `NeriPlayer/logs/`，崩溃报告在其 `crashes/` 子目录。
  文件日志开关需要重启生效，日志级别可即时调整。
- 排查 FFmpeg 关注 `audio-decoder` 日志，检查库目录、架构、版本、依赖和签名。
  加载结果在进程内缓存，替换库或调整目录后重启应用 / 测试进程。
- 报告问题时附系统 / 架构、构建版本 / UUID、复现步骤、音源和实际结果。
  分享前人工检查脱敏结果，删除 Cookie、Token、私有路径与签名链接等敏感内容。

#### Linux WebKitGTK / NVIDIA

`src-tauri/src/linux_graphics.rs` 在 GTK / WebKit 初始化前检测已经加载的 NVIDIA 私有驱动，
通过 `/proc` 或 `/sys` 判断，不将仅存在 NVIDIA 硬件或 nouveau 等同于私有驱动。
检测到后，默认禁用 NVIDIA explicit sync、将驱动在途帧上限设为 1、关闭 DMA-BUF atlas，
并使用 SHM 共享内存合成。这里的在途帧上限不是帧率承诺，也不等于关闭整个 DMA-BUF renderer。
前端会将整页路由切换、播放页滑动和遮罩动画降为即时切换，减少大图层重绘。

已经显式设置的 NVIDIA / WebKit 环境变量优先，兼容层不覆盖它们。
例如显式设置 `WEBKIT_DISABLE_DMABUF_RENDERER` 时，不再叠加强制 SHM。
可按问题逐项对照以下开关，修改后以新进程验证：

| 环境变量 | 作用 |
| --- | --- |
| `NERI_DISABLE_NVIDIA_QUIRK=1` | 跳过整个兼容层，也不启用由该状态触发的前端转场降级 |
| `NERI_WEBKIT_KEEP_DMABUF=1` | 不强制 SHM，保留 DMA-BUF 硬件合成；atlas 仍按独立规则处理 |
| `NERI_WEBKIT_KEEP_DMABUF_ATLAS=1` | 不自动关闭 DMA-BUF atlas，不改变 SHM 的独立选择 |
| `NERI_NVIDIA_FRAMES_ALLOWED=N` | 覆盖默认在途帧上限 1；`0` 表示不设置该上限 |
| `NERI_NVIDIA_NO_VSYNC=1` | 显式关闭垂直同步，可能出现撕裂，默认不启用 |
| `NERI_WEBKIT_DMABUF_NO_GBM=1` | 请求绕过 NVIDIA GBM 分配路径，默认不启用 |
| `NERI_WEBKIT_FORCE_VBLANK_TIMER=1` | 请求使用定时 vblank，默认不启用 |
| `NERI_WEBKIT_SOFTWARE_COMPOSITING=1` | 关闭加速合成，作为排障兜底，可能明显降低性能 |

在 Linux shell 中仅对本次开发启动设置变量，例如：

```bash
NERI_WEBKIT_KEEP_DMABUF=1 pnpm tauri dev
```

不需要为排障全局修改系统配置。记录驱动、桌面会话、WebKitGTK 版本、实际环境变量和
`linux-graphics` 日志；这些兼容策略不能替代目标系统上的实际界面验证。

---

### 测试与提交流程 / Testing & PR

1. 阅读相关源码、文档和现有测试，检查 `git status`，保留已有未提交修改。
2. 保持修改范围聚焦，为行为修复增加能复现问题的回归。
3. 先执行针对性检查，再按影响范围运行类型检查、Rust 检查、原生验收和必要的打包。
4. 检查完整 diff，不引入可修复 warning，不提交构建产物、凭据、私有数据或无关依赖升级。
5. 严格填写 [PR 模板](.github/PULL_REQUEST_TEMPLATE.md)，记录实际命令、结果、未验证内容及原因；
   UI 变化附截图或录屏。Commit 与 PR 标题使用英文 Conventional Commits，
   例如 `fix(player): preserve track identity during fallback`。

常用检查入口：

```bash
pnpm test
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml --locked
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --locked
```

`pnpm test` 通过 `scripts/test-frontend.mjs` 枚举并串行运行 `package.json` 的全部 `test:*`。
这些脚本覆盖纯逻辑、store、组件 / IPC mock 与源码约束，不是浏览器端到端测试。
其中 `test:config-backup-codec` 使用项目固定 Rust 工具链运行实际配置编解码器与 Node 加密互通检查，
因此 `pnpm test` 也需要 Rust。测试工作目录位于被忽略的 `docs/tmp/`，结束后自动清理。
新增回归在 `package.json` 注册并沿用相邻测试模式。针对性入口示例：

| 变更领域 | 示例命令 |
| --- | --- |
| 播放、队列与续播 | `pnpm test:playback-request`、`pnpm test:playback-queue`、`pnpm test:long-form-progress` |
| 下载、本地扫描与持久化 | `pnpm test:download-cancellation`、`pnpm test:local-scan-store`、`pnpm test:user-data-persistence` |
| 歌词与桌面歌词 | `pnpm test:lyrics-request`、`pnpm test:lyric-offset-store`、`pnpm test:desktop-lyrics` |
| 音效 | `pnpm test:audio-effects-model`、`pnpm test:audio-effects-store`、`pnpm test:audio-effects-ui` |
| 同步与分 P 规则 | `pnpm test:sync-protocol-upgrade`、`pnpm test:history-sync`、`pnpm test:bili-video-skip` |
| 一起听 | `pnpm test:listen-together-protocol`、`pnpm test:listen-together-queue`、`pnpm test:listen-together-reconnect` |
| 原生界面与打包配置 | `pnpm test:tray-popup`、`pnpm test:title-bar-window-state`、`pnpm test:ffmpeg-bundle` |

完整清单以 `package.json` 为准。Rust 可以按模块过滤：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib sync
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib listen_together
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib library::local_file_tags::tests
```

FFmpeg 配置测试使用模拟库；实际解码测试还需匹配共享库。
未设置 `NERI_REQUIRE_FFMPEG=1` 时，部分测试缺库会跳过。
按 [FFmpeg 文档](src-tauri/native/ffmpeg/README.md)设置 `NERI_FFMPEG_DIR` 和该变量后运行解码测试，
再单独验证安装包内资源与安装后播放。

Android JVM 反向解码辅助检查：

```powershell
./scripts/test-sync-android-interop.ps1 -AndroidRoot <Android仓库路径> -ExportFixtures
```

脚本需要 PowerShell 7、Python、Java、Android 源码及指定版本的已有 Kotlin /
kotlinx.serialization Gradle 缓存。它还直接调用 `node:zlib` 的 `zstdDecompressSync`，
因此需要支持该 API 的 Node，例如 22 系列的 22.15.0 或更新版本；主项目 CI 使用的 Node 20
不能运行此辅助步骤。该 API 的加入版本见 [Node 官方文档](https://nodejs.org/download/release/v22.16.0/docs/api/zlib.html#zlibzstddecompresssyncbuffer-options)。
可用 `-NodePath` 指定对应的 Node 可执行文件；先用同一可执行文件自检，结果应为 `function`：

```powershell
node -p "typeof require('node:zlib').zstdDecompressSync"
```

脚本只读取 Android 源码与缓存、保留审计产物，
不安装缺失依赖、不访问云端，也不替代 Android 整仓 Gradle、provider 或设备互通测试。
跨端验收优先使用模拟器；操作真实手机或其他实体设备前需要获得明确授权。

当前工作流边界：

- **CI / `build.yml`**：main / master 推送、PR 或手动触发，运行前端回归与构建、四目标 Rust
  locked check，以及 Windows x64、macOS arm64、Linux x64 随包 FFmpeg 解码检查。
  不运行完整 Rust 单元测试集或 Clippy。
- **Artifacts / `artifacts.yml`**：main / master 推送或手动触发，生成四目标测试包；
  安装包保留 3 天、构建元数据保留 14 天，不发布 GitHub Release。
- **FFmpeg / `ffmpeg.yml`**：手动或被其他工作流调用，生成四份 `ffmpeg-<目标>` artifact。
- **Release / `release.yml`**：`v*` 标签或手动指定 `target_ref`，独立构建并检查安装包。
  手动路径生成构建版本标签；标签触发且标签含连字符时标记 prerelease。
  Release 附 FFmpeg 源码、构建脚本、macOS 安装说明与 `SHA256SUMS.txt`。

CI 和 Artifacts 对纯文档、许可证和模板变更设有路径忽略；没有运行记录不等于检查通过。
GitHub prerelease 标记与项目仍处于预览阶段是不同概念。工作流检查包内容不替代安装后的原生验收。

---

### 法律与许可 / Legal & License

- 本项目仅供学习与研究，请遵守适用法律与来源平台规则；项目及维护者不接受任何形式的
  赞助、捐赠或商业资助。
- 当前项目与贡献使用 **GPL-3.0-or-later**，见 [LICENSE](LICENSE)。
  2026-10-07 之前按 MIT 发布的代码保留 [原声明](LICENSES/MIT.txt)。
- `vendor/applemusic-like-lyrics` 按自身 AGPL-3.0 许可分发。
  FFmpeg 共享库未启用 GPL / nonfree 组件，按 LGPL-2.1-or-later 分发，
  需要保留许可、构建信息、对应源码和构建脚本。
- Android DSP 移植来源及 GPL-3.0 文本见 [native/dsp](src-tauri/native/dsp/README.md)；
  桌面 DSP 不采用 Android USB 路径的替代授权。
- 移植 Android 或其他项目实现时，保留版权和许可声明，在提交正文注明来源及对照版本。
  不提交或公开 Cookie、Token、密码、完整配置导出、私有曲库、数据库或敏感日志。

---

### 沟通方式 / Communication

缺陷报告与功能建议请使用 [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues)，
提供可复现步骤、构建信息和已脱敏证据，并遵守 [社区行为准则](CODE_OF_CONDUCT.md)。
涉及平台接入、同步模型、一起听协议或大幅架构调整，建议先通过 Issue 对齐范围与兼容性要求。


