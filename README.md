[English](./README_EN.md) | [中文](./README.md)

<h1 align="center">NeriPlayer Desktop (音理音理!)</h1>

<div align="center">

<h3>把多源在线播放、本地管理、逐字歌词和自有同步带到桌面的音频播放器</h3>

<p>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/cwuom/NeriPlayer-Desktop/total?style=social" />
  </a>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Release" src="https://img.shields.io/github/v/release/cwuom/NeriPlayer-Desktop?include_prereleases&label=Release" />
  </a>
  <img alt="Preview" src="https://img.shields.io/badge/status-preview-orange" />
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-desktop-4C8BF5" />
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml">
    <img alt="CI" src="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml/badge.svg" />
  </a>
  <a href="https://t.me/ouom_pub">
    <img alt="Telegram" src="https://img.shields.io/badge/Telegram-@ouom__pub-blue" />
  </a>
</p>

<p>
  <img src="app-icon.png" width="200" alt="NeriPlayer Desktop" />
</p>

<p>
本项目的名称及图标灵感来源于《星空鉄道とシロの旅》中的角色「风又音理」。
</p>

<p>
项目是 <a href="https://github.com/cwuom/NeriPlayer">NeriPlayer Android</a> 的桌面移植，
采用 <strong>Tauri 2 + Vue 3 + Rust</strong>，面向 Windows、macOS 和 Linux，
围绕「多源探索、原生播放、本地可控、跨端同步」持续打磨。
</p>

<strong>Preview / 预览版，持续迭代中</strong>

</div>

> [!WARNING]
> 本项目仅供学习与研究使用，请勿将其用于任何非法用途。
> 请只在你拥有权利、授权或第三方平台规则允许的范围内访问、播放或保存内容。
> 本项目不提供媒体内容、密钥、规避付费/DRM/地区限制的方案，也不提供公共媒体代理或再分发服务。
>
> 本项目及维护者不接受任何形式的赞助、捐赠或商业资助。

---

> [!NOTE]
> NeriPlayer Desktop 不提供公共云端曲库或媒体分发服务。
> 在线能力取决于第三方平台接口、网络、账号与内容权限，会员或受限内容仍需遵循原平台规则。
> 当前版本仍处于预览阶段；更新或迁移前请保留重要歌单和配置备份。
> 打包元数据中的 `1.0.0` 不表示已经发布稳定版，具体变化以对应构建和发布说明为准。

---

> [!CAUTION]
> **Linux 版本为测试环境**：渲染依赖 WebKitGTK，可能不稳定、性能明显低于 Windows / macOS，页面切换时也可能闪一下。
>
> **光敏性癫痫提示（所有 Linux 版本）**：播放页的动态背景（WebGL 着色器）在该环境下切换背景或页面时会**闪烁**，且无法通过设置消除，因此首次运行时会主动关闭并给出提示。
> 若你或身边的人有光敏性癫痫，或对闪烁敏感，请**不要**手动开启（**设置 → 动效设置 → 动态背景**，开启时会再次严重警告），可改用「封面模糊背景」。
> 其他平台版本不受该问题影响。

---

## 快速定位 / Start here

