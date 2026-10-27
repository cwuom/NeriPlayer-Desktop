[English](./README_EN.md) | [简体中文](./README.md)

<h1 align="center">NeriPlayer Desktop</h1>

<div align="center">

<h3>🎵 多源在线播放、本地音乐管理、逐字歌词与自有云同步</h3>

<p>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/cwuom/NeriPlayer-Desktop/total?style=social" /></a>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases"><img alt="Preview release" src="https://img.shields.io/github/v/release/cwuom/NeriPlayer-Desktop?include_prereleases&label=Preview" /></a>
  <img alt="Status: Preview" src="https://img.shields.io/badge/status-preview-orange" />
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-desktop-4C8BF5" />
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml"><img alt="CI" src="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml/badge.svg" /></a>
  <a href="./LICENSE"><img alt="License" src="https://img.shields.io/badge/License-GPL--3.0-blue" /></a>
  <a href="https://t.me/ouom_pub"><img alt="Telegram" src="https://img.shields.io/badge/Telegram-@ouom__pub-blue" /></a>
</p>

<p><img src="app-icon.png" width="200" alt="NeriPlayer Desktop logo" /></p>

<p><a href="https://github.com/cwuom/NeriPlayer">NeriPlayer (Android)</a> 的桌面移植，基于 <strong>Tauri 2 + Vue 3 + Rust</strong>，面向 Windows、macOS 和 Linux。</p>

<p>🚧 <strong>当前为预览版（Preview）</strong></p>

<p>名称及图标灵感来源于《星空鉄道とシロの旅》中的角色「风又音理」。</p>

</div>

> [!IMPORTANT]
> 当前版本可供体验和反馈，尚未进入稳定版阶段。本文按当前仓库实现整理；第三方平台接口、系统集成和跨设备兼容性仍可能变化。
> 更新或迁移前，请导出重要歌单与配置并保留备份；不同构建的具体变化以发布说明为准。

> [!WARNING]
> 本项目仅供学习与研究使用，请勿用于任何非法用途。本项目及维护者不接受任何形式的赞助、捐赠或商业资助。

NeriPlayer Desktop 不提供公共云端曲库或媒体分发服务。在线能力取决于第三方平台的可用接口、账号、地区与内容权限；会员或受限内容仍遵循原平台规则。

## 🧭 文档导航

