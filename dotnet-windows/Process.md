# NeriPlayer → Windows 音乐播放器 移植方案（Process）

> 编制日期：2026-08-12
> 编制依据：《Analysis.md》（NeriPlayer 源码深度分析 24 章）+ `NeriPlayer-clone/`（HEAD bc4142bc，1748 个 Kotlin 文件）
> 目标平台：Windows 10/11（x64）
> 目标框架：.NET 8 + Avalonia UI
> 预计工期：12 ~ 16 周

---

## 目录

1. [概述与目标](#一概述与目标)
2. [技术栈映射](#二技术栈映射)
3. [项目结构设计](#三项目结构设计)
4. [核心模块实现策略](#四核心模块实现策略)
5. [数据模型与数据库设计](#五数据模型与数据库设计)
6. [播放引擎设计](#六播放引擎设计)
7. [音效系统设计](#七音效系统设计)
8. [API 客户端设计](#八api-客户端设计)
9. [下载管理系统设计](#九下载管理系统设计)
10. [数据同步设计](#十数据同步设计)
11. [UI 设计](#十一ui-设计)
12. [后台服务与系统集成](#十二后台服务与系统集成)
13. [安全与崩溃恢复](#十三安全与崩溃恢复)
14. [测试体系](#十四测试体系)
15. [打包与发布](#十五打包与发布)
16. [分阶段实施计划](#十六分阶段实施计划)
17. [架构简化清单](#十七架构简化清单)
18. [风险与缓解](#十八风险与缓解)
19. [验收标准](#十九验收标准)

---

## 一、概述与目标

### 1.1 项目背景

NeriPlayer 是一款 Android 端的多源音乐播放器，支持网易云音乐、Bilibili、YouTube Music 三大平台，具备播放核心、下载管理、歌词系统、数据同步（GitHub/WebDAV）、一起听、USB 独占播放等 770+ 个 Kotlin 文件的复杂架构。本方案的目标是将该播放器的**核心能力**迁移至 Windows 桌面端，形成一款功能对等、架构清晰的桌面音乐播放器。

### 1.2 产品目标

| 目标 | 说明 | 优先级 |
|------|------|--------|
| 多源在线播放 | 网易云 / Bilibili / YouTube Music 在线播放 | P0 |
| 本地音乐管理 | 本地文件夹扫描、元数据解析、播放 | P0 |
| 歌词系统 | LRC/TTML 解析、在线歌词、滚动歌词、翻译 | P0 |
| 音效系统 | 均衡器、立体声平衡、音量归一化、变速变调 | P1 |
| 下载管理 | 多任务并发下载、断点续传、标签写入 | P1 |
| 数据同步 | GitHub / WebDAV 同步歌单与配置 | P1 |
| 高级音频输出 | WASAPI 独占模式、ASIO | P2 |
| 桌面集成 | SMTC 媒体控制、任务栏缩略图、桌面歌词、Toast | P2 |
| 视觉效果 | 流体背景 Shader、毛玻璃、封面取色 | P2 |

### 1.3 非目标

- ❌ 不做一起听（Listen Together）—— WebSocket 多人实时同步过于依赖移动场景
- ❌ 不做 USB 独占（Native C++）—— Windows 下以 WASAPI 独占替代
- ❌ 不做桌面小组件（Android Widget）—— Windows 无对等机制，用 SMTC 替代
- ❌ 不做逐文件 Kotlin 移植 —— 采用架构级移植 + 语言重写

---

## 二、技术栈映射

### 2.1 技术选型总表

| NeriPlayer (Android) | Windows 替代方案 | 选型理由 |
|---|---|---|
| Jetpack Compose | **Avalonia UI 11.x** | 声明式 UI + MVVM，跨平台，样式系统强大，原生手感接近 Compose |
| Kotlin / KSP | **C# (.NET 8)** | 类型安全、LINQ、async/await、Source Generators 替代 KSP |
| Media3 ExoPlayer | **LibVLCSharp 8.x** | 全格式解码（mp3/flac/aac/opus/ogg/hls），流媒体支持完善 |
| Room (SQLite) | **EF Core 8 + SQLite** | ORM 成熟，迁移机制（Migrations）对应 Room 的 13 版迁移 |
| OkHttp | **HttpClient + Refit** | 原生 .NET HTTP 栈，Refit 提供接口式 REST 客户端 |
| Kotlinx Coroutines/Flow | **System.Reactive (Rx.NET)** | 响应式流处理，对应 Flow 语义 |
| DataStore Preferences | **JSON 配置 + Microsoft.Extensions.Configuration** | 结构化配置，易于手动编辑与同步 |
| WorkManager | **BackgroundService + Quartz.NET** | Windows 服务后台任务调度 |
| Android MediaSession | **SystemMediaTransportControls (SMTC)** | Windows 系统级媒体控制标准接口 |
| AGSL/GLSL Shader | **HLSL + SkiaSharp / Win2D** | GPU 流体背景与模糊效果 |
| KSP 设置代码生成 | **C# Source Generators** | 编译期代码生成，等价替代 KSP |
| JNI Native（USB 独占） | **NAudio WasapiOut（Exclusive）** | WASAPI 独占模式替代 USB 独占音频 |
| androidx.security (加密存储) | **DPAPI + AES-GCM (System.Security.Cryptography)** | Windows 本地凭据保护 |
| Coil (图片加载) | **AsyncImage + SkiaSharp 缓存** | 图片加载与磁盘缓存 |
| TinyPinyin（拼音排序） | **Microsoft.International.Converters.PinYinConverter** | 汉字拼音转换排序 |
| TagLib（标签写入） | **TagLib# 2.3.x** | 音频标签读写 |
| org.json / kotlinx.serialization | **System.Text.Json** | 标准 JSON 库 |

### 2.2 开发环境

| 项目 | 版本 |
|---|---|
| OS | Windows 10 22H2 或 Windows 11 |
| IDE | Visual Studio 2022 17.8+（Community 版即可） |
| .NET SDK | .NET 8 LTS |
| Avalonia | 11.0.x |
| LibVLC | 3.0.x（x64） |
| Git | 任意较新版本 |

### 2.3 NuGet 依赖清单

```xml
<PackageReference Include="Avalonia" Version="11.0.10" />
<PackageReference Include="Avalonia.Desktop" Version="11.0.10" />
<PackageReference Include="Avalonia.Themes.Fluent" Version="11.0.10" />
<PackageReference Include="Avalonia.ReactiveUI" Version="11.0.10" />
<PackageReference Include="LibVLCSharp" Version="3.8.0" />
<PackageReference Include="LibVLCSharp.Avalonia" Version="3.8.0" />
<PackageReference Include="Microsoft.EntityFrameworkCore.Sqlite" Version="8.0.0" />
<PackageReference Include="Microsoft.EntityFrameworkCore.Design" Version="8.0.0" />
<PackageReference Include="Microsoft.Extensions.DependencyInjection" Version="8.0.0" />
<PackageReference Include="Microsoft.Extensions.Configuration.Json" Version="8.0.0" />
<PackageReference Include="Microsoft.Extensions.Logging" Version="8.0.0" />
<PackageReference Include="Serilog" Version="3.1.1" />
<PackageReference Include="Serilog.Sinks.File" Version="5.0.0" />
<PackageReference Include="Serilog.Sinks.Console" Version="5.0.0" />
<PackageReference Include="System.Reactive" Version="6.0.0" />
<PackageReference Include="Refit" Version="7.0.0" />
<PackageReference Include="Refit.HttpClientFactory" Version="7.0.0" />
<PackageReference Include="NAudio" Version="2.2.1" />
<PackageReference Include="SkiaSharp" Version="2.88.8" />
<PackageReference Include="TagLibSharp" Version="2.3.0" />
<PackageReference Include="Quartz" Version="3.8.1" />
<PackageReference Include="Microsoft.International.Converters.PinYinConverter" Version="1.0.0" />
<PackageReference Include="WebDav.Client" Version="2.8.1" />
```

---

## 三、项目结构设计

### 3.1 解决方案结构

```
NeriPlayer.Windows/
├── NeriPlayer.Windows.sln
├── Directory.Build.props              # 全局编译属性
├── Directory.Packages.props           # 中央包版本管理
├── NuGet.config
├── README.md
│
├── src/
│   ├── NeriPlayer.App/                # 主应用入口（Avalonia Desktop）
│   │   ├── Program.cs                 # Main 入口 + DI 装配
│   │   ├── App.axaml / App.axaml.cs   # 应用定义、异常捕获
│   │   ├── AppStartup.cs              # 启动规划（安全模式）
│   │   ├── app.manifest               # 高 DPI、Windows 声明
│   │   └── Resources/                 # 图标 / 全局样式
│   │
│   ├── NeriPlayer.Core/               # 核心业务层（不依赖 UI）
│   │   ├── Player/                    # 播放核心
│   │   │   ├── PlayerManager.cs       # 播放总控（状态机+事件）
│   │   │   ├── Engine/
│   │   │   │   ├── IPlaybackEngine.cs
│   │   │   │   ├── VlcPlaybackEngine.cs        # LibVLC 封装
│   │   │   │   ├── ExclusivePlaybackEngine.cs  # WASAPI 独占
│   │   │   │   └── EngineException.cs
│   │   │   ├── Effects/               # 音效
│   │   │   │   ├── PlaybackEffectsController.cs
│   │   │   │   ├── EqualizerEffect.cs
│   │   │   │   ├── StereoBalanceEffect.cs
│   │   │   │   ├── VolumeNormalizationEffect.cs
│   │   │   │   ├── SpeedPitchEffect.cs
│   │   │   │   └── FftAnalyzer.cs              # 频谱数据
│   │   │   ├── Lyrics/                # 歌词
│   │   │   │   ├── LyricsProvider.cs           # 聚合歌词源
│   │   │   │   ├── LrcParser.cs
│   │   │   │   ├── TtmlParser.cs
│   │   │   │   ├── LyricTimeline.cs
│   │   │   │   └── LyricSearchMatcher.cs
│   │   │   ├── Playlist/
│   │   │   │   ├── PlaylistManager.cs
│   │   │   │   └── ShuffleEngine.cs
│   │   │   ├── Timer/
│   │   │   │   └── SleepTimer.cs
│   │   │   ├── Persistence/
│   │   │   │   ├── PlayerStatePersistence.cs   # 播放状态持久化
│   │   │   │   └── PlayerStateSnapshot.cs
│   │   │   ├── Policy/                # 策略层（合并 23 个策略包）
│   │   │   │   ├── PlaybackFailurePolicy.cs    # 连续失败处理
│   │   │   │   ├── MediaUrlRefreshPolicy.cs    # URL 保鲜 10min
│   │   │   │   ├── TrackEndDedupPolicy.cs      # 曲目结束去重
│   │   │   │   ├── PendingMediaLoadPolicy.cs
│   │   │   │   ├── ProgressUpdatePolicy.cs     # 进度节流
│   │   │   │   └── PlaybackCommandPolicy.cs
│   │   │   └── Model/
│   │   │       ├── SongItem.cs
│   │   │       ├── PlaybackAudioInfo.cs
│   │   │       ├── PlayerEvent.cs
│   │   │       ├── PlayerQueueDisplayState.cs
│   │   │       └── AudioDevice.cs
│   │   │
│   │   ├── Api/                       # API 客户端
│   │   │   ├── Common/
│   │   │   │   ├── IPlatformClient.cs           # 平台统一接口
│   │   │   │   ├── PlatformResult.cs
│   │   │   │   └── HttpClientFactory.cs
│   │   │   ├── Netease/
│   │   │   │   ├── NeteaseClient.cs
│   │   │   │   ├── NeteaseCrypto.cs             # WBI/AES/RSA
│   │   │   │   ├── NeteasePlaylistApi.cs
│   │   │   │   ├── NeteaseSongUrlApi.cs
│   │   │   │   ├── NeteaseLyricApi.cs
│   │   │   │   └── NeteaseQrLoginClient.cs
│   │   │   ├── Bili/
│   │   │   │   ├── BiliClient.cs
│   │   │   │   ├── BiliAudioUrlApi.cs
│   │   │   │   └── BiliLyricApi.cs
│   │   │   ├── YouTube/
│   │   │   │   ├── YouTubeMusicClient.cs
│   │   │   │   ├── YouTubePlayerScriptStore.cs  # player.js 缓存+PoToken
│   │   │   │   └── YouTubeEjsChallengeSolver.cs
│   │   │   ├── Lyrics/               # 歌词源
│   │   │   │   ├── KugouLyricsClient.cs
│   │   │   │   ├── LrcLibClient.cs
│   │   │   │   └── LyricsSourceAggregator.cs
│   │   │   └── Search/
│   │   │       ├── SearchManager.cs
│   │   │       └── SearchResultMerger.cs
│   │   │
│   │   ├── Download/                  # 下载管理
│   │   │   ├── DownloadManager.cs
│   │   │   ├── DownloadQueue.cs
│   │   │   ├── DownloadTask.cs
│   │   │   ├── MetadataWriter.cs               # TagLib# 封装
│   │   │   └── DownloadDirectoryIndexer.cs     # catalog/snapshot
│   │   │
│   │   ├── Diagnostics/               # 崩溃与安全模式
│   │   │   ├── ExceptionHandler.cs
│   │   │   ├── CrashReporter.cs
│   │   │   ├── AppStartupPlanner.cs
│   │   │   └── SafeModeManager.cs
│   │   │
│   │   └── Logging/
│   │       └── AppLogger.cs           # Serilog 封装
│   │
│   ├── NeriPlayer.Data/               # 数据层
│   │   ├── Database/
│   │   │   ├── NeriDbContext.cs
│   │   │   ├── DbSeeder.cs
│   │   │   └── Migrations/            # EF Core Migrations
│   │   ├── Entities/                  # EF Core 实体（对应 Room @Entity）
│   │   │   ├── SongEntity.cs
│   │   │   ├── PlaylistEntity.cs
│   │   │   ├── PlaylistMemberEntity.cs
│   │   │   ├── PlayHistoryEntity.cs
│   │   │   ├── PlaybackQueueEntity.cs
│   │   │   ├── PlaybackStatsEntity.cs
│   │   │   ├── DownloadEntity.cs
│   │   │   ├── DownloadSnapshotEntity.cs
│   │   │   ├── SyncMetadataEntity.cs
│   │   │   ├── TrafficStatsEntity.cs
│   │   │   ├── CoverUrlMappingEntity.cs
│   │   │   └── SettingsEntity.cs
│   │   ├── Repositories/              # 仓储模式
│   │   │   ├── SongRepository.cs
│   │   │   ├── PlaylistRepository.cs
│   │   │   ├── PlayHistoryRepository.cs
│   │   │   ├── DownloadRepository.cs
│   │   │   ├── PlaybackStatsRepository.cs
│   │   │   ├── SyncMetadataRepository.cs
│   │   │   └── SettingsRepository.cs
│   │   ├── LocalMedia/                # 本地音乐管理
│   │   │   ├── LocalMusicScanner.cs            # 文件夹扫描
│   │   │   ├── TagMetadataReader.cs            # 标签读取
│   │   │   └── LocalSongLibraryManager.cs
│   │   ├── Sync/
│   │   │   ├── ISyncProvider.cs
│   │   │   ├── GitHubSyncProvider.cs
│   │   │   ├── WebDavSyncProvider.cs
│   │   │   ├── SyncMergeStrategy.cs
│   │   │   └── SyncCoordinator.cs
│   │   ├── Settings/
│   │   │   ├── SettingsManager.cs
│   │   │   ├── SettingsSchema.cs              # Source Generator 输入
│   │   │   └── SettingsSection.cs
│   │   ├── Auth/
│   │   │   ├── CredentialStore.cs             # DPAPI 加密
│   │   │   ├── CookieStore.cs
│   │   │   └── LoginStateManager.cs
│   │   └── Traffic/
│   │       └── TrafficStatsService.cs
│   │
│   ├── NeriPlayer.UI/                 # UI 层（Avalonia）
│   │   ├── Views/
│   │   │   ├── MainWindow.axaml               # 主窗口（三栏布局）
│   │   │   ├── NowPlayingView.axaml           # 正在播放
│   │   │   ├── LyricsView.axaml               # 歌词页
│   │   │   ├── LibraryView.axaml              # 音乐库
│   │   │   ├── PlaylistView.axaml             # 歌单详情
│   │   │   ├── SearchView.axaml               # 搜索
│   │   │   ├── DiscoverView.axaml             # 首页推荐
│   │   │   ├── DownloadsView.axaml            # 下载中心
│   │   │   ├── SettingsView.axaml             # 设置
│   │   │   ├── EqualizerView.axaml            # 均衡器
│   │   │   └── LoginView.axaml                # 登录（QR）
│   │   ├── ViewModels/
│   │   │   ├── MainWindowViewModel.cs
│   │   │   ├── NowPlayingViewModel.cs
│   │   │   ├── LibraryViewModel.cs
│   │   │   ├── PlaylistViewModel.cs
│   │   │   ├── SearchViewModel.cs
│   │   │   ├── DiscoverViewModel.cs
│   │   │   ├── DownloadsViewModel.cs
│   │   │   ├── SettingsViewModel.cs
│   │   │   └── PlayerBarViewModel.cs           # 底部播放条
│   │   ├── Controls/
│   │   │   ├── PlayerBar.axaml                # 底部播放控制条
│   │   │   ├── SongCard.axaml
│   │   │   ├── AlbumArtControl.axaml          # 旋转封面
│   │   │   ├── FluidBackground.axaml          # 流体背景（SkiaShader）
│   │   │   ├── LyricsScroller.axaml           # 歌词滚动
│   │   │   ├── WaveformVisualizer.axaml       # 频谱可视化
│   │   │   ├── CircularProgress.axaml
│   │   │   └── ToastControl.axaml
│   │   ├── Themes/
│   │   │   ├── ColorPalette.cs                # 动态取色
│   │   │   ├── ThemeManager.cs
│   │   │   └── Styles/                        # Fluent 主题覆盖
│   │   ├── Converters/
│   │   │   ├── TimeSpanConverter.cs
│   │   │   ├── CoverUrlConverter.cs
│   │   │   └── BoolToVisibilityConverter.cs
│   │   ├── Effects/
│   │   │   ├── ShaderBackgroundRenderer.cs    # Skia 流体
│   │   │   └── AcrylicBlurEffect.cs
│   │   └── Services/
│   │       ├── ImageCacheService.cs
│   │       └── WindowManager.cs
│   │
│   ├── NeriPlayer.Background/         # 后台服务
│   │   ├── PlaybackBackgroundService.cs       # 无窗口播放
│   │   ├── SmtcIntegration.cs                 # 系统媒体控制
│   │   ├── TaskbarThumbnailButtons.cs         # 任务栏缩略图
│   │   ├── FloatingLyricsWindow.cs            # 桌面歌词
│   │   ├── SyncScheduledService.cs            # 定时同步
│   │   └── Notifications/
│   │       └── ToastNotificationService.cs
│   │
│   └── NeriPlayer.SourceGen/          # C# Source Generators
│       ├── SettingsGenerator.cs               # @AutoSetting 等价物
│       └── SettingsKeysGenerator.cs
│
└── tests/
    ├── NeriPlayer.Core.Tests/         # 核心逻辑单元测试
    ├── NeriPlayer.Data.Tests/         # 数据库集成测试
    └── NeriPlayer.Api.Tests/          # API 解析测试（Mock HTTP）
```

### 3.2 依赖方向

```
NeriPlayer.App        → NeriPlayer.Core, NeriPlayer.Data, NeriPlayer.UI, NeriPlayer.Background
NeriPlayer.UI         → NeriPlayer.Core, NeriPlayer.Data
NeriPlayer.Background → NeriPlayer.Core, NeriPlayer.Data
NeriPlayer.Core       → (无 UI/Data 依赖，纯领域逻辑)
NeriPlayer.Data       → (EF Core、Refit，不依赖 UI)
NeriPlayer.SourceGen  → (仅编译期，不参与运行时依赖)
```

---

## 四、核心模块实现策略

### 4.1 PlayerManager 总控（对标 `core/player/PlayerManager.kt`）

PlayerManager 是 NeriPlayer 的中枢，负责：播放状态机、URL 解析、持久化、策略分发、事件广播。

```csharp
public sealed class PlayerManager : IPlayerManager, IDisposable
{
    // 常量对标 Analysis.md 24.1 节
    private static readonly TimeSpan MediaUrlStale = TimeSpan.FromMinutes(10);      // MEDIA_URL_STALE_MS
    private static readonly TimeSpan UrlRefreshCooldown = TimeSpan.FromSeconds(10); // URL_REFRESH_COOLDOWN_MS
    private const int MaxConsecutiveFailures = 10;                                   // 连续失败上限
    private static readonly TimeSpan StatePersistInterval = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan DefaultFadeDuration = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan ProgressThrottle = TimeSpan.FromMilliseconds(80);

    // 状态流（StateFlow 等价物 → IObservable）
    public IObservable<PlaybackState> State { get; }
    public IObservable<SongItem?> CurrentSong { get; }
    public IObservable<TimeSpan> Position { get; }
    public IObservable<PlaybackAudioInfo> AudioInfo { get; }
    public IObservable<PlayerEvent> Events { get; }

    // 命令入口（对标 PlaybackCommand 策略）
    public Task PlayAsync(IReadOnlyList<SongItem> playlist, int startIndex, PlaybackCommandSource source);
    public Task PauseAsync(PlaybackCommandSource source);
    public Task ResumeAsync(PlaybackCommandSource source);
    public Task StopAsync(bool persist = true);
    public Task NextAsync(bool auto = false);
    public Task PreviousAsync();
    public Task SeekAsync(TimeSpan position);
    public Task SetVolumeAsync(float volume);
    public Task SetRepeatModeAsync(RepeatMode mode);
    public Task ToggleShuffleAsync();
    public Task SetPlaybackSpeedAsync(float speed);
    public Task SetPitchAsync(float pitch);
    public Task SetStereoBalanceAsync(float balance);
    public Task SetEqualizerBandsAsync(IReadOnlyList<float> gains);

    // 内部策略管道
    private readonly IPlaybackFailurePolicy _failurePolicy;
    private readonly IMediaUrlRefreshPolicy _urlRefreshPolicy;
    private readonly ITrackEndDedupPolicy _trackEndDedup;
    private readonly IProgressUpdatePolicy _progressPolicy;
    private readonly IPlayerStatePersistence _persistence;
}
```

### 4.2 播放状态机

```
[IDLE] --Play--> [LOADING] --Prepared--> [PLAYING] --Pause--> [PAUSED]
  ^                   |                         |                |
  |                   |--Failure(超限)-->[ERROR]|                |
  |                   |                        +--Seek-->       |
  +--Stop--[STOPPED]--+                        +--Next-->[LOADING]
```

| 行为 | 实现要点 |
|------|----------|
| URL 保鲜 | 播放中每 10min 检查 URL 过期，后台刷新（冷却 10s 防抖） |
| 连续失败保护 | 连续 10 次失败自动停止并广播事件（MAX_CONSECUTIVE_FAILURES） |
| 淡入淡出 | 播放/暂停时 500ms 线性淡变（DEFAULT_FADE_DURATION_MS） |
| 曲目结束去重 | 相邻结束事件 500ms 间隔守卫（TrackEndDeduplication） |
| 进度节流 | 进度流 80ms 节流 + 2s 桶内去重（ProgressUpdatePolicy） |
| 状态持久化 | 15s 周期 + 命令触发即时持久化（STATE_PERSIST_INTERVAL_MS） |
| 播放计数 | 单曲听满 30s 才记 1 次（MIN_LISTEN_MS_FOR_PLAY_COUNT） |

### 4.3 播放引擎封装

```csharp
public interface IPlaybackEngine : IDisposable
{
    Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options);
    Task PlayAsync();
    Task PauseAsync();
    Task SeekAsync(TimeSpan position);
    Task SetVolumeAsync(float volume);       // 0.0 ~ 1.0
    Task SetRateAsync(float speed);          // 变速
    IObservable<PlaybackEngineEvent> Events { get; }

    // 音效管道
    void ApplyEqualizer(IReadOnlyList<float> gains);      // 10-band
    void ApplyStereoBalance(float balance);                // -1.0 ~ 1.0
    void ApplyVolumeNormalization(float gainDb);
    void ApplyPitch(float semitones);

    // 信息
    TimeSpan Duration { get; }
    IObservable<TimeSpan> Position { get; }
    IObservable<float[]> FftData { get; }                  // 可视化
}
```

实现类：

1. **VlcPlaybackEngine** —— 默认引擎
   - LibVLCSharp 封装，支持 mp3/flac/aac/opus/wav/hls/http
   - `MediaPlayer` + `Equalizer`（自带 10-band）
   - FFT 通过独立读取 PCM 或 VLC 可视化数据实现
2. **ExclusivePlaybackEngine** —— WASAPI 独占模式
   - NAudio `WasapiOut`（`AudioClientShareMode.Exclusive`）
   - 采样率跟随源文件（最高 192kHz）
   - 独立音效处理链（ISampleProvider 管道）
   - 输出格式自动协商 + 失败回退共享模式

### 4.4 音效处理管道

```
MediaSource → Decoder → Resampler → [EQ] → [Balance] → [Normalizer] → [Pitch] → [Reverb] → WasapiOut
                                                              ↓
                                                        FFT → Waveform/频谱
```

| 效果 | 对标 NeriPlayer | 实现 |
|------|----------------|------|
| 均衡器 | VLC EQ | VLC Equalizer 或 BIQUAD 滤波器组（10 band） |
| 立体声平衡 | `StereoBalanceAudioProcessor.kt` | NAudio ISampleProvider：(L+R)/2 混音 |
| 音量归一化 | `VolumeNormalizationAudioProcessor.kt` | EBU R128 响度测量 + 增益补偿 |
| 变速 | `normalizePlaybackSpeed` | VLC SetRate（保持音高） |
| 变调 | `normalizePlaybackPitch` | SoundTouch 算法 |
| 混响 | `PlaybackEffectsController` | Freeverb 算法实现 |
| 可视化 | `AudioReactive.kt` | FFT（Cooley-Tukey）+ 平滑处理 |

### 4.5 策略模式（对标 NeriPlayer 200+ 策略文件 → 简化）

| 原版策略包 | 合并后策略类 | 职责 |
|------------|-------------|------|
| `policy/failure` | `PlaybackFailurePolicy` | 连续失败计数、停止阈值、错误分类 |
| `policy/refresh` | `MediaUrlRefreshPolicy` | URL 过期检测、刷新冷却、在途请求去重 |
| `policy/skip` | `VideoSkipPolicy` | 跳过/可裁剪段（Bilibili） |
| `policy/progress` | `ProgressUpdatePolicy` | 进度节流、长音频特殊处理 |
| `policy/pending` | `PendingMediaLoadPolicy` | 待播放媒体位置恢复 |
| `policy/command` | `PlaybackCommandPolicy` | 命令源校验（UI/SMTC/快捷键） |
| `policy/usb/*`（23 个） | `ExclusiveOutputPolicy` | WASAPI 设备路由、失败回退、缓冲 |
| `policy/wake/*` | 无需（Windows 无 WakeLock） | — |
| `policy/offload/*` | 无需（无硬件解码 offload 概念） | — |

---

## 五、数据模型与数据库设计

### 5.1 SongItem（对标 `data/model/SongItem.kt`）

```csharp
public sealed record SongItem
{
    public long Id { get; init; }                    // 本地自增 ID
    public required string Name { get; init; }
    public required string Artist { get; init; }
    public required string Album { get; init; }
    public long AlbumId { get; init; }
    public long DurationMs { get; init; }
    public string? CoverUrl { get; init; }
    public string? MediaUri { get; init; }            // 原始媒体 URI
    public string? StreamUrl { get; init; }           // 已解析的流地址

    // 平台标识（对标 channelId / audioId / subAudioId）
    public string? ChannelId { get; init; }           // "local"|"netease"|"bilibili"|"youtube_music"
    public string? AudioId { get; init; }
    public string? SubAudioId { get; init; }

    // 歌词
    public string? MatchedLyric { get; init; }
    public string? MatchedTranslatedLyric { get; init; }
    public PlaybackSource? MatchedLyricSource { get; init; }
    public long UserLyricOffsetMs { get; init; }

    // 自定义元数据
    public string? CustomName { get; init; }
    public string? CustomArtist { get; init; }
    public string? CustomCoverUrl { get; init; }
    public string? OriginalName { get; init; }
    public string? OriginalArtist { get; init; }

    // 本地文件
    public string? LocalFileName { get; init; }
    public string? LocalFilePath { get; init; }

    // 同步
    // PLANNED: SyncMembershipTokens 对应 Analysis.md 的 observed-remove 语义
    // （syncMembershipTokens / removedMembershipTokens）；当前 .NET SongItem 未定义此字段，
    // 待数据同步（第九章）完善后补充实现、序列化持久化及往返测试。
    public List<SyncToken>? SyncMembershipTokens { get; init; }
    public long AddedAt { get; init; }
}

public enum PlaybackSource
{
    Local,
    Netease,
    Bilibili,
    YouTubeMusic,
}
```

### 5.2 StableKey 算法（对标 `SongIdentity.kt`）

```csharp
public static class SongIdentity
{
    /// <summary>生成跨版本稳定的歌曲标识，用于去重、同步、持久化</summary>
    public static string StableKey(this SongItem song)
    {
        // 本地文件：规范化绝对路径
        if (song.IsLocalSong())
            return $"local|{NormalizePath(song.LocalFilePath ?? song.MediaUri)}";

        // 远程歌曲：平台 + 音频ID
        return song.ChannelId switch
        {
            "netease" => $"netease|{song.AudioId ?? song.Id.ToString()}",
            "bilibili" => $"bilibili|{song.AudioId}|{song.SubAudioId}",
            "youtube_music" => $"ytm|{ExtractYouTubeVideoId(song.MediaUri)}",
            _ => $"id|{song.Id}|{song.Album}|{song.MediaUri}"
        };
    }
}
```

### 5.3 数据库 Schema（对标 Room 13 版迁移）

```
┌──────────────────────────────────────────┐
│              neriplayer.db               │
├──────────────────────────────────────────┤
│ songs             id INTEGER PK, stable_key TEXT UNIQUE,
│                   name, artist, album, album_id, duration_ms,
│                   cover_url, media_uri, stream_url, channel_id,
│                   audio_id, sub_audio_id, matched_lyric,
│                   matched_translated_lyric, user_lyric_offset_ms,
│                   custom_name, custom_artist, custom_cover_url,
│                   local_file_name, local_file_path, added_at
│                   INDEX idx_songs_stable_key
├──────────────────────────────────────────┤
│ playlists         id INTEGER PK, name, kind(本地/收藏/系统),
│                   remote_platform, remote_id, created_at, updated_at
├──────────────────────────────────────────┤
│ playlist_members  playlist_id FK, song_id FK, position INTEGER,
│                   PRIMARY KEY(playlist_id, position)
├──────────────────────────────────────────┤
│ play_history      id INTEGER PK, song_id FK, played_at, source
├──────────────────────────────────────────┤
│ playback_queue    song_id FK, position INTEGER
├──────────────────────────────────────────┤
│ queue_state       id INTEGER PK=1, index, position_ms,
│                   repeat_mode, shuffle_enabled,
│                   shuffle_restore_playlist_json TEXT
├──────────────────────────────────────────┤
│ playback_stats    song_id PK, play_count, total_play_ms,
│                   last_played_at
│ stat_buckets      song_id, day_key, play_count
├──────────────────────────────────────────┤
│ downloads         song_id PK, local_path, status, quality_key,
│                   progress, error, created_at
│ download_snapshots root_key, bucket, entry_key, name, reference,
│                   media_uri, local_file_path, size_bytes,
│                   last_modified_ms, is_directory
│                   PRIMARY KEY(root_key, bucket, entry_key)
├──────────────────────────────────────────┤
│ sync_metadata     key TEXT PK, etag, revision, updated_at
│ sync_outbox       id, song_id, action(UPSERT/DELETE), payload_json
│ sync_checkpoints  scope PK, token, updated_at
├──────────────────────────────────────────┤
│ traffic_stats     day_key PK, bytes_sent, bytes_received
├──────────────────────────────────────────┤
│ cover_url_mapping local_url PK, network_url, updated_at
├──────────────────────────────────────────┤
│ settings          key TEXT PK, value_json, updated_at
├──────────────────────────────────────────┤
│ cookie_credentials platform PK, encrypted_cookies BLOB,
│                   encrypted_refresh_token BLOB, updated_at
│                   (值使用 DPAPI 加密)
└──────────────────────────────────────────┘
```

### 5.4 数据库迁移策略

- 使用 **EF Core Migrations**，对应 NeriPlayer Room 的 13 版迁移
- 每个 Schema 版本一个 Migration 类，保留历史迁移（`Database/Migrations/`）
- 启动时自动 `Database.Migrate()`（对标 NeriPlayer 的迁移兼容）
- 破坏性变更走「新建表 + 数据复制 + 旧表删除」三段式（Room 同款策略）

### 5.5 播放状态持久化（对标 `PersistedPlayerState.kt`）

```csharp
public sealed record PersistedPlaybackState
{
    public IReadOnlyList<SongItem> Playlist { get; init; } = [];
    public int Index { get; init; }
    public string? MediaUrl { get; init; }
    public long PositionMs { get; init; }
    public bool ShouldResumePlayback { get; init; }
    public RepeatMode? RepeatMode { get; init; }
    public bool? ShuffleEnabled { get; init; }
}
```

- 存储位置：`%APPDATA%/NeriPlayer/playback_state.json`
- 写盘时机：每 15s + 播放命令（停止/切换）时
- 恢复策略：启动后 1.5s 延迟恢复（对标 `INITIAL_SCAN_DELAY_MS`），失败静默跳过
- 队列中的本地歌曲丢失时回退到「可恢复的本地媒体」检查（对标 `RestorableLocalMediaPolicy`）

---

## 六、播放引擎设计

### 6.1 LibVLC 引擎详细设计

```
VlcPlaybackEngine
├── LibVLC 初始化（--no-video、--audio-resampler、--network-caching=800ms）
├── MediaPlayer 实例
├── Equalizer（10-band：31Hz~16kHz，-20dB ~ +20dB）
├── 事件桥接（Playing/Paused/EndReached/EncounteredError/Buffering/PositionChanged）
├── 位置同步（MediaPlayer.Time 轮询 + 事件）
└── URL 解析前置（HTTP/HLS 由 VLC 原生支持，无需额外解码器）
```

| 场景 | 处理 |
|------|------|
| 网络流 | `--network-caching=800` 缓冲；播放失败 → URL 重新解析 → 重试 1 次 |
| 高音质 | FLAC 24bit 原生支持；采样率自动协商 |
| 断网 | `EncounteredError` 事件 → 失败策略：连续失败计数 |
| 拖拽 | `MediaPlayer.Time = ms` 毫秒级精确 |
| 变速 | `MediaPlayer.Rate`（0.5~2.0） |
| 音量 | `MediaPlayer.Volume`（0~100）映射到 0~1 |
| 均衡器 | `Equalizer.Create(10 bands)` + `SetBands` |

### 6.2 WASAPI 独占引擎详细设计（对标 USB 独占）

```
ExclusivePlaybackEngine (NAudio)
├── WasapiOut(device, AudioClientShareMode.Exclusive, 30ms buffer)
├── 源文件 → AudioFileReader → ISampleProvider 管道
│   ├── VolumeNormalizationSampleProvider
│   ├── StereoBalanceSampleProvider
│   ├── EqualizerSampleProvider (BIQUAD)
│   └── PitchShifterSampleProvider (SoundTouch)
├── FftSampleProvider（实时 FFT 输出，用于可视化）
├── 设备事件（MMDeviceEnumerator 监听默认设备变化）
└── 失败回退（Exclusive 失败 → Shared 模式回退，对标 UsbExclusiveFallbackPolicy）
```

### 6.3 播放 URL 解析器

```csharp
public interface ISongUrlResolver
{
    Task<SongUrlResolution> ResolveAsync(SongItem song, QualityPreference quality);
    bool Supports(SongItem song);
}

public sealed class SongUrlResolverChain
{
    private readonly IReadOnlyList<ISongUrlResolver> _resolvers =
    [
        new LocalFileUrlResolver(),        // 本地文件直接返回路径
        new NeteaseSongUrlResolver(),      // 网易云 v1/v2 加密 API
        new BiliAudioUrlResolver(),        // Bilibili 音频流
        new YouTubeMusicUrlResolver(),     // InnerTube + PoToken + EJS
    ];

    public async Task<ResolvedMedia> ResolveAsync(SongItem song, QualityPreference quality)
    {
        // 缓存命中检查（10min 有效）
        // 逐个解析器尝试
        // URL 保鲜：后台任务提前刷新将过期 URL
    }
}

public record QualityPreference
{
    public string Netease { get; init; } = "exhigh";   // standard/higher/exhigh/loseless
    public string YouTube { get; init; } = "high";     // low/medium/high/very_high
    public string Bili { get; init; } = "high";
}
```

### 6.4 曲目结束去重

对标 `TrackEndDeduplication.kt`（500ms 间隔守卫）：

```csharp
public sealed class TrackEndDeduplication
{
    private DateTimeOffset _lastEndAt;
    private static readonly TimeSpan GuardWindow = TimeSpan.FromMilliseconds(500);

    public bool TryConsume()
    {
        var now = DateTimeOffset.UtcNow;
        if (now - _lastEndAt < GuardWindow) return false;
        _lastEndAt = now;
        return true;
    }
}
```

---

## 七、音效系统设计

### 7.1 效果链

```
AudioPipeline
├── EqualizerEffect      (10-band BIQUAD, 31Hz-16kHz, ±20dB, 支持预设)
│   预设：流行/摇滚/爵士/古典/电子/人声/自定义
├── StereoBalanceEffect  (-1.0 全左 ~ 0 平衡 ~ +1.0 全右)
├── VolumeNormalization  (EBU R128：测量 LUFS → 目标 -14 LUFS 增益)
├── SpeedEffect          (0.5x ~ 2.0x, 保音高)
├── PitchEffect          (-12 ~ +12 半音)
└── ReverbEffect         (房间大小/衰减/干湿比)
```

### 7.2 均衡器实现

采用 **Direct Form I Biquad**（音频行业标准）：

```csharp
public sealed class BiquadFilter
{
    public enum FilterType { Peaking, LowShelf, HighShelf }
    public double B0, B1, B2, A1, A2;   // 系数由频率/增益/带宽计算

    public float Process(float input)    // 直通式差分方程
    {
        // y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
    }
}
```

10 段频率：`31.25, 62.5, 125, 250, 500, 1k, 2k, 4k, 8k, 16k`（Hz）

### 7.3 音频可视化

- FFT 实现：Cooley-Tukey 基 2 算法（512~4096 点，50% overlap）
- 输出：对数频率刻度（20Hz~20kHz，64 频带）+ 峰值保持 + 衰减
- 消费端：播放页频谱条、波形可视化

---

## 八、API 客户端设计

### 8.1 统一平台接口

```csharp
public interface IPlatformClient
{
    string PlatformId { get; }                        // "netease" / "bili" / "youtube_music"
    bool IsLoggedIn { get; }
    Task<LoginResult> LoginAsync(LoginMethod method); // QR / Cookie / Token

    // 搜索
    Task<SearchResponse> SearchAsync(string keyword, SearchScope scope, int page);

    // 歌单
    Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page);
    Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId);

    // 歌曲播放
    Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string qualityKey);

    // 歌词
    Task<LyricResult?> GetLyricAsync(SongItem song);

    // 推荐
    Task<RecommendationFeed> GetRecommendationsAsync();
}
```

### 8.2 网易云客户端（对标 `NeteaseClient.kt` + `NeteaseCrypto.kt`）

| 能力 | 实现 |
|------|------|
| 搜索 | `weapi/search/get`（POST，JSON） |
| 歌曲 URL | `weapi/song/enhance/player/url/v1`（加密参数） |
| 歌单详情 | `weapi/v6/playlist/detail` |
| 歌词 | `weapi/song/lyric`（LRC + 翻译） |
| 首页推荐 | `weapi/v3/homepage/page` |
| 私人 FM | `weapi/radio/get` |
| 加密 | AES-CBC（`0CoJUm6Qyw8W8jud` key）+ RSA（公钥 `010001`）+ 随机 secretKey |
| WBI 签名 | 时间戳 + 随机数 + MD5/SHA1 摘要 |
| 登录 | 二维码轮询（create + check） |
| Cookie | DPAPI 加密文件，每次请求注入 |

### 8.3 Bilibili 客户端（对标 `BiliClient.kt`）

| 能力 | 实现 |
|------|------|
| 音频搜索 | `x/web-interface/search/type`（search_type=audio） |
| 音频 URL | `audio/music-service-c/songs/url`（v2 接口，WBI 签名） |
| 视频音频流 | `x/player/playurl`（fnval=16 提取 DASH audio） |
| 歌词 | 音频详情接口内嵌 |
| 登录 | 二维码（`x/passport-login/web/qrcode/generate`） |

### 8.4 YouTube Music 客户端（对标 `YouTubeMusicClient.kt`）

| 能力 | 实现 |
|------|------|
| 搜索 | InnerTube `youtubei/v1/search`（music 上下文） |
| 歌曲 URL | 播放列表加载 → 提取 streamingData（PoToken + signature） |
| player.js 解析 | 缓存到磁盘，48h 过期刷新（对标 `YouTubePlayerScriptStore`） |
| PoToken | 缓存 6h；EJS 挑战失败回退 NewPipeExtractor |
| 歌词 | InnerTube browse + watch 页面 |
| 登录 | Cookie（SAPISID / __Secure-3PAPISID） |

### 8.5 歌词源聚合（对标 LyriconManager）

```
LyricsProvider
├── 内嵌歌词（文件内标签 / 远程匹配）
├── 网易云歌词
├── QQ 音乐歌词（DEFAULT_QQ_MUSIC_LYRIC_OFFSET_MS 补偿）
├── Kugou 歌词（KC 解密 + LRC 转换）
├── LrcLib 歌词
└── 合并排序：内嵌 > 匹配源 > 平台官方 > 第三方
```

### 8.6 搜索聚合（对标 `SearchManager.kt`）

- 多平台并发搜索（HttpClient 并行请求）
- 结果合并去重（按 stableKey）
- 排序：平台优先级（网易云 > YouTube > Bilibili）+ 相关度
- 缓存：搜索结果 LRU（每平台 20 条）

---

## 九、下载管理系统设计

### 9.1 架构（对标 `core/download/` 40+ 文件）

```
DownloadManager
├── DownloadQueue          # 任务队列（Semaphore 并发控制）
│   ├── DefaultConcurrency = 6
│   ├── MaxConcurrency = 8
│   └── 取消后 5000ms 稳定期（DOWNLOAD_CANCEL_SETTLE_TIMEOUT_MS）
├── DownloadTask           # 单个任务（HttpClient 流式下载 + 进度上报）
├── MetadataWriter         # TagLib# 写标签（重试上限 3 次）
├── DownloadDirectoryIndexer  # 三层索引
│   ├── catalog      # 主目录清单
│   ├── snapshot     # 快照
│   └── recovery     # 恢复（异常退出后扫描）
└── DownloadRepository    # EF Core 持久化
```

### 9.2 下载流程

```
用户选择歌曲 + 质量 → 解析 URL → 创建 DownloadTask
→ 信号量获取（并发 ≤ 8）
→ HttpClient 流式下载到 .part 临时文件
→ 完成后校验 → 重命名 → MetadataWriter 写入标签（封面/歌词/标题/艺人）
→ 更新 catalog + snapshot 索引 → 通知 UI
```

### 9.3 断点续传

- 下载中断 → 保留 `.part` + 已下载字节数（记录到 DB）
- 恢复时 `Range: bytes=<已下载>-` 续传
- 服务端不支持 Range → 全量重下

### 9.4 下载目录结构

```
NeriPlayer/Music/
├── netease/           # 按平台分类
│   └── {artist}/{album}/{title}.flac
├── bilibili/
├── youtube/
└── downloads.db       # 索引数据库
```

---

## 十、数据同步设计

### 10.1 同步架构（对标 `data/sync/` + Analysis.md 第九章）

```
SyncCoordinator
├── ISyncProvider (策略注入)
│   ├── GitHubSyncProvider    # Octokit → repo 内 JSON 文件
│   └── WebDavSyncProvider    # WebDAV → 远程目录
├── SyncMergeStrategy         # 冲突合并
├── SyncOutbox                # 待同步操作队列（断网缓存）
├── SyncCheckpoint            # 游标（增量同步）
└── 计划任务                  # Quartz：每日 / 手动 / 事件触发
```

### 10.2 同步内容

| 数据 | 格式 | 冲突策略 |
|------|------|----------|
| 歌单（含歌曲元数据） | JSON（不含本地文件路径，仅稳定键） | 按 updated_at + 因果 Token |
| 播放历史 | JSON（最近 1000 条） | 合并去重 |
| 收藏 | JSON（platform + songId） | 合并 |
| 设置 | `settings.json` | 最后写入胜出 |
| 播放统计 | JSON | 合并（取 max 计数） |

### 10.3 因果一致性（对标 SyncCausalToken）

- 每条同步记录携带 `SyncToken { songId, baseVersion, operationId }`
- 冲突时按「因果序 + 时间戳」裁决
- 对应 `SyncOutboxEntity` / `SyncReplicaCheckpointEntity` 设计

---

## 十一、UI 设计

### 11.1 主窗口布局（对标 NeriPlayer 三栏布局）

```
┌──────────────────────────────────────────────────────────────┐
│ 标题栏（自绘：Logo + 导航切换 + 搜索框 + 窗口控制）           │
├──────────┬──────────────────────────────────┬─────────────────┤
│ 侧边栏   │       内容区（页面容器）          │   播放队列     │
│          │                                  │   （可折叠）   │
│ 🏠 首页  │                                  │                │
│ 📻 发现  │   (NowPlaying / Library /        │  ┌──────────┐  │
│ 💿 我的  │    Search / Playlist / ...)      │  │ 当前队列  │  │
│ 📥 下载  │                                  │  └──────────┘  │
│ ⚙ 设置  │                                  │                │
├──────────┴──────────────────────────────────┴─────────────────┤
│ 底部播放条：封面 │ 歌名-艺人 │ 进度条 │ ♥ │ ⏮ ▶/⏸ ⏭ │ EQ │ 歌词 │ 音量 │
└────────────────────────────────────────────────────────────────┘
```

### 11.2 正在播放页

| 区域 | 实现 |
|------|------|
| 背景 | 流体背景 Shader（SkiaShader 重写 AGSL `hyper_background_effect.glsl`） |
| 封面 | 旋转唱片动画（模糊 + 圆角遮罩 + 光影） |
| 主控区 | 进度条（可拖拽）+ 播放/暂停/上下曲/循环/随机 |
| 歌词区 | 可切换歌词视图（滚动高亮 + 翻译） |
| 音效区 | EQ 面板（10 滑块）+ 混响 + 立体声 |
| 频谱 | WaveformVisualizer 组件 |

### 11.3 流体背景 Shader

对标 `assets/shaders/hyper_background_effect.glsl`（5 色点 + 噪声流动）：

- **Skia 实现**：用 `SKRuntimeEffect`（Skia 的 Runtime Effect，GLSL 兼容）
- 直接迁移原 GLSL 到 Skia `sksl` 语法（改动极小）
- 参数：5 个色点（uniform vec4[5]）+ 时间流 u_time + 鼠标交互偏移
- 降级策略：GPU 不支持 → 静态渐变

### 11.4 毛玻璃效果

对标 `advanced_glass_*.agsl`（高级模糊毛玻璃）：

- 方案 A：`SKImageFilter.CreateBlur` + 背景快照（静态场景）
- 方案 B：Avalonia `ExperimentalAcrylicBorder`（实时模糊）
- 推荐：B（Avalonia 内置，性能好）

### 11.5 歌词页面

- `LyricsScroller` 控件：ItemsControl + 虚拟化 + 居中高亮行
- 滚动算法：positionMs → 行索引，当前行 1.25x 字号 + 主题色
- 翻译歌词：双行（原文 + 翻译），可开关
- 歌词偏移：长按 +/- 微调（1s 步进，对标 userLyricOffsetMs）
- 歌词搜索：标题+艺人 → 多源歌词匹配
- 分享卡片：1080px 宽渲染

### 11.6 首页推荐

| 分区 | 数据源 | 上限 |
|------|--------|------|
| 每日推荐 | 网易云 `recommend/songs` | 30 首 |
| 推荐歌单 | 网易云 `personalized/playlist` | 30 个 |
| 私人 FM | 网易云 `radio/get` | 批量 10 |
| 雷达歌单 | 固定 5 个 ID（时光/宝藏/新歌/乐迷/神秘） | 5 个 |
| 失败降级 | code 301 / 50000005 → 回退热门歌单 | — |

### 11.7 主题系统

- 暗色/亮色 + 跟随系统（ActualThemeVariant）
- 动态取色：封面 → 主色调（中位切分算法，对标 Palette ktx）
- Material.Avalonia 主题样式（Material 3 风格）
- 设置：主题、强调色、字体大小、模糊强度、流体背景开/关

### 11.8 图片加载

- `ImageCacheService`：HTTP 图片 → 磁盘缓存（%LOCALAPPDATA%/NeriPlayer/image-cache，LRU 1GB）
- 内存缓存：ConcurrentDictionary + 大小预算（32MB）
- 封面占位：渐变 + 首字母
- 对标 Coil 的请求优先级 + 内存/磁盘双层缓存

---

## 十二、后台服务与系统集成

### 12.1 SMTC（SystemMediaTransportControls）

对标 Android MediaSession：

```csharp
public sealed class SmtcIntegration
{
    private readonly SystemMediaTransportControls _smtc;

    // 更新
    void UpdateMetadata(SongItem song, string? albumArtPath);
    void UpdatePlaybackStatus(PlaybackStatus status);  // Playing/Paused/Stopped
    void UpdatePosition(TimeSpan position, TimeSpan duration);

    // 事件 → PlayerManager
    ButtonPressed (Play/Pause/Next/Previous/Seek/Stop)
}
```

功能点：
- 任务栏媒体悬浮窗（封面 + 标题 + 进度 + 控制按钮）
- Win+L 锁屏媒体控制
- 媒体键（键盘上的 ⏯⏭⏮）

### 12.2 任务栏缩略图按钮

- 播放/暂停、上一曲、下一曲 3 个按钮
- `ThumbButtonInfo` + `TaskbarItemInfo`

### 12.3 桌面歌词（对标 FloatingLyricsOverlayManager）

- `FloatingLyricsWindow`：Topmost + 透明 + 无边框 + 可拖动
- 双行：原文 + 翻译
- 设置：字号、颜色、位置锁定、点击穿透

### 12.4 后台播放（对标 AudioPlayerService）

- 无 UI 场景：窗口关闭后播放不中断
- 实现：托盘图标（TrayIcon）+ 后台线程持锁 + SMTC 常驻
- 退出策略：托盘「退出」才真正释放引擎（对标 `PlaybackServiceIdlePolicy`）

### 12.5 通知

- 播放状态变化 → Toast 通知（ToastNotificationManager）
- 下载完成 → Toast + 点击打开位置
- 同步完成/失败 → Toast

### 12.6 定时任务（对标 WorkManager）

| 任务 | 调度 | 说明 |
|------|------|------|
| 同步 | 每日 02:00 + 手动 | Quartz cron |
| 播放统计刷盘 | 15s 周期 | PLAYBACK_STATS_PERIODIC_FLUSH_MS |
| URL 保鲜 | 事件触发 | 播放中每 10min |
| 清理过期缓存 | 每周 | 图片缓存 LRU 清理 |

---

## 十三、安全与崩溃恢复

### 13.1 凭据安全（对标 androidx.security-crypto）

| 数据 | 存储 | 加密 |
|------|------|------|
| Cookie（三平台） | `%APPDATA%/NeriPlayer/secure/cookies.bin` | **DPAPI**（CurrentUser） |
| 刷新 Token | 同上 | DPAPI |
| 网易云设备 ID | `%APPDATA%/NeriPlayer/secure/device_id` | 明文 + 权限限制 |

### 13.2 崩溃处理（对标 ExceptionHandler + AnrWatchdog）

```csharp
public static class ExceptionHandler
{
    public static void Install()
    {
        AppDomain.CurrentDomain.UnhandledException += OnUnhandled;
        TaskScheduler.UnobservedTaskException += OnUnobservedTask;
        Application.Current.DispatcherUnhandledException += OnDispatcher;  // Avalonia
    }

    // 崩溃报告：写入 %LOCALAPPDATA%/NeriPlayer/crash/
    // 上次崩溃检测：启动时扫描 crash/ 目录 → 提示是否发送报告
    // 安全模式：连续 2 次崩溃 → 下次启动禁用自动同步/自启
}
```

### 13.3 安全模式（对标 SafeModeManager + AppStartupPlanner）

- 触发条件：连续 2 次启动即崩溃、数据库迁移失败、设置文件损坏
- 行为：禁用自动同步、禁用 YouTube PoToken 自研解析（走回退）、清空异常缓存
- 用户可从设置页退出安全模式

### 13.4 数据备份

- 一键导出：settings.json + playlists.json + 播放历史 JSON（对标同步格式）
- 手动备份到任意目录

---

## 十四、测试体系

### 14.1 单元测试（NeriPlayer.Core.Tests）

| 被测模块 | 用例 |
|----------|------|
| 歌词解析器 | LRC 偏移/多语言/空行/损坏输入 |
| StableKey | 本地/远程/YouTube 视频 ID 提取 |
| 曲目去重 | 500ms 窗口内去重 |
| 失败策略 | 连续失败计数与停止 |
| 播放状态机 | 状态转移合法/非法 |
| 音效处理 | EQ 增益正确性、立体声平衡端点值 |
| 下载队列 | 并发限制、取消稳定期 |
| 同步合并 | 因果 Token 冲突裁决 |
| 网易云加密 | 已知输入输出向量（固定密钥） |
| 搜索合并 | 多源去重排序 |

### 14.2 集成测试（NeriPlayer.Data.Tests）

- EF Core 迁移正确性（SQLite 内存库）
- Repository CRUD
- 播放状态持久化 round-trip

### 14.3 UI 测试

- Avalonia Headless 测试框架（Avalonia.Headless.XUnit）
- 关键视图：播放条、歌词滚动、主窗口布局

### 14.4 性能基准

| 场景 | 目标 |
|------|------|
| 启动（冷启动到可播放） | ≤ 3s |
| 本地库 10,000 首扫描 | ≤ 30s（增量扫描 ≤ 2s） |
| 内存占用（5000 首歌单） | ≤ 400MB |
| 连续播放 48h | 无泄漏、无崩溃 |
| 封面缓存 | 启动后第二次加载 ≤ 100ms |

---

## 十五、打包与发布

### 15.1 打包格式

| 格式 | 用途 |
|------|------|
| MSIX | Microsoft Store / 企业分发（自动更新） |
| 便携版 (self-contained) | 免安装单文件夹 |
| NSIS 安装包 | 传统安装器（可选） |

### 15.2 发布配置

- `dotnet publish -c Release -r win-x64 --self-contained true`
- 包含 libvlc.dll + libvlccore.dll + plugins/ 目录
- 代码签名（EV 证书，可选）

### 15.3 CI/CD（GitHub Actions）

```
workflows/build.yml
├── job: windows-build
│   ├── checkout + setup .NET 8
│   ├── restore + build（含 VLC 依赖）
│   ├── run tests（xunit）
│   ├── publish win-x64
│   └── upload artifact（MSIX + portable）
```

---

## 十六、分阶段实施计划

### 阶段 1：项目脚手架与环境搭建（第 1 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 1.1 | 创建解决方案 + 各项目骨架 | sln + 4 个 csproj | `dotnet build` 通过 |
| 1.2 | 配置 Directory.Build.props / Packages.props | 全局编译配置 | 统一 SDK/Nullable |
| 1.3 | 配置 DI 容器（AppStartup） | ServiceCollection 装配 | 应用可启动 |
| 1.4 | 配置 Serilog（文件 + 控制台） | 日志目录 | 启动日志可查 |
| 1.5 | 安装 Avalonia 模板 + 空窗口 | 空主窗口 | 窗口显示 |

### 阶段 2：核心数据模型（第 1-2 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 2.1 | SongItem / SongIdentity / PlaybackSource | model 类 | 单元测试通过 |
| 2.2 | PlaybackAudioInfo / 质量选项 | model 类 | 质量标签格式化正确 |
| 2.3 | PlayerEvent / 事件枚举 | model 类 | 事件订阅测试 |
| 2.4 | StableKey 算法 + 测试 | SongIdentity.cs | 去重测试通过 |
| 2.5 | JSON 序列化契约测试 | 序列化测试 | round-trip 稳定 |

### 阶段 3：本地数据库（第 2-3 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 3.1 | EF Core + SQLite 集成 | NeriDbContext | 迁移可生成 |
| 3.2 | 设计全部 Entity（15+ 表） | Entities/ | Schema 检查 |
| 3.3 | 初始迁移 + 种子数据 | Migrations/ | 迁移执行成功 |
| 3.4 | Repository 层 | Repositories/ | CRUD 集成测试 |
| 3.5 | 设置存储（SettingsRepository） | 配置读写 | 读写 round-trip |
| 3.6 | 播放状态持久化 | PlayerStatePersistence | 恢复测试 |

### 阶段 4：播放引擎（第 3-5 周）← 最核心

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 4.1 | IPlaybackEngine 接口定义 | 接口 | 编译通过 |
| 4.2 | LibVLC 集成 + 本地文件播放 | VlcPlaybackEngine | 播放本地 mp3/flac |
| 4.3 | 播放/暂停/Seek/音量/循环/随机 | PlayerManager 命令 | 状态机测试 |
| 4.4 | HTTP 流播放 + URL 解析链 | SongUrlResolverChain | 播放网络流 |
| 4.5 | 淡入淡出 + 失败保护 | 策略类 | 失败恢复测试 |
| 4.6 | 播放状态持久化恢复 | persistence | 重启恢复播放 |
| 4.7 | HLS 播放（VLC 原生） | 引擎测试 | HLS 流可播放 |

### 阶段 5：音效系统（第 5-6 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 5.1 | EQ 10-band（BIQUAD） | EqualizerEffect | 频响测试 |
| 5.2 | 立体声平衡 | StereoBalanceEffect | 端点值测试 |
| 5.3 | 音量归一化（R128） | VolumeNormalization | 响度测试 |
| 5.4 | 变速变调 | SpeedPitchEffect | 音质主观验收 |
| 5.5 | FFT 可视化数据 | FftAnalyzer | 频谱输出稳定 |
| 5.6 | EQ 预设 + 保存 | UI 集成 | 设置持久化 |

### 阶段 6：API 客户端（第 6-8 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 6.1 | 网易云搜索 + 歌单 | NeteaseClient | 真实搜索可返回 |
| 6.2 | 网易云 URL 解析（WBI 签名） | NeteaseCrypto | 播放验证 |
| 6.3 | 网易云歌词 | NeteaseLyricApi | LRC 正确 |
| 6.4 | 网易云登录（二维码） | NeteaseQrLoginClient | 扫码登录 |
| 6.5 | Bilibili 音频搜索 + URL | BiliClient | 播放验证 |
| 6.6 | YouTube Music 搜索 + URL（PoToken） | YouTubeMusicClient | 播放验证（P2） |
| 6.7 | 歌词源聚合（Kugou/LrcLib） | LyricsProvider | 多源回退 |
| 6.8 | 搜索聚合器 | SearchManager | 多平台结果合并 |

### 阶段 7：下载管理（第 8-9 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 7.1 | 下载队列 + 并发控制 | DownloadManager | 并发限制测试 |
| 7.2 | 流式下载 + 进度 | DownloadTask | 进度连续 |
| 7.3 | 断点续传 | 恢复逻辑 | 中断恢复 |
| 7.4 | 标签写入（TagLib#） | MetadataWriter | 封面嵌入成功 |
| 7.5 | 目录索引（catalog/snapshot） | Indexer | 扫描正确 |
| 7.6 | 下载中心 UI | DownloadsView | 管理界面可用 |

### 阶段 8：数据同步（第 9-10 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 8.1 | GitHub 同步 Provider | GitHubSyncProvider | 推送/拉取成功 |
| 8.2 | WebDAV 同步 Provider | WebDavSyncProvider | 推送/拉取成功 |
| 8.3 | 合并策略 + 因果 Token | SyncMergeStrategy | 冲突测试 |
| 8.4 | 增量同步 + 游标 | SyncCoordinator | 只传增量 |
| 8.5 | 设置备份/恢复 | 备份功能 | 备份还原 |

### 阶段 9：UI 主框架（第 10-11 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 9.1 | 主窗口三栏布局 + 导航 | MainWindow | 布局完成 |
| 9.2 | 底部播放条（绑定 PlayerManager） | PlayerBar | 播放联动 |
| 9.3 | 正在播放页 | NowPlayingView | 核心控制可用 |
| 9.4 | 歌词页（滚动高亮） | LyricsView | 歌词滚动正确 |
| 9.5 | 音乐库 + 本地扫描 | LibraryView | 扫描 + 展示 |
| 9.6 | 歌单详情 | PlaylistView | 歌曲列表操作 |
| 9.7 | 搜索页 | SearchView | 搜索展示 |
| 9.8 | 首页推荐 | DiscoverView | 推荐加载 |
| 9.9 | 设置页（SourceGen 生成） | SettingsView | 设置可用 |
| 9.10 | 主题系统（暗/亮 + 动态取色） | ThemeManager | 切换即时生效 |

### 阶段 10：后台与系统集成（第 10-12 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 10.1 | SMTC 集成 | SmtcIntegration | 任务栏媒体控制 |
| 10.2 | 托盘图标 + 后台播放 | TrayIntegration | 关窗继续播放 |
| 10.3 | 任务栏缩略图按钮 | ThumbnailButtons | 按钮可用 |
| 10.4 | 桌面歌词窗口 | FloatingLyricsWindow | 悬浮歌词 |
| 10.5 | Toast 通知 | NotificationService | 下载完成通知 |
| 10.6 | Quartz 定时任务 | SyncScheduledService | 定时同步 |

### 阶段 11：高级特性（第 11-13 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 11.1 | WASAPI 独占引擎 | ExclusivePlaybackEngine | 独占输出 |
| 11.2 | 流体背景 Shader | FluidBackground | GPU 流畅运行 |
| 11.3 | 毛玻璃效果 | AcrylicBlurEffect | 视觉效果 |
| 11.4 | 频谱可视化 | WaveformVisualizer | 随音乐跳动 |
| 11.5 | 迷你播放器 | MiniPlayerView | 简洁模式 |
| 11.6 | 快捷键 | HotkeyManager | 全局快捷键 |

### 阶段 12：测试与打磨（第 13-14 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 12.1 | 核心单元测试补全 | tests/ | 覆盖率 ≥ 60% |
| 12.2 | 数据层集成测试 | tests/ | 迁移/CRUD 通过 |
| 12.3 | UI Headless 测试 | tests/ | 关键视图渲染 |
| 12.4 | 性能基准 | benchmark 报告 | 达标（见 14.4） |
| 12.5 | 崩溃恢复验证 | 验证报告 | 安全模式可用 |
| 12.6 | 长时间播放稳定性（48h） | 稳定性报告 | 无泄漏无崩溃 |

### 阶段 13：打包与发布（第 14-16 周）

| # | 任务 | 产出 | 验收 |
|---|------|------|------|
| 13.1 | MSIX 打包 | .msix | 安装成功 |
| 13.2 | 便携版发布 | portable zip | 解压即用 |
| 13.3 | GitHub Actions CI/CD | workflows | 自动构建测试 |
| 13.4 | 版本号管理 + 更新检查 | AutoUpdater | 版本提示 |
| 13.5 | 用户文档 + README | docs | 使用说明 |

---

## 十七、架构简化清单

NeriPlayer 共 1748 个 Kotlin 文件，Windows 重写必须做架构简化（保留能力、精简实现）：

| 原版模块 | 原版规模 | 简化后规模 | 说明 |
|----------|----------|-----------|------|
| `core/player/` | 124 文件（23 策略包） | ~25 文件 | 策略合并为 6 个内聚类 |
| `core/api/` | 34 文件 | ~25 文件 | 保留三平台，统一接口 |
| `data/local/` | ~55 文件 | ~25 文件 | EF Core 减少 DAO/Store 冗余 |
| `data/sync/` | ~20 文件 | ~10 文件 | 抽象 Provider |
| `data/settings/` | ~15 文件 | ~8 文件 | SourceGen 自动生成 |
| `core/download/` | ~45 文件 | ~8 文件 | 队列模型简化 |
| `listentogether/` | ~20 文件 | **0**（砍掉） | 非 Windows 场景 |
| `core/player/usb/` | ~35 文件 | ~3 文件 | WASAPI 独占替代 |
| `widget/` | ~15 文件 | **0**（砍掉） | SMTC 替代 |
| `core/lyricon/` | ~8 文件 | ~2 文件 | 内置歌词源替代 |
| **合计** | **~770** | **~130** | 代码量约减少 83% |

### 必须保留的复杂设计

| 设计 | 保留原因 |
|------|----------|
| URL 保鲜 + 刷新冷却 | 在线播放核心体验 |
| 连续失败保护 | 网络波动兜底 |
| StableKey + 去重 | 同步/队列基础 |
| 因果 Token 同步 | 多端数据一致性 |
| 下载三层索引 | 异常退出后文件可恢复 |
| 播放状态 15s 持久化 | 崩溃恢复基础 |
| 听满 30s 计次 | 统计准确 |
| 进度节流 + 曲目去重 | 状态一致性 |

---

## 十八、风险与缓解

| # | 风险 | 概率 | 影响 | 缓解措施 |
|---|------|------|------|----------|
| 1 | 网易云/B站 API 变更 | 高 | 高 | 版本化接口 + 失败降级 + URL 保鲜重试 |
| 2 | YouTube PoToken 反爬升级 | 高 | 中 | 分层：自研解析 → NewPipe 回退 → 用户 Cookie |
| 3 | LibVLC 集成问题（Avalonia 兼容） | 中 | 高 | 提前 PoC（第 4 阶段第 1 周先行验证） |
| 4 | EF Core 迁移与 Room 13 版 schema 差异 | 低 | 中 | 数据导出/导入 JSON 作为迁移桥 |
| 5 | 流体 Shader 性能（低端 GPU） | 中 | 低 | 降级为静态渐变 |
| 6 | WASAPI 独占设备独占失败 | 中 | 低 | 自动回退共享模式 + 设备变化监听 |
| 7 | 大曲库性能（>5 万首） | 低 | 中 | 分页 + 虚拟化 + 增量扫描 |
| 8 | 工期超预期 | 中 | 高 | 严格按 P0/P1 优先级裁剪；阶段 6.6/11.x 可后置 |

**关键技术 PoC（先于主开发）**：
1. Avalonia 空窗口 + LibVLC 播放本地音频（半天）
2. LibVLC 播放网易云 HTTP 流（1 天）
3. EF Core SQLite 迁移执行（半天）
4. Skia RuntimeEffect 流体 Shader 跑通（1 天）

---

## 十九、验收标准

### 19.1 功能验收（对标 NeriPlayer 核心能力）

| 能力 | 验收标准 |
|------|----------|
| 本地播放 | mp3/flac/wav/aac/ogg/m4a 全部可播，标签正确 |
| 网易云播放 | 搜索→歌单→播放→歌词全链路可用，登录后可播高音质 |
| Bilibili 播放 | 音频搜索可播（P1），视频提取音频可播（P2） |
| YouTube Music | 搜索可播（P2，PoToken 可回退 NewPipe） |
| 歌词 | LRC 滚动 + 翻译 + 偏移校准 + 多源回退 |
| 音效 | EQ 10 段可调、立体声、变速不变调 |
| 下载 | 并发 8、断点续传、封面嵌入、目录索引恢复 |
| 同步 | GitHub/WebDAV 双向同步、冲突正确合并 |
| 播放恢复 | 重启后恢复队列/进度/模式 |
| 统计 | 播放计数（30s 规则）、流量统计 |

### 19.2 非功能验收

| 维度 | 标准 |
|------|------|
| 启动时间 | 冷启动 ≤ 3s |
| 内存 | 峰值 ≤ 400MB（5000 首队列） |
| 稳定性 | 48h 连续播放无崩溃 |
| 兼容性 | Win10 21H2+ / Win11 |
| 可维护性 | 核心逻辑单元测试覆盖率 ≥ 60% |
| 部署 | 便携版免安装 + MSIX 可选 |

### 19.3 里程碑

| 里程碑 | 时间点 | 交付物 |
|--------|--------|--------|
| M1 可播放本地音乐 | 第 5 周末 | 本地文件可播 + UI 播放条 |
| M2 可播在线音乐 | 第 8 周末 | 网易云/B站播放全链路 |
| M3 Beta 版 | 第 11 周末 | 核心功能 + UI + 后台集成 |
| M4 RC 版 | 第 14 周末 | 全功能 + 测试通过 |
| M5 v1.0 发布 | 第 16 周末 | 打包 + CI + 文档 |

---

*NeriPlayer → Windows 移植方案 · Process.md · 2026-08-12*