如果你只是想体验应用，请看 [快速体验](#快速体验--getting-started)。
如果你想了解项目能力，请看 [项目亮点](#项目亮点--why-it-stands-out)
和 [核心特性](#核心特性--key-features)。
如果你准备贡献代码，请直接阅读 [CONTRIBUTING.md](./CONTRIBUTING.md)。
如果你要自建一起听服务端，请看
[一起听服务端部署](#一起听服务端部署--listen-together-deployment)。
跨端使用前请先阅读 [GitHub 同步](#github-同步--github-sync)
和 [WebDAV 同步](#webdav-同步--webdav-sync)中的兼容说明。

```text
NeriPlayer Desktop
├── 多源在线播放：网易云 / Bilibili / YouTube Music
├── 本地优先数据：曲库、下载、歌单、历史、队列、统计
├── 可选自有同步：GitHub / WebDAV 元数据同步
├── 桌面播放体验：原生音频、逐字歌词、音效、托盘、媒体键、桌面歌词
└── 调试与反馈：实时日志、文件日志、崩溃报告、连通性探针
```

## 项目简介 / About

NeriPlayer Desktop 使用 **Vue 3 + TypeScript** 构建界面，
以 **Rust** 处理平台请求、音频解码、播放、存储和同步，通过 **Tauri 2** 接入桌面系统。
它整合网易云音乐、Bilibili、YouTube Music 与本地音频，
提供从内容发现到队列播放、歌词显示、文件管理和元数据同步的完整链路。

当前定位：

- **本地优先**：歌单、收藏、历史、队列、播放统计与缓存默认保存在本机；
  导入本地文件不要求登录平台或开启云同步。
- **原生音频**：音频由 Rust 解码并通过 `cpal` 输出，
  播放页的 WebView 负责界面、逐字歌词与动态背景。
- **账号与内容权限**：按需登录来源平台，访问该账号可用的内容；
  QQ 音乐目前仅有有限游客接口和歌词/元数据能力。
- **可选同步**：将元数据保存到用户自己的 GitHub 仓库或 WebDAV，
  支持手动与自动同步，通过三路合并保留各端编辑。
- **跨端协作**：同步与一起听沿用 Android 端模型和协议，
  兼容版本间可同步歌单、历史和统计，也可加入同一房间一起听。
- **桌面体验**：独立桌面歌词、托盘、系统媒体会话、媒体键与可配置快捷键，
  并提供输出设备选择、原生 DSP 和 macOS 菜单栏歌词。

---

## 项目亮点 / Why it stands out

- **多源播放与明确的回退策略**：
  探索页按平台搜索，音乐库整合平台歌单、收藏夹与本地内容。
  网易云歌曲无法完整播放时，可按设置优先匹配本地音频，再尝试 Bilibili 替代音源；
  队列保留原歌曲身份，下载仍使用原平台音源。
- **独立于 WebView 的音频链路**：
  内置 Symphonia 解码常见格式，随包 FFmpeg 扩展解码与解封装能力。
  提供输出设备选择、独立倍速与音调、响度均衡、淡入淡出和交叉淡入淡出。
- **从简单调节到原生 DSP**：
  29 种内置预设、10 段图示均衡、参数均衡、AutoEQ 导入、音色与空间处理、
  压缩器、限幅器和扬声器优化，可按输出设备记忆音效。
  原生 DSP 来源和桌面扩展见 [DSP 说明](./src-tauri/native/dsp/README.md)。
- **完整的逐字歌词体验**：
  基于 AMLL 显示逐字高亮、翻译和音译，支持多源自动获取、手动匹配、
  编辑与偏移调整；播放页提供封面取色的 WebGL 流体背景和音频响应。
- **真正可管理的本地曲库**：
  多文件夹索引、文件监视与增量扫描、筛选直接播放、歌单与艺术家聚合、
  标签读写和下载管理相互衔接；移除曲库文件夹不会删除原始音频。
- **同步保留编辑与删除语义**：
  GitHub/WebDAV 采用三路合并和 V4 分块归档，
  保留歌单成员、最近播放、统计和跳过区间的删除/清空记录，
  避免只用最后写入覆盖整个曲库。
- **适合桌面的日常操作**：
  独立桌面歌词、托盘、关闭到托盘、媒体键、系统媒体会话、
  可配置的应用内与全局快捷键，以及 macOS 菜单栏歌词。
- **诊断信息可以主动获取**：
  开发者模式提供实时日志、崩溃报告和连通性探针，
  文件日志可单独开启，反馈可以定位到具体构建版本。

---

## 快速体验 / Getting Started

### a. 下载 Release 版本（推荐）

1. 前往 [GitHub Releases](https://github.com/cwuom/NeriPlayer-Desktop/releases)，阅读对应发布说明
2. 根据操作系统和架构选择安装包：

   | 平台 | 架构 | 发布包 | 环境说明 |
   | --- | --- | --- | --- |
   | Windows | x64 | `.msi` / NSIS `.exe` | 需要 WebView2 运行时 |
   | macOS | Apple Silicon / arm64 | `*-adhoc.dmg` | 发布构建最低系统配置为 11.0 |
   | macOS | Intel / x64 | `*-adhoc.dmg` | 发布构建最低系统配置为 10.15 |
   | Linux | x64 | `.deb` / `.rpm` / `.AppImage.tar.gz` | 需要兼容的图形环境及系统运行库 |

3. Release 附带 `SHA256SUMS.txt`、macOS 安装说明，以及随包 FFmpeg 的源码和构建脚本
4. 启动后按需在设置中登录网易云、Bilibili 或 YouTube，也可先导入本地音乐
5. 根据需要调整输出设备、播放音质、下载目录、歌词和快捷键；同步服务另行配置

Windows 安装包当前未做代码签名。macOS 使用 ad-hoc 签名，未经 Apple 公证；
请先阅读发布附带的 `macOS-install.txt`，系统拦截的处理方式参考
[Apple 官方说明](https://support.apple.com/en-us/102445)。

Linux AppImage 先解压，再为其中的 `.AppImage` 添加执行权限。
deb/rpm 可使用发行版包管理器安装；打包配置包含 WebKitGTK 4.1、GTK 3、ALSA 等依赖。
表中的系统下限来自构建配置，不代表每个版本和桌面环境都已验证。

### b. 下载 CI 版本

前往 [Artifacts 工作流](https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/artifacts.yml)，
选择成功构建，下载对应平台的 artifact，再解压使用其中的安装包。
下载 Actions artifact 通常需要登录 GitHub，构建产物有保留期限。

CI 与 Release 可能对应不同提交。反馈时请提供「设置 → 关于」中可复制的构建版本，
不要只写 `1.0.0` 或安装包名称。

### c. 本地构建

准备 Git、Node.js、**pnpm 9.15.9**、rustup 和
[Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)。
仓库固定 **Rust 1.95.0**，CI 使用 **Node.js 20**；
原生构建还需要支持 C++17 的编译器。

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

已有克隆缺少歌词子模块时，执行 `git submodule update --init --recursive`。
`pnpm tauri dev` 同时启动 Vite 与原生应用，默认端口 `1420`，占用时自动选择空闲端口。

> `pnpm dev` 仅启动界面预览，不提供 Rust IPC、原生音频或系统集成。
> Debug 构建的凭据存储与性能也不同于发布版，不能用浏览器预览替代原生验收。

本地发布构建与 FFmpeg 打包请阅读
[CONTRIBUTING.md](./CONTRIBUTING.md#构建发布版--release-build)
和 [FFmpeg 构建说明](./src-tauri/native/ffmpeg/README.md)。
普通 `pnpm tauri build` 不会自动下载或加入 FFmpeg 共享库。

---

## 核心特性 / Key Features

- **首页与探索**：最近播放、常用歌单与分区推荐，网易云/Bilibili/YouTube Music 搜索，
  可管理搜索历史；国际化模式下支持已登录账号的 YouTube Music 首页货架。
- **音乐库与艺术家**：本地歌单、收藏歌单与艺术家、下载、网易云歌单/专辑、
  Bilibili 收藏夹与 YouTube Music 歌单；支持分类搜索、分页、多选、拖拽和右键操作。
- **队列与续播**：随机、循环、顺序调整、播放进度记忆和长音频续播；
  随机模式可查看实际播放顺序，切回原始顺序后可拖拽排序。
- **原生音频与音效**：输出设备、声道平衡、0.1x–4x 倍速、0.25x–2x 独立音调、
  唱片效果、响度均衡、图示/参数均衡、AutoEQ、音色与空间处理、用户预设及设备记忆。
- **歌词显示与匹配**：逐行/逐字、翻译、罗马字、字体和字号、歌词模糊、
  本地/嵌入歌词、来源与单曲偏移；可从网易云、QQ 音乐、酷狗、LRCLIB 和 AMLL TTML 手动匹配。
- **独立桌面歌词**：单/双/三行、逐字或整行显示、翻译/罗马字、字体、渐变、描边、
  阴影、透明度、背景、位置与尺寸记忆，以及锁定和点击穿透。
  Windows 创建窗口时若遇到 WebView2 断连错误 `0x80010108`，会使用专用浏览器数据目录
  尝试重建一次，恢复结果记录到日志。
- **本地文件**：多文件夹曲库、增量扫描、扫描预览与取消、搜索和多选、
  本地艺术家聚合、直接播放与导入歌单，支持标题、歌手和专辑标签编辑。
  可按默认顺序、文件修改时间、歌曲名称、歌手、专辑和时长升降序排列，
  自动记忆排序偏好；播放全部和播放所选遵循当前顺序。
- **曲库交互**：默认单击播放，可在「设置 → 播放设置 → 双击播放曲目」切换为双击；
  当前播放曲目以整行高亮和边框标识，暂停时保留。
- **下载管理**：单曲与批量下载、1–8 并发（默认 6）、独立音质、目录和文件名模板、
  取消、重试、重新下载、文件定位与重启恢复。
- **下载元数据**：可补齐并读回核验标签、封面、歌词、翻译和罗马字；
  标准化歌词嵌入可单独开启。歌词保存在 `Lyrics/`，封面保存在 `Covers/`，
  来源引用保存在音频旁的 `.npmeta.json`。
- **评论**：网易云和 Bilibili 评论浏览、楼中楼、复制，以及登录后的发表、回复和点赞；
  请求结果不确定时保留草稿，不自动重发。
- **Bilibili 分 P 与跳过区间**：选择起始分 P、整组加入队列，按 `bvid + cid` 编辑、
  预览和同步多段跳过区间；一起听期间暂停自动跳过。
- **备份与统计**：歌单 JSON 导入/导出、完整配置导入/导出、最近播放管理、
  播放次数与收听时长统计，以及 GitHub/WebDAV 元数据同步。
- **一起听**：创建/加入房间、邀请、播放与队列同步、成员控制、重连、
  循环/随机模式、自定义服务端与可选的会话播放地址共享。
- **外观与语言**：简体中文、繁体中文、English、日本語，浅色/深色主题、封面取色、
  自定义背景、流体背景、界面/歌词字体、界面缩放和设置搜索。
- **桌面集成与调试**：托盘、关闭到托盘、单实例、系统媒体会话、媒体键、
  应用内/全局快捷键、macOS 菜单栏歌词、实时日志与调试报告。

### 默认快捷键

`Mod` 表示 Windows/Linux 的 `Ctrl`，macOS 的 `Cmd`。
可在「设置 → 快捷键」录制、清空或恢复绑定，界面会提示冲突和注册失败。

| 应用内快捷键 | 动作 |
| --- | --- |
| `Space` | 播放/暂停 |
| `Mod + →` / `Mod + ←` | 下一首/上一首 |
| `→` / `←` | 快进/快退 |
| `Shift + →` / `Shift + ←` | 较大步长快进/快退 |
| `↑` / `↓` | 音量加/减 |
| `M` / `S` / `R` | 静音/随机/循环模式 |
| `L` / `D` | 喜欢/取消喜欢、切换桌面歌词 |
| `Mod + P` / `Mod + F` | 展开播放页、搜索 |
| `Esc` | 依次关闭弹窗、面板或播放页 |

全局快捷键默认关闭。开启后，`Mod + Alt + P` 控制播放，
`Mod + Alt + ←/→` 切歌，`Mod + Alt + ↑/↓` 调音量，
`Mod + Alt + L` 切换喜欢，`Mod + Alt + D` 切换桌面歌词，
`Mod + Alt + N` 显示/隐藏窗口。

输入框和正在操作的控件会接管相应按键。`Esc` 与 `Tab` 保留给关闭和焦点导航，不能改绑。

---

## 平台现状 / Platform Status

| 来源 | 当前能力 | 能力边界 |
| --- | --- | --- |
| 网易云音乐 | 登录、搜索、推荐、歌单/专辑、艺术家、喜欢、多音质播放、下载、YRC/翻译/罗马字歌词、评论 | 会员、地区和账号权限仍生效；替代音源是可选播放策略 |
| Bilibili | 登录、搜索、发现、收藏夹、UP 主、DASH 音频与 MP4 回退、分 P、下载、跳过区间、评论 | 主要是视频的音频播放；取流受内容及账号权限影响，歌词通过外部来源匹配 |
| YouTube Music | 登录、首页、搜索、歌单与艺术家、多客户端取流、下载和歌词 | 依赖网络、地区、会话及客户端可用性；兼容逻辑不代表绕过平台限制 |
| QQ 音乐 | 后端搜索、游客接口可用范围内的取流、下载和歌词/元数据匹配 | 依赖游客接口及内容可用性 |
| 本地音频 | 多文件夹扫描、直接播放、歌单与艺术家管理、标签和歌词 | 路径必须在当前设备有效；格式支持取决于可用解码器 |

桌面端提供原生音频输出、设备选择和 DSP 音效，配合独立桌面歌词、托盘、
系统媒体会话、媒体键与可配置快捷键，支持窗口内操作和后台播放控制。

---

## 实现概览 / Implementation Notes

### 构建与版本

- 技术栈：Tauri 2、Vue 3、TypeScript、Pinia、Vite、Rust 与 C++17
- 包管理器：`pnpm@9.15.9`；Rust 工具链：`1.95.0`；CI Node.js：`20`
- 应用内版本：`<git短哈希>.<MMddHHmm>`，另提供构建时间和 UUID
- 原生配置：`src-tauri/tauri.conf.json`；发布矩阵：`.github/workflows/release.yml`
- 歌词组件：`vendor/applemusic-like-lyrics` Git 子模块
- 完整解码包需额外合并 FFmpeg 资源配置；本地开发不要求预先安装 FFmpeg 开发包

### 模块结构

| 路径 | 职责 |
| --- | --- |
| `src/views/`、`src/components/` | 页面、播放器、歌词、菜单和设置界面 |
| `src/stores/`、`src/modules/` | 前端状态、播放调度、歌词、快捷键与一起听 |
| `src-tauri/src/commands/` | Tauri 命令与前后端数据边界 |
| `src-tauri/src/api/`、`auth/` | 平台请求、登录、签名与会话 |
| `src-tauri/src/audio/`、`src-tauri/native/` | 原生播放、解码、输出、FFmpeg 与 DSP |
| `src-tauri/src/lyrics/`、`library/` | 歌词解析匹配、曲库、标签和统计 |
| `src-tauri/src/db/`、`sync/` | SQLite、迁移、同步模型、合并和 V4 归档 |
| `src-tauri/src/listen_together/` | WebSocket 会话与协议 |
| `scripts/`、`.github/workflows/` | 回归检查、构建、打包和 CI |

### 入口与导航

主界面由首页、探索、音乐库、设置与开发者模式下的 Debug 页组成，
Mini Player、Now Playing、队列与评论面板围绕共享播放状态工作。
桌面歌词和托盘弹窗使用独立窗口，通过原生命令与事件同步。

### 播放、缓存与服务

- `audio/` 负责播放、seek、缓冲、解码、输出与媒体会话；
  在线播放支持 Range 拉流及分片定位，这与下载文件的断点续传是不同链路。
- Symphonia 是内置解码后端；FFmpeg 共享库按运行时搜索顺序加载，
  缺失或不兼容时尝试回退，不能保证缺库后所有格式仍可播放。
- DSP 支持均衡、音色、空间与动态处理；多声道解码和下混不表示 Atmos 或空间音频输出。
- 清理缓存与删除下载分别管理，普通缓存清理保留用户主动下载的音频。

### 搜索与数据来源

探索页按网易云、Bilibili 和 YouTube Music 分类。
播放地址、音质、封面与歌词由 Rust 平台接口和歌词管理器获取；
QQ 音乐、酷狗、LRCLIB 与 AMLL TTML 也参与歌词匹配。
外部匹配结果受曲目版本、歌手与时长影响，现场、翻唱或剪辑版应人工核对。

### 本地数据与安全

- `neri_user_data.db` 保存歌单、收藏、历史、队列、统计、下载记录、同步元数据与详情/歌词缓存。
- 本地曲库文件夹及文件索引另存为 `local-library.json`，设置和凭据也有独立存储。
- 旧 JSON 与 localStorage 按数据域迁移，旧 JSON 备份保存在 `legacy-json-backup/`。
- Release 使用应用侧加密凭据文件；Debug 使用随机路径的明文凭据文件。
  这不等同于系统硬件密钥保护，也不承诺抵御同机同用户进程。
- 完整配置导出为可能包含平台授权与同步密码的明文 JSON，不能作为可公开分享的附件。

### 下载、本地导入与备份

- 本地文件可以直接播放，也可由用户导入歌单；扫描不会自动创建默认歌单。
- 下载过程包含音源解析、传输、元数据处理和读回校验。
  开启元数据补齐时，封面或标签处理失败会显示任务失败并允许重试。
- 元数据补齐默认开启，标准化歌词嵌入默认关闭；来源未提供的信息不会补造。
- 重启会恢复未完成队列并重新解析、下载；取消与重试不提供按字节续传保证。
- 歌单 JSON 用于歌单迁移，完整配置用于设置与授权迁移；
  两者都不包含完整音频文件，也不是数据库的完整备份。

实现路径、扩展入口和验证命令请阅读 [CONTRIBUTING.md](./CONTRIBUTING.md)。

---

## 一起听服务端部署 / Listen Together Deployment

NeriPlayer Desktop 内置一起听客户端，附带默认服务地址，也可以连接自建的兼容服务端。
服务端基于 **Cloudflare Workers + Durable Objects**，通过 WebSocket 同步房间状态。

服务端源码与部署入口：

- Android 仓库中的
  [np-submodule/NeriPlayer-LTW](https://github.com/cwuom/NeriPlayer/tree/master/np-submodule/NeriPlayer-LTW)
- 公开部署模板：
  [TheSmallHanCat/NeriPlayer-LTW](https://github.com/TheSmallHanCat/NeriPlayer-LTW)

桌面仓库不包含服务端源码；部署和协议要求以服务端仓库文档为准。

### 一键部署到 Cloudflare Workers

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/TheSmallHanCat/NeriPlayer-LTW)

默认服务地址为 `https://neriplayer.hancat.work`。
自建部署后，在「设置 → 一起听」替换服务端地址并测试，再从一起听面板创建或加入房间。

- 创建房间前先播放可共享的在线曲目；当前房间队列不共享本地文件和 QQ 音乐曲目
- 房间同步播放、暂停、进度、队列和循环/随机模式，支持成员控制、成员变化暂停与重连
- 首次加入需要有效邀请；请完整保留邀请信息，不要只发送房间号
- 临时会话播放地址共享默认开启，可在设置中关闭；参与房间前请确认该设置，
  开启时会向服务端和成员提供当前曲目的临时播放地址
- 跨端同房要求 Android、桌面客户端和服务端协议兼容，各成员仍需具备对应内容的访问条件

---

## GitHub 同步 / GitHub Sync

可将本地元数据同步到**用户自己的 GitHub 仓库**。
应用内新建仓库时默认创建为私有仓库，也支持接入已有仓库。

当前同步对象包括：

- 本地歌单、曲目元数据、自定义信息、匹配歌词和单曲歌词偏移
- 收藏歌单与以收藏记录保存的艺术家
- 歌单最近打开时间、次数及删除记录
- 最近播放、删除记录与长音频续播信息
- 播放次数、收听时长、每日统计桶及清空标记
- 按 `bvid + cid` 绑定的 Bilibili 跳过区间及清空标记

### 技术细节

- **V4 归档**：当前写入 ProtoBuf 清单与 Zstd 分块，清单文件名仍为
  `neriplayer-sync-v3.manifest`；不能据文件名判断协议版本。
- **合并策略**：保留三路合并、成员因果记录与删除标记，
  不以整库最后写入覆盖代替冲突处理。
- **条件发布**：读取固定提交，发布时检查分支头；分支冲突会报告或重试合并，
  不强制覆盖其它设备提交。GitHub 服务需要支持相关 GraphQL 发布端点。
- **旧格式**：兼容读取旧 JSON、GZIP、历史 Base64 和 V3 归档；
  新清单损坏或遇到未知版本时会报错，不静默使用旧快照。
- **跨端升级**：所有参与设备应支持 V4。旧客户端继续读写旧文件时，
  无法与 V4 归档自动汇合，迁移前请更新各端并保留备份。
- **安全边界**：远端归档不做端到端加密；不上传音频、下载文件、平台 Cookie 或播放 Token。
  本地文件路径和输出设备等配置仍需在另一设备单独准备。
- **协议互操作**：仓库提供 Android 冻结向量和
  [Kotlin 互操作脚本](./scripts/test-sync-android-interop.ps1)，
  用于核对跨端归档兼容性。

### 使用方法

1. 打开「设置 → 备份与同步」
2. 按应用内说明创建并校验 GitHub Personal Access Token；当前 classic Token 流程使用 `repo` 权限，
   令牌类型和权限含义参考 [GitHub 官方文档](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
3. 新建默认私有仓库，或选择已有同步仓库
4. 跨设备连接同一目标；出现格式升级提示时，先更新其它设备并备份，再确认迁移
5. 手动执行一次同步，根据需要启用自动同步；后台同步是最终一致的，不是实时推送

---

## WebDAV 同步 / WebDAV Sync

WebDAV 使用与 GitHub 相同的同步对象、V4 归档和合并规则。

1. 在「设置 → 备份与同步」选择 WebDAV，填写远端地址、用户名和密码
2. 测试目标可读写，建议使用 HTTPS
3. 首次同步前确认备份与格式升级提示，跨设备使用相同目标

WebDAV 会在目标位置保存清单与内容对象，并使用租约、ETag 条件写入和回读核验保护发布。
首次锁请求被服务端拒绝时，兼容路径可允许无锁同步；
没有可用 ETag 时只能用内容指纹和回读检测冲突，不能保证并发写入的原子性。
自动清理旧分块仍要求有限租约和强 ETag。

WebDAV 远端同样不是端到端加密备份，用户名、密码与配置导出文件均需自行保管。
同步不传输音频文件，也不会自动修复另一设备的本地路径。

---

## 发展规划 / Roadmap

### 方向探索

这些方向会根据维护精力、平台变化与反馈调整，不承诺固定周期。

- [ ] 更完整的 QQ 音乐账号、探索和音乐库能力
- [ ] 下载按字节断点续传
- [ ] 应用内自动更新
- [ ] 更多系统、桌面环境与跨设备场景的兼容验证

### 近期已落地

- [x] 原生音频输出、FFmpeg 扩展解码与完整 DSP 设置
- [x] AMLL 逐字歌词、多源手动匹配与独立桌面歌词
- [x] 本地多文件夹曲库、增量扫描、艺术家管理与标签编辑
- [x] SQLite 用户数据与旧存储迁移
- [x] GitHub/WebDAV V4 归档、三路合并与 Android 协议向量
- [x] 一起听房间、队列与播放模式同步
- [x] 网易云/Bilibili 评论浏览与登录后互动
- [x] Bilibili 分 P、下载管理和跳过区间
- [x] 托盘、媒体会话、全局快捷键与多语言界面

---

## 问题反馈 / Bug Report

1. 在「设置 → 关于」复制构建版本，记录操作系统、架构和复现步骤
2. 在设置页连续点击**版本号 7 次**开启开发者模式，进入 Debug 查看实时日志、崩溃报告或导出调试报告
3. 需要持久日志时，在设置中开启文件日志并重启应用后重新复现；日志级别调整即时生效
4. 前往 [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues)，使用相应模板提交

播放或歌词问题请说明平台、曲目版本和操作过程；跨端问题请同时提供桌面、Android 与服务端版本。
分享日志前检查个人路径、临时播放地址与凭据，不要附上完整配置或数据库。

---

## 已知问题 / Known Issues

### 网络

- 搜索、登录、播放和歌词可能受网络、代理、地区、账号权限及接口变化影响。
  先检查原平台内容是否可访问，再核对本应用登录态与代理设置。
- 代理设置提供「绕过系统代理」开关；应按当前网络选择，而不是统一要求关闭或启用。
- Debug 连通性探针只表示传输层收到响应，不证明业务请求、账号授权或媒体播放成功。

### 能力边界

- **预览版与更新**：当前不提供应用内自动更新，请从对应 Release 手动安装。
- **解码与音频设备**：格式支持取决于内置和已加载的 FFmpeg 解码器；
  输出与媒体键行为也受系统及设备影响。
- **下载恢复**：未完成任务会重新下载，不支持按字节续传；
  元数据处理失败也可能使已传输任务显示失败。
- **歌词匹配**：逐字、翻译和音译依赖来源数据；没有对应内容时无法生成完整时间轴或译文。
- **系统集成**：托盘、点击穿透、全局快捷键与媒体会话受操作系统和桌面环境影响；
  快捷键可能与其它应用冲突。
- **Linux 渲染**：Linux 端整体属于测试环境，渲染依赖 WebKitGTK；NVIDIA 私有驱动下会额外启用兼容处理并减少整页转场。
  WebKitGTK 2.54 重写了合成器后，动态背景与页面切换会出现间歇性闪烁（上游 WebKit bug 327524，2.54.2 仍未修复），
  因此首次运行会关闭动态背景并提示；仍有闪烁时可按 [贡献指南](./CONTRIBUTING.md#调试与日志--debugging--logs)排查。
- **同步边界**：远端只同步元数据；本地音频、下载、设备路径和账号授权需另行准备。
  V4 升级和多设备并发仍应先备份并确认客户端兼容。

---

## 隐私与数据 / Privacy

- 不提供自己的公共云端媒体分发服务，不接入广告 SDK、第三方统计或崩溃分析 SDK。
- 歌单、收藏、历史、队列、播放统计、设置、缓存与授权默认保存在用户设备本地。
- 主动开启 GitHub/WebDAV 同步后，元数据写入用户配置的远端；
  不将音频、下载文件、平台 Cookie 或播放 Token 上传给项目维护者。
- 本地播放历史和统计不回写第三方音乐平台；登录、搜索、播放、评论等请求仍由相应平台处理。
- 一起听会与配置的服务端和房间成员交换会话信息；
  临时播放地址共享默认开启，可在设置中关闭。
- Release 的平台授权与同步凭据使用应用侧加密文件；
  Debug 使用明文凭据文件，两者的安全边界不同。
- 完整配置导出为明文 JSON，可能包含 Cookie、Token 和密码，请妥善保管，避免公开分享。
  本地数据库、日志与崩溃报告也可能含个人曲库和路径。
- 第三方平台和同步服务的访问日志、保存策略与风控规则，由对应服务自行负责。

---

## 鸣谢 / Reference

<table>
<tr>
  <td><a href="https://github.com/cwuom/NeriPlayer">NeriPlayer Android</a></td>
  <td>功能、同步模型、一起听协议与原生 DSP 来源</td>
</tr>
<tr>
  <td><a href="https://github.com/amll-dev/applemusic-like-lyrics">applemusic-like-lyrics</a></td>
  <td>逐字歌词渲染与流体背景组件</td>
</tr>
<tr>
  <td><a href="https://github.com/chaunsin/netease-cloud-music">netease-cloud-music</a></td>
  <td>网易云接口实现参考</td>
</tr>
<tr>
  <td><a href="https://github.com/SocialSisterYi/bilibili-API-collect">bilibili-API-collect</a></td>
  <td>Bilibili 接口资料</td>
</tr>
<tr>
  <td><a href="https://github.com/yt-dlp/ejs">ejs</a></td>
  <td>YouTube JavaScript 挑战求解参考与随包资源</td>
</tr>
<tr>
  <td><a href="https://lrclib.net">LRCLIB</a></td>
  <td>开放歌词服务</td>
</tr>
<tr>
  <td><a href="https://ffmpeg.org/">FFmpeg</a></td>
  <td>扩展音频解码与解封装</td>
</tr>
</table>

---

## 更新周期 / Update Cycle

- 项目处于预览版持续迭代阶段，Release 按功能和维护情况发布。
- 核心播放、本地数据、歌词、同步与跨平台兼容会持续维护。
- CI 构建用于体验开发中的改动；变化和已知事项以对应提交及发布说明为准。

---

## 支持方式 / Support

- 项目及维护者不接受赞助、捐赠或商业资助。
- 欢迎通过提交可复现的 Issue、PR、改进文档或参与预览版验证支持项目。

---

## 许可证 / License

NeriPlayer Desktop 使用 **GPL-3.0-or-later** 发布，详细条款见 [LICENSE](./LICENSE)。

- 2026-10-07 之前以 MIT 发布的代码保留原声明，见 [LICENSES/MIT.txt](./LICENSES/MIT.txt)。
- `vendor/applemusic-like-lyrics` 按其 AGPL-3.0 许可证分发。
- 移植的原生 DSP 遵循其 GPL-3.0 来源许可，详见 [DSP 说明](./src-tauri/native/dsp/README.md)；
  不适用 Android USB 路径的替代授权。
- 随包精简 FFmpeg 未启用 GPL 组件，以 LGPL-2.1-or-later 分发；
  来源、构建参数和源码获取方式见 [FFmpeg 说明](./src-tauri/native/ffmpeg/README.md)及 Release 附件。
- 第三方资源与源码保留各自许可，不能将仓库主许可证视为对全部依赖的重新授权。

---

# Contributing to NeriPlayer Desktop / 贡献指南

贡献前请先阅读完整的 [CONTRIBUTING.md](./CONTRIBUTING.md)，并遵守
[社区行为准则](./CODE_OF_CONDUCT.md)。

---

## Star History

<a href="https://www.star-history.com/?repos=cwuom%2FNeriPlayer-Desktop&type=timeline&legend=bottom-right">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=cwuom/NeriPlayer-Desktop&type=timeline&theme=dark&legend=bottom-right" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=cwuom/NeriPlayer-Desktop&type=timeline&legend=bottom-right" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=cwuom/NeriPlayer-Desktop&type=timeline&legend=bottom-right" />
  </picture>
</a>