- [下载安装与首次使用](#installation)
- [功能与平台能力](#features)
- [本地音乐与下载](#library-downloads)
- [歌词与桌面集成](#lyrics-desktop)
- [快捷键](#shortcuts)
- [备份、同步与 Android 兼容](#sync)
- [一起听](#listen-together)
- [数据与隐私](#data-privacy)
- [已知限制与排查](#limitations)
- [本地构建](#development)
- [贡献指南](./CONTRIBUTING.md) / [English contribution guide](./CONTRIBUTING_EN.md)

<a id="installation"></a>
## 📦 下载安装与首次使用

### 下载预览版

前往 [GitHub Releases](https://github.com/cwuom/NeriPlayer-Desktop/releases)，阅读对应发布说明并选择适合平台和架构的安装包。发布工作流的分发目标如下：

| 平台 | 架构 | 安装包 | 环境要求 |
| --- | --- | --- | --- |
| Windows | x64 | `.msi` / `.exe`（NSIS） | WebView2 运行时 |
| macOS | Apple Silicon / arm64 | `*-adhoc.dmg` | 构建目标为 macOS 11.0+ |
| macOS | Intel / x64 | `*-adhoc.dmg` | 构建目标为 macOS 10.15+ |
| Linux | x64 | `.deb` / `.rpm` / `.AppImage.tar.gz` | WebKitGTK 4.1、GTK 3、ALSA 等运行依赖 |

这些最低版本来自打包配置，不代表每个系统版本都已完成运行验收。Release 还提供 `SHA256SUMS.txt`、macOS 安装说明以及随包 FFmpeg 的源码与构建脚本。

- **Windows**：当前安装包未做代码签名，可能出现 SmartScreen 提示。缺少 WebView2 时先安装运行时。
- **macOS**：安装包采用 ad-hoc 签名，未经 Apple 公证。将应用拖入「应用程序」后尝试打开；如被拦截，可按 [Apple 官方说明](https://support.apple.com/en-us/102445)在「系统设置 → 隐私与安全性」中选择「仍要打开」。对应发布附带的 `macOS-install.txt` 提供该构建的具体处理步骤。
- **Linux**：AppImage 需先解压并赋予执行权限；deb/rpm 使用发行版包管理器安装。AppImage 仍依赖宿主系统的图形与运行环境。

### CI 构建

需要体验尚未发布的改动时，可前往 [Artifacts 工作流](https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/artifacts.yml)，选择成功构建并下载对应平台的 artifact。下载 Actions artifact 通常需要登录 GitHub；先解压外层 artifact，再使用其中的安装包。

CI 构建和 Release 可能对应不同提交。反馈问题时请附上「设置 → 关于」中可复制的构建版本，而不是只写安装包名称或 `1.0.0`。

### 首次使用

1. 在「设置」中登录所需平台：网易云音乐、Bilibili、YouTube。可使用内置登录窗口，也支持手动导入 Cookie。
2. 在「探索」中搜索或浏览内容，在「音乐库」中打开平台歌单、收藏夹或导入本地文件。
3. 按需要设置播放音质、音频输出设备、下载目录、歌词和快捷键。
4. 需要跨设备元数据时再配置「设置 → 备份与同步」；本地播放不要求开启云同步。

<a id="features"></a>
## 🎧 功能与平台能力

### 功能概览

| 模块 | 当前实现 |
| --- | --- |
| 首页 | 顶部「继续播放」展示最近打开的歌单，按打开时间与次数排序；最近播放歌曲、分区推荐，国际化模式且已登录 YouTube 时展示 YouTube Music 首页货架 |
| 探索 | 网易云、Bilibili、YouTube Music 分平台搜索和发现；搜索历史可关闭，搜索结果可选择对应版本播放 |
| 音乐库 | 本地歌单、收藏歌单与艺术家、下载、网易云歌单/专辑、Bilibili 收藏夹、YouTube Music 歌单；分类内搜索、分页、多选、拖拽和右键菜单 |
| 艺术家 | 网易云、Bilibili、YouTube 详情与关注；本地艺术家聚合、详情搜索与筛选播放 |
| 播放 | 队列拖动排序（保留当前歌曲与进度，排序把手支持上下方向键）、随机/循环、进度与模式记忆、长音频续播、渐进缓冲、Range 拉流与分片音频定位 |
| 音效 | 输出设备、声道平衡、独立倍速与音调、唱片效果、响度增益与均衡、10 段 EQ、10 段参数均衡与 AutoEQ、29 种预设、音色与空间处理、压缩与限幅、扬声器优化、按输出设备记忆、暂停/恢复淡入淡出、切歌交叉淡入淡出、多声道动态范围压缩选项 |
| 歌词 | AMLL 逐字渲染、翻译/罗马字、多源自动获取与手动匹配、编辑、本地歌词、逐源与单曲偏移、独立桌面歌词 |
| 桌面 | 系统托盘、关闭到托盘、可配置全局快捷键、系统媒体会话、单实例、macOS 菜单栏歌词 |
| 数据 | SQLite 本地持久化、歌单和完整配置导入/导出、GitHub / WebDAV 元数据同步、播放统计、最近播放管理 |
| 界面 | 简体中文、繁体中文、English、日本語；浅色/深色、封面取色、自定义背景、流体动态背景与音频响应、设置搜索；外观设置支持主界面缩放（macOS 需 11+），上限按屏幕可用空间与系统 DPI 计算，最小窗口尺寸随比例调整并自动保存 |
| 诊断 | 连通性探针、实时日志、文件日志、崩溃报告与调试报告导出 |

### 平台能力

| 来源 | 账号与浏览 | 播放、下载与歌词 | 当前边界 |
| --- | --- | --- | --- |
| 网易云音乐 | 登录、搜索、每日推荐、歌单/专辑、艺术家、喜欢/取消喜欢 | 多音质、YRC 逐字/翻译/罗马字歌词、下载 | 账号权限、会员与地区限制仍生效 |
| QQ 音乐 | 后端搜索及元数据/歌词匹配 | 游客接口可用范围内的取流、歌词与下载 | 当前没有 QQ 账号登录和独立音乐库/探索分类 |
| Bilibili | 登录、视频搜索、发现、创建/订阅收藏夹、UP 主详情 | DASH 音频、HTML5 MP4 回退、分 P、下载、跳过区间 | 视频下架、账号与音轨权限可能影响取流；歌词通过外部来源匹配 |
| YouTube Music | 登录、首页、搜索、歌单、艺术家 | 多客户端取流、下载、原生或外部匹配歌词 | 网络、地区、平台校验与客户端可用性可能变化 |
| 本地音频 | 多文件夹曲库、增量扫描、直接播放、歌单与艺术家管理 | 本地解码、标签读写、嵌入/sidecar 歌词 | 文件路径需在当前设备有效；扩展编码取决于解码组件 |
| 外部歌词 | QQ 音乐、网易云、酷狗、LRCLIB、AMLL TTML | 自动获取、逐字补全及手动匹配 | 匹配准确度和逐字时间轴取决于来源 |

### 播放源与音频信息

- 播放页「音频效果」提供速度、独立音调、唱片效果与预设；唱片效果使音调跟随速度，关闭后恢复已保存的独立音调。速度范围为 **0.1x–4x**，音调范围为 **0.25x–2x**，快档包括 **0.95x / 1.05x**。
- 「详细音效设置」进入「设置 → 音效与倍速」，按分区提供 10 段图示均衡、最多 10 段 / 7 种类型的参数均衡、AutoEQ / Equalizer APO 文本或文件导入、理论频响曲线、音色与人声、立体声与混响、完整压缩器与限幅器、扬声器优化及处理质量。每个参数均支持滑块和数值输入，关闭分区时保留参数，可通过总开关进行 A/B 原声对比。
- 29 种内置预设按风格、音色、场景与设备分类，可命名保存、删除与导入 / 导出用户预设；手动调节仅显示为当前状态。可按实际输出设备名称记忆音色，详细页显示原生 DSP 负载和增益衰减状态；浏览器预览无法提供真实原生状态。
- 音色预设以卡片平铺，常用页提供快速起点，全部页可直接选择 29 种音色。默认「简单调节」以低音、人声、明亮度、温暖感和空间感微调，细调曲线在零点附近更温和，并自动启用增益余量和峰值保护；「专业调节」保留完整参数。耳机校正和已启用的其他处理保持，已关闭的分区从自然状态重新起步。扬声器尺寸仅标注小型、中型、大型；响度、截止频率和压缩补偿增益在原生链中平滑变化。
- 低音处理对齐 Android 的增强限量与次声保护，低音、高音和虚拟低音的新增信号在峰值时收量，减少整段音乐随底鼓起伏；自动余量更轻，两个低音预设与频响曲线同步更新。负增益仍保留原有衰减，已保存的自定义音效参数不变。
- 「设置 → 播放源」可指定 YouTube 首选客户端：自动、visionOS、Android VR、Web Remix、TV HTML5、Web Creator；失败时继续尝试其它可用客户端。
- 网易云无完整播放权限或仅有试听时，可分别开启本地音频匹配和 Bilibili 替代音源。本地匹配优先；原曲的队列与同步身份保留，下载仍使用原平台音源。
- 新默认显示实际码率和格式；声道、采样率、位深可在个性化设置中独立开启。无法确认的参数省略，已有显示偏好保留。
- Rust 的 `cpal` 负责输出，内置 Symphonia 解码常见格式；随包 FFmpeg 扩展 Opus、AC-3/E-AC-3、ALAC、APE、WavPack、DSD 等解码能力。实际支持项以加载到的解码器为准，缺少 FFmpeg 时回退到内置解码器；多声道解码与下混不等于空间音频/Atmos 输出。

<a id="library-downloads"></a>
## 🗂️ 本地音乐与下载

### 本地音乐

「音乐库 → 本地文件」可添加多个音乐文件夹，建立持久曲库，通过原生文件监视和增量扫描更新。支持扫描进度、取消、重新扫描、搜索和多选，可隐藏已在歌单中的文件与重复元数据；可直接播放当前列表，也可加入现有或新建歌单。移除曲库文件夹不会删除磁盘文件，扫描不会自动生成默认歌单。

本地歌单支持创建、重命名、删除、排序、拖拽和批量操作。本地艺术家详情支持搜索，筛选后播放使用当前筛选列表。标题、歌手和专辑可写回音频文件，写入过程检查原有歌词、封面及其它标签。

音乐库标题栏提供播放统计和最近播放入口，首页「最近播放」也可进入历史页面；支持浏览与删除记录，删除标记参与云同步。

### 下载

歌曲右键菜单和批量选择提供下载入口。「下载」页显示队列、音源解析、传输、元数据处理、完成或失败状态，支持取消、重试、删除、重新下载和在文件管理器中显示。

- 并发数可设为 **1–8**，默认 **6**；下载音质默认跟随播放，也可按平台单独设置。
- 自定义目录和文件名模板，下载后检查可解码内容、时长及可用的长度/校验信息，包含分片音频检查。
- 元数据补齐默认开启，标准化歌词嵌入默认关闭；开启后仍保留原始歌词。下载会嵌入并读回核验歌名、歌手、专辑及可获取的封面、歌词、翻译和罗马字；封面获取或标签写入失败会显示下载失败，可重试，只有核验通过才标记完成。来源未提供的信息不会凭空补造，关闭元数据补齐后不强制嵌入。
- 重启时恢复未完成队列，重新解析地址并下载；**不支持按字节断点续传**。
- 操作正在播放的下载文件时，先释放本地解码器占用，再根据最新播放意图恢复；删除后可切回在线播放。

| 内容 | 保存位置 |
| --- | --- |
| 音频 | 选定下载目录 |
| 歌词、翻译和罗马字 | `Lyrics/` |
| 封面 | `Covers/` |
| NP 来源与元数据引用 | 音频旁的 `<音频文件名>.npmeta.json` |
| 下载临时文件 | `.tmp/` |

清理应用缓存不会删除主动下载的音频。云同步不传输下载文件，换设备后需重新下载或自行复制。

<a id="lyrics-desktop"></a>
## ✨ 歌词与桌面集成

### 播放页歌词

播放页使用 [applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics)，支持逐字/逐词高亮、翻译、罗马字、模糊与字号调整，搭配封面取色的 WebGL 流体背景、音频响应或封面模糊背景。

「设置 → 外观」可分别选择界面字体和播放页歌词字体，支持常用预设、自定义已安装的字体名称及实时预览；默认保留原有字体，歌词字体不受界面字体影响。MiSans、Inter 随应用提供，其它字体需在系统中安装，不可用时回退到默认字体。桌面歌词继续使用独立的字体设置。

鼠标悬停在歌词区域时，底部居中按歌词内容显示翻译和音译快捷开关，没有对应内容时隐藏该按钮；音译包含行级罗马字和逐字音译。两者可独立开启或同时显示，并与「设置 → 歌词」共用显示偏好。键盘聚焦时也会显示开关，触摸设备上保持可见。右键开关后可在菜单中选择隐藏；右键任意歌词可在原有复制和跳转菜单中选择显示或隐藏开关，取消菜单不会改变显隐状态。

「设置 → 歌词 → 长音动画触发时长」可在 300–2000ms 间调整，默认 700ms。数值越小越容易触发，仅在逐字高级动画开启时生效。

自动获取综合本地歌词、平台曲目 ID、缓存与多源匹配。「设置 → 歌词」可选择默认歌词源并控制逐字优先；没有合适结果时回退自动。Bilibili 开启逐字优先时先尝试逐字来源。手动匹配支持网易云、QQ 音乐、酷狗、LRCLIB 和 AMLL TTML，可改关键词、勾选来源、预览确认，候选陆续展示。

歌词显示会过滤明确的中英文制作、乐器、发行和宣传署名，包括 MIDI 程序、和声、带空格的英文角色及 `by:` 上传署名。首尾带分隔符的歌名与歌手标题行也会按当前曲目信息过滤，覆盖缓存、本地及同步歌词。含糊的短标签和普通歌词句子保留，正文逐字时间轴、翻译和音译不变；本地文件与已有存储内容不会因显示过滤而改写。

酷狗歌词缺少翻译或罗马字时，后台优先从网易云补全；翻译仍缺失时再尝试 QQ 音乐。补全只采用歌名、歌手和时长匹配的高置信度结果，保留酷狗原文、逐字时间轴和偏移，并写回缓存；已确认编辑或清空的副轨不会自动覆盖。

歌词偏移可按来源设置，也可单独调整曲目。手动匹配、编辑或导入时请核对曲目版本与时长；现场、翻唱和剪辑版可能需要重新匹配。

### 桌面歌词、托盘与媒体键

- 独立桌面歌词可通过设置、托盘或快捷键切换，支持单/双/三行、逐字/整行显示、翻译/罗马字、字体、渐变颜色、描边、阴影、透明度与背景；保留窗口位置和尺寸。
- 桌面歌词可锁定并点击穿透，悬停时保留解锁入口。
- 点击歌词或进度条跳转时，桌面歌词随主窗口停在目标位置，待播放进度确认后继续扫字。
- 「关闭到托盘」开启后，关闭主窗口保留后台播放；托盘提供窗口、播放、歌词和退出操作。
- macOS 菜单栏歌词可独立开启，无歌词时显示歌名，过长内容截断。
- 系统媒体键与媒体会话经 `souvlaki` 接入 Windows SMTC、Linux MPRIS、macOS Now Playing；具体呈现受系统和桌面环境影响。

<a id="shortcuts"></a>
## ⌨️ 快捷键

「设置 → 快捷键」分别配置应用内和系统全局绑定，支持录制、清空、恢复默认和冲突提示。`Mod` 表示 Windows/Linux 的 `Ctrl`，macOS 的 `Cmd`。

### 应用内默认快捷键

| 快捷键 | 动作 |
| --- | --- |
| `Space` | 播放/暂停 |
| `Mod + →` / `Mod + ←` | 下一首/上一首 |
| `→` / `←` | 快进/快退 |
| `Shift + →` / `Shift + ←` | 较大步长快进/快退 |
| `↑` / `↓` | 音量加/减 |
| `M` / `S` / `R` | 静音/随机/循环模式 |
| `L` | 喜欢/取消喜欢（支持的曲目） |
| `D` | 切换桌面歌词 |
| `Mod + P` | 展开/收起正在播放 |
| `Mod + F` | 搜索 |
| `Esc` | 按层级关闭弹窗、面板或播放页 |

输入框和正在操作的控件会接管相应按键。`Esc` 和 `Tab` 保留给关闭与焦点导航，不能改绑。

### 系统全局默认快捷键

全局快捷键**默认关闭**，开启后可在窗口失去焦点或收进托盘时使用。

| 快捷键 | 动作 |
| --- | --- |
| `Mod + Alt + P` | 播放/暂停 |
| `Mod + Alt + ←` / `Mod + Alt + →` | 上一首/下一首 |
| `Mod + Alt + ↑` / `Mod + Alt + ↓` | 音量加/减 |
| `Mod + Alt + L` | 喜欢/取消喜欢 |
| `Mod + Alt + D` | 切换桌面歌词 |
| `Mod + Alt + N` | 显示主窗口 |

如组合被系统或其它应用占用，设置页提示注册失败，可改用其它组合。输入法或桌面环境也可能影响实际触发。

<a id="sync"></a>
## ☁️ 备份、同步与 Android 兼容

### 同步范围

「设置 → 备份与同步」连接自己的 GitHub 仓库或 WebDAV 目标，采用与 Android 对齐的数据模型与合并规则。

| 数据 | 云同步行为 |
| --- | --- |
| 本地歌单 | 曲目元数据、自定义标题/歌手/封面、匹配歌词、单曲歌词偏移及成员删除记录 |
| 收藏 | 收藏歌单与以收藏记录保存的艺术家 |
| 歌单使用记录 | 最近打开时间、次数与删除记录，用于首页「继续播放」 |
| 最近播放 | 记录与删除标记 |
| 播放统计 | 次数、收听时长、每日统计桶与清空标记 |
| Bilibili 跳过区间 | 按 `bvid + cid` 同步各分 P 的多段区间及清空标记；编辑草稿仅在本机 |
| 音频与设备配置 | 不上传音频、下载文件、平台 Cookie 或播放 Token；设备设置与路径需另行配置 |

同步本地曲目元数据不会让另一设备获得文件。下载目录、输出设备和本地路径也不能直接迁移为可用环境。

### 配置与格式升级

1. **GitHub**：按界面说明配置 Personal Access Token 并校验，创建默认私有仓库或选择已有仓库。当前界面的 classic Token 流程使用 `repo` 权限；种类和权限说明见 [GitHub 官方文档](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)。
2. **WebDAV**：填写服务地址、账号和密码，确认远端目标可读写。
3. 配置完成后手动同步，本地变更也可触发自动同步；跨设备使用需连接相同目标。
4. 发现旧格式时按提示确认升级，先更新其它设备客户端并保留远端备份。

当前写入 **V4 ProtoBuf 清单 + Zstd 分块归档**，兼容读取旧 JSON、GZIP 和 V3。清单仍使用 `neriplayer-sync-v3.manifest` 文件名，文件名不表示内容仍为 V3。合并保留删除/清空标记，避免旧设备恢复已删除内容。

归档叶子分块每批最多 4 个并发下载。GitHub 按固定提交的目录树校验后复用本地缓存，小批量变更通过一次条件提交发布，较大的变更沿用分阶段上传；WebDAV 保留远端正文校验、租约和条件写入保护。

WebDAV 对齐 Android 的坚果云兼容方式：首次锁请求返回 403/405/501 时允许无锁同步，曾支持有限租约的目标仍禁止降级。读取要求原始编码，坚果云不带引号的 ETag 原样用于条件写入；发布前核对清单，发布后回读确认。服务端没有可用 ETag 时采用指纹核对与回读，但不能保证并发写入的原子性；自动清理旧分块仍要求有限租约和强 ETag。

旧版 Bilibili 歌曲的 `Bilibili|CID|BV号` 元数据按 Android 规则仅提取 CID 作为子音轨身份，使歌词引用能匹配对应的独立覆盖记录；缺失覆盖记录或版本不符的归档仍会被拒绝。

跨端使用依赖双方支持当前协议。仓库包含冻结的 Android 测试向量和 [Kotlin 互操作验证脚本](./scripts/test-sync-android-interop.ps1)；编解码检查不能代替真实账号、同一远端与多设备交替同步验证。

### 手动备份

- **歌单 JSON 导入/导出**：适合歌单备份与迁移。
- **完整配置导入/导出**：迁移设置、授权与同步配置等；是可能包含 Cookie、Token、密码的**明文 JSON**，不是音频文件或数据库的完整备份。
- **GitHub / WebDAV**：持续同步上述元数据，远端归档不做端到端加密。

### Bilibili 跳过区间

播放页更多菜单和 Bilibili 曲目右键菜单提供区间编辑，可按分 P 管理多段区间、用当前进度设置起止时间、预览与清空。保存后自动跳过，清空也参与同步。**一起听房间内暂停自动跳过**，避免单端跳转影响房间进度。

Android 的 USB 独占播放、SAF、悬浮/状态栏歌词等移动平台能力不属于桌面端范围；桌面端提供自己的托盘、桌面歌词、媒体会话与快捷键。

<a id="listen-together"></a>
## 🎧 一起听

支持创建/加入房间，通过 WebSocket 同步播放、暂停、位置、队列、切歌和循环/随机模式。可设置成员控制、成员变化自动暂停、音频直链共享与自定义服务端。

桌面与 Android 使用同一套一起听协议，需要可用且兼容的服务端以及各自可播放的内容。房间共享不替代平台账号权限；开启直链共享前，请确认接受向成员提供临时音频地址。

创建房间前需要先播放可共享的在线曲目；当前一起听队列不共享本地文件和 QQ 音乐曲目。

服务端基于 Cloudflare Workers + Durable Objects，参考 [NeriPlayer-LTW](https://github.com/TheSmallHanCat/NeriPlayer-LTW)。

<a id="data-privacy"></a>
## 🔒 数据与隐私

- 歌单、收藏、最近播放、队列、播放统计、下载目录记录、同步元数据与详情/歌词缓存由本地 SQLite 数据库 `neri_user_data.db` 管理。下载音频仍在选定目录。
- 旧 JSON 和 WebView localStorage 按数据域迁移，旧 JSON 备份在用户数据目录的 `legacy-json-backup/`。本地曲库文件夹与文件索引另存为 `local-library.json`，设置、登录态和同步配置使用各自存储。
- **Release 构建**使用应用侧加密存储平台 Cookie、GitHub Token 和 WebDAV 密码；**Debug 构建**使用随机路径的明文凭据文件。加密不承诺抵御同机同用户进程；日志会脱敏，分享前仍应检查内容。
- 默认数据保存在本机；启用同步后写入指定的 GitHub/WebDAV 远端；一起听与配置的服务端和成员交换会话数据。
- 不接入广告 SDK、第三方统计或崩溃分析 SDK，不将本地播放历史和统计回写音乐平台。登录、搜索、播放与歌词请求仍受相应平台隐私政策和访问日志规则约束。

<a id="limitations"></a>
## ⚠️ 已知限制与排查

| 场景 | 说明与处理 |
| --- | --- |
| 搜索、登录或取流失败 | 接口、网络、地区或账号权限可能变化；确认登录态和原平台内容可用，再检查代理 |
| QQ 音乐能力受限 | 当前无账号登录，受游客接口限制 |
| 托盘、桌面歌词或媒体键差异 | 受系统与桌面环境影响；源码支持不等于所有环境已验收 |
| 全局快捷键不响应 | 开启全局开关，查看注册失败提示并检查系统/其它程序占用 |
| 旧客户端同步失败 | 更新全部客户端，升级远端前备份并阅读确认提示 |
| 应用升级 | 当前不提供应用内自动更新，下载对应预览版手动安装 |

代理设置提供「绕过系统代理」开关，可按实际网络切换并测试。Debug 连通性探针只判断传输层是否收到响应，不证明登录有效、内容可播放或业务接口成功。

### 问题反馈

1. 在「设置 → 关于」复制构建版本，记录操作系统、架构与复现步骤。
2. 连续点击版本号 **7 次**开启开发者模式，进入 `Debug` 查看实时日志、崩溃报告或导出调试报告。
3. 需要持久日志时在设置中开启文件日志并选择级别。
4. 前往 [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues) 模板反馈；跨端问题提供桌面和 Android 版本，分享前移除个人信息与凭据。

<a id="development"></a>
## 🛠️ 本地构建

仓库固定 **pnpm 9.15.9** 与 **Rust 1.95.0**，CI 使用 **Node.js 20**。准备 Git、Node.js、pnpm、rustup 和 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)；本项目还需编译 FFmpeg C++ 垫片的工具链。

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

已有克隆缺少歌词子模块时执行 `git submodule update --init --recursive`。`pnpm tauri dev` 启动 Vite 与 Tauri，默认端口 `1420`，占用时自动选择其它空闲端口。

| 命令 | 用途 |
| --- | --- |
| `pnpm dev` | 仅启动 Vite，不提供 Rust IPC/原生播放 |
| `pnpm build` | Vue/TypeScript 检查并生成前端 `dist/` |
| `pnpm test` | 聚合前端回归脚本 |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | Rust 静态编译检查 |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked` | Rust 测试，FFmpeg 检查需可用运行库 |
| `pnpm tauri build` | 原生编译打包，默认不会自动加入 FFmpeg 动态库 |

完整解码能力的安装包还需准备匹配目标的 FFmpeg 动态库，运行 `scripts/ffmpeg/bundle-config.mjs` 并将生成配置传给 `tauri build --config`。完整平台安装、命令、产物路径与 CI/Release 流程见 [贡献指南](./CONTRIBUTING.md)和 [FFmpeg 构建说明](./src-tauri/native/ffmpeg/README.md)。

应用内构建版本为 `<git短哈希>.<MMddHHmm>`，另提供构建时间和 UUID；打包元数据中的 `1.0.0` 不表示已经是稳定版。发布频率随功能与维护情况调整，变化见 Release 与提交记录。

## 🤝 贡献与后续方向

欢迎提交可复现的 Issue、改进文档、参与预览版验证或贡献代码。贡献前请阅读 [CONTRIBUTING.md](./CONTRIBUTING.md)和 [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)。

继续关注播放稳定性、歌词匹配、跨端数据兼容、下载恢复与跨平台体验。QQ 账号登录、应用内自动更新和按字节断点续传仍是后续方向，不承诺完成时间；托盘、全局快捷键和桌面歌词已提供。

## 🙏 鸣谢

- [NeriPlayer (Android)](https://github.com/cwuom/NeriPlayer)：功能、协议与交互参考
- [applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics)：逐字歌词组件
- [netease-cloud-music](https://github.com/chaunsin/netease-cloud-music)：网易云接口实现参考
- [bilibili-API-collect](https://github.com/SocialSisterYi/bilibili-API-collect)：Bilibili 接口资料
- [LRCLIB](https://lrclib.net)：开放歌词服务
- [FFmpeg](https://ffmpeg.org/)：扩展解码与解封装

## 📄 许可证

本项目以 **GPL-3.0-or-later** 发布，详见 [LICENSE](./LICENSE)。2026-10-07 之前以 MIT 发布的代码保留原许可声明，见 [LICENSES/MIT.txt](./LICENSES/MIT.txt)。

`vendor/applemusic-like-lyrics` 使用 AGPL-3.0，按其许可证分发。随包精简 FFmpeg 未启用 GPL 组件，以 LGPL-2.1-or-later 分发；许可证、构建信息和源码获取方式见 [FFmpeg 说明](./src-tauri/native/ffmpeg/README.md)及 Release 附件。
