# NeriPlayer 源码深度分析（Analysis）

> 分析对象：`NeriPlayer-clone/`（git HEAD bc4142bc，上游最新 master，2026-08-12 克隆）
> 规模：770 个 Kotlin 文件 + Native C++ + AGSL Shader + 4 个子模块
> 对比基准：旧仓库 NeriPlayer（HEAD fd9d70b3） vs 新克隆 NeriPlayer-clone（HEAD bc4142bc）
> 差异规模：60 个文件变更 = 16 新增 + 44 修改 + 0 删除 + 0 重命名，+6225 / -1425 行
> 技术栈：Jetpack Compose + Media3 (ExoPlayer) + Room + KSP + OkHttp + WebSocket
> 内容：一至十七章为整体功能分析；十八、十九章为新版变化与两仓库差异；
> 二十至二十四章为补充深潜（播放服务/数据模型/网络安全/构建系统/常量速查）
> 合并说明：本文档为完整版，已合并《NeriPlayer-源码分析.md》（第 1-19 章，613 行）全部内容，并扩展第二十至二十四章深度分析。

---

## 目录

1. [项目架构总览](#一项目架构总览)
2. [应用入口与启动链路](#二应用入口与启动链路)
3. [播放核心 PlayerManager](#三播放核心-playermanager)
4. [多源在线播放](#四多源在线播放)
5. [音效系统](#五音效系统)
6. [歌词系统](#六歌词系统)
7. [下载管理系统](#七下载管理系统)
8. [本地音乐管理](#八本地音乐管理)
9. [数据同步（GitHub / WebDAV）](#九数据同步github--webdav)
10. [一起听（Listen Together）](#十一起听listen-together)
11. [UI 系统与流体背景](#十一ui-系统与流体背景)
12. [设置系统（KSP 代码生成）](#十二设置系统ksp-代码生成)
13. [存储与流量统计](#十三存储与流量统计)
14. [安全与崩溃恢复](#十四安全与崩溃恢复)
15. [USB 独占播放（Native）](#十五usb-独占播放native)
16. [桌面小组件与快捷方式](#十六桌面小组件与快捷方式)
17. [测试体系](#十七测试体系)
18. [新版变化深度分析（fd9d70b3 → bc4142bc）](#十八新版变化深度分析fd9d70b3--bc4142bc)
19. [两个仓库差异详细清单](#十九两个仓库差异详细清单fd9d70b3-vs-bc4142bc)
20. [播放服务 AudioPlayerService](#二十播放服务-audioplayerservice-深度解析)
21. [核心数据模型与数据库 Schema](#二十一核心数据模型与数据库-schema)
22. [网络层与安全](#二十二网络层与安全)
23. [构建系统与代码生成](#二十三构建系统与代码生成)
24. [关键常量与策略速查表](#二十四关键常量与策略速查表)

---

## 一、项目架构总览

```
app/src/main/java/moe/ouom/neriplayer/
├── activity/           Activity 入口与登录页面（QR/WebView）
├── core/
│   ├── api/            三平台 API 客户端（netease/bili/youtube）+ 歌词源 + 搜索
│   ├── crash/          全局异常处理器
│   ├── di/             AppContainer 手动 DI 容器
│   ├── download/       下载管理器（40+ 文件）
│   ├── lyricon/        Lyricon 歌词服务桥接
│   ├── player/         播放核心（PlayerManager + 200+ 策略文件）
│   └── startup/        启动规划/安全模式
├── data/
│   ├── auth/           三平台 Cookie 认证仓库
│   ├── backup/         配置备份与迁移
│   ├── config/         配置文件管理
│   ├── history/        播放历史
│   ├── listentogether/ 一起听偏好
│   ├── local/          Room 数据库 + 本地音频导入
│   ├── model/          SongItem 核心数据模型
│   ├── platform/       平台数据缓存仓库
│   ├── playlist/       歌单仓库与使用统计
│   ├── settings/       设置仓库 + AutoSettingsSchema
│   ├── stats/          播放统计
│   ├── storage/        存储占用分析
│   ├── sync/           同步（github/webdav + 合并策略）
│   └── traffic/        流量统计
├── listentogether/     一起听（WebSocket 会话 + 协议 + 校验）
├── navigation/         路由定义 + 启动器快捷方式
├── ui/                 Compose 屏幕/组件/主题/效果/ViewModel
├── util/               工具（崩溃日志/ANR/IO/JSON/网络/平台）
└── widget/             桌面小组件
```

## 二、应用入口与启动链路

### 2.1 NeriPlayerApplication

`NeriPlayerApplication.onCreate()` 启动流程：

1. `AppFeedback.initialize()` — 初始化反馈系统
2. `PlayerManager.bindApplication()` — 尽早绑定 Application 到播放器
3. 进程分类：`AppProcessClassifier` 判断是否主进程，WebView 目录加后缀区分多进程
4. 语言初始化：`LanguageManager.init()`
5. 启动规划：`AppStartupPlanner.plan()` 返回是否进入安全模式
6. ANR 捕获：`AnrWatchdog.capturePreviousAnrIfNeeded()` 捕获上次 ANR
7. 异常处理器：`ExceptionHandler.init()`（含 NativeCrashHandler）

正常组件初始化（`initializeNormalComponents`）：
- `AppContainer.initialize()` 初始化 DI
- 后台预热收藏/历史/统计/缓存仓库
- `UsbDeviceAttachHandling` 注册 USB 设备接入监听
- `FloatingLyricsOverlayManager.initialize()` 悬浮歌词
- `ManagedDownloadStorage.initialize()` 下载存储
- `YouTubeAuthRotationWorker.schedulePeriodicRotation()` 周期续期 YouTube 会话
- `GlobalDownloadManager.initialize()` 全局下载
- `LyriconManager.initialize()`（启用时）

### 2.2 MainActivity → NeriApp

`MainActivity` 通过 `setContent` 挂载 `NeriTheme` + `NeriApp`。`NeriApp.kt` 是 UI 根：

- 启动阶段：`Loading → Disclaimer → Onboarding → Main`（SafeMode 优先）
- 主导航：`NeriBottomBar`（底部 Tab）+ `NeriMiniPlayer` + NowPlaying 覆盖层
- 主题切换：动态/浅色/深色 + `ThemeRevealOverlay` 主题切换过渡动画
- USB 前后台恢复：`recoverUsbExclusivePlaybackOnForeground`

### 2.3 导航 Destinations

`Destinations.kt` 定义所有路由，主 Tab 为 `home/explore/library/settings`，详情页通过 JSON 参数传递（`{playlistJson}` 等）。Tab 切换由 `MainTabTransitionController` 做可打断的横向换页。

## 三、播放核心 PlayerManager

`PlayerManager` 是 2678 行的 **object 单例**，核心字段与机制：

### 3.1 状态流（StateFlow）

| 状态 | 用途 |
|------|------|
| `_currentSongFlow` | 当前歌曲 |
| `_isPlayingFlow` / `_playWhenReadyFlow` | 播放状态 |
| `_playbackPositionMs` / `_playbackDurationMs` | 进度（80ms 更新一次） |
| `_currentQueueFlow` | 播放队列 |
| `_repeatModeFlow` / `_shuffleModeFlow` | 循环/随机 |
| `audioLevelFlow` / `beatImpulseFlow` | 音频可视化（驱动流体背景） |
| `_preferredQualityKeys` | 三平台音质偏好 |

### 3.2 关键策略参数

- `MEDIA_URL_STALE_MS = 10min` — URL 过期自动刷新
- `URL_REFRESH_COOLDOWN_MS = 10s` — 刷新冷却
- `MAX_CONSECUTIVE_FAILURES = 10` — 连续失败自动停止
- `STATE_PERSIST_INTERVAL_MS = 15s` — 状态持久化间隔
- `DEFAULT_FADE_DURATION_MS = 500ms` — 淡入淡出时长

### 3.3 播放流程（playPlaylistImpl）

```
检查初始化 → 一起听拦截 → USB高音量确认 → 设置队列/索引
→ 随机播放时保存恢复快照+乱序 → playAtIndex → 广播 PLAY_PLAYLIST 事件 → 延迟持久化
```

### 3.4 playImpl（恢复播放）

关键逻辑：若歌曲已预载但 URL 过期（`MEDIA_URL_STALE_MS`）或 YouTube 需要刷新则先 `refreshCurrentSongUrl()`，否则直接 `player.play()`；有暂停中恢复、手动恢复、队列空三种分支，通过 `resolveManualResumePlaybackDecision` 决定是否带位置续播。

### 3.5 事件系统

`PlaybackCommand(type, source, queue, currentIndex, positionMs)` 通过 SharedFlow 广播，一起听等模块订阅。`PlaybackCommandSource` 区分 `LOCAL` / `REMOTE_SYNC`（一起听来源）。

### 3.6 曲目结束去重

`handleTrackEndedIfNeededImpl` 使用 `trackEndDeduplicationKey` + 500ms 间隔守卫，防止 ExoPlayer 重复上报曲目结束。

## 四、多源在线播放

### 4.1 网易云（NeteaseClient）

- **加密**：`NeteaseCrypto` AES-CBC（固定 key）+ RSA + MD5 签名
- **Cookie**：OkHttp CookieJar 管理，`MUSIC_U` 判定登录，持久化注入 `__csrf`
- **网络**：共享 OkHttpClient + `DynamicProxySelector`（运行时切换代理）+ Brotli/GZIP 解压
- **播放失败**：音质降级 → 自动源切换 → 本地兜底

### 4.2 Bilibili（BiliClient）

- **WBI 签名**：`MIXIN_INDEX` 重排参数 → MD5 生成 `w_rid`，含 `wts` 时间戳
- **反爬三件套**：`spi` 指纹 + WebTicket + 专用 UA（Web/iOS/Firefox 三套）
- **接口**：playurl（WBI）、view、搜索、收藏夹、UP主空间、合集/系列
- **播放**：`BiliPlaybackRepository` 支持 DASH 音频重试和 html5/mp4 渐进流回退

### 4.3 YouTube Music（YouTubeMusicClient）

- **InnerTube API**：WEB_REMIX 客户端（id=67），continuation 分页，最多 80 页
- **多级回退链**：登录 Cookie → 匿名 visitor → PoToken → player.js 缓存 → EJS 挑战（JS 求解队列 + WebView 兜底）→ NewPipe 回退
- **Cookie 轮换**：`YouTubeCookieRotator` + `YouTubeAuthRotationWorker` 周期续期
- **URL 预热**：登录/匿名都预取 bootstrap，播放前预取队列窗口

### 4.4 网易云自动源切换

`PlayerManagerNeteaseAutoSourceSwitch.tryResolveNeteaseAutoBiliSource`：
1. 构建查询（`歌名+歌手`、`歌手+歌名`、`歌名`）
2. B 站搜索（追加"无损"关键字）
3. 按歌名/歌手相似度 + 时长打分，阈值 70
4. 最多 6 候选，2 个后备候选组成播放候选列表

## 五、音效系统

`PlaybackEffectsController` 统一管理：

- **倍速/音调**：ExoPlayer `PlaybackParameters`
- **均衡器**：Android `Equalizer` API，预设 + 5 段手动调节，按 audioSessionId 绑定
- **响度增强**：`LoudnessEnhancer`（mB 增益）
- **声道平衡**：`StereoBalanceAudioProcessor`（自定义 Media3 AudioProcessor）
- **响度归一**：`VolumeNormalizationAudioProcessor` 按歌曲实时分析
- **32-bit 高解析输出**：`playbackHighResolutionOutputEnabled`，旁路应用内处理

`AudioReactive` 提供音频能量/节拍流，驱动流体背景 `uMusicLevel/uBeat`。

## 六、歌词系统

### 6.1 来源链

网易云 lrc/tlrc → QQ音乐（补全源）→ LrcLib → 酷狗 → AMLL TTML → 手动匹配（`EditableLyricsMatcher`）

### 6.2 解析与缓存

- `accompanist-lyrics-core` 解析 LRC/TTML/YRC
- `LruCache`：YouTube 歌词缓存 20 条、网易云歌词缓存 20 条，避免重复请求
- `EditableLyricSanitizer` 清理标题/制作信息行

### 6.3 输出形态

| 形态 | 实现 |
|------|------|
| 播放页歌词 | `SyncedLyricsView` / `AdvancedLyricsView`（逐词/逐字高亮+翻译+音译） |
| 悬浮歌词 | `FloatingLyricsOverlayManager` + `WindowManager` 系统窗口 + 长按拖动 |
| 状态栏歌词 | `StatusBarLyricNotificationState` |
| 蓝牙歌词 | `ExternalBluetoothLyrics` 走 AVRCP |
| Lyricon | `LyriconManager` + `SuperLyricHelper` 桥接 |
| 灵动岛 | `dynamicIslandLyricsEnabled` |
| 歌词卡片 | `LyricShareSheet` 生成 1080px 卡片分享 |

## 七、下载管理系统

`GlobalDownloadManager`（object 单例）核心机制：

- **不用系统 DownloadManager**，用共享 OkHttpClient + Semaphore 并发控制（默认 6，最高 8）
- **三种传输**：直接 HTTP、显式 Range（分块续传）、HLS
- **持久化**：任务队列 `DownloadTaskStore` + 恢复状态 `DownloadRecoveryRoomStore`，重启恢复
- **目录管理**：应用目录或 SAF 自定义目录，`ManagedDownloadMigration*` 支持迁移
- **原子写**：`ManagedDownloadAtomicFile` + 工作文件 + sidecar 元数据，崩溃不损坏
- **标签写入**：`DownloadedAudioTagWriter` 失败 3 次后仍保留音频文件
- **目录树缓存**：`ManagedDownloadTreeChildCache` 缓存 SAF 子节点，避免频繁查询
- **快照**：`ManagedDownloadSnapshotDiskCache` 磁盘+内存双层缓存

## 八、本地音乐管理

`LocalAudioImportManager` 三种导入方式：

1. **Intent 导入**：响应 `VIEW/SEND/SEND_MULTIPLE` 的 `audio/*`
2. **SAF 文件夹扫描**：`DocumentFile` 递归 + `Os.listdir` 提速
3. **MediaStore 扫描**：设备媒体库

**Sidecar**：自动识别 `.lrc/.txt` 歌词和 `cover/folder/front` 封面并复制。
**快速预览**：大批量扫描先出 `QuickImportedSongSeed`，后台补全元信息。

`LocalPlaylistRepository` + `LocalArtistSummary`：
- 系统歌单（我喜欢/本地文件）
- 歌手按展示艺术家聚合，拆分 `feat./with/和/与/顿号/分号/斜杠`
- 歌手页支持播放全部/多选/导出歌单/批量下载

## 九、数据同步（GitHub / WebDAV）

### 9.1 GitHub

- `GitHubApiClient` + Git Data API 提交二进制正文
- `SyncDataSerializer` GZIP + JSON 编码
- `SyncDataChangeDetector` 变更检测，只在有变更时上传
- `GitHubSyncUploadPolicy` 上传策略，`SyncUploadRetryExecutor` 重试
- 冲突：并发分支更新失败报告冲突，不强制覆盖
- Token 存 `SecureTokenStorage`（加密）

### 9.2 WebDAV

`WebDavApiClient` + `WebDavStorage` + 并发回退策略。

### 9.3 合并策略

`SyncPlaylistSongMergePolicy`（stableKey 去重合并）、`SyncSongMetadataMergePolicy`、`SyncPlaybackStatsMergePolicy`（取最大）、`SyncPlaylistDeletionPolicy`（删除记录同步）、`SyncPlaylistUsageStatsMergePolicy`。

### 9.4 触发器

`GitHubSyncWorker`/`WebDavSyncWorker` 用 WorkManager 延迟+周期同步，`SyncCoordinator` 互斥锁保证不同时并发。

## 十、一起听（Listen Together）

### 10.1 架构

`ListenTogetherSessionManager` + `ListenTogetherWebSocketClient`（OkHttp WebSocket）：

- **协议**：JSON envelope（`ListenTogetherSocketEnvelope`），消息上限 2MB，超限断开（1009）
- **心跳**：`np_ping` 新协议 + `ping` 兼容旧协议
- **重连**：`ListenTogetherReconnectPolicy`（最多 N 次，指数退避，终端错误不重连）
- **角色**：`ListenTogetherSessionRole`（Controller / Listener）

### 10.2 播放同步

- Controller 上报播放命令（PLAY_PLAYLIST 等）→ Listener 应用 `ListenTogetherPlayerStateApplier`
- 进度同步：`ListenTogetherPlaybackPosition` + 心跳周期上报
- Listener 停滞恢复：`ListenTogetherListenerStallRecovery`
- 安全暂停：`ListenTogetherListenerSafetyPausePolicy`（音频路由丢失等）
- 防回声：`ListenTogetherControllerEchoPolicy` + `ForwardedRequestDeduper` 去重
- 漂移保护：`ListenTogetherIncomingStatePolicy` / `RoomStateAcceptance`

### 10.3 通道映射

`ListenTogetherChannels`：`netease` / `bilibili` / `youtubeMusic` / `local`，歌曲用 `stableKey` 跨设备对齐。

## 十一、UI 系统与流体背景

### 11.1 AGSL 流体背景

`BgEffectPainter.java`（API 33+）加载 `assets/shaders/hyper_background_effect.glsl`：

- `RuntimeShader` 逐帧渲染
- 5 个色点（`uPoints[5]`）+ 5 组颜色（`uColors[5]`），从封面取色
- HSV 空间调色（饱和度/亮度偏移）
- 音频响应：`uLevelEase`（位移）、`uBeatEase`（节拍波+径向脉冲）、`uMotionEase`（波动）
- 变焦 `uZoom` + 全局运动 `uGlobalMotion` + 颗粒噪声

### 11.2 主要屏幕

`NowPlayingScreen`、`LyricsScreen`、`HomeScreen`、`ExploreScreen`、`LibraryScreen`、`SettingsScreen`、各平台详情页、`DownloadManagerScreen`、`PlaybackStatsScreen`、`SafeModeScreen`、调试探针（`*ApiProbeScreen`）。

## 十二、设置系统（KSP 代码生成）

- `ksp-annotations`：`@AutoSettingsCatalog` / `@AutoSetting` / `@AutoSettingsSection` 注解
- `ksp-processor`：`AutoSettingsProcessorProvider` 生成 `SettingsKeys`、备份白名单、Repository、section 常量
- `AutoSettingsSchema.kt`：声明式设置登记表（general/播放/下载/歌词/主题/同步等分区）
- 设置项用 `autoSwitchSetting/autoIntSetting/autoSetting` 等 DSL 声明，KSP 自动生成 DataStore 访问代码

## 十三、存储与流量统计

### 13.1 缓存

`SimpleCache + LRU`，默认 1GB（`currentCacheSize`），`CacheSizePolicy` 管理，可分别清理音频/图片/分享/歌单缓存。

### 13.2 StorageUsageAnalyzer

分组统计：音频缓存/图片缓存/下载暂存/分享暂存/平台歌单缓存/下载内容/日志/崩溃报告/核心数据。

### 13.3 播放统计

`PlaybackStatsTracker`：
- 每 15 秒周期 flush + 关键生命周期 flush
- 听满 30 秒才计 1 次播放（`MIN_LISTEN_MS_FOR_PLAY_COUNT`）
- 位置回绕检测（结尾→开头判定为播完）
- `PlaybackStatsRepository`：Room 主存 + JSON 兼容迁移，记录播放次数/收听时长/每日桶

### 13.4 流量统计

`TrafficStatsRepository`：
- 区分 WiFi/移动/漫游 + 播放/下载来源 + 缓存命中
- 每日桶（`dayStartAt`），延迟批量写入
- 高风险网络下载弹窗提示

## 十四、安全与崩溃恢复

### 14.1 ExceptionHandler

- 全局未捕获异常 → 写崩溃报告（进程/PID/线程/栈/ABI）→ 主线程错误弹窗
- 崩溃前 `UsbExclusiveSessionController.emergencyShutdown()` 紧急释放 USB 设备
- `NativeCrashHandler` C++ 层崩溃捕获

### 14.2 AnrWatchdog

- 主线程卡顿监测 + 上次 ANR 捕获
- Safe Mode 记录崩溃/ANR → 下次启动进入 `SafeModeScreen`

### 14.3 配置备份

`BackupManager` + `AppConfigBackup`：完整导出（含设置/授权/同步配置），版本化迁移。

## 十五、USB 独占播放（Native）

### 15.1 分层

Kotlin 层（`core/player/usb/`，70+ 文件）+ JNI 桥（`UsbExclusiveNativeBridge`）+ C++（`cpp/usb/`）。

### 15.2 C++ 模块

| 模块 | 职责 |
|------|------|
| `uac1/` | UAC1.0 格式解析 |
| `uac2/` | UAC2 描述符/时钟图/反馈模型/候选模型 |
| `feedback/` | 显式反馈引擎、时钟捕获、速率估计、包调度 |
| `iso/` | 等时传输窗口与健康度 |
| `pcm/` | PCM 编码管线与播放重放缓冲 |
| `exclusive/` | 桥接、恢复动作闩锁、运行时报告 |

### 15.3 核心机制

- **时钟拓扑解析**：`usb_uac2_clock_graph` + 反馈端点解析
- **反馈时钟捕获**：`usb_feedback_clock`，长调度间隙后重新捕获
- **率估计**：`usb_feedback_rate_math` 反馈速率数学
- **背压恢复**：等时传输健康监测 + 动态扩缩容 + 软恢复
- **后台锚点**：`UsbExclusiveBackgroundAudioAnchor` 保持 USB 通道，播放停止时静音/零均值载波
- **看门狗**：播放启动看门狗、前后台健康审计、卡死自动恢复
- **比特完美**：软件增益 0 dB，DAC 硬件控音量

## 十六、桌面小组件与快捷方式

### 16.1 小组件

`PlaybackWidgetProviders`：
- 4x2 播放卡片（`widget_playback_4x2`）+ 2x2 迷你（`widget_playback_2x2`）
- 控件通过 `ACTION_PLAYBACK_WIDGET_CONTROL` → `AudioPlayerService.dispatchPlaybackWidgetAction` 走播放服务链路
- 支持 `MY_PACKAGE_REPLACED` 刷新、尺寸变化 `onAppWidgetOptionsChanged`

### 16.2 启动器快捷方式

`LauncherShortcuts.kt`：继续播放、打开探索、打开媒体库、随机播放我喜欢。

## 十七、测试体系

- `app/src/test/`：单元测试（服务策略、悬浮歌词策略、下载策略、同步合并等）
- `app/src/androidTest/`：设备测试（设置、下载存储、安全模式等）
- `cpp/tests/usb/`：Native host 测试（corpus 语料 + fixtures 轨迹 + 4 ABI 编译验证）
- `tools_pub/ytmusic_api_probe.py`：YouTube 播放兼容探针
- 关键链路均有对应测试：下载存储、同步合并、YouTube 兼容、一起听、歌词解析、播放策略、配置备份、安全模式

---

## 十八、新版变化深度分析（fd9d70b3 → bc4142bc）

> 本次重新克隆自 https://github.com/cwuom/NeriPlayer（--recurse-submodules），
> 相比上一分析版本新增约 60 个文件变更、6225 行新增、1425 行删除。

### 18.1 网易云首页推荐系统（全新）

`NeteaseHomeRecommendations.kt` + `HomeViewModel`（+710 行）构建了完整的首页推荐体系：

**歌曲来源分区**（`NeteaseHomeSongSource`）：
- `TOP_SOARING` 飙升榜 / `TOP_HOT` 热歌榜 / `TOP_NEW` 新歌榜（无需登录）
- `PERSONAL_RADAR` 私人雷达 / `DAILY_RECOMMEND` 日推 / `PRIVATE_FM` 私人FM（需登录）
- `PERSONALIZED_NEW_SONGS` 新歌速递（无需登录）

**雷达歌单**：5 个固定歌单 ID（时光 5320167908 / 宝藏 5362359247 / 新歌 5300458264 / 乐迷 5327906368 / 神秘 5341776086）

**歌单来源分区**（`NeteaseHomePlaylistSource`）：PERSONALIZED 每日推荐 / DAILY_RESOURCE / HIGH_QUALITY 精品 / HOT 热门 / ACG 分区

**关键机制**：
- **登录感知**：`requiresLogin` 标记，未登录自动过滤需要登录的源
- **去重合并**：`appendUniqueNeteaseHomeSongs` 按 `audioId`/`channelId:id:name` 去重，各分区互不重复
- **失败回退**：`shouldFallbackRecommend` 对 code 301/50000005 自动回退
- **预取**：登录 Cookie 变化时自动刷新全部推荐分区
- 刷新会更新全部分区，含榜单、新歌、日推、私人FM、精品歌单等

### 18.2 WaveformSlider 播放进度预测（全新）

`WaveformSlider.kt`（+137 行）将进度条升级为带播放预测的波形滑块：

**核心：`WaveProgressPredictor`**
- 由于 UI 进度流是 80ms 更新一次，存在可见延迟
- Predictor 基于「锚点值 + 歌曲时长 + 当前倍速」逐帧推算实时进度
- `updateTarget()`：目标变化或开始动画时重置锚点
- `onFrame(frameNs)`：按已播放时长插值预测当前进度
- `resetFrameAnchor()`：拖动后重新锚定

**状态机**：`isPlaying`（波形动画）/ `isPlaybackWaiting`（等待脉冲动画）/ `isProgressStalled`（停滞检测）/ `isProgressPreviewing`（进度预览）。拖动时暂停动画、结束时重置锚点。

### 18.3 存储分析大增强（StorageUsageAnalyzer +677 行）

`StorageUsageAnalyzer.kt` 重构为 20+ 细粒度类别的分析器：

**新增类别**（`StorageUsageItemKind`）：`DownloadedMusic`（下载音乐）/ `DownloadedLyrics`（下载歌词）/ `DownloadedCovers`（下载封面）/ `DownloadIndex`（下载索引）/ `LocalCovers`（本地封面）/ `CustomBackground`（自定义背景）/ `LegacyMigrationFiles`（遗留迁移文件）/ `Database`（数据库）/ `AppData`（应用数据）

**数据库占用统计**：
- `DownloadIndexRoomStore`：用 `dbstat`（真实页大小）或 `PRAGMA page_size` 估算 SQLite 表占用
- `PlatformPlaylistCacheRoomStore`：统计平台歌单缓存三张表的记录数与页字节
- 行开销 24B + `length(CAST(col AS TEXT))` 逐列估算

**清除选项**（`StorageCacheClearOptions`）：按平台分别清除（网易云歌单 / B站收藏夹 / B站视频 / YouTube歌单 / 日志 / 崩溃日志），缓存清理不触碰用户下载内容。

配套新增 `StorageCacheDetailsContent.kt`（+584 行）详情页和 `SettingsStorageCacheSection` 重构。

### 18.4 平台歌单缓存迁移到 Room（从 JSON 到 SQLite）

新增 `PlatformPlaylistCacheDao` + `PlatformPlaylistCacheRoomStore` + `PlatformPlaylistCacheEntities`：
- 三表结构：cache（元数据）+ tracks（歌曲）+ artists（艺术家，按 trackPosition 关联）
- 事务性读写，`replaceIfNewer` 按 `savedAtMs` 版本控制，避免旧缓存覆盖新缓存
- 支持按平台批量清除与统计
- 数据库迁移到 MIGRATION_13_14

### 18.5 媒体缓存生命周期与播放恢复增强

- `PlayerManagerUrlExtensions`（+281 行）：媒体缓存完整性校验、缓存预取准备
- `PlayerManagerLifecycleExtensions`（+261 行）：播放恢复逻辑重构
- `PlayerManagerStartupWatchdogExtensions`（+49 行）：启动看门狗增强
- `PlayerManagerYouTubePrefetchExtensions`（+35 行）：YouTube 预取调整
- 配套测试：`CachedResourceIntegrityTest`、`PlayerManagerCachePrefetchPreparationTest`、`PlayerManagerYouTubePlaybackRecoveryTest`、`PlayerManagerMediaCacheLifecycleTest`

### 18.6 下载封面支持与索引简化

- `ManagedDownloadCoverLookup`（-77 行）：移除可复用封面查询，简化 sidecar 解析（commit #336）
- 下载封面作为独立存储项纳入存储分析
- `AudioDownloadManager`（-101 行）重构，配合新索引

### 18.7 其他增强

- `NeteaseClient`（+382 行）：新增接口与容错
- `ExceptionHandler`（+43 行）：跨进程 WebView 状态清理（#329）
- `NPLogger`（+37 行）：日志能力增强
- `FileCleanup.kt`：通用批量清理工具（全部成功才返回 true）
- README 更新：首页推荐、存储分析、下载索引描述

### 18.8 新增测试

| 测试 | 覆盖点 |
|------|--------|
| `NeteaseHomeRecommendationsTest` | 首页推荐解析/去重/登录过滤 |
| `WaveformSliderTest` | 进度预测、拖动、等待动画 |
| `StorageUsageSummaryTest` | 存储分类统计 |
| `StorageCacheDetailsContentTest` | 缓存详情 UI |
| `CachedResourceIntegrityTest` | 缓存完整性 |
| `PlayerManagerMediaCacheLifecycleTest` | 缓存生命周期 |
| `PlayerManagerYouTubePlaybackRecoveryTest` | YouTube 播放恢复 |
| `PlayerManagerCachePrefetchPreparationTest` | 缓存预取 |
| `PlatformPlaylistCacheRoomMigrationTest` | Room 迁移 |
| `ManagedDownloadCoverLookupTest` | 封面查找简化 |
| `NeteaseClientTest` | 网易云客户端 |
| `FileCleanupTest` | 文件清理 |

---

## 十九、两个仓库差异详细清单（fd9d70b3 vs bc4142bc）

### 19.1 版本与仓库状态对比

| 维度 | 旧仓库 `NeriPlayer` | 新克隆 `NeriPlayer-clone` |
|------|--------------------|--------------------------|
| HEAD | fd9d70b3 | bc4142bc（上游最新 master） |
| 状态 | 有本地改动（gradle-wrapper.properties 被改、buildSrc/build-logic 有 untracked 文件） | 干净（刚 clone） |
| 源码 Kotlin 数 | 765 | 770 |
| 子模块 | 1f476060 / 825661a1 / d844e105 / 48bd198a | 完全相同 |
| 构建产物 | 含 build/、.gradle/ 等产物（文件总数 3789） | 无构建产物（文件总数 2432） |
| Native C++ | 60 | 60（完全一致） |

### 19.2 新增文件（16 个）

**生产代码（4 个）**：
- `core/player/download/../data/local/database/store/DownloadIndexRoomStore.kt` — 下载索引数据库统计（dbstat / PRAGMA page_size 估算）
- `ui/screen/tab/settings/component/StorageCacheDetailsContent.kt` — 存储缓存详情页 UI
- `ui/viewmodel/tab/NeteaseHomeRecommendations.kt` — 网易云首页推荐（榜单/雷达/日推/私人FM/精品歌单）
- `util/io/FileCleanup.kt` — 通用批量文件清理工具

**测试代码（12 个）**：NeteaseClientTest、ManagedDownloadCoverLookupTest、PlayerManagerMediaCacheLifecycleTest、CachedResourceIntegrityTest、PlayerManagerCachePrefetchPreparationTest、PlayerManagerPlaybackCandidateRecoveryTest、StorageUsageSummaryTest、NeteaseHomeRecommendationsTest、FileCleanupTest、StorageUsageResourceTest、PlaylistModernVisualColorsProviderLayoutTest、StorageCacheDetailsContentTest

### 19.3 核心逻辑差异逐项分析

#### (a) 媒体缓存生命周期重构（PlayerManagerLifecycleExtensions +261 行）

`PlayerManager.cache` 从 `lateinit var` 改为 `@Volatile var cache: Cache? = null`（可空），为缓存重建与旁路让路。

**新增 `createVerifiedMediaCache`**：
1. 创建 `SimpleCache` 后调用 `checkInitialization()` 验证
2. 初始化失败分类：
   - 文件夹被锁（`isSimpleCacheFolderLocked`，其他进程的 SimpleCache 实例占用）→ 本次进程旁路缓存继续播放
   - 缓存库损坏（`hasCacheInitializationFailure` 含 `CacheException`）→ 删除并重建缓存目录
   - 重建仍失败 → 旁路缓存播放
3. 新增 `releaseMediaCache()`：安全释放可空缓存

**配套：`PlayerManagerUrlExtensions` 缓存完整性检查**
- 新增 `CachedResourceIntegrity`（isComplete / requiresRepair / coveredLength）
- `inspectCachedResourceSpans`：遍历排序后的 CacheSpan，检测文件缺失、长度不匹配（file.length != span.length）、区间重叠、越界、覆盖间隙；不完整或损坏 → `invalidateCachedResourceForPlaybackRecovery` 失效该缓存
- 新增 `CachePrefetchReadiness`（COMPLETE / READY_FOR_PREFETCH / UNAVAILABLE）供预取决策

#### (b) 播放失败恢复策略扩展（PlayerManagerUrlExtensions）

`isRecoverableRemotePlaybackCacheError` 从 2 种错误码扩展到 6 种：
```
BAD_HTTP_STATUS / INVALID_HTTP_CONTENT_TYPE / NETWORK_CONNECTION_TIMEOUT /
NETWORK_CONNECTION_FAILED / READ_POSITION_OUT_OF_RANGE / IO_UNSPECIFIED
+ TIMEOUT（且不是 STUCK_PLAYING_NOT_ENDING 卡死）
```
- 新增 `StuckPlayerException` 识别：`shouldTreatPlaybackFailureAsTrackEnd` 沿 cause 链查找，区分「卡死未结束」与真实曲目结束
- 缓存失效策略：远程可恢复错误 → 失效缓存重试；离线缓存格式错误也失效；`shouldInvalidateCacheAfterPlaybackFailure` 保证普通失败不盲目清缓存

#### (c) NeteaseClient Cookie 会话管理增强（+382 行）

- `mergeNeteaseRequestCookies`：三层 Cookie 合并（请求上下文 → 持久化 → 运行时），自动补 `os=pc` / `appver=8.10.35`
- `mergeNeteaseSessionCookies`：仅回填 `NMTID` / `__csrf` 动态会话字段
- `shouldPreheatNeteaseWeapiSession`：登录态存在但缺 `__csrf` 时预热 WeAPI 会话
- 新增 `requestContextCookies`：`__remember_me=true` + `_ntes_nuid`（SecureRandom 生成） + `NMTID`（SecureRandom 生成）
- `loadForRequest` 改为基于合并 Cookie 重建 `Cookie.Builder`（hostOnlyDomain），替换原持久的 CookieJar 存储
- `setPersistedCookies` 增加指纹检测（`authCookieFingerprint`），登录变化时重置预热的 `__csrf`

#### (d) 下载封面复用简化（commit #336）

- `AudioDownloadManager` 删除共享封面复用（`findSharedCoverReference`、`rememberSharedCoverReference`、`buildSharedCoverLookupKeys`、`sharedCoverReferencesByLookupKey`）
- `ManagedDownloadCoverLookup` 删除 `findReusableCoverReference`（跨歌曲按 remote cover key / 专辑匹配复用）
- `DownloadedAudioMetadataStore` 不再调用 `findReusableCoverReference`
- 收益：简化 sidecar 解析路径，下载封面改为每个音频独立解析；配合存储分析把「下载封面」作为独立统计项

#### (e) 崩溃日志与日志系统加固

- `ExceptionHandler`：新增 `crashLogLock` + `crashLogGeneration`（世代计数），写日志前校验世代防竞态；新增 `clearCrashLogs()` 供存储清理（清空目录 + `CrashReportStore.clearPendingCrashReport` + 重开日志文件）
- `NPLogger`：`LogFileEntry` 增加 `generation`，`fileLogGeneration` 世代计数，清日志后旧写任务自动丢弃；同样接入 `clearAllFiles`

#### (f) 存储分析增强（StorageUsageAnalyzer +677 行）

- `StorageCacheKind` 从单一 `PlatformList` 拆分为 4 个平台 + 日志 + 崩溃日志
- 新增 `StorageUsageItemKind` 20+ 细粒度类别（下载音乐/歌词/封面、下载索引、本地封面、自定义背景、遗留迁移文件、数据库、AppData）
- 新增 `DownloadIndexRoomStore` / `PlatformPlaylistCacheRoomStore` 的数据库占用统计（真实页大小或估算）
- 新增 `StorageCacheDetailsContent.kt` 详情页 + `StorageCacheDetailsContentTest`

#### (g) 首页网易云推荐（NeteaseHomeRecommendations + HomeViewModel +710 行）

已在第十八章详述；核心要点：动态分区（登录感知）、雷达歌单 5 个固定 ID、`appendUniqueNeteaseHomeSongs` 去重、301/50000005 回退、登录变化自动刷新全部分区。

#### (h) WaveformSlider 播放进度预测（+137 行）

已在第十八章详述；`WaveProgressPredictor` 基于锚点值 + 时长 + 倍速逐帧预测，配套停滞/预览/等待脉冲状态机。

#### (i) 其他修改

- `AutoSettingsSchema`（+6 行）：新增相关设置项
- `HomeScreen`（+538 行）/ `HomeHostScreen` / `SettingsScreen`（+183 行）/ `SettingsPage` / `SettingsSearchIndex`：UI 重构配合新功能
- `NowPlayingScreen` / `LyricsScreen`：接入 WaveformSlider
- `NeteaseCollectionDetailViewModel`（+105 行）+ 测试：歌单详情增强
- `NPLogger` / `ExceptionHandler`：如上
- `README` / `CONTRIBUTING`：文档同步更新

### 19.4 结论

- 两个仓库的**子模块完全一致**，Native C++ 完全一致，差异全部在 Kotlin/资源/测试层
- 核心演进方向：**缓存健壮性**（完整性检查 + 自动重建 + 失效重试）、**网易云会话保鲜**（SecureRandom 会话 Cookie + 会话预热）、**首页推荐**、**存储分析精细化**、**封面解析简化**
- 旧仓库仅有本地未提交改动（构建相关），不含新功能；新克隆 bc4142bc 为干净的最新代码

---

## 二十、播放服务 AudioPlayerService 深度解析

### 20.1 服务架构

`AudioPlayerService.kt`（130KB，Kotlin）是一个 **普通 `Service`**（非 `MediaSessionService`），单实例，由 `PlayerManager` 统一驱动。位于 `core/player/service/`。

**核心字段**：
- `mediaSession: MediaSession` — `USAGE_MEDIA` + `CONTENT_TYPE_MUSIC` 音频属性
- `usbExclusiveVolumeProvider: UsbExclusiveLockScreenVolumeProvider` — USB 独占时接管锁屏音量键
- 封面加载状态机：`currentCoverSongKey` / `currentMediaArtwork` / `currentNotificationLargeIcon` / `artworkLoadJob` / `lastArtworkLoadFailedSource`（封面加载失败去抖）
- `becomingNoisyReceiver: BroadcastReceiver` — 监听耳机拔出（AUDIO_BECOMING_NOISY）

### 20.2 服务生命周期策略（纯函数策略层）

服务目录下 9 个策略文件构成完整决策体系：

| 文件 | 职责 |
|------|------|
| `PlaybackNotificationPolicy` | 通知显示策略 |
| `MediaSessionPlaybackStateThrottler` | MediaSession 状态上报节流：最小更新间隔 1000ms、位置漂移阈值 1500ms |
| `MediaSessionVolumePolicy` | MediaSession 音量策略 |
| `PlaybackServiceIdlePolicy` | 空闲判定（无播放任务时降级） |
| `PlaybackServiceIdleShutdownCoordinator` | 空闲关闭协调：空闲超时（可配置 `playback_service_idle_shutdown_minutes`）后停止服务 |
| `PlaybackServiceRestartPolicy` | 服务重启策略 |
| `FloatingLyricsNotificationPolicy` | 悬浮歌词通知策略 |
| `StatusBarLyricNotificationState` | 状态栏歌词通知状态 |
| `PlayerManagerServiceLifecycleExtensions` | 服务生命周期桥接 PlayerManager |

### 20.3 关键逻辑

**任务移除处理**（`resolveTaskRemovedPlaybackAction`）：用户划掉最近任务时，根据是否处于传输活跃态（`isTransportActive`）决定继续前台播放还是停止服务，避免"后台播着歌却没了通知"。

**小组件动作分发**（`dispatchPlaybackWidgetAction`）：
- 校验动作白名单（`isSupportedPlaybackWidgetAction`）
- 安全模式下拒绝执行
- 根据服务是否已在前台决定 `startForegroundService` 或 `startService`
- 捕获 Android 12+ 后台启动限制（`isServiceStartNotAllowedFailure`），`IllegalStateException` 优雅降级

**封面加载**：`resolveMetadataCoverSource` / `shouldRequestArtworkLoad` / `resolveRemoteMetadataArtworkUri`，远程封面通过 `MediaMetadataRetriever` 或协程加载，失败记录时间戳避免反复请求。

**USB 独占联动**：`usbExclusiveKeepAliveIntervalMs` 保活心跳、`shouldReassertUsbExclusiveForeground` 前台重断言、`updateUsbExclusiveBackgroundState` 后台状态同步。

## 二十一、核心数据模型与数据库 Schema

### 21.1 SongItem（平台无关歌曲模型）

`@Parcelize data class`，覆盖三平台 + 本地，关键字段：
`id/name/artist/album/albumId/durationMs/coverUrl/mediaUri`、
`matchedLyric/matchedTranslatedLyric/matchedLyricSource/matchedSongId/userLyricOffsetMs`、
`custom*/original*`（自定义与原始元数据分离）、
`localFileName/localFilePath`、`channelId/audioId/subAudioId/playlistContextId`、
`sourceStableKey/streamUrl`、`neteaseArtists`、`syncMembershipTokens`

**身份标识**：`stableKey()`（跨设备同步主键）、`identity`（专辑/媒体URI身份）、`sameIdentityAs()`（等价判断）。

### 21.2 NeriUserDataDatabase（Room）

- 数据库名：`neri_user_data.db`，仅允许主进程打开
- **13 次迁移**：`MIGRATION_1_2` → `MIGRATION_13_14`，逐版本演进
- 15 个 DAO：PlayHistory / PlaybackStats / TrafficStats / LocalPlaylist / PlaylistUsage / LocalPlaylistPlayback / FavoritePlaylist / SyncMetadata / PlaybackQueue / BiliVideoSkip / CoverUrlMapping / DownloadRecovery / DownloadedSongCatalog / DownloadSnapshot / PlatformPlaylistCache
- 15 个 Entity 文件：覆盖历史、统计（含 counter shard 分片）、歌单（含成员 token）、同步（outbox + checkpoint）、播放队列、下载（目录索引/恢复/快照/已下载目录）、平台歌单缓存、B站视频跳过、封面URL映射

**统计分片**：`PlaybackStatCounterShardEntity` / `PlaybackStatDailyCounterShardEntity` 用分片计数器缓解高频写放大。

**平台歌单缓存表**（新版）：cache + tracks + artists 三表，事务性读写 + `replaceIfNewer` 版本控制。

## 二十二、网络层与安全

### 22.1 共享 OkHttpClient

`AppContainer` 构建统一 OkHttpClient（`buildSharedOkHttpClient`）：
- `connectTimeout` 与 `readTimeout` 均明确配置，`callTimeout = 0`（禁用总时限，防止截断长播放/同步/WebSocket 请求）
- 连接池：`SHARED_HTTP_MAX_IDLE_CONNECTIONS` + `SHARED_HTTP_KEEP_ALIVE_MINUTES`
- `retryOnConnectionFailure(true)`

播放（三平台音源/歌词）、同步（GitHub/WebDAV）、一起听（WebSocket）共用此客户端。

### 22.2 DynamicProxySelector（运行时代理）

`DynamicProxySelector` 实现 `ProxySelector`，允许**运行时切换代理**（如绕过网络限制），三平台客户端都接入，无需重建 OkHttpClient。

### 22.3 安全防护

- `SecurityGuards`：通用安全守卫
- `TrustedHostSupport` / `HostValidation`：受信主机校验（防止 SSRF / 恶意 URL）
- `NetworkExceptions`：网络异常分类（`isTransientHttp2StreamReset` 等瞬时错误识别）
- `OkHttpCallAwait`：OkHttp 调用的协程 await 封装
- `readBytesLimited`：响应体读取上限（NeteaseClient `MAX_RESPONSE_BYTES = 4MB`）
- 隐私策略：不接入广告/统计/崩溃分析 SDK；同步走用户自有的 GitHub/WebDAV；Cookie/Token 本地存储

### 22.4 平台反爬对策

| 平台 | 对策 |
|------|------|
| 网易云 | AES-CBC + RSA + MD5 加密参数；三层 Cookie 合并；`os=pc/appver=8.10.35` 指纹 |
| Bilibili | WBI 签名（MIXIN_INDEX 重排 + w_rid）；spi 指纹 + WebTicket；三套 UA 分离 |
| YouTube | InnerTube WEB_REMIX；visitor 匿名 + PoToken + player.js 缓存 + EJS 挑战 + NewPipe 回退 |

## 二十三、构建系统与代码生成

### 23.1 Gradle 模块

`settings.gradle.kts` 定义多模块 + 3 个源码级子模块（np-submodule）：

| 模块 | 职责 |
|------|------|
| `app` | 主应用（Kotlin + Compose + Native CMake） |
| `ksp-annotations` | KSP 注解定义（@AutoSetting 等） |
| `ksp-processor` | KSP 处理器（自动生成设置代码） |
| `build-logic` / `buildSrc` | Convention 插件（版本目录 libs.versions.toml） |
| `np-submodule/*` | NeriPlayer-LTW / accompanist-lyrics-core/ui / miuix |

### 23.2 KSP 设置代码生成

`AutoSettingsSchema.kt` 声明式登记表 → `AutoSettingsProcessorProvider` 生成：
- `SettingsKeys` 常量
- `AutoSettingsRepository`（DataStore 访问）
- 设置备份白名单
- section 常量与 scope

新增设置只需在 `AutoSettingsSchema` 中声明，KSP 自动生成其余代码。

### 23.3 Native 构建

- `app/src/main/cpp/CMakeLists.txt` + NDK 编译
- 依赖 `libusb`（LGPL-2.1）实现 USB 音频独占
- `usb/` 下：exclusive / feedback / iso / pcm / uac1 / uac2 六模块
- `tests/usb/`：host 测试（corpus + fixtures，4 ABI）
- JNI 桥：`UsbExclusiveNativeBridge` 连接 Kotlin 与 C++

### 23.4 AGSL/GLSL 资源

`assets/shaders/`：`hyper_background_effect.glsl`（流体背景）+ 3 个 `advanced_glass_*.agsl`（高级模糊毛玻璃）。

## 二十四、关键常量与策略速查表

### 24.1 播放核心

| 常量 | 值 | 含义 |
|------|-----|------|
| `MEDIA_URL_STALE_MS` | 10 min | 播放 URL 过期自动刷新 |
| `URL_REFRESH_COOLDOWN_MS` | 10 s | URL 刷新冷却 |
| `MAX_CONSECUTIVE_FAILURES` | 10 | 连续播放失败自动停止 |
| `STATE_PERSIST_INTERVAL_MS` | 15 s | 播放状态持久化间隔 |
| `DEFAULT_FADE_DURATION_MS` | 500 ms | 淡入淡出时长 |
| `PLAYBACK_STATS_PERIODIC_FLUSH_MS` | 15 s | 播放统计周期刷盘 |
| `MIN_LISTEN_MS_FOR_PLAY_COUNT` | 30 s | 听满 30s 才计 1 次播放 |
| 进度流更新 | 80 ms | UI 播放进度节流 |
| 曲目结束去重 | 500 ms | 相邻结束事件间隔守卫 |

### 24.2 下载管理

| 常量 | 值 | 含义 |
|------|-----|------|
| 默认并发 / 最大并发 | 6 / 8 | Semaphore 控制 |
| `INITIAL_SCAN_DELAY_MS` | 1500 ms | 启动恢复扫描延迟 |
| `METADATA_WRITE_MAX_ATTEMPTS` | 3 | 标签写入重试上限 |
| `DOWNLOAD_CANCEL_SETTLE_TIMEOUT_MS` | 5000 ms | 取消任务稳定期 |
| 下载目录索引表 | 4 张 | catalog/snapshot/recovery/queue |

### 24.3 一起听

| 常量 | 值 | 含义 |
|------|-----|------|
| WS 消息上限 | 2 MB | 超限断开（code 1009） |
| 心跳 | `np_ping` / `ping` | 新/旧协议兼容 |
| 频道 | netease/bilibili/youtubeMusic/local | 平台通道映射 |
| 重连 | 指数退避 + 终端错误不重连 | `ListenTogetherReconnectPolicy` |

### 24.4 歌词

| 项目 | 值 | 说明 |
|------|-----|------|
| 歌词 LRU 缓存 | 各 20 条 | YouTube / 网易云 |
| 平台偏移 | `DEFAULT_CLOUD_MUSIC_LYRIC_OFFSET_MS` / `DEFAULT_QQ_MUSIC_LYRIC_OFFSET_MS` | 内置歌词补偿 |
| 用户偏移 | `userLyricOffsetMs` | 手动校准 |
| 歌词卡片 | 1080px | 分享卡片宽度 |

### 24.5 存储与缓存

| 项目 | 值 | 说明 |
|------|-----|------|
| 流媒体缓存 | 默认 1GB | `SimpleCache + LRU` |
| 存储类别 | 20+ 项 | StorageUsageItemKind |
| 清理策略 | 只清可再生成缓存 | 不删用户下载 |
| 下载索引行开销 | 24 B | SQLite 估算基准 |

### 24.6 首页推荐

| 项目 | 值 | 说明 |
|------|-----|------|
| 歌曲/歌单上限 | 各 30 | 每个分区 |
| 私人 FM 最大批次 | 10 | 批量获取 |
| 失败告警阈值 | 3 次 | 分区失败提示 |
| 雷达歌单 | 5 个固定 ID | 时光/宝藏/新歌/乐迷/神秘 |
| 推荐回退 | code 301 / 50000005 | 自动回退降级 |

### 24.7 USB 独占

| 项目 | 值 | 说明 |
|------|-----|------|
| 色点/颜色 | 5 | 流体背景 AGSL uniform |
| 看门狗 | 启动 + 前后台 + 卡死 | 三级恢复 |
| 后台锚点 | 静音/零均值载波 | 保持 USB 通道 |
| UAC2 | 显式反馈端点 | 拓扑校验后启用 |

---

## 分析结论总览

NeriPlayer 是一个**工程深度极高**的 Android 音乐播放器：770+ Kotlin 文件、60 个 Native C++ 文件、4 个子模块、19 次数据库迁移、13 个数据库迁移版本、KSP 代码生成、AGSL 实时渲染、UAC2 异步 USB 音频链路。它围绕「多源探索、在线播放、本地可控、数据自持」四条主线，把播放健壮性（缓存修复/URL保鲜/失败恢复）、平台合规（WBI/InnerTube/加密参数）、数据自主（本地优先 + 用户自有 GitHub/WebDAV 同步）做到了极致，是研究 Android 音频应用架构的优秀范本。

*NeriPlayer 源码深度分析 · 完整版 · 2026-08-12*
