# NeriPlayer → Windows 移植 · 详细实现过程（Start）

> 本文档是《Process.md》的**逐步骤执行版**：每个阶段给出可复制的命令、完整代码、验证命令与验收标准。
> 依据：《Analysis.md》（源码分析 24 章）+《Process.md》（移植方案 19 章）
> 目标：.NET 8 + Avalonia 11 + LibVLCSharp 3.8 的 Windows 音乐播放器
> 建议按章节顺序执行，每章末尾的「✅ 验收」通过后才进入下一章。

---

## 目录

1. [环境搭建](#一环境搭建第01-03天)
2. [项目脚手架](#二项目脚手架第04-06天)
3. [核心数据模型](#三核心数据模型第07-09天)
4. [本地数据库](#四本地数据库第10-14天)
5. [播放引擎](#五播放引擎第15-22天)
6. [音效系统](#六音效系统第23-27天)
7. [API 客户端](#七api-客户端第28-37天)
8. [下载管理](#八下载管理第38-44天)
9. [数据同步](#九数据同步第45-50天)
10. [UI 主框架](#十ui-主框架第51-62天)
11. [后台与系统集成](#十一后台与系统集成第63-68天)
12. [视觉效果](#十二视觉效果第69-73天)
13. [安全与崩溃恢复](#十三安全与崩溃恢复第74-77天)
14. [测试体系](#十四测试体系贯穿全程)
15. [打包发布](#十五打包与发布第78-84天)

---

## 〇、协作与提交流程（每个章节完成后必须执行）

> 本文档的每个章节（每一步）完成后，**除完成本地验收外，还必须同步更新远端仓库并提交上游 PR**。
> 该流程从「第三章 核心数据模型」完成时开始执行（第一次已执行，见下方记录）。

### 0.1 仓库布局

| 仓库 | 地址 | 用途 |
|------|------|------|
| 本地源码 | `D:\Project\Library\NeriPlayer.Windows` | 主开发目录（git 仓库） |
| 独立远端 | `https://github.com/ALIve114514awa/NeriPlayer-Windows-DotNet` | 个人备份 / 独立展示（origin） |
| 上游 fork | `D:\Project\Library\NeriPlayer-Desktop-fork` | 用于向官方仓库提交 PR |
| 上游仓库 | `https://github.com/cwuom/NeriPlayer-Desktop` | Tauri 官方实现（PR 目标，base=`main`） |
| PR 分支 | `feat/dotnet-windows-impl` | fork 内固定特性分支（反复更新） |
| 子目录 | `dotnet-windows/` | .NET 实现在 fork 内的存放位置（不动根目录） |

### 0.2 每步提交流程（6 个动作）

```powershell
# 1) 本地提交
cd D:\Project\Library\NeriPlayer.Windows
git add -A
git commit -m "feat(第N章): <说明>"

# 2) 推送到个人独立远端（保留完整历史）
git push origin master

# 3) 同步 .NET 代码到 fork 的 dotnet-windows/ 子目录
#    复制除 .git / bin / obj 之外的全部源码
#    简单做法：整个目录覆盖复制后删除嵌套 .git（git add 时 bin/obj 会被 .gitignore 自动排除）
Remove-Item D:\Project\Library\NeriPlayer-Desktop-fork\dotnet-windows -Recurse -Force
Copy-Item D:\Project\Library\NeriPlayer.Windows D:\Project\Library\NeriPlayer-Desktop-fork\dotnet-windows -Recurse -Force
Remove-Item D:\Project\Library\NeriPlayer-Desktop-fork\dotnet-windows\.git -Recurse -Force

# 4) fork 内提交并推送（自动更新既有 PR）
cd D:\Project\Library\NeriPlayer-Desktop-fork
git checkout feat/dotnet-windows-impl
git add dotnet-windows/
git commit -m "feat(dotnet-windows): 第N章 <说明>"
git push origin feat/dotnet-windows-impl

# 5) 确认 PR 仍为 open 且无冲突
#    PR: https://github.com/cwuom/NeriPlayer-Desktop/pull/20
#    （每次 push 后 PR 自动更新，无需重建）

# 6) 记录本次提交信息到 0.3 提交记录表
```

### 0.3 提交流程的触发规则

- ✅ **必须执行**：每个章节的「✅ 验收」通过后，按 0.2 流程执行。
- 🚫 **例外（原作者拒绝时停止）**：如果上游维护者（cwuom）在 PR 中**明确表示拒绝/不接受**此类 .NET 独立实现，则停止向 `cwuom/NeriPlayer-Desktop` 提交新 PR；此时仅保留 0.2 第 1-2 步（本地 + 个人独立远端）。
- 🚧 **未回复默认继续**：只要原作者未明确拒绝，就按每步完成继续提交/更新 PR。

### 0.4 提交记录表（每次执行后追加一行）

| 日期 | 对应章节 | 本地提交 | 独立远端 | fork 分支 | 上游 PR |
|------|----------|----------|----------|-----------|---------|
| 2026-08-17 | 三、核心数据模型（初版） | `097e8b7` | master 已推送 | `feat/dotnet-windows-impl` @ `2e8d15e` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-17 | 三、核心数据模型（CodeRabbit 审核修复） | `22dfae6` | master @ `22dfae6` | `feat/dotnet-windows-impl` @ `c39ced8` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open，已修复） |
| 2026-08-17 | 四、本地数据库 | `7700872` | master @ `7700872` | `feat/dotnet-windows-impl` @ `e4786d1` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-20 | 五、播放引擎 | `6139c28` | master @ `6139c28` | `feat/dotnet-windows-impl` @ `e0595fe` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-20 | 六、音效系统 | `60fa815` | master @ `60fa815` | `feat/dotnet-windows-impl` @ `b9e3314` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-20 | 六、音效系统（CodeRabbit 审核修复） | `41c11f9` | master @ `41c11f9` | `feat/dotnet-windows-impl` @ `60d25e3` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open，已修复） |
| 2026-08-20 | 七、API 客户端 | `7cb2e6a` | master @ `7cb2e6a` | `feat/dotnet-windows-impl` @ `5b27dce` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-27 | 八、下载管理 | `d859a77` | master @ `d859a77` | `feat/dotnet-windows-impl` @ `680513a` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-27 | 八、下载管理（记录提交日志） | `d6c603e` | master @ `d6c603e` | `feat/dotnet-windows-impl` @ `959626f` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-27 | 八、下载管理（更新提交记录表） | `2c6d559` | master @ `2c6d559` | `feat/dotnet-windows-impl` @ `6cffbd5` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-29 | 九、数据同步 | `64c29bd` | master @ `64c29bd` | `feat/dotnet-windows-impl` @ `19e1698` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open，修复 dotnet-windows 为普通文件） |
| 2026-08-29 | 九、数据同步（记录提交日志） | `e8b5619` | master @ `e8b5619` | `feat/dotnet-windows-impl` @ `83c06be` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-29 | 阶段A 在线播放URL保鲜 + 清理 | `8e911e5` | master @ `8e911e5` | `feat/dotnet-windows-impl` @ `16560fd` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-29 | 阶段A（记录提交日志） | `89cfc12` | master @ `89cfc12` | `feat/dotnet-windows-impl` @ `d1a3817` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-29 | 十、UI主框架 | `5303f99` | master @ `5303f99` | `feat/dotnet-windows-impl` @ `a35e0d2` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-29 | 十、UI主框架（记录提交日志） | `4abe829` | master @ `4abe829` | `feat/dotnet-windows-impl` @ `bdc759a` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |
| 2026-08-30 | 十一、后台与系统集成（代码完成，待 Windows 10 SDK 启用 SMTC） | `04d0081` | master @ `04d0081` | `feat/dotnet-windows-impl` @ `a055e99` | [PR #20](https://github.com/cwuom/NeriPlayer-Desktop/pull/20)（open） |


---

## 一、环境搭建（第 01-03 天）

### 1.1 安装清单

| 组件 | 版本 | 下载/命令 |
|------|------|-----------|
| Windows | 10 22H2 / 11 | 现有系统即可 |
| Visual Studio 2022 | 17.8+ Community | 勾选「.NET 桌面开发」工作负载 |
| .NET SDK | 8.0 LTS | `winget install Microsoft.DotNet.SDK.8` |
| Git | 任意新版本 | `winget install Git.Git` |
| LibVLC | 3.0.x x64 | https://get.videolan.org/vlc/ → 解压到 `D:\libs\vlc-3.0.20` |
| Windows Terminal | 最新 | 可选，便于多窗格操作 |

### 1.2 验证安装

```powershell
'dotnet --version'      # 期望 ≥ 8.0.x
dotnet --list-sdks
git --version
# VLC 验证：解压目录下应有 libvlc.dll / libvlccore.dll / plugins/
Test-Path 'D:\libs\vlc-3.0.20\libvlc.dll'
```

### 1.3 安装 Avalonia 模板

```powershell
dotnet new install Avalonia.Templates
dotnet new list | findstr /i avalonia    # 应看到 avalonia.app / avalonia.mvvm 等
```

### 1.4 设置 NuGet 源（可选，国内加速）

创建 `C:\Users\<you>\AppData\Roaming\NuGet\NuGet.Config`：

```xml
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" />
    <add key="aliyun" value="https://mirrors.aliyun.com/nuget/v3/index.json" />
  </packageSources>
</configuration>
```

### 1.5 初始化 Git 仓库

```powershell
cd D:\Project\Library
mkdir NeriPlayer.Windows
cd NeriPlayer.Windows
git init
# 创建 .gitignore（.NET 模板）
dotnet new gitignore
```

### 1.6 运行时配置（供播放引擎探测 VLC 路径）

创建 `%APPDATA%\NeriPlayer\appsettings.json`（运行时读取，见第 5 章）：

```json
{
  "Vlc": { "LibDirectory": "D:\\libs\\vlc-3.0.20" },
  "Storage": {
    "DataRoot": "%APPDATA%\\NeriPlayer",
    "MusicRoot": "D:\\Music\\NeriPlayer"
  }
}
```

**✅ 验收**
- [ ] `dotnet --version` ≥ 8.0
- [ ] `dotnet new list` 中出现 avalonia 模板
- [ ] `D:\libs\vlc-3.0.20\libvlc.dll` 存在
- [ ] Git 仓库已初始化

---
## 二、项目脚手架（第 04-06 天）

### 2.1 创建解决方案与项目

```powershell
cd NeriPlayer.Windows
dotnet new sln -n NeriPlayer.Windows

mkdir src
dotnet new avalonia.app -o src/NeriPlayer.App -n NeriPlayer.App
dotnet new classlib -o src/NeriPlayer.Core -n NeriPlayer.Core
dotnet new classlib -o src/NeriPlayer.Data -n NeriPlayer.Data
dotnet new classlib -o src/NeriPlayer.UI -n NeriPlayer.UI
dotnet new classlib -o src/NeriPlayer.Background -n NeriPlayer.Background

mkdir tests
dotnet new xunit -o tests/NeriPlayer.Core.Tests -n NeriPlayer.Core.Tests
dotnet new xunit -o tests/NeriPlayer.Data.Tests -n NeriPlayer.Data.Tests
dotnet new xunit -o tests/NeriPlayer.Api.Tests -n NeriPlayer.Api.Tests

# 加入解决方案
dotnet sln add src/NeriPlayer.App src/NeriPlayer.Core src/NeriPlayer.Data `
              src/NeriPlayer.UI src/NeriPlayer.Background `
              tests/NeriPlayer.Core.Tests tests/NeriPlayer.Data.Tests tests/NeriPlayer.Api.Tests

# 引用关系（对标 Process.md 3.2 依赖方向）
dotnet add src/NeriPlayer.App reference src/NeriPlayer.Core src/NeriPlayer.Data src/NeriPlayer.UI src/NeriPlayer.Background
dotnet add src/NeriPlayer.UI reference src/NeriPlayer.Core src/NeriPlayer.Data
dotnet add src/NeriPlayer.Background reference src/NeriPlayer.Core src/NeriPlayer.Data
dotnet add tests/NeriPlayer.Core.Tests reference src/NeriPlayer.Core
dotnet add tests/NeriPlayer.Data.Tests reference src/NeriPlayer.Data src/NeriPlayer.Core
dotnet add tests/NeriPlayer.Api.Tests reference src/NeriPlayer.Core
```

### 2.2 中央包管理（Directory.Packages.props）

`Directory.Packages.props`（启用 Central Package Management）：

```xml
<Project>
  <PropertyGroup>
    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>
  </PropertyGroup>
  <ItemGroup>
    <PackageVersion Include="Avalonia" Version="11.0.10" />
    <PackageVersion Include="Avalonia.Desktop" Version="11.0.10" />
    <PackageVersion Include="Avalonia.Themes.Fluent" Version="11.0.10" />
    <PackageVersion Include="Avalonia.ReactiveUI" Version="11.0.10" />
    <PackageVersion Include="Avalonia.Diagnostics" Version="11.0.10" />
    <PackageVersion Include="Avalonia.Headless.XUnit" Version="11.0.10" />
    <PackageVersion Include="LibVLCSharp" Version="3.8.0" />
    <PackageVersion Include="LibVLCSharp.Avalonia" Version="3.8.0" />
    <PackageVersion Include="Microsoft.EntityFrameworkCore.Sqlite" Version="8.0.0" />
    <PackageVersion Include="Microsoft.EntityFrameworkCore.Design" Version="8.0.0" />
    <PackageVersion Include="Microsoft.Extensions.DependencyInjection" Version="8.0.0" />
    <PackageVersion Include="Microsoft.Extensions.Configuration.Json" Version="8.0.0" />
    <PackageVersion Include="Microsoft.Extensions.Hosting" Version="8.0.0" />
    <PackageVersion Include="Microsoft.Extensions.Http" Version="8.0.0" />
    <PackageVersion Include="Serilog" Version="3.1.1" />
    <PackageVersion Include="Serilog.Sinks.File" Version="5.0.0" />
    <PackageVersion Include="Serilog.Sinks.Console" Version="5.0.0" />
    <PackageVersion Include="Serilog.Extensions.Hosting" Version="8.0.0" />
    <PackageVersion Include="System.Reactive" Version="6.0.0" />
    <PackageVersion Include="Refit" Version="7.0.0" />
    <PackageVersion Include="Refit.HttpClientFactory" Version="7.0.0" />
    <PackageVersion Include="NAudio" Version="2.2.1" />
    <PackageVersion Include="SkiaSharp" Version="2.88.8" />
    <PackageVersion Include="TagLibSharp" Version="2.3.0" />
    <PackageVersion Include="Quartz" Version="3.8.1" />
    <PackageVersion Include="Microsoft.International.Converters.PinYinConverter" Version="1.0.0" />
    <PackageVersion Include="WebDav.Client" Version="2.8.1" />
    <PackageVersion Include="Octokit" Version="9.0.0" />
    <PackageVersion Include="coverlet.collector" Version="6.0.0" />
  </ItemGroup>
</Project>
```

`Directory.Build.props`（全局编译属性）：

```xml
<Project>
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <LangVersion>12.0</LangVersion>
  </PropertyGroup>
</Project>
```
> ⚠️ **模板与 CPM 冲突处理（重要修正，执行前必读）**
>
> 1. `dotnet new avalonia.app` / `dotnet new xunit` 生成的 `.csproj` 自带带 `Version` 的 `PackageReference`，启用 `Directory.Packages.props`（Central Package Management）后会触发 **NU1008 错误**。
>    必须在 2.3 之前删除各 `.csproj` 中所有 `PackageReference` 的 `Version` 属性（或直接删除模板自带的引用），再由 2.3 的 `dotnet add package`（不带版本号）按中央版本统一拉取。
> 2. **Avalonia 模板目标框架**：Avalonia.Templates 11.3.0 生成的 App 项目默认面向 `net9.0`，本机仅有 .NET 8 SDK 会报 NETSDK1045，需将 `src/NeriPlayer.App/NeriPlayer.App.csproj` 的 `<TargetFramework>` 改为 `net8.0`。
> 3. **NuGet 源**：若 `mirrors.aliyun.com` 服务索引不可达（NU1301），从 `NuGet.Config` 移除 aliyun 源，仅保留 nuget.org。
> 4. **CPM 下 `dotnet add package`（不带版本号）会解析源上最新版做兼容性校验**：若最新版不兼容 net8.0（如 EF Core 10.x 只支持 net10.0）会直接报 **NU1202** 而失败。稳妥做法是显式带版本：`dotnet add package Microsoft.EntityFrameworkCore.Sqlite --version 8.0.0`。
> 5. **Avalonia.Fonts.Inter 必须保留**：模板 `Program.cs` 调用 `.WithInterFont()`，需在中央包清单补充 `<PackageVersion Include="Avalonia.Fonts.Inter" Version="11.0.10" />`。
> 6. **测试项目额外依赖**：xunit 模板自带 `Microsoft.NET.Test.Sdk` / `xunit` / `xunit.runner.visualstudio`（17.8.0 / 2.5.3 / 2.5.3），需在中央包清单补充对应 `PackageVersion`，否则 CPM 下报 NU1009。
> 7. **WebDav.Client**：nuget.org 上无 2.8.1，实际可用 2.9.0（NU1603 会自动近似匹配），建议中央清单直接写 2.9.0。

### 2.3 安装 NuGet 包

```powershell
# Core
dotnet add src/NeriPlayer.Core package System.Reactive
dotnet add src/NeriPlayer.Core package Microsoft.Extensions.Http
dotnet add src/NeriPlayer.Core package Microsoft.Extensions.Configuration.Json
dotnet add src/NeriPlayer.Core package Serilog
dotnet add src/NeriPlayer.Core package Serilog.Sinks.Console
dotnet add src/NeriPlayer.Core package Serilog.Sinks.File

# Data
dotnet add src/NeriPlayer.Data package Microsoft.EntityFrameworkCore.Sqlite
dotnet add src/NeriPlayer.Data package Microsoft.EntityFrameworkCore.Design
dotnet add src/NeriPlayer.Data package TagLibSharp
dotnet add src/NeriPlayer.Data package Octokit
dotnet add src/NeriPlayer.Data package WebDav.Client
dotnet add src/NeriPlayer.Data package Microsoft.International.Converters.PinYinConverter
dotnet add src/NeriPlayer.Data package Quartz

# App
dotnet add src/NeriPlayer.App package Avalonia.Desktop
dotnet add src/NeriPlayer.App package Avalonia.Themes.Fluent
dotnet add src/NeriPlayer.App package Avalonia.ReactiveUI
dotnet add src/NeriPlayer.App package Serilog
dotnet add src/NeriPlayer.App package Serilog.Sinks.File
dotnet add src/NeriPlayer.App package Serilog.Sinks.Console
dotnet add src/NeriPlayer.App package Microsoft.Extensions.Hosting

# UI
dotnet add src/NeriPlayer.UI package Avalonia
dotnet add src/NeriPlayer.UI package SkiaSharp
dotnet add src/NeriPlayer.UI package LibVLCSharp.Avalonia

# Background
dotnet add src/NeriPlayer.Background package NAudio
dotnet add src/NeriPlayer.Background package Quartz

# Tests
dotnet add tests/NeriPlayer.Api.Tests package Refit
dotnet add tests/NeriPlayer.Core.Tests package coverlet.collector
```

### 2.4 Serilog 日志（Core/Logging/AppLogger.cs）

```csharp
using Serilog;

namespace NeriPlayer.Core.Logging;

public static class AppLogger
{
    public static readonly Serilog.Core.Logger Instance = new LoggerConfiguration()
        .MinimumLevel.Information()
        .WriteTo.Console(outputTemplate:
            "[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}")
        .WriteTo.File(
            path: Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "NeriPlayer", "logs", "app-.log"),
            rollingInterval: RollingInterval.Day,
            retainedFileCountLimit: 14,
            outputTemplate: "[{Timestamp:yyyy-MM-dd HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}")
        .CreateLogger();

    public static void Flush() => Instance.Dispose();
}
```

### 2.5 DI 容器装配（App/AppStartup.cs）

对标 `AppContainer.initialize()` + `AppStartupPlanner.plan()`：

```csharp
using Microsoft.Extensions.DependencyInjection;

namespace NeriPlayer.App;

public static class AppStartup
{
    public static ServiceProvider BuildServices()
    {
        var services = new ServiceCollection();

        // 数据层
        services.AddDbContext<Data.Database.NeriDbContext>();

        // 核心层
        services.AddSingleton<Core.Player.PlayerManager>();
        services.AddSingleton<Core.Api.Common.HttpClientFactory>();
        services.AddSingleton<Core.Api.Netease.NeteaseClient>();
        services.AddSingleton<Core.Api.Bili.BiliClient>();
        services.AddSingleton<Core.Api.YouTube.YouTubeMusicClient>();

        // 后台
        services.AddHostedService<Background.Services.SyncScheduledService>();

        return services.BuildServiceProvider();
    }
}
```

> 说明（修正）：本文件引用的 `NeriDbContext` / `PlayerManager` / `HttpClientFactory` / `NeteaseClient` / `BiliClient` / `YouTubeMusicClient` / `SyncScheduledService` 在第三/五/七/九章才实现；骨架阶段必须**注释掉**这些注册行（保留 `ServiceCollection` 骨架），保证可编译。

### 2.6 首次编译验证

```powershell
dotnet build NeriPlayer.Windows.sln
```

**✅ 验收**
- [x] `dotnet build` 无错误（0 错误 / 10 警告：NU1701 PinYinConverter 兼容性提示、NU1603 WebDav 近似匹配）
- [x] 解决方案含 8 个项目，依赖方向符合 Process.md 3.2
- [x] `dotnet run --project src/NeriPlayer.App` 弹出空白 Avalonia 窗口（实测：进程启动后存活 5s 无崩溃，PID=12532）

---
## 三、核心数据模型（第 07-09 天）

### 3.1 SongItem（Core/Player/Model/SongItem.cs）

对标 Analysis.md 21.1 节 + Process.md 5.1 节：

```csharp
namespace NeriPlayer.Core.Player.Model;

public enum PlaybackSource { Local, Netease, Bilibili, YouTubeMusic }

public sealed record SongItem
{
    public long Id { get; init; }
    public required string Name { get; init; }
    public required string Artist { get; init; }
    public required string Album { get; init; }
    public long AlbumId { get; init; }
    public long DurationMs { get; init; }
    public string? CoverUrl { get; init; }
    public string? MediaUri { get; init; }
    public string? StreamUrl { get; init; }

    public string? ChannelId { get; init; }   // local | netease | bilibili | youtube_music
    public string? AudioId { get; init; }
    public string? SubAudioId { get; init; }

    public string? MatchedLyric { get; init; }
    public string? MatchedTranslatedLyric { get; init; }
    public PlaybackSource? MatchedLyricSource { get; init; }
    public long UserLyricOffsetMs { get; init; }

    public string? CustomName { get; init; }
    public string? CustomArtist { get; init; }
    public string? CustomCoverUrl { get; init; }
    public string? OriginalName { get; init; }
    public string? OriginalArtist { get; init; }

    public string? LocalFileName { get; init; }
    public string? LocalFilePath { get; init; }

    public long AddedAt { get; init; }

    public string DisplayName => CustomName ?? OriginalName ?? Name;
    public string DisplayArtist => CustomArtist ?? OriginalArtist ?? Artist;

    public bool IsLocalSong() =>
        ChannelId == "local" || (!string.IsNullOrEmpty(LocalFilePath) && ChannelId is null);
}
```

### 3.2 SongIdentity.StableKey（Core/Player/Model/SongIdentity.cs）

```csharp
using System.Text.RegularExpressions;

namespace NeriPlayer.Core.Player.Model;

public static partial class SongIdentity
{
    /// <summary>生成跨版本稳定的歌曲标识：去重、同步、持久化主键（对标 SongIdentity.kt）</summary>
    public static string StableKey(this SongItem song)
    {
        if (song.IsLocalSong())
            return $"local|{NormalizePath(song.LocalFilePath ?? song.MediaUri ?? "")}";

        return song.ChannelId switch
        {
            "netease" => $"netease|{song.AudioId ?? song.Id.ToString()}",
            "bilibili" => $"bilibili|{song.AudioId}|{song.SubAudioId}",
            "youtube_music" => $"ytm|{ExtractYouTubeVideoId(song.MediaUri)}",
            _ => $"id|{song.Id}|{song.Album}|{song.MediaUri}"
        };
    }

    private static string NormalizePath(string p) =>
        p.Replace('\\', '/').TrimEnd('/').ToLowerInvariant();

    /// <summary>从 YouTube 链接/播放列表 URI 提取视频 ID</summary>
    public static string ExtractYouTubeVideoId(string? uri)
    {
        if (string.IsNullOrEmpty(uri)) return "";
        var m = YoutubeVideoIdRegex().Match(uri);
        return m.Success ? m.Groups[1].Value : "";
    }

    [GeneratedRegex(@"(?:v=|youtu\.be/|/shorts/)([A-Za-z0-9_-]{11})")]
    private static partial Regex YoutubeVideoIdRegex();
}
```
### 3.3 单元测试（tests/NeriPlayer.Core.Tests/SongIdentityTests.cs）

```csharp
using NeriPlayer.Core.Player.Model;
using Xunit;

namespace NeriPlayer.Core.Tests;

public class SongIdentityTests
{
    [Fact]
    public void LocalSong_StableKey_IsNormalizedPath()
    {
        var song = new SongItem
        {
            Id = 1, Name = "A", Artist = "B", Album = "C",
            ChannelId = "local", LocalFilePath = @"D:\Music\a\b\c.flac"
        };
        Assert.Equal("local|d:/music/a/b/c.flac", song.StableKey());
    }

    [Theory]
    [InlineData("https://www.youtube.com/watch?v=abcDEF12345")]
    [InlineData("https://youtu.be/abcDEF12345?si=xxx")]
    public void YouTube_ExtractVideoId_Works(string uri)
    {
        var song = new SongItem
        {
            Id = 2, Name = "A", Artist = "B", Album = "C",
            ChannelId = "youtube_music", MediaUri = uri
        };
        Assert.Equal("abcDEF12345", SongIdentity.ExtractYouTubeVideoId(uri));
        Assert.Equal("ytm|abcDEF12345", song.StableKey());
    }

    [Fact]
    public void Netease_StableKey_UsesAudioId()
    {
        var song = new SongItem
        {
            Id = 9, Name = "A", Artist = "B", Album = "C",
            ChannelId = "netease", AudioId = "3456789"
        };
        Assert.Equal("netease|3456789", song.StableKey());
    }
}
```

```powershell
dotnet test tests/NeriPlayer.Core.Tests
```

**✅ 验收**
- [ ] 4 个测试全部通过（本地路径规范化 / 2 个 YouTube 用例 / 网易云 StableKey）
- [ ] `SongItem` 可被其他项目引用编译

---
## 四、本地数据库（第 10-14 天）

### 4.1 实体定义（Data/Entities/）

对标 Analysis.md 21.2 节 Room 15 张表 → EF Core 实体。核心实体：

```csharp
// Data/Entities/SongEntity.cs
namespace NeriPlayer.Data.Entities;

public sealed class SongEntity
{
    public long Id { get; set; }
    public required string StableKey { get; set; }
    public required string Name { get; set; }
    public required string Artist { get; set; }
    public required string Album { get; set; }
    public long AlbumId { get; set; }
    public long DurationMs { get; set; }
    public string? CoverUrl { get; set; }
    public string? MediaUri { get; set; }
    public string? StreamUrl { get; set; }
    public string? ChannelId { get; set; }
    public string? AudioId { get; set; }
    public string? SubAudioId { get; set; }
    public string? MatchedLyric { get; set; }
    public string? MatchedTranslatedLyric { get; set; }
    public string? MatchedLyricSource { get; set; }
    public long UserLyricOffsetMs { get; set; }
    public string? CustomName { get; set; }
    public string? CustomArtist { get; set; }
    public string? CustomCoverUrl { get; set; }
    public string? LocalFileName { get; set; }
    public string? LocalFilePath { get; set; }
    public long AddedAt { get; set; }
}
```

```csharp
// Data/Entities/PlaylistEntity.cs
namespace NeriPlayer.Data.Entities;

public sealed class PlaylistEntity
{
    public long Id { get; set; }
    public required string Name { get; set; }
    public string Kind { get; set; } = "local";   // local | favorite | system
    public string? RemotePlatform { get; set; }
    public string? RemoteId { get; set; }
    public long CreatedAt { get; set; }
    public long UpdatedAt { get; set; }
    public List<PlaylistMemberEntity> Members { get; set; } = [];
}

public sealed class PlaylistMemberEntity
{
    public long PlaylistId { get; set; }
    public long SongId { get; set; }
    public int Position { get; set; }
    public PlaylistEntity? Playlist { get; set; }
    public SongEntity? Song { get; set; }
}
```

```csharp
// Data/Entities/PlaybackStatsEntity.cs
namespace NeriPlayer.Data.Entities;

public sealed class PlaybackStatsEntity
{
    public long SongId { get; set; }
    public long PlayCount { get; set; }
    public long TotalPlayMs { get; set; }
    public long LastPlayedAt { get; set; }
}

/// <summary>每日统计分片桶（对标 PlaybackStatDailyCounterShardEntity，缓解写放大）</summary>
public sealed class StatBucketEntity
{
    public long SongId { get; set; }
    public long DayKey { get; set; }        // yyyyMMdd
    public long PlayCount { get; set; }
    public long ListenMs { get; set; }
}
```
### 4.2 DbContext（Data/Database/NeriDbContext.cs）

```csharp
using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Entities;

namespace NeriPlayer.Data.Database;

public sealed class NeriDbContext(DbContextOptions<NeriDbContext> options) : DbContext(options)
{
    public DbSet<SongEntity> Songs => Set<SongEntity>();
    public DbSet<PlaylistEntity> Playlists => Set<PlaylistEntity>();
    public DbSet<PlaylistMemberEntity> PlaylistMembers => Set<PlaylistMemberEntity>();
    public DbSet<PlaybackStatsEntity> PlaybackStats => Set<PlaybackStatsEntity>();
    public DbSet<StatBucketEntity> StatBuckets => Set<StatBucketEntity>();

    // 后续章节补充：PlayHistory / PlaybackQueue / QueueState / Downloads /
    // DownloadSnapshots / SyncMetadata / SyncOutbox / SyncCheckpoints /
    // TrafficStats / CoverUrlMapping / Settings / CookieCredentials

    protected override void OnModelCreating(ModelBuilder b)
    {
        b.Entity<SongEntity>(e =>
        {
            e.ToTable("songs");
            e.HasIndex(x => x.StableKey).IsUnique();
        });

        b.Entity<PlaylistEntity>(e => e.ToTable("playlists"));

        b.Entity<PlaylistMemberEntity>(e =>
        {
            e.ToTable("playlist_members");
            e.HasKey(x => new { x.PlaylistId, x.Position });
            e.HasOne(x => x.Playlist).WithMany(p => p.Members)
                .HasForeignKey(x => x.PlaylistId).OnDelete(DeleteBehavior.Cascade);
            e.HasOne(x => x.Song).WithMany()
                .HasForeignKey(x => x.SongId).OnDelete(DeleteBehavior.Cascade);
        });

        b.Entity<PlaybackStatsEntity>(e =>
        {
            e.ToTable("playback_stats");
            e.HasKey(x => x.SongId);
        });

        b.Entity<StatBucketEntity>(e =>
        {
            e.ToTable("stat_buckets");
            e.HasKey(x => new { x.SongId, x.DayKey });
        });
    }
}
```

数据库连接工厂（Data/Database/DbContextFactory.cs）：

```csharp
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Design;

namespace NeriPlayer.Data.Database;

/// <summary>供 dotnet ef migrations 使用（设计时工厂）</summary>
public sealed class NeriDbContextFactory : IDesignTimeDbContextFactory<NeriDbContext>
{
    public NeriDbContext CreateDbContext(string[] args)
    {
        var dbPath = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "NeriPlayer", "neriplayer.db");
        Directory.CreateDirectory(Path.GetDirectoryName(dbPath)!);

        var options = new DbContextOptionsBuilder<NeriDbContext>()
            .UseSqlite($"Data Source={dbPath}")
            .Options;
        return new NeriDbContext(options);
    }
}
```

### 4.3 迁移

```powershell
# 安装 EF 工具（若未安装）
dotnet tool install --global dotnet-ef
cd src/NeriPlayer.Data
dotnet ef migrations add InitialCreate
dotnet ef database update
```

> 之后每个 Schema 变更：`dotnet ef migrations add <Name>` 生成新版本；
> 破坏性变更采用「新建表 + 复制数据 + 删旧表」三段式，对标 Room 迁移策略（Process.md 5.4）。

### 4.4 仓储基类（Data/Repositories/RepositoryBase.cs）

```csharp
using Microsoft.EntityFrameworkCore;

namespace NeriPlayer.Data.Repositories;

public abstract class RepositoryBase<TDbContext>(TDbContext db) where TDbContext : DbContext
{
    protected TDbContext Db { get; } = db;

    protected async Task<T> RunAsync<T>(Func<TDbContext, Task<T>> action)
    {
        await using var t = await Db.Database.BeginTransactionAsync();
        var r = await action(Db);
        await t.CommitAsync();
        return r;
    }
}
```

### 4.5 SongRepository 示例

```csharp
using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;

namespace NeriPlayer.Data.Repositories;

public sealed class SongRepository(NeriDbContext db)
{
    public async Task<SongEntity?> GetByStableKeyAsync(string stableKey) =>
        await db.Songs.FirstOrDefaultAsync(s => s.StableKey == stableKey);

    public async Task<long> UpsertAsync(SongEntity song)
    {
        var existing = await GetByStableKeyAsync(song.StableKey);
        if (existing is not null)
        {
            db.Entry(existing).CurrentValues.SetValues(song);
            await db.SaveChangesAsync();
            return existing.Id;
        }
        db.Songs.Add(song);
        await db.SaveChangesAsync();
        return song.Id;
    }
}
```

**✅ 验收**
- [ ] `dotnet ef migrations list` 输出 `InitialCreate`
- [ ] `neriplayer.db` 在 `%APPDATA%\NeriPlayer\` 生成，含 `songs` 等表
- [ ] `SongRepository.UpsertAsync` 可重复调用且不产生重复行（stable_key 唯一）

---
## 五、播放引擎（第 15-22 天）

### 5.1 引擎接口（Core/Player/Engine/IPlaybackEngine.cs）

对标 Process.md 4.3 节：

```csharp
using System.Reactive.Subjects;

namespace NeriPlayer.Core.Player.Engine;

public enum PlaybackEngineEventKind { Loaded, Playing, Paused, Stopped, Ended, Error, Buffering }

public sealed record PlaybackEngineEvent(PlaybackEngineEventKind Kind, string? Message = null);

public sealed record PlaybackEngineOptions
{
    public float Volume { get; init; } = 1.0f;
    public float Rate { get; init; } = 1.0f;
    public bool FadeOnPlay { get; init; } = true;
}

public interface IPlaybackEngine : IDisposable
{
    Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options);
    Task PlayAsync();
    Task PauseAsync();
    Task SeekAsync(TimeSpan position);
    Task SetVolumeAsync(float volume);
    Task SetRateAsync(float speed);
    IObservable<PlaybackEngineEvent> Events { get; }

    void ApplyEqualizer(IReadOnlyList<float> gains);   // 10-band
    void ApplyStereoBalance(float balance);            // -1.0 ~ 1.0
    void ApplyVolumeNormalization(float gainDb);
    void ApplyPitch(float semitones);

    TimeSpan Duration { get; }
    IObservable<TimeSpan> Position { get; }
    IObservable<float[]> FftData { get; }
}
```

### 5.2 VLC 引擎（Core/Player/Engine/VlcPlaybackEngine.cs）

```csharp
using System.Reactive.Subjects;
using LibVLCSharp.Shared;

namespace NeriPlayer.Core.Player.Engine;

public sealed class VlcPlaybackEngine : IPlaybackEngine
{
    private static readonly LibVLC? _libVlc;
    private readonly MediaPlayer _player;
    private readonly Subject<PlaybackEngineEvent> _events = new();
    private readonly Subject<TimeSpan> _position = new();
    private readonly Subject<float[]> _fft = new();

    static VlcPlaybackEngine()
    {
        // 路径来自配置（见 1.6 appsettings.json）或环境变量
        var vlcDir = Environment.GetEnvironmentVariable("NERIPLAYER_VLC_DIR")
                     ?? @"D:\libs\vlc-3.0.20";
        Core.Initialize(vlcDir);
        _libVlc = new LibVLC("--no-video", "--no-video-title-show");
    }

    public VlcPlaybackEngine()
    {
        _player = new MediaPlayer(_libVlc!);
        _player.TimeChanged += (_, e) => _position.OnNext(TimeSpan.FromMilliseconds(e.Time));
        _player.EndReached += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Ended));
        _player.Playing += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Playing));
        _player.Paused += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Paused));
        _player.Stopped += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Stopped));
        _player.EncounteredError += (_, _) =>
            _events.OnNext(new(PlaybackEngineEventKind.Error, "VLC EncounteredError"));
        _player.Buffering += (_, e) =>
            _events.OnNext(new(PlaybackEngineEventKind.Buffering, e.Cache.ToString()));
    }

    public IObservable<PlaybackEngineEvent> Events => _events;
    public IObservable<TimeSpan> Position => _position.AsObservable();
    public IObservable<float[]> FftData => _fft.AsObservable();
    public TimeSpan Duration =>
        _player.Length > 0 ? TimeSpan.FromMilliseconds(_player.Length) : TimeSpan.Zero;

    public Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options)
    {
        using var media = new Media(_libVlc!, mediaUri);
        if (!_player.Play(media)) return Task.FromException(new EngineException("VLC Play failed"));
        _player.Volume = (int)(options.Volume * 100);
        return Task.CompletedTask;
    }

    public Task PlayAsync() { _player.Play(); return Task.CompletedTask; }
    public Task PauseAsync() { _player.Pause(); return Task.CompletedTask; }
    public Task SeekAsync(TimeSpan position) { _player.Time = (long)position.TotalMilliseconds; return Task.CompletedTask; }
    public Task SetVolumeAsync(float volume) { _player.Volume = Math.Clamp((int)(volume * 100), 0, 200); return Task.CompletedTask; }
    public Task SetRateAsync(float speed) { _player.SetRate(speed); return Task.CompletedTask; }

    public void ApplyEqualizer(IReadOnlyList<float> gains)
    {
        var eq = new Equalizer();
        var bands = new[] { 31.25f, 62.5f, 125f, 250f, 500f, 1_000f, 2_000f, 4_000f, 8_000f, 16_000f };
        for (var i = 0; i < Math.Min(gains.Count, 10); i++)
            eq.SetAmp(Math.Clamp(gains[i], -20, 20) * 100f, bands[i]);
        _player.Equalizer = eq;
    }

    public void ApplyStereoBalance(float balance) { /* 简化：VLC 通道控制 */ }
    public void ApplyVolumeNormalization(float gainDb) { /* 见第 6 章 */ }
    public void ApplyPitch(float semitones) { /* SoundTouch 见第 6 章 */ }

    public void Dispose()
    {
        _player.Stop();
        _player.Dispose();
        _events.Dispose();
        _position.Dispose();
        _fft.Dispose();
    }
}
```

> `EngineException` 定义：`public sealed class EngineException(string message) : Exception(message);`
### 5.3 PlayerManager 总控（Core/Player/PlayerManager.cs）

对标 Analysis.md 第 3 章 + Process.md 4.1。常量全部对标 Analysis.md 24.1 节：

```csharp
using System.Reactive.Subjects;
using NeriPlayer.Core.Player.Engine;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Player;

public enum PlaybackState { Idle, Loading, Playing, Paused, Stopped, Error }
public enum RepeatMode { Off, All, One }
public enum PlaybackCommandSource { Local, Smtc, Shortcut, Auto }

public sealed class PlayerManager : IDisposable
{
    // 常量对标 Analysis.md 24.1 播放核心
    private static readonly TimeSpan MediaUrlStale = TimeSpan.FromMinutes(10);   // MEDIA_URL_STALE_MS
    private static readonly TimeSpan UrlRefreshCooldown = TimeSpan.FromSeconds(10);
    private const int MaxConsecutiveFailures = 10;
    private static readonly TimeSpan StatePersistInterval = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan DefaultFadeDuration = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan ProgressThrottle = TimeSpan.FromMilliseconds(80);
    private const long MinListenMsForPlayCount = 30_000;   // 听满 30s 计 1 次

    private readonly IPlaybackEngine _engine;
    private readonly Subject<PlaybackState> _state = new();
    private readonly Subject<SongItem?> _currentSong = new();
    private readonly Subject<TimeSpan> _position = new();

    private List<SongItem> _queue = [];
    private int _index;
    private int _consecutiveFailures;
    private RepeatMode _repeatMode;
    private bool _shuffle;
    private DateTimeOffset _lastUrlRefreshAt;

    public IObservable<PlaybackState> State => _state;
    public IObservable<SongItem?> CurrentSong => _currentSong;
    public IObservable<TimeSpan> Position => _position;
    public IReadOnlyList<SongItem> Queue => _queue;
    public int CurrentIndex => _index;

    public PlayerManager(IPlaybackEngine engine) => _engine = engine;
    public PlayerManager() : this(new VlcPlaybackEngine()) { }

    /// <summary>播放歌单（对标 playPlaylistImpl：设置队列 → 播放 → 持久化）</summary>
    public async Task PlayAsync(IReadOnlyList<SongItem> playlist, int startIndex,
        PlaybackCommandSource source = PlaybackCommandSource.Local)
    {
        if (playlist.Count == 0) return;
        _queue = playlist.ToList();
        _index = Math.Clamp(startIndex, 0, _queue.Count - 1);
        _state.OnNext(PlaybackState.Loading);
        await PlayAtIndexAsync();
    }

    private async Task PlayAtIndexAsync()
    {
        var song = _queue[_index];
        var url = await ResolveFreshUrlAsync(song);   // URL 保鲜逻辑
        try
        {
            await _engine.LoadAsync(new Uri(url), new PlaybackEngineOptions { Volume = 1.0f });
            await _engine.PlayAsync();
            _consecutiveFailures = 0;
            _currentSong.OnNext(song);
            _state.OnNext(PlaybackState.Playing);
        }
        catch (Exception)
        {
            _consecutiveFailures++;
            if (_consecutiveFailures >= MaxConsecutiveFailures)
            {
                _state.OnNext(PlaybackState.Error);
                await _engine.PauseAsync();
                return;
            }
            await NextAsync(auto: true);   // 自动切下一首
        }
    }

    private async Task<string> ResolveFreshUrlAsync(SongItem song)
    {
        if (string.IsNullOrEmpty(song.StreamUrl)) return song.StreamUrl!;
        var age = DateTimeOffset.UtcNow - _lastUrlRefreshAt;
        // URL 超过 10min 或标记为可刷新时重新解析（实际实现接入第 7 章 API 客户端）
        if (age >= MediaUrlStale) _lastUrlRefreshAt = DateTimeOffset.UtcNow;
        return song.StreamUrl;
    }

    public async Task PauseAsync() { await _engine.PauseAsync(); _state.OnNext(PlaybackState.Paused); }
    public async Task ResumeAsync() { await _engine.PlayAsync(); _state.OnNext(PlaybackState.Playing); }

    public async Task NextAsync(bool auto = false)
    {
        if (_index >= _queue.Count - 1)
        {
            if (_repeatMode == RepeatMode.All) _index = 0;
            else { _state.OnNext(PlaybackState.Stopped); return; }
        }
        else _index++;
        await PlayAtIndexAsync();
    }

    public async Task PreviousAsync()
    {
        _index = _index > 0 ? _index - 1 : _queue.Count - 1;
        await PlayAtIndexAsync();
    }

    public async Task SeekAsync(TimeSpan position) => await _engine.SeekAsync(position);
    public async Task SetVolumeAsync(float volume) => await _engine.SetVolumeAsync(volume);
    public void SetRepeatMode(RepeatMode mode) => _repeatMode = mode;
    public void ToggleShuffle() => _shuffle = !_shuffle;

    public void Dispose() { _engine.Dispose(); _state.Dispose(); _currentSong.Dispose(); _position.Dispose(); }
}
```
### 5.4 策略类（Core/Player/Policy/）

对标 Analysis.md 3.2 + Process.md 4.5（23 个策略包 → 6 个类）。示例：

```csharp
// PlaybackFailurePolicy.cs —— 连续失败计数与停止阈值
namespace NeriPlayer.Core.Player.Policy;

public sealed class PlaybackFailurePolicy
{
    private const int MaxConsecutiveFailures = 10;
    private int _count;

    public bool RecordFailure(out bool shouldStop)
    {
        _count++;
        shouldStop = _count >= MaxConsecutiveFailures;
        return shouldStop;
    }

    public void RecordSuccess() => _count = 0;
}

// TrackEndDedupPolicy.cs —— 500ms 相邻结束事件去重（Analysis.md 24.1）
public sealed class TrackEndDedupPolicy
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

// MediaUrlRefreshPolicy.cs —— URL 10min 过期 + 10s 冷却防抖
public sealed class MediaUrlRefreshPolicy
{
    private static readonly TimeSpan Stale = TimeSpan.FromMinutes(10);
    private static readonly TimeSpan Cooldown = TimeSpan.FromSeconds(10);
    private DateTimeOffset _lastRefreshAt = DateTimeOffset.MinValue;

    public bool ShouldRefresh(DateTimeOffset? urlCreatedAt)
    {
        if (urlCreatedAt is not null && DateTimeOffset.UtcNow - urlCreatedAt < Stale) return false;
        if (DateTimeOffset.UtcNow - _lastRefreshAt < Cooldown) return false;
        _lastRefreshAt = DateTimeOffset.UtcNow;
        return true;
    }
}
```

### 5.5 手动播放验证（临时控制台）

在 `tests/NeriPlayer.Core.Tests` 中临时创建（仅手动执行，不进 CI）：

```csharp
public static class ManualPlay
{
    // 用法：ManualPlay.RunAsync(new Uri(@"D:\Music\demo.flac"), TimeSpan.FromSeconds(10))
    public static async Task RunAsync(Uri uri, TimeSpan seconds)
    {
        using var engine = new VlcPlaybackEngine();
        await engine.LoadAsync(uri, new PlaybackEngineOptions());
        await engine.PlayAsync();
        await Task.Delay(seconds);
        await engine.PauseAsync();
    }
}
```

**✅ 验收**
- [ ] VLC 引擎可播放本地 `mp3/flac/ogg` 文件并出声
- [ ] `PlayerManager.PlayAsync` → 状态流依次为 `Loading → Playing`
- [ ] 连续失败 10 次后进入 `Error` 并自动停止
- [ ] 同一文件 500ms 内的 `Ended` 事件被 `TrackEndDedupPolicy` 过滤

---
## 六、音效系统（第 23-27 天）

### 6.1 Biquad 滤波器（Core/Player/Effects/BiquadFilter.cs）

对标 Process.md 7.2（Direct Form I Biquad / RBJ 系数）：

```csharp
namespace NeriPlayer.Core.Player.Effects;

public sealed class BiquadFilter
{
    public enum FilterType { Peaking, LowShelf, HighShelf }

    public double B0, B1, B2, A1, A2;   // 系数
    private double _x1, _x2, _y1, _y2;

    public void Configure(FilterType type, double freqHz, double gainDb,
        double sampleRate, double q = 0.707)
    {
        var a = Math.Pow(10, gainDb / 40.0);
        var w0 = 2 * Math.PI * freqHz / sampleRate;
        var cos = Math.Cos(w0);
        var sin = Math.Sin(w0);
        var alpha = sin / (2 * q);

        double b0, b1, b2, a1, a2;
        switch (type)
        {
            case FilterType.Peaking:
                b0 = 1 + alpha * a;  b1 = -2 * cos;  b2 = 1 - alpha * a;
                a1 = -2 * cos;       a2 = 1 + alpha / a;
                break;
            case FilterType.LowShelf:
                var sq = 2 * Math.Sqrt(a) * alpha;
                b0 = a * ((a + 1) - (a - 1) * cos + sq);
                b1 = 2 * a * ((a - 1) - (a + 1) * cos);
                b2 = a * ((a + 1) - (a - 1) * cos - sq);
                a1 = -2 * ((a - 1) + (a + 1) * cos);
                a2 = (a + 1) - (a - 1) * cos - sq;
                break;
            case FilterType.HighShelf:
                var sqh = 2 * Math.Sqrt(a) * alpha;
                b0 = a * ((a + 1) + (a - 1) * cos + sqh);
                b1 = -2 * a * ((a - 1) + (a + 1) * cos);
                b2 = a * ((a + 1) + (a - 1) * cos - sqh);
                a1 = 2 * ((a - 1) - (a + 1) * cos);
                a2 = (a + 1) - (a - 1) * cos - sqh;
                break;
            default: return;
        }

        var a0 = 1.0 + a1 + a2;   // 标准 RBJ：a0 归一化
        B0 = b0 / a0; B1 = b1 / a0; B2 = b2 / a0;
        A1 = a1 / a0; A2 = a2 / a0;
    }

    public float Process(float input)
    {
        var y = B0 * input + B1 * _x1 + B2 * _x2 - A1 * _y1 - A2 * _y2;
        _x2 = _x1; _x1 = input;
        _y2 = _y1; _y1 = y;
        return (float)y;
    }

    public void Reset() { _x1 = _x2 = _y1 = _y2 = 0; }
}
```

> 注：以上为 RBJ Audio EQ Cookbook 标准实现；直通验证（全 0 dB）见 6.5 测试。

### 6.2 均衡器（EqualizerEffect.cs）

```csharp
namespace NeriPlayer.Core.Player.Effects;

public sealed class EqualizerEffect
{
    public static readonly double[] BandsHz =
        { 31.25, 62.5, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000 };

    public static readonly IReadOnlyDictionary<string, double[]> Presets =
        new Dictionary<string, double[]>
        {
            ["默认"] = [0,0,0,0,0,0,0,0,0,0],
            ["流行"] = [-1,0,1,2,3,2,0,1,2,1],
            ["摇滚"] = [3,2,0,-1,1,2,3,2,1,0],
            ["爵士"] = [2,1,0,1,2,2,0,0,1,2],
            ["古典"] = [2,1,0,0,-1,-1,0,1,2,3],
            ["电子"] = [2,2,1,0,-1,0,1,2,3,3],
            ["人声"] = [-2,-1,0,1,2,3,3,2,1,-1],
        };

    private readonly BiquadFilter[] _filters;

    public EqualizerEffect(double sampleRate = 44100)
    {
        _filters = BandsHz.Select(_ => new BiquadFilter()).ToArray();
        SampleRate = sampleRate;
    }

    public double SampleRate { get; }

    public void ApplyGains(IReadOnlyList<double> gainsDb)
    {
        for (var i = 0; i < _filters.Length; i++)
        {
            var type = i == 0 ? BiquadFilter.FilterType.LowShelf
                      : i == _filters.Length - 1 ? BiquadFilter.FilterType.HighShelf
                      : BiquadFilter.FilterType.Peaking;
            _filters[i].Configure(type, BandsHz[i],
                Math.Clamp(gainsDb[i], -20, 20), SampleRate);
        }
    }

    public float Process(float sample)
    {
        foreach (var f in _filters) sample = f.Process(sample);
        return sample;
    }
}
```
### 6.3 立体声平衡（StereoBalanceEffect.cs）

```csharp
namespace NeriPlayer.Core.Player.Effects;

/// <summary>对标 StereoBalanceAudioProcessor.kt：(L+R) 混音权重</summary>
public sealed class StereoBalanceEffect
{
    private float _balance;   // -1.0 全左 ~ 0 平衡 ~ +1.0 全右

    public void SetBalance(float balance) => _balance = Math.Clamp(balance, -1f, 1f);

    /// <summary>输入交错立体声 buffer，原地处理</summary>
    public void Process(Span<float> interleaved)
    {
        if (_balance == 0) return;
        var lw = 1f - Math.Max(0, _balance);          // 左权重
        var rw = 1f + Math.Min(0, _balance);          // 右权重
        for (var i = 0; i < interleaved.Length; i += 2)
        {
            var l = interleaved[i];
            var r = interleaved[i + 1];
            interleaved[i] = l * lw + r * (1 - lw);
            interleaved[i + 1] = r * rw + l * (1 - rw);
        }
    }
}
```

### 6.4 FFT 频谱（FftAnalyzer.cs）

```csharp
namespace NeriPlayer.Core.Player.Effects;

/// <summary>Cooley-Tukey 基 2 FFT + Hann 窗，输出 64 频带对数刻度</summary>
public sealed class FftAnalyzer
{
    private readonly int _size;
    private readonly float[] _window;

    public FftAnalyzer(int size = 1024)
    {
        _size = size;
        _window = Enumerable.Range(0, size)
            .Select(i => 0.5f * (1 - MathF.Cos(2 * MathF.PI * i / (size - 1))))  // Hann
            .ToArray();
    }

    public float[] Compute(Span<float> samples)
    {
        var re = new float[_size];
        var im = new float[_size];
        for (var i = 0; i < Math.Min(samples.Length, _size); i++)
        {
            re[i] = samples[i] * _window[i];
            im[i] = 0;
        }
        Fft(re, im);

        const int bands = 64;
        var result = new float[bands];
        var nyquist = 20_000f;
        var logMin = MathF.Log10(20);
        var logMax = MathF.Log10(nyquist);

        for (var b = 0; b < bands; b++)
        {
            var f0 = MathF.Pow(10, logMin + (logMax - logMin) * b / bands);
            var f1 = MathF.Pow(10, logMin + (logMax - logMin) * (b + 1) / bands);
            var i0 = Math.Clamp((int)(f0 / nyquist * (_size / 2)), 0, _size / 2 - 1);
            var i1 = Math.Clamp((int)(f1 / nyquist * (_size / 2)), i0 + 1, _size / 2);
            var sum = 0f;
            for (var i = i0; i < i1; i++)
                sum += MathF.Sqrt(re[i] * re[i] + im[i] * im[i]);
            result[b] = sum / MathF.Max(1, i1 - i0);
        }
        return result;
    }

    private static void Fft(Span<float> re, Span<float> im)
    {
        var n = re.Length;
        for (var i = 1, j = 0; i < n; i++)
        {
            var bit = n >> 1;
            for (; (j & bit) != 0; bit >>= 1) j ^= bit;
            j ^= bit;
            if (i < j) (re[i], re[j]) = (re[j], re[i]);
        }
        for (var len = 2; len <= n; len <<= 1)
        {
            var ang = -2 * MathF.PI / len;
            var wRe = MathF.Cos(ang);
            var wIm = MathF.Sin(ang);
            for (var i = 0; i < n; i += len)
            {
                var curRe = 1f; var curIm = 0f;
                for (var k = 0; k < len / 2; k++)
                {
                    var uRe = re[i + k]; var uIm = im[i + k];
                    var vRe = re[i + k + len/2] * curRe - im[i + k + len/2] * curIm;
                    var vIm = re[i + k + len/2] * curIm + im[i + k + len/2] * curRe;
                    re[i + k] = uRe + vRe;       im[i + k] = uIm + vIm;
                    re[i + k + len/2] = uRe - vRe;  im[i + k + len/2] = uIm - vIm;
                    (curRe, curIm) = (curRe*wRe - curIm*wIm, curRe*wIm + curIm*wRe);
                }
            }
        }
    }
}
```
### 6.5 音效单元测试

```csharp
using NeriPlayer.Core.Player.Effects;
using Xunit;

namespace NeriPlayer.Core.Tests;

public class EqualizerEffectTests
{
    [Fact]
    public void Default_IsTransparent()
    {
        var eq = new EqualizerEffect(44100);
        eq.ApplyGains(new double[10]);   // 全 0 dB
        Assert.Equal(1.0f, eq.Process(1.0f), 3);   // 输出 ≈ 输入
    }

    [Fact]
    public void StereoBalance_Endpoints_AreSane()
    {
        var sb = new StereoBalanceEffect();
        sb.SetBalance(1.0f);   // 全左
        var data = new float[] { 1f, -1f, 1f, -1f };
        sb.Process(data);
        Assert.True(data[0] > data[1]);   // 左声道占优
    }

    [Fact]
    public void Fft_OfSine_ReturnsNonZeroPeak()
    {
        var fft = new FftAnalyzer(1024);
        var samples = new float[1024];
        for (var i = 0; i < 1024; i++)
            samples[i] = MathF.Sin(2 * MathF.PI * 440f * i / 44100f);   // 440Hz
        var bands = fft.Compute(samples);
        Assert.True(bands.Max() > 0f);
    }
}
```

```powershell
dotnet test tests/NeriPlayer.Core.Tests
```

**✅ 验收**
- [ ] 均衡器全 0 dB 时输出≈输入（透明直通）
- [ ] 立体声平衡端点值行为正确
- [ ] FFT 对 440Hz 正弦波能检测到非零能量

---
## 七、API 客户端（第 28-37 天）

### 7.1 统一平台接口（Core/Api/Common/IPlatformClient.cs）

```csharp
namespace NeriPlayer.Core.Api.Common;

public enum LoginMethod { QrCode, Cookie, Token }

public sealed record LoginResult(bool Success, string? Message, string? QrUrl = null);
public sealed record SongUrlResult(bool Success, string? Url, string? QualityKey);
public sealed record LyricResult(string Lrc, string? TranslatedLrc, string Source);
public sealed record RemotePlaylist(string Id, string Name, string? CoverUrl);
public sealed record RemotePlaylistDetail(string Id, string Name, IReadOnlyList<SongItem> Songs);
public sealed record RecommendationFeed(IReadOnlyList<SongItem> Songs, IReadOnlyList<RemotePlaylist> Playlists);
public sealed record SearchResponse(IReadOnlyList<SongItem> Songs, bool HasMore);

public interface IPlatformClient
{
    string PlatformId { get; }            // "netease" | "bili" | "youtube_music"
    bool IsLoggedIn { get; }
    Task<LoginResult> LoginAsync(LoginMethod method);

    Task<SearchResponse> SearchAsync(string keyword, int page = 1);
    Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page = 1);
    Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId);
    Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null);
    Task<LyricResult?> GetLyricAsync(SongItem song);
    Task<RecommendationFeed> GetRecommendationsAsync();
}
```

### 7.2 网易云加密（Core/Api/Netease/NeteaseCrypto.cs）

对标 Analysis.md 22.4 + Process.md 8.2：

```csharp
using System.Security.Cryptography;
using System.Text;

namespace NeriPlayer.Core.Api.Netease;

/// <summary>weapi 加密：AES-CBC + RSA + 随机 secretKey（对标 NeteaseCrypto.kt）</summary>
public static class NeteaseCrypto
{
    private const string AesKey = "0CoJUm6Qyw8W8jud";      // 固定 key（weapi）
    private const string AesIv = "0102030405060708";
    private const string RsaExponent = "010001";
    private const string RsaModulus =
        "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b7251" +
        "52b3b17d8762718ed6396fddc39e9f8e93d1d3e3d9e9a4f8e8e8e8e8e8e8e8e8e8e";

    private static readonly RandomNumberGenerator Rng = RandomNumberGenerator.Create();

    public static Dictionary<string, string> Weapi(Dictionary<string, object> payload)
    {
        var text = System.Text.Json.JsonSerializer.Serialize(payload);
        var secretKey = Random16Hex();
        var params1 = AesEncrypt(text, AesKey);
        var params2 = AesEncrypt(params1, secretKey);
        var encSecKey = RsaEncrypt(secretKey);

        return new Dictionary<string, string>
        {
            ["params"] = params2,
            ["encSecKey"] = encSecKey,
        };
    }

    private static string Random16Hex()
    {
        var bytes = new byte[16];
        Rng.GetBytes(bytes);
        return Convert.ToHexString(bytes).ToLowerInvariant();
    }

    private static string AesEncrypt(string input, string key)
    {
        using var aes = Aes.Create();
        aes.Key = Encoding.UTF8.GetBytes(key);
        aes.IV = Encoding.UTF8.GetBytes(AesIv);
        aes.Mode = CipherMode.CBC;
        aes.Padding = PaddingMode.PKCS7;
        using var enc = aes.CreateEncryptor();
        var bytes = Encoding.UTF8.GetBytes(input);
        var outBytes = enc.TransformFinalBlock(bytes, 0, bytes.Length);
        return Convert.ToHexString(outBytes).ToLowerInvariant();
    }

    private static string RsaEncrypt(string input)
    {
        using var rsa = RSA.Create();
        var exponent = Convert.FromHexString(RsaExponent);
        var modulus = Convert.FromHexString(RsaModulus);
        rsa.ImportParameters(new RSAParameters { Exponent = exponent, Modulus = modulus });

        var text = Encoding.UTF8.GetBytes(input).Reverse().ToArray();   // 逆序
        var encrypted = rsa.Encrypt(text, RSAEncryptionPadding.Pkcs1);
        return Convert.ToHexString(encrypted).ToLowerInvariant();
    }
}
```
### 7.3 网易云客户端（Core/Api/Netease/NeteaseClient.cs）

```csharp
using System.Text;
using System.Text.Json;
using NeriPlayer.Core.Api.Common;

namespace NeriPlayer.Core.Api.Netease;

public sealed class NeteaseClient(HttpClient http) : IPlatformClient
{
    private const string BaseUrl = "https://music.163.com/weapi/";
    private const int MaxResponseBytes = 4 * 1024 * 1024;   // MAX_RESPONSE_BYTES

    public string PlatformId => "netease";
    public bool IsLoggedIn { get; private set; }

    private async Task<string> PostWeapiAsync(string path, Dictionary<string, object> payload)
    {
        var form = NeteaseCrypto.Weapi(payload);
        using var content = new FormUrlEncodedContent(form);
        using var resp = await http.PostAsync(BaseUrl + path, content);
        resp.EnsureSuccessStatusCode();
        var body = await resp.Content.ReadAsByteArrayAsync();
        if (body.Length > MaxResponseBytes)
            throw new InvalidOperationException("Response too large");
        return Encoding.UTF8.GetString(body);
    }

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        var json = await PostWeapiAsync("search/get", new Dictionary<string, object>
        {
            ["s"] = keyword,
            ["type"] = 1,
            ["limit"] = 30,
            ["offset"] = (page - 1) * 30,
        });
        // 解析 result.songs[] → List<SongItem>（songs 的 ar/al 字段映射）
        return new SearchResponse([], true);
    }

    public async Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null)
    {
        var level = qualityKey switch
        {
            "lossless" => "lossless",
            "hires" => "hires",
            "high" => "exhigh",
            _ => "standard",
        };
        var json = await PostWeapiAsync("song/enhance/player/url/v1", new Dictionary<string, object>
        {
            ["ids"] = new[] { song.AudioId ?? song.Id.ToString() },
            ["level"] = level,
            ["encodeType"] = "mp3",
        });
        // 解析 data[0].url；为空 → 音质降级重试（standard → mp3 兜底）
        return new SongUrlResult(false, null, level);
    }

    public Task<LoginResult> LoginAsync(LoginMethod method) =>
        method == LoginMethod.QrCode ? QrLoginAsync() : Task.FromResult(new LoginResult(false, "仅支持二维码"));

    private async Task<LoginResult> QrLoginAsync()
    {
        // 1) weapi/login/qrcode/unikey 获取 key
        // 2) 返回 QrUrl = https://music.163.com/login?code_key={key}
        // 3) 轮询 weapi/login/qrcode/client/login（code 800 未扫 / 801 过期 / 803 成功）
        // 4) 成功后持久化 Cookie（见第 13 章 CredentialStore）
        return new LoginResult(false, "未实现");
    }

    public Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId) => throw new NotImplementedException();
    public Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page) => throw new NotImplementedException();
    public Task<LyricResult?> GetLyricAsync(SongItem song) => throw new NotImplementedException();
    public Task<RecommendationFeed> GetRecommendationsAsync() => throw new NotImplementedException();
}
```

> **实现要点**（对标 Analysis.md 4.1 / 18.1）：
> - Cookie 由 `CookieStore` 注入 HttpClient（CookieContainer 或手动 Header）
> - 播放失败回退链：音质降级 → 自动源切换（第 4.4 节）→ 本地兜底
> - 首页推荐：登录感知（未登录过滤 requiresLogin 分区）+ 失败回退（code 301/50000005）

### 7.4 Bilibili 客户端（Core/Api/Bili/BiliClient.cs）

```csharp
using System.Security.Cryptography;
using System.Text;
using NeriPlayer.Core.Api.Common;

namespace NeriPlayer.Core.Api.Bili;

/// <summary>WBI 签名（对标 Analysis.md 4.2 + 22.4）</summary>
public static class WbiSignature
{
    private static readonly int[] MixinKeyEncTab =
        [46,47,18,2,53,8,23,32,15,50,10,31,58,3,45,35,27,43,5,49,33,9,42,19,29,28,14,39,12,38,41,13,37,48,7,16,24,55,40,61,26,17,0,1,60,51,30,4,22,25,54,21,56,59,6,63,57,62,11,36,20,34,44,52];

    public static string Sign(Dictionary<string, string> query, string imgKey, string subKey)
    {
        var raw = imgKey + subKey;
        var mixed = new string(MixinKeyEncTab.Select(i => raw[i]).ToArray());
        // w_rid = md5(mixedKey + wts + 排序后的 "k1=v1&k2=v2" 完整串)
        var joined = string.Join("&", query.OrderBy(kv => kv.Key)
            .Select(kv => $"{kv.Key}={kv.Value}"));
        var m = MD5.HashData(Encoding.UTF8.GetBytes(mixed + joined));
        return Convert.ToHexString(m).ToLowerInvariant();
    }

    // 完整流程：
    // 1) GET x/web-interface/nav 获取 img_url / sub_url 的 key 部分
    // 2) 拼装 query：wts = Unix 时间戳 + 业务参数
    // 3) 计算 w_rid = Sign(query, imgKey, subKey) 追加到 URL
}
```

> 实际 WBI 拼接格式需与 `tools_pub/ytmusic_api_probe.py` 与线上抓包对照校准（Analysis.md 17 章）。

**✅ 验收**
- [ ] `NeteaseCrypto.Weapi` 对固定输入产出固定格式（params + encSecKey）
- [ ] 网易云搜索接口可返回真实歌曲列表
- [ ] Bilibili WBI 签名通过线上接口校验（不返回 -403）

---
### 7.5 YouTube Music 客户端（Core/Api/YouTube/YouTubeMusicClient.cs）

```csharp
using System.Text.Json;
using NeriPlayer.Core.Api.Common;

namespace NeriPlayer.Core.Api.YouTube;

/// <summary>InnerTube WEB_REMIX 客户端（对标 Analysis.md 4.3 + 22.4）</summary>
public sealed class YouTubeMusicClient(HttpClient http, YouTubePlayerScriptStore scriptStore) : IPlatformClient
{
    private const string InnerTubeApi = "https://music.youtube.com/youtubei/v1/";
    private const string ClientName = "WEB_REMIX";
    private const int ClientVersion = 67;

    public string PlatformId => "youtube_music";
    public bool IsLoggedIn { get; private set; }

    private async Task<string> PostInnerTubeAsync(string endpoint, Dictionary<string, object> payload)
    {
        var body = new Dictionary<string, object>
        {
            ["context"] = new Dictionary<string, object>
            {
                ["client"] = new Dictionary<string, object>
                {
                    ["clientName"] = ClientName,
                    ["clientVersion"] = ClientVersion.ToString(),
                    ["hl"] = "zh-Hans",
                    ["gl"] = "CN",
                }
            },
        };
        foreach (var kv in payload) body[kv.Key] = kv.Value;

        using var resp = await http.PostAsJsonAsync(InnerTubeApi + endpoint, body);
        resp.EnsureSuccessStatusCode();
        return await resp.Content.ReadAsStringAsync();
    }

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        var json = await PostInnerTubeAsync("search", new Dictionary<string, object>
        {
            ["query"] = keyword,
            ["params"] = "EgWKAQIIAWoKEAoQCRADEAA%3D",   // music 歌曲过滤参数
        });
        // 解析 contents.tabbedSearchResultsRenderer...musicResponsiveListItemRenderer
        return new SearchResponse([], true);
    }

    public async Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null)
    {
        // 1) 播放列表加载：browse/videoId → streamingData
        // 2) 若需 PoToken：走 player.js 缓存 → EJS 挑战 → NewPipe 回退（多级回退链）
        // 3) 解析 streamingData.adaptiveFormats 选最佳音频格式
        return new SongUrlResult(false, null, qualityKey);
    }

    public Task<LoginResult> LoginAsync(LoginMethod method) =>
        // Cookie 登录：SAPISID / __Secure-3PAPISID → CredentialStore 持久化
        Task.FromResult(new LoginResult(false, "Cookie 登录未实现"));
    // 其余成员同 IPlatformClient 模式（略）
}
```

**YouTube 多级回退链**（对标 Analysis.md 4.3）：

```
登录 Cookie → 匿名 visitor → PoToken（缓存 6h）→ player.js 缓存（48h 过期）
→ EJS 挑战（JS 求解队列）→ NewPipe 回退
```

### 7.6 歌词聚合（Core/Api/Lyrics/LyricsSourceAggregator.cs）

对标 Analysis.md 6.1 + Process.md 8.5：

```csharp
using NeriPlayer.Core.Api.Common;

namespace NeriPlayer.Core.Api.Lyrics;

public sealed class LyricsSourceAggregator(IEnumerable<LyricsSource> sources, LyricsCache cache)
{
    public sealed class LyricsCache(int capacity = 20)
    {
        private readonly Dictionary<string, LyricResult> _map = new();
        public LyricResult? Get(string key) => _map.TryGetValue(key, out var r) ? r : null;
        public void Put(string key, LyricResult r)
        {
            if (_map.Count >= capacity) _map.Remove(_map.Keys.First());
            _map[key] = r;
        }
    }

    public async Task<LyricResult?> GetLyricAsync(SongItem song)
    {
        var key = $"{song.ChannelId}|{song.AudioId}";
        var cached = cache.Get(key);
        if (cached is not null) return cached;

        // 合并排序：内嵌 > 匹配源 > 平台官方 > 第三方（QQ/Kugou/LrcLib）
        foreach (var source in sources.OrderBy(s => s.Priority))
        {
            var r = await source.TryGetAsync(song);
            if (r is not null)
            {
                cache.Put(key, r);
                return r;
            }
        }
        return null;
    }
}

public abstract class LyricsSource(int priority)
{
    public int Priority { get; } = priority;
    public abstract Task<LyricResult?> TryGetAsync(SongItem song);
}
```

### 7.7 搜索聚合（Core/Api/Search/SearchManager.cs）

```csharp
using System.Collections.Concurrent;
using NeriPlayer.Core.Api.Common;

namespace NeriPlayer.Core.Api.Search;

public sealed class SearchManager(IEnumerable<IPlatformClient> clients)
{
    private readonly ConcurrentDictionary<string, SearchResponse> _cache = new();

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        var cacheKey = $"{keyword}|{page}";
        if (_cache.TryGetValue(cacheKey, out var hit)) return hit;

        // 三平台并发搜索
        var tasks = clients.Select(c => c.SearchAsync(keyword, page)).ToArray();
        var results = await Task.WhenAll(tasks);

        // 按 stableKey 去重合并
        var seen = new HashSet<string>();
        var merged = new List<SongItem>();
        foreach (var r in results)
        foreach (var song in r.Songs)
        {
            if (seen.Add(song.StableKey())) merged.Add(song);
        }
        var resp = new SearchResponse(merged, results.Any(r => r.HasMore));
        _cache[cacheKey] = resp;
        return resp;
    }
}
```

**✅ 验收**
- [ ] 网易云 + B 站 + YouTube 三平台搜索可并行返回、按 stableKey 去重
- [ ] 歌词聚合按优先级回退，LRU 缓存（各 20 条）生效
- [ ] YouTube 匿名访问可解析出音频流（离线可测：用本地 yt-dlp 校验 URL 有效性）

---
## 八、下载管理（第 38-44 天）

### 8.1 下载任务（Core/Download/DownloadTask.cs）

```csharp
using System.Reactive.Subjects;

namespace NeriPlayer.Core.Download;

public enum DownloadStatus { Queued, Downloading, Paused, Completed, Failed, Cancelled }

public sealed record DownloadProgress(long TaskId, long BytesReceived, long? TotalBytes,
    double Percent, DownloadStatus Status);

public sealed class DownloadTask
{
    public long TaskId { get; init; }
    public required string StableKey { get; init; }
    public required string Url { get; init; }
    public required string TargetPath { get; init; }
    public DownloadStatus Status { get; private set; } = DownloadStatus.Queued;
    public long BytesReceived { get; private set; }

    private readonly Subject<DownloadProgress> _progress = new();
    public IObservable<DownloadProgress> Progress => _progress;
    private readonly CancellationTokenSource _cts = new();
    private readonly HttpClient _http;

    public DownloadTask(HttpClient http) => _http = http;

    public async Task RunAsync()
    {
        Status = DownloadStatus.Downloading;
        try
        {
            // 断点续传：Range: bytes={BytesReceived}-（服务端需支持）
            using var req = new HttpRequestMessage(HttpMethod.Get, Url);
            if (BytesReceived > 0) req.Headers.Range =
                new System.Net.Http.Headers.RangeHeaderValue(BytesReceived, null);

            using var resp = await _http.SendAsync(req, HttpCompletionOption.ResponseHeadersRead, _cts.Token);
            resp.EnsureSuccessStatusCode();

            var total = resp.Content.Headers.ContentLength ?? 0;
            await using var src = await resp.Content.ReadAsStreamAsync(_cts.Token);
            Directory.CreateDirectory(Path.GetDirectoryName(TargetPath)!);
            var tmpPath = TargetPath + ".part";
            await using var dst = File.Open(tmpPath, FileMode.Append);   // .part 断点续写

            var buffer = new byte[81920];
            while (true)
            {
                var read = await src.ReadAsync(buffer, _cts.Token);
                if (read == 0) break;
                await dst.WriteAsync(buffer.AsMemory(0, read), _cts.Token);
                BytesReceived += read;
                _progress.OnNext(new DownloadProgress(TaskId, BytesReceived, total,
                    total > 0 ? BytesReceived * 100.0 / total : 0, DownloadStatus.Downloading));
            }

            await dst.FlushAsync(_cts.Token);
            if (File.Exists(TargetPath)) File.Delete(TargetPath);
            File.Move(tmpPath, TargetPath);
            Status = DownloadStatus.Completed;
            _progress.OnNext(new DownloadProgress(TaskId, BytesReceived, total, 100, DownloadStatus.Completed));
        }
        catch (OperationCanceledException)
        {
            Status = DownloadStatus.Cancelled;
        }
        catch (Exception)
        {
            Status = DownloadStatus.Failed;
        }
    }

    public void Cancel() => _cts.Cancel();
}
```

### 8.2 下载队列（Core/Download/DownloadQueue.cs）

```csharp
namespace NeriPlayer.Core.Download;

/// <summary>Semaphore 并发控制：默认 6 / 最大 8（Analysis.md 24.2）</summary>
public sealed class DownloadQueue
{
    public const int DefaultConcurrency = 6;
    public const int MaxConcurrency = 8;
    public const int CancelSettleTimeoutMs = 5000;   // DOWNLOAD_CANCEL_SETTLE_TIMEOUT_MS

    private readonly SemaphoreSlim _semaphore = new(DefaultConcurrency, MaxConcurrency);
    private readonly Queue<DownloadTask> _pending = new();
    private readonly List<Task> _running = [];

    public event Action<DownloadTask>? Completed;

    public void Enqueue(DownloadTask task)
    {
        _pending.Enqueue(task);
        _ = PumpAsync();
    }

    private async Task PumpAsync()
    {
        while (_pending.Count > 0)
        {
            await _semaphore.WaitAsync();
            var task = _pending.Dequeue();
            var run = Task.Run(task.RunAsync);
            _running.Add(run);
            _ = run.ContinueWith(_ =>
            {
                _semaphore.Release();
                Completed?.Invoke(task);
            });
        }
    }
}
```

### 8.3 标签写入（Core/Download/MetadataWriter.cs）

```csharp
using TagLib;

namespace NeriPlayer.Core.Download;

/// <summary>TagLib# 写标签，失败重试上限 3 次（Analysis.md 24.2）</summary>
public static class MetadataWriter
{
    public const int MaxAttempts = 3;

    public static async Task WriteAsync(string filePath, SongMetadata meta, CancellationToken ct = default)
    {
        for (var attempt = 1; attempt <= MaxAttempts; attempt++)
        {
            try
            {
                await Task.Run(() => WriteCore(filePath, meta), ct);
                return;
            }
            catch when (attempt < MaxAttempts) { await Task.Delay(200 * attempt, ct); }
        }
    }

    private static void WriteCore(string filePath, SongMetadata meta)
    {
        using var file = TagLib.File.Create(filePath);
        file.Tag.Title = meta.Title;
        file.Tag.Performers = [meta.Artist];
        file.Tag.Album = meta.Album;
        if (meta.CoverBytes is not null && file is IPictureTag pt)
            pt.Pictures = [new Picture(meta.CoverBytes) { MimeType = "image/jpeg" }];
        file.Save();
    }
}

public sealed record SongMetadata(string Title, string Artist, string Album, byte[]? CoverBytes);
```

### 8.4 下载索引（Data/Entities 扩展 + 三层索引）

对标 Analysis.md 24.2「catalog/snapshot/recovery/queue 四张索引表」：

| 表 | 作用 | 说明 |
|----|------|------|
| `downloads` | 主目录清单 | 歌曲 → 本地路径、状态、质量、进度 |
| `download_snapshots` | 快照 | 目录树捕获，主键 (root_key, bucket, entry_key) |
| `download_recovery` | 恢复 | 异常退出后扫描 `.part` 与孤儿文件 |
| `download_queue` | 队列 | 待下载任务持久化 |

**✅ 验收**
- [ ] 8 路并发下载不超限（Semaphore 生效）
- [ ] 中断后 `.part` 保留，重试从 `Range: bytes=N-` 续传
- [ ] 下载完成后 TagLib# 写入标题/艺人/专辑/封面并 `file.Save()` 无异常

---
## 九、数据同步（第 45-50 天）

### 9.1 同步接口（Data/Sync/ISyncProvider.cs）

```csharp
namespace NeriPlayer.Data.Sync;

public sealed record SyncFile(string Name, byte[] Content, string? Etag, DateTimeOffset ModifiedAt);
public sealed record SyncResult(bool Success, int ChangedCount, string? Message);

/// <summary>同步后端抽象：GitHub 仓库 或 WebDAV 目录</summary>
public interface ISyncProvider
{
    string ProviderName { get; }
    Task<bool> TestConnectionAsync();
    Task<IReadOnlyList<SyncFile>> ListAsync(string scope);
    Task<SyncFile?> DownloadAsync(string scope, string name);
    Task<bool> UploadAsync(string scope, string name, byte[] content, string? etag);
    Task<bool> DeleteAsync(string scope, string name);
}
```

### 9.2 GitHub Provider（Data/Sync/GitHubSyncProvider.cs）

```csharp
using Octokit;

namespace NeriPlayer.Data.Sync;

/// <summary>对标 SyncGithubManager：仓库内 JSON 文件</summary>
public sealed class GitHubSyncProvider : ISyncProvider
{
    private readonly GitHubClient _client;
    private readonly string _owner;
    private readonly string _repo;
    private readonly string _pathPrefix;

    public GitHubSyncProvider(string token, string owner, string repo, string pathPrefix = "neriplayer")
    {
        _client = new GitHubClient(new ProductHeaderValue("NeriPlayer.Windows"))
        {
            Credentials = new Credentials(token)
        };
        _owner = owner; _repo = repo; _pathPrefix = pathPrefix;
    }

    public string ProviderName => "github";
    public async Task<bool> TestConnectionAsync() =>
        (await _client.User.Current()).Login.Length > 0;

    public async Task<IReadOnlyList<SyncFile>> ListAsync(string scope)
    {
        var items = await _client.Repository.Content.GetAllContents(_owner, _repo,
            $"{_pathPrefix}/{scope}");
        return items.Select(i => new SyncFile(i.Name, Array.Empty<byte>(),
            i.Sha, i.UpdatedAt ?? DateTimeOffset.MinValue)).ToList();
    }

    public async Task<SyncFile?> DownloadAsync(string scope, string name)
    {
        var item = await _client.Repository.Content.GetAllContents(_owner, _repo,
            $"{_pathPrefix}/{scope}/{name}");
        var content = item[0].Content ?? "";
        return new SyncFile(name, System.Text.Encoding.UTF8.GetBytes(content),
            item[0].Sha, item[0].UpdatedAt ?? DateTimeOffset.MinValue);
    }

    public async Task<bool> UploadAsync(string scope, string name, byte[] content, string? etag)
    {
        var path = $"{_pathPrefix}/{scope}/{name}";
        var text = System.Text.Encoding.UTF8.GetString(content);
        if (etag is not null)
            await _client.Repository.Content.UpdateFile(_owner, _repo, path,
                new UpdateFileRequest($"sync {name}", text, etag));
        else
            await _client.Repository.Content.CreateFile(_owner, _repo, path,
                new CreateFileRequest($"sync {name}", text));
        return true;
    }

    public Task<bool> DeleteAsync(string scope, string name) => throw new NotImplementedException();
}
```

### 9.3 WebDAV Provider（Data/Sync/WebDavSyncProvider.cs）

```csharp
using WebDav;

namespace NeriPlayer.Data.Sync;

public sealed class WebDavSyncProvider : ISyncProvider
{
    private readonly WebDavClient _client;
    private readonly string _root;

    public WebDavSyncProvider(Uri server, string user, string password, string root = "neriplayer")
    {
        _client = new WebDavClient(new WebDavClientParams
        {
            ServerUrl = server,
            Credentials = new System.Net.NetworkCredential(user, password),
        });
        _root = root;
    }

    public string ProviderName => "webdav";

    public async Task<IReadOnlyList<SyncFile>> ListAsync(string scope)
    {
        var result = await _client.Propfind($"{_root}/{scope}");
        return result.Resources
            .Where(r => !r.IsCollection)
            .Select(r => new SyncFile(r.Uri.Split('/').Last(), Array.Empty<byte>(),
                r.ETag, r.LastModified ?? DateTimeOffset.MinValue))
            .ToList();
    }

    public async Task<SyncFile?> DownloadAsync(string scope, string name)
    {
        var resp = await _client.GetRawFile($"{_root}/{scope}/{name}");
        return resp.IsSuccessful
            ? new SyncFile(name, await resp.Stream.ToArrayAsync(),
                resp.Headers["ETag"], DateTimeOffset.UtcNow)
            : null;
    }

    public async Task<bool> UploadAsync(string scope, string name, byte[] content, string? etag)
    {
        var path = $"{_root}/{scope}/{name}";
        using var ms = new MemoryStream(content);
        var resp = etag is null
            ? await _client.PutFile(path, ms)
            : await _client.PutFile(path, ms,
                headers: new[] { new System.Net.WebHeaderCollection { { "If-Match", etag } } });
        return resp.IsSuccessful;
    }

    public Task<bool> DeleteAsync(string scope, string name) => throw new NotImplementedException();
}
```
### 9.4 因果 Token 合并（Data/Sync/SyncMergeStrategy.cs）

```csharp
namespace NeriPlayer.Data.Sync;

/// <summary>同步记录携带因果 Token（对标 SyncCausalToken / Analysis.md 9 章）</summary>
public sealed record SyncToken(string SongId, string BaseVersion, string OperationId);

public static class SyncMergeStrategy
{
    /// <summary>冲突裁决：因果序优先，其次时间戳</summary>
    public static int Compare(SyncToken a, SyncToken b, long aTimestamp, long bTimestamp)
    {
        if (a.BaseVersion == b.OperationId) return -1;   // a 是 b 的祖先
        if (b.BaseVersion == a.OperationId) return 1;    // b 是 a 的祖先
        return aTimestamp.CompareTo(bTimestamp);         // 无因果关系 → 时间裁决
    }

    /// <summary>歌单合并：按 stableKey 去重 + updated_at 冲突裁决（Process.md 10.2）</summary>
    public static IReadOnlyList<SongItem> MergePlaylists(
        IReadOnlyList<SongItem> local, IReadOnlyList<SongItem> remote)
    {
        var map = new Dictionary<string, SongItem>();
        foreach (var s in local) map[s.StableKey()] = s;
        foreach (var s in remote)
        {
            if (!map.TryGetValue(s.StableKey(), out var existing))
                map[s.StableKey()] = s;
            else if (s.AddedAt > existing.AddedAt)
                map[s.StableKey()] = s;
        }
        return map.Values.ToList();
    }
}
```

### 9.5 定时同步（Background/Services/SyncScheduledService.cs）

```csharp
using Microsoft.Extensions.Hosting;
using Quartz;
using Quartz.Impl;

namespace NeriPlayer.Background.Services;

/// <summary>对标 WorkManager：每日 02:00 同步 + 手动触发</summary>
public sealed class SyncScheduledService : BackgroundService
{
    private readonly ISyncProvider _provider;

    public SyncScheduledService(ISyncProvider provider) => _provider = provider;

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var factory = new StdSchedulerFactory();
        var scheduler = await factory.GetScheduler(stoppingToken);
        await scheduler.Start(stoppingToken);

        var job = JobBuilder.Create<SyncJob>().Build();
        var trigger = TriggerBuilder.Create()
            .WithCronSchedule("0 0 2 * * ?")          // 每日 02:00
            .Build();
        await scheduler.ScheduleJob(job, trigger, stoppingToken);
    }
}

public sealed class SyncJob : IJob
{
    public async Task Execute(IJobExecutionContext context)
    {
        // 1) 收集本地 outbox 变更
        // 2) 拉取远端 checkpoint 之后的新文件
        // 3) MergePlaylists 合并 → 回写
        // 4) 更新 checkpoint 游标
        await Task.CompletedTask;   // 实际逻辑见 9.1-9.4
    }
}
```

**✅ 验收**
- [ ] GitHub Provider 可上传/下载/列表（用测试 token + 私有空仓库）
- [ ] WebDAV Provider 通过（可用坚果云/Nextcloud 测试）
- [ ] 因果合并裁决：同 key 冲突按 updated_at 取新、因果链正确
- [ ] 定时任务每天 02:00 触发（日志可查）

---
## 十、UI 主框架（第 51-62 天）

### 10.1 主窗口三栏布局（UI/Views/MainWindow.axaml）

对标 Process.md 11.1：

```xml
<Window xmlns="https://github.com/avaloniaui"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        xmlns:views="using:NeriPlayer.UI.Views"
        x:Class="NeriPlayer.UI.Views.MainWindow"
        Title="NeriPlayer"
        Width="1280" Height="800"
        Background="{DynamicResource AppBackgroundBrush}">
  <DockPanel>
    <!-- 底部播放条 -->
    <Border DockPanel.Dock="Bottom" Height="72" Background="{DynamicResource PlaybackBarBrush}">
      <views:PlaybackBarView />
    </Border>

    <Grid ColumnDefinitions="240,*,320">
      <!-- 左侧边栏 -->
      <Border Background="{DynamicResource SidebarBrush}">
        <StackPanel Margin="16" Spacing="4">
          <TextBlock Text="NeriPlayer" FontSize="20" FontWeight="Bold" />
          <Button Content="🏠 首页" Command="{Binding NavigateCommand}" CommandParameter="Home" />
          <Button Content="📻 发现" Command="{Binding NavigateCommand}" CommandParameter="Discover" />
          <Button Content="💿 音乐库" Command="{Binding NavigateCommand}" CommandParameter="Library" />
          <Button Content="📥 下载" Command="{Binding NavigateCommand}" CommandParameter="Downloads" />
          <Button Content="⚙ 设置" Command="{Binding NavigateCommand}" CommandParameter="Settings" />
        </StackPanel>
      </Border>

      <!-- 内容区（页面容器） -->
      <Border Grid.Column="1">
        <ContentControl Content="{Binding CurrentPage}" />
      </Border>

      <!-- 右侧播放队列（可折叠） -->
      <Border Grid.Column="2" Background="{DynamicResource SidebarBrush}">
        <views:QueueView />
      </Border>
    </Grid>
  </DockPanel>
</Window>
```

### 10.2 主窗口 ViewModel（UI/ViewModels/MainWindowViewModel.cs）

```csharp
using ReactiveUI;

namespace NeriPlayer.UI.ViewModels;

public sealed class MainWindowViewModel : ReactiveObject
{
    private readonly Dictionary<string, object> _pages = new();
    private object _currentPage;

    public MainWindowViewModel()
    {
        // 页面注册（懒加载）
        _pages["Home"] = new HomeViewModel();
        _pages["Discover"] = new DiscoverViewModel();
        _pages["Library"] = new LibraryViewModel();
        _pages["Downloads"] = new DownloadsViewModel();
        _pages["Settings"] = new SettingsViewModel();
        _currentPage = _pages["Home"];
    }

    public object CurrentPage
    {
        get => _currentPage;
        private set => this.RaiseAndSetIfChanged(ref _currentPage, value);
    }

    public void NavigateTo(string key)
    {
        if (_pages.TryGetValue(key, out var page)) CurrentPage = page;
    }
}
```

### 10.3 底部播放条（UI/Views/PlaybackBarView.axaml + ViewModel）

```xml
<Grid ColumnDefinitions="*,*,2*,Auto" Margin="12,0">
  <StackPanel Orientation="Horizontal" Spacing="12" VerticalAlignment="Center">
    <Border Width="48" Height="48" CornerRadius="8">
      <Image Source="{Binding Cover}" Stretch="UniformToFill" />
    </Border>
    <StackPanel VerticalAlignment="Center">
      <TextBlock Text="{Binding CurrentTitle}" FontWeight="SemiBold" />
      <TextBlock Text="{Binding CurrentArtist}" Opacity="0.7" FontSize="12" />
    </StackPanel>
  </StackPanel>

  <StackPanel Grid.Column="1" Orientation="Horizontal" HorizontalAlignment="Center"
              Spacing="8" VerticalAlignment="Center">
    <Button Content="⏮" Command="{Binding PreviousCommand}" />
    <Button Content="▶/⏸" Command="{Binding TogglePlayCommand}" Width="44" Height="44"
            CornerRadius="22" HorizontalAlignment="Center"/>
    <Button Content="⏭" Command="{Binding NextCommand}" />
  </StackPanel>

  <Grid Grid.Column="2" VerticalAlignment="Center">
    <Slider Value="{Binding PositionMs, Mode=TwoWay}"
            Minimum="0" Maximum="{Binding DurationMs}" />
  </Grid>
</Grid>
```

```csharp
// PlaybackBarViewModel.cs
using System.Reactive;
using ReactiveUI;
using NeriPlayer.Core.Player;

public sealed class PlaybackBarViewModel : ReactiveObject
{
    private string _currentTitle = "";
    private string _currentArtist = "";
    private double _positionMs;
    private double _durationMs;

    public string CurrentTitle { get => _currentTitle; private set => this.RaiseAndSetIfChanged(ref _currentTitle, value); }
    public string CurrentArtist { get => _currentArtist; private set => this.RaiseAndSetIfChanged(ref _currentArtist, value); }
    public double PositionMs { get => _positionMs; set => this.RaiseAndSetIfChanged(ref _positionMs, value); }
    public double DurationMs { get => _durationMs; private set => this.RaiseAndSetIfChanged(ref _durationMs, value); }

    public ReactiveCommand<Unit, Unit> TogglePlayCommand { get; }
    public ReactiveCommand<Unit, Unit> NextCommand { get; }
    public ReactiveCommand<Unit, Unit> PreviousCommand { get; }

    public PlaybackBarViewModel() : this(new PlayerManager()) { }

    public PlaybackBarViewModel(PlayerManager player)
    {
        TogglePlayCommand = ReactiveCommand.Create(async () =>
        {
            if (player.State is null) return;   // 简化：按实际状态切换
        });
        NextCommand = ReactiveCommand.Create(() => player.NextAsync());
        PreviousCommand = ReactiveCommand.Create(() => player.PreviousAsync());

        player.CurrentSong.Subscribe(s =>
        {
            CurrentTitle = s?.DisplayName ?? "";
            CurrentArtist = s?.DisplayArtist ?? "";
        });
        player.Position.Subscribe(ms => PositionMs = ms.TotalMilliseconds);
    }
}
```
### 10.4 歌词滚动视图（UI/Controls/LyricsScroller.axaml）

对标 Process.md 11.5（ItemsControl + 居中高亮行 + 1.25x 字号）：

```xml
<UserControl xmlns="https://github.com/avaloniaui"
             xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
             x:Class="NeriPlayer.UI.Controls.LyricsScroller">
  <ScrollViewer Name="Scroller">
    <ItemsControl ItemsSource="{Binding Lines}">
      <ItemsControl.ItemTemplate>
        <DataTemplate>
          <TextBlock Text="{Binding Text}"
                     FontSize="{Binding IsActive, Converter={StaticResource ActiveFontSizeConverter}}"
                     Foreground="{Binding IsActive, Converter={StaticResource ActiveBrushConverter}}"
                     TextAlignment="Center" Margin="0,10" />
        </DataTemplate>
      </ItemsControl.ItemTemplate>
    </ItemsControl>
  </ScrollViewer>
</UserControl>
```

```csharp
// LyricsScroller.axaml.cs 核心逻辑
using Avalonia.Controls;

public partial class LyricsScroller : UserControl
{
    private const double LineHeight = 40;   // 行高估算
    private int _activeIndex = -1;

    public void UpdatePosition(TimeSpan positionMs)
    {
        // 行索引 = LyricTimeline.FindIndexAt(positionMs)（二分查找）
        var index = -1;   // 由 LyricTimeline 计算
        if (index == _activeIndex) return;
        _activeIndex = index;
        // 滚动到当前行：offset = index * LineHeight - Viewport.Height / 2
    }
}
```

### 10.5 歌词解析器（Core/Player/Lyrics/LrcParser.cs）

对标 Analysis.md 6.2（LRC/TTML/YRC）：

```csharp
namespace NeriPlayer.Core.Player.Lyrics;

public sealed record LyricLine(TimeSpan Start, string Text, string? Translated);

public static class LrcParser
{
    /// <summary>解析标准 LRC：多语言/偏移标签/损坏输入容错</summary>
    public static IReadOnlyList<LyricLine> Parse(string lrc, long offsetMs = 0)
    {
        var lines = new List<LyricLine>();

        foreach (var raw in lrc.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            var line = raw.Trim();
            if (line.StartsWith('[') && line.Contains(':'))
            {
                var idx = line.IndexOf(']');
                var tag = line[1..idx];
                if (tag.Contains(':'))
                {
                    var parts = tag.Split(':');
                    if (int.TryParse(parts[0], out var m) && double.TryParse(parts[1], out var s))
                    {
                        var text = line[(idx + 1)..].Trim();
                        if (text.Length == 0) continue;
                        var start = TimeSpan.FromMilliseconds(m * 60_000 + s * 1000 + offsetMs);
                        lines.Add(new LyricLine(start, text, null));
                    }
                }
                else if (tag.StartsWith("offset:") && int.TryParse(tag[7..], out var off))
                {
                    return Parse(lrc, off);   // 整体偏移后重新解析
                }
            }
        }
        return lines.OrderBy(l => l.Start).ToList();
    }
}
```

### 10.6 设置系统（Source Generator）

对标 KSP 设置生成（Analysis.md 第 12 章 + 23.2）：

```csharp
// NeriPlayer.SourceGen/SettingsGenerator.cs —— 增量 Source Generator（简化版）
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.Text;

[Generator(LanguageNames.CSharp)]
public sealed class SettingsGenerator : IIncrementalGenerator
{
    public void Initialize(IncrementalGeneratorInitializationContext context)
    {
        var settings = context.SyntaxProvider
            .CreateSyntaxProvider(
                static (node, _) => node is Microsoft.CodeAnalysis.CSharp.Syntax.ClassDeclarationSyntax,
                static (ctx, _) => ctx.Node)
            .Where(static _ => true);

        context.RegisterSourceOutput(settings, static (spc, _) =>
        {
            // 扫描 [Setting] 属性 → 生成 SettingsKeys 常量类
            var sb = new StringBuilder();
            sb.AppendLine("namespace NeriPlayer.Data.Settings;");
            sb.AppendLine("public static partial class SettingsKeys {");
            sb.AppendLine("    public const string ThemeMode = \"theme.mode\";");
            sb.AppendLine("    public const string AccentColor = \"theme.accent\";");
            sb.AppendLine("}");
            spc.AddSource("SettingsKeys.g.cs", SourceText.From(sb.ToString(), Encoding.UTF8));
        });
    }
}
```

> 完整实现：基于 `SettingsSchema` 类的 `[Setting(key, defaultValue)]` 属性标记生成
> 常量 + 读写访问器，替代 Kotlin 的 `@AutoSetting`（Analysis.md 23.2）。

**✅ 验收**
- [ ] 主窗口三栏布局渲染正常，Tab 切换无卡顿
- [ ] 底部播放条与 PlayerManager 状态联动（切歌/进度/播放暂停）
- [ ] LRC 解析：偏移标签、多语言行、空行、损坏输入均有测试
- [ ] Source Generator 生成 `SettingsKeys` 且可被编译期消费

---
## 十一、后台与系统集成（第 63-68 天）

> **技术决定（2026-08-29 方案 A「乙」确认 · 当前为 net8.0 骨架）**：
> - **目标框架**：本机 .NET SDK 8.0.424 的 RID 图缺少 `win10-*` 条目（`PortableRuntimeIdentifierGraph.json` 仅有
>   `win/win-arm/win-arm64/win-x64/win-x86`），导致 `net8.0-windows10.0.19041.0` 报 `NETSDK1083`；
>   而 `net8.0-windows`（无版本）不含 WinRT 投影（`Windows.Media.*` 不可用）。
>   **结论：本机当前无法编译 WinRT 版**，故 `App`/`Background` 暂用 `net8.0`，SMTC/Toast 走骨架（日志占位）。
> - **SMTC API 修正**：不用 `SystemMediaTransportControls.GetForCurrentView()`（UWP 专属，需 CoreWindow），
>   改用 **`Windows.Media.Playback.MediaPlayer.SystemMediaTransportControls`**（桌面/Win32 可用的标准路径）。
>   （曾考虑 `GlobalSystemMediaTransportControlsSessionManager`，该 API 是"观察当前会话"，自发布媒体源需额外桥接，
>   `MediaPlayer.SystemMediaTransportControls` 更直接。）
> - **启用 WinRT 前置**：安装完整 Windows SDK（含 `Microsoft.Windows.SDK.NET.Ref` 的 MSBuild targets，当前 10.0.19041.56
>   NuGet 包缺 `.targets`），并把 `App`/`Background` TFM 设为 `net8.0-windows10.0.19041.0`。
>   完整实现代码已写入 `SmtcIntegration.cs` / `ToastNotificationService.cs` 的注释中。
> - **SyncScheduledService**（第 9 章已写好）**已启用**（`AppStartup.cs` 取消注释）。
> - **任务栏缩略图按钮**（11.2 `ITaskbarList3`）**放到主体验收通过后单独做**。
> - **新增文件**：`SmtcIntegration.cs`、`PlaybackService.cs`、`ToastNotificationService.cs`、`FloatingLyricsWindow.axaml(.cs)`。

### 11.0 本章完成记录（2026-08-29）

**完成情况**：本章逻辑主体（后台播放服务 + SMTC 桥接骨架 + 通知 + 桌面歌词窗口）已落地，且**可编译、可测试、可运行**。
**实际验证结果**（AI 实测）：
- `dotnet build NeriPlayer.Windows.sln` → ✅ 9 项目 0 错误
- `dotnet test` → ✅ 109/109 通过（Core 76 + API 12 + Data 21），无回归
- `dotnet run --project src/NeriPlayer.App` → ✅ 进程存活 8s，无异常，stderr 为空

**文件变更**：
| 文件 | 动作 | 说明 |
|---|---|---|
| `Background/Services/SmtcIntegration.cs` | 新增 | SMTC 骨架层（按钮→PlayerManager 桥接）；完整 WinRT 实现见注释 |
| `Background/Services/PlaybackService.cs` | 新增 | 后台播放服务（PlayerManager↔SMTC 双向桥接 + 空闲关闭策略） |
| `Background/Notifications/ToastNotificationService.cs` | 新增 | 通知服务（接口形状对齐 WinRT 版） |
| `UI/FloatingLyricsWindow.axaml(.cs)` | 新增 | 桌面歌词窗口（置顶透明、可拖动、原文+翻译双行） |
| `App/AppStartup.cs` | 修改 | 注册 SmtcIntegration(单例) + PlaybackService(HostedService) + 启用 SyncScheduledService |
| `Directory.Packages.props` | 已还原 | 曾加 CsWinRT，因当前 net8.0 未使用已移除 |

**WinRT 启用前置（待做，需环境）**：本机 .NET SDK 8.0.424 的 RID 图缺 `win10-*`，无法编 `net8.0-windows10.0.19041.0`。
需补全 Windows Desktop/SDK 环境后：① 改 `App`/`Background` TFM 为 `net8.0-windows10.0.19041.0`；
② 按 `SmtcIntegration.cs` / `ToastNotificationService.cs` 注释启用 WinRT 实现；③ 重测 SMTC/Toast。

### 11.1 SMTC 媒体控制（Background/Services/SmtcIntegration.cs）

对标 Analysis.md 16 章（MediaSession）+ Process.md 12.1。
**实现方式**：`GlobalSystemMediaTransportControlsSessionManager`（桌面标准路径，非 `GetForCurrentView`）。

```csharp
using Windows.Media.Control;

namespace NeriPlayer.Background.Services;

/// <summary>
/// SMTC 桌面集成（对标 Analysis.md 16 章 / Process.md 12.1）。
/// 使用 GlobalSystemMediaTransportControlsSessionManager（面向桌面/.NET），
/// 而非 GetForCurrentView()（UWP 专属，需 CoreWindow，桌面应用不可用）。
/// </summary>
public sealed class SmtcIntegration : IAsyncDisposable
{
    private GlobalSystemMediaTransportControlsSessionManager? _manager;
    private GlobalSystemMediaTransportControlsSession? _session;
    private bool _initialized;

    /// <summary>
    /// 桌面媒体键按钮被按下时触发。
    /// 外部（PlaybackService）订阅后映射到 PlayerManager.Play/Pause/Next/Previous。
    /// </summary>
    public event Action<GlobalSystemMediaTransportControlsSessionPlaybackControls>? ButtonPressed;

    /// <summary>异步初始化：获取 SMTC SessionManager（需在 UI 线程或后台线程调用一次）。</summary>
    public async Task InitializeAsync(CancellationToken ct = default)
    {
        try
        {
            _manager = await GlobalSystemMediaTransportControlsSessionManager.RequestAsync();
            _session = _manager.GetCurrentSession();
            _manager.CurrentSessionChanged += OnSessionChanged;
            if (_session is not null)
                _session.PlaybackInfoChanged += OnPlaybackInfoChanged;
            _initialized = true;
            NeriPlayer.Core.Logging.AppLogger.Instance.Information("SMTC session manager initialized");
        }
        catch (Exception ex)
        {
            // 降级：SMTC 不可用时仅记录日志，不影响播放
            NeriPlayer.Core.Logging.AppLogger.Instance.Warning(ex, "SMTC initialization failed, media controls unavailable");
        }
    }

    /// <summary>发布歌曲元数据到系统媒体控制。</summary>
    public async Task UpdateMetadataAsync(string title, string artist, string album, byte[]? albumArtPng = null)
    {
        if (_session is null) return;
        try
        {
            // GlobalSystemMediaTransportControlsSession 通过 SystemMediaTransportControlsDisplayManager 更新元数据
            var updater = _session.TryGetMediaPropertiesAsync();
            if (updater is null) return;
            // 注意：GlobalSession 通过源应用更新；对于自发布场景需要额外处理。
            // 这里采用 "注册为媒体源" 的标准模式。
            // 实际元数据发布由播放引擎集成层（见 PlaybackService）完成。
            await Task.CompletedTask;
        }
        catch (Exception ex)
        {
            NeriPlayer.Core.Logging.AppLogger.Instance.Debug("SMTC metadata update failed: {Error}", ex.Message);
        }
    }

    /// <summary>发布播放状态。</summary>
    public async Task UpdatePlaybackStatusAsync(bool isPlaying)
    {
        if (_session is null) return;
        try
        {
            // 通过源应用注册的 MediaPlaybackCommandManager 发布状态
            // 具体见 PlaybackService 中的事件桥接
            await Task.CompletedTask;
        }
        catch { }
    }

    /// <summary>发布播放进度。</summary>
    public async Task UpdatePositionAsync(TimeSpan position, TimeSpan duration)
    {
        if (_session is null) return;
        await Task.CompletedTask;
    }

    private void OnSessionChanged(GlobalSystemMediaTransportControlsSessionManager sender,
        CurrentSessionChangedEventArgs args)
    {
        var oldSession = _session;
        if (oldSession is not null)
            oldSession.PlaybackInfoChanged -= OnPlaybackInfoChanged;

        _session = sender.GetCurrentSession();
        if (_session is not null)
            _session.PlaybackInfoChanged += OnPlaybackInfoChanged;
    }

    private void OnPlaybackInfoChanged(GlobalSystemMediaTransportControlsSession sender,
        PlaybackInfoChangedEventArgs args)
    {
        try
        {
            var controls = sender.GetPlaybackInfo().Controls;
            // 将按钮事件广播给订阅者（PlaybackService）
            // 按钮状态由 PlaybackService 根据 PlayerManager 状态动态映射
        }
        catch { }
    }

    public bool IsInitialized => _initialized;

    public async ValueTask DisposeAsync()
    {
        if (_session is not null)
            _session.PlaybackInfoChanged -= OnPlaybackInfoChanged;
        if (_manager is not null)
            _manager.CurrentSessionChanged -= OnSessionChanged;
        _session = null;
        _manager = null;
        await ValueTask.CompletedTask;
    }
}
```

> **关于 `GlobalSystemMediaTransportControlsSession` 的关键说明**：
> 该 API 设计为"观察并控制当前活跃的媒体会话"。在桌面应用中，我们的播放器
> 通过注册为 `MediaPlaybackCommandManager` 源（`Windows.Media.Playback`）来
> "成为"系统的当前媒体源，从而使 SMTC 控制我们的播放器。
> 上述代码框架为骨架；`Metadata/Position/PlaybackStatus` 的发布
> 通过 `PlaybackService` 中的 `MediaPlayer`（Windows.Media.Playback.MediaPlayer）
> 与 `MediaPlaybackCommandManager` 桥接完成（见 11.5）。

### 11.2 任务栏缩略图按钮

Avalonia 无内置 `TaskbarItemInfo`，使用 Win32 `ITaskbarList3`：

```
// 获取 hwnd：(nint)TopLevel.GetTopLevel(window)?.TryGetPlatformHandle()?.Handle
// ITaskbarList3::ThumbBarAddButtons(hwnd, 3, [⏮ ▶/⏸ ⏭])
// ITaskbarList3::ThumbBarUpdateButtons —— 播放状态切换图标
// 事件：WM_COMMAND (wParam = 0x8000 + 按钮索引) → 分发到 PlayerManager
// 参考实现：WindowsAPICodePack TaskbarManager 或手写 P/Invoke（约 100 行）
```
### 11.3 桌面歌词（UI/FloatingLyricsWindow.axaml）

对标 FloatingLyricsOverlayManager（Analysis.md 6.3 / Process.md 12.3）：

```xml
<Window xmlns="https://github.com/avaloniaui"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        x:Class="NeriPlayer.UI.FloatingLyricsWindow"
        ShowInTaskbar="False" Topmost="True"
        WindowStyle="None" TransparencyLevelHint="Transparent"
        Background="Transparent" Width="900" Height="80">
  <StackPanel VerticalAlignment="Center" Spacing="4">
    <TextBlock Name="CurrentLine" TextAlignment="Center"
               FontSize="20" Foreground="White" />
    <TextBlock Name="TranslatedLine" TextAlignment="Center"
               FontSize="13" Foreground="LightGray" />
  </StackPanel>
</Window>
```

- 拖动：`PointerPressed/PointerMoved` 记录偏移（对标 Android 长按拖动）
- 设置：字号/颜色/位置锁定/点击穿透

### 11.4 Toast 通知（Background/Notifications/ToastNotificationService.cs）

```csharp
using Windows.UI.Notifications;

namespace NeriPlayer.Background.Notifications;

public static class ToastNotificationService
{
    public static void Show(string title, string message, string? tag = null)
    {
        var xml = $@"""
        <toast><visual><binding template="ToastGeneric">
          <text>{System.Security.SecurityElement.Escape(title)}</text>
          <text>{System.Security.SecurityElement.Escape(message)}</text>
        </binding></visual></toast>
        """;
        var doc = new Windows.Data.Xml.Dom.XmlDocument();
        doc.LoadXml(xml);
        var toast = new ToastNotification(doc) { Tag = tag };
        ToastNotificationManager.CreateToastNotifier("NeriPlayer").Show(toast);
    }
}
```

触发场景：播放状态变化、下载完成（点击打开位置）、同步完成/失败（Process.md 12.5）。

### 11.5 后台播放服务（Background/Services/PlaybackService.cs）

对标 AudioPlayerService（Analysis.md 第 20 章）：

```csharp
using Microsoft.Extensions.Hosting;

namespace NeriPlayer.Background.Services;

/// <summary>主窗口关闭后保持播放（托盘 + SMTC 控制）</summary>
public sealed class PlaybackService : BackgroundService
{
    private readonly SmtcIntegration _smtc;
    private readonly Core.Player.PlayerManager _player;

    public PlaybackService(SmtcIntegration smtc, Core.Player.PlayerManager player)
    {
        _smtc = smtc;
        _player = player;
        _smtc.ButtonPressed += b =>
        {
            switch (b)
            {
                case SystemMediaTransportControlsButton.Play: _ = _player.ResumeAsync(); break;
                case SystemMediaTransportControlsButton.Pause: _ = _player.PauseAsync(); break;
                case SystemMediaTransportControlsButton.Next: _ = _player.NextAsync(); break;
                case SystemMediaTransportControlsButton.Previous: _ = _player.PreviousAsync(); break;
            }
        };
    }

    protected override Task ExecuteAsync(CancellationToken stoppingToken) => Task.CompletedTask;
}
```

> 空闲关闭策略（PlaybackServiceIdlePolicy 对标）：无播放任务超过
> `playback_service_idle_shutdown_minutes`（可配置，默认 10min）→ 停止服务。

**✅ 验收**
- [ ] SMTC 悬浮窗显示封面/标题/进度，媒体键可控制播放
- [ ] 主窗口关闭后播放不中断（托盘图标可恢复窗口）
- [ ] 桌面歌词窗口置顶、可拖动、随歌词滚动
- [ ] 下载完成弹出 Toast

---
## 十二、视觉效果（第 69-73 天）

### 12.1 流体背景 Shader（UI/Effects/FluidBackgroundEffect.cs）

对标 `hyper_background_effect.glsl`（Analysis.md 11.1：5 色点 + 音频响应 + 颗粒噪声）：

```csharp
using SkiaSharp;

namespace NeriPlayer.UI.Effects;

/// <summary>Skia RuntimeEffect 迁移 AGSL 流体背景（Process.md 11.3）</summary>
public sealed class FluidBackgroundEffect : IDisposable
{
    private const string ShaderSource = """
        uniform float u_time;
        uniform float uLevelEase;     // 音频能量位移
        uniform float uBeatEase;      // 节拍脉冲
        uniform vec2  uPoints[5];
        uniform vec4  uColors[5];
        uniform vec2  uResolution;

        float hash(vec2 p) {
            return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
        }

        float noise(vec2 p) {
            vec2 i = floor(p), f = fract(p);
            f = f * f * (3.0 - 2.0 * f);
            return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x),
                       mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
        }

        half4 main(float2 fragCoord) {
            vec2 uv = fragCoord / uResolution;
            float t = u_time * 0.15;
            vec4 acc = vec4(0.0);
            for (int i = 0; i < 5; i++) {
                vec2 p = uPoints[i] + vec2(sin(t + float(i) * 1.7),
                                           cos(t * 0.8 + float(i))) * 0.06 * uLevelEase;
                float d = distance(uv, p);
                acc += uColors[i] * exp(-d * 3.2) * (1.0 + uBeatEase * 0.25);
            }
            acc += vec4(noise(uv * 6.0 + t) * 0.06);
            return half4(acc.rgb, 1.0);
        }
        """;

    private readonly SKRuntimeEffect? _effect;

    public FluidBackgroundEffect()
    {
        _effect = SKRuntimeEffect.Create(ShaderSource, out var errors);
        if (errors?.Length > 0) AppLogger.Instance.Warning("shader errors: {Errors}", errors);
    }

    public SKPaint CreatePaint(float time, float level, float beat,
        SKPoint[] points, SKColor[] colors, SKSize resolution)
    {
        var uniforms = _effect!.Uniforms;
        uniforms["u_time"].AsScalar() = time;
        uniforms["uLevelEase"].AsScalar() = level;
        uniforms["uBeatEase"].AsScalar() = beat;
        uniforms["uResolution"].AsVec2() = new SKPoint(resolution.Width, resolution.Height);
        for (var i = 0; i < 5 && i < points.Length; i++)
        {
            uniforms["uPoints[" + i + "]"].AsVec2() = points[i];
            uniforms["uColors[" + i + "]"].AsVec4() =
                new SKColorF(colors[i].Red / 255f, colors[i].Green / 255f,
                             colors[i].Blue / 255f, colors[i].Alpha / 255f);
        }
        return new SKPaint { Shader = _effect.ToShader(false, uniforms) };
    }

    public bool IsSupported => _effect is not null;

    public void Dispose() => _effect?.Dispose();
}
```
### 12.2 封面取色（UI/Effects/CoverColorExtractor.cs）

对标 `Palette`（中位切分算法，Process.md 11.7）：

```csharp
using SkiaSharp;

namespace NeriPlayer.UI.Effects;

/// <summary>封面主色调提取：缩小 + 量化直方图（取最显著色）</summary>
public static class CoverColorExtractor
{
    public static SKColor ExtractDominant(SKBitmap cover)
    {
        using var small = cover.Resize(new SKImageInfo(32, 32), SKFilterQuality.Medium);
        var counts = new Dictionary<int, int>();
        for (var y = 0; y < small.Height; y++)
        for (var x = 0; x < small.Width; x++)
        {
            var c = small.GetPixel(x, y);
            if (c.Alpha < 128) continue;   // 忽略透明
            var r = c.Red >> 3, g = c.Green >> 3, b = c.Blue >> 3;   // 5 bit/通道
            var key = (r << 10) | (g << 5) | b;
            counts[key] = counts.GetValueOrDefault(key) + 1;
        }
        var top = counts.OrderByDescending(kv => kv.Value).FirstOrDefault();
        var k = top.Key;
        return new SKColor((byte)(((k >> 10) & 31) << 3 | 7),
                           (byte)(((k >> 5) & 31) << 3 | 7),
                           (byte)((k & 31) << 3 | 7));
    }

    /// <summary>由主色派生 5 组流体背景色点（HSV 偏移，对标 BgEffectPainter）</summary>
    public static SKColor[] DerivePalette(SKColor dominant)
    {
        dominant.ToHsv(out var h, out var s, out var v);
        return
        [
            dominant,
            SKColor.FromHsv((h + 30) % 360, Math.Min(1f, s * 1.2f), Math.Min(1f, v + 0.1f)),
            SKColor.FromHsv((h + 150) % 360, s, v),
            SKColor.FromHsv((h + 210) % 360, Math.Max(0, s - 0.3f), Math.Max(0, v - 0.15f)),
            SKColor.FromHsv(h, Math.Max(0, s - 0.2f), Math.Max(0, v - 0.3f)),
        ];
    }
}
```

### 12.3 毛玻璃（Process.md 11.4）

- 方案 A（静态）：`SKImageFilter.CreateBlur(radius, radius)` + 背景快照
- 方案 B（推荐）：Avalonia `ExperimentalAcrylicBorder`（实时模糊，性能好）：

```xml
<Window.Resources>
  <ExperimentalAcrylicMaterial x:Key="AcrylicBrush"
      BackgroundSource="Wallpaper" TintColor="#80000000" TintOpacity="0.5" MaterialOpacity="0.6" />
</Window.Resources>
<Border Background="{StaticResource AcrylicBrush}" />
```

### 12.4 音频可视化（WaveformVisualizer）

- 数据源：`FftAnalyzer`（第 6.4 节）→ 64 频带
- 绘制：Avalonia `DrawingContext` / Skia 直绘（`SKCanvas`）
- 效果：峰值保持 + 衰减（对标 WaveformSlider 预测动画 Analysis.md 18.2）

**✅ 验收**
- [ ] 流体背景随封面主色渲染、音频能量驱动位移/节拍脉冲
- [ ] 封面取色在 30 张测试封面上均得到合理主色
- [ ] 毛玻璃叠加不显著掉帧（≥ 55 FPS）
- [ ] GPU 不支持时自动降级为静态渐变

---
## 十三、安全与崩溃恢复（第 74-77 天）

### 13.1 凭据加密存储（Data/Auth/CredentialStore.cs）

对标 DPAPI（Process.md 13.1）：

```csharp
using System.Security.Cryptography;

namespace NeriPlayer.Data.Auth;

/// <summary>DPAPI（CurrentUser）加密 Cookie / 刷新 Token</summary>
public sealed class CredentialStore(string baseDir)
{
    private string Dir { get; } = baseDir;

    public void Save(string key, byte[] plaintext)
    {
        Directory.CreateDirectory(Dir);
        var encrypted = ProtectedData.Protect(plaintext, null, DataProtectionScope.CurrentUser);
        File.WriteAllBytes(Path.Combine(Dir, key), encrypted);
    }

    public byte[]? Load(string key)
    {
        var path = Path.Combine(Dir, key);
        if (!File.Exists(path)) return null;
        var encrypted = File.ReadAllBytes(path);
        try
        {
            return ProtectedData.Unprotect(encrypted, null, DataProtectionScope.CurrentUser);
        }
        catch (CryptographicException)
        {
            File.Delete(path);   // 密钥失效 → 清除
            return null;
        }
    }

    public void Delete(string key)
    {
        var path = Path.Combine(Dir, key);
        if (File.Exists(path)) File.Delete(path);
    }
}
```

存储布局（Process.md 13.1 对标）：

| 数据 | 文件 | 加密 |
|------|------|------|
| 三平台 Cookie | `%APPDATA%/NeriPlayer/secure/cookies.bin` | DPAPI |
| 刷新 Token | 同上 | DPAPI |
| 网易云设备 ID | `%APPDATA%/NeriPlayer/secure/device_id` | 明文 |

### 13.2 全局异常处理（Core/Diagnostics/ExceptionHandler.cs）

对标 Analysis.md 14.1：

```csharp
using System.Text;
using NeriPlayer.Core.Logging;

namespace NeriPlayer.Core.Diagnostics;

public static class ExceptionHandler
{
    private static string CrashDir =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "NeriPlayer", "crash");

    public static void Install()
    {
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
            WriteCrash("Unhandled", e.ExceptionObject as Exception);
        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            WriteCrash("UnobservedTask", e.Exception);
            e.SetObserved();
        };
    }

    private static void WriteCrash(string kind, Exception? ex)
    {
        try
        {
            Directory.CreateDirectory(CrashDir);
            var sb = new StringBuilder();
            sb.AppendLine($"Kind: {kind}");
            sb.AppendLine($"Time: {DateTimeOffset.Now:O}");
            sb.AppendLine($"Process: {Environment.ProcessId}");
            sb.AppendLine($"Thread: {Environment.CurrentManagedThreadId}");
            sb.AppendLine($"Exception: {ex}");
            var path = Path.Combine(CrashDir, $"crash-{DateTime.Now:yyyyMMdd-HHmmss}.log");
            File.WriteAllText(path, sb.ToString());
            AppLogger.Instance.Error("crash written: {Path}", path);
        }
        catch { /* 崩溃恢复自身失败则静默 */ }
    }

    /// <summary>启动时扫描：发现上次崩溃文件 → 返回给 UI 提示</summary>
    public static IReadOnlyList<string> ListRecentCrashes() =>
        Directory.Exists(CrashDir)
            ? Directory.GetFiles(CrashDir, "crash-*.log")
                .OrderByDescending(f => f).Take(5).ToList()
            : [];
}
```
### 13.3 安全模式（Core/Diagnostics/SafeModeManager.cs）

对标 SafeModeManager + AppStartupPlanner（Analysis.md 14.2 / Process.md 13.3）：

```csharp
namespace NeriPlayer.Core.Diagnostics;

public sealed class SafeModeManager(string stateFile)
{
    private const int CrashThreshold = 2;
    private int _consecutiveCrashCount;

    public bool IsSafeMode { get; private set; }

    /// <summary>App 启动早期调用：读计数 → 判定是否进入安全模式</summary>
    public bool EvaluateStartup()
    {
        LoadState();
        if (_consecutiveCrashCount >= CrashThreshold)
        {
            IsSafeMode = true;
            _consecutiveCrashCount = 0;   // 进入安全模式后复位
            SaveState();
        }
        return IsSafeMode;
    }

    public void RecordStartupSuccess()
    {
        _consecutiveCrashCount = 0;
        SaveState();
    }

    public void RecordCrash() => SaveState(++_consecutiveCrashCount);

    private void SaveState(int count = -1)
    {
        if (count >= 0) _consecutiveCrashCount = count;
        File.WriteAllText(stateFile, _consecutiveCrashCount.ToString());
    }

    private void LoadState()
    {
        if (File.Exists(stateFile) &&
            int.TryParse(File.ReadAllText(stateFile).Trim(), out var c))
            _consecutiveCrashCount = c;
    }
}
```

### 13.4 崩溃恢复流程（对标 Analysis.md 2.1 启动链路）

```
App.OnStartup
 ├── 1. ExceptionHandler.Install()
 ├── 2. SafeModeManager.EvaluateStartup()  → 连续 2 次崩溃进入安全模式
 ├── 3. 正常路径：初始化 DI → 预热缓存 → 恢复上次播放状态（15s 间隔持久化）
 ├── 4. 安全模式：禁用自动同步 / 禁用 YouTube PoToken 自研解析（走回退）/
 │            清空异常缓存 → 显示 SafeModeScreen（可从设置退出）
 └── 5. 退出前 PlayerStatePersistence.Save()（命令触发即时持久化）
```

**✅ 验收**
- [ ] Cookie 经 DPAPI 加密后不可明文读取；同用户可解密
- [ ] 手动制造未捕获异常 → `crash/*.log` 生成
- [ ] 连续 2 次崩溃后下次启动进入安全模式并禁用自动同步

---
## 十四、测试体系（贯穿全程）

### 14.1 测试项目布局

| 项目 | 类型 | 覆盖 |
|------|------|------|
| `NeriPlayer.Core.Tests` | 单元 | 歌词解析、StableKey、曲目去重、失败策略、状态机、音效、下载队列、同步合并、搜索合并 |
| `NeriPlayer.Data.Tests` | 集成 | EF Core 迁移、Repository CRUD、播放状态 round-trip |
| `NeriPlayer.Api.Tests` | API | Mock HTTP（自定义 HttpMessageHandler）解析三平台响应 |

### 14.2 Mock HTTP 示例（tests/NeriPlayer.Api.Tests）

```csharp
using System.Net;
using NeriPlayer.Core.Api.Netease;
using Xunit;

namespace NeriPlayer.Api.Tests;

public sealed class MockHttpHandler : HttpMessageHandler
{
    public Func<HttpRequestMessage, HttpResponseMessage>? Responder { get; set; }

    protected override Task<HttpResponseMessage> SendAsync(
        HttpRequestMessage request, CancellationToken ct)
    {
        var resp = Responder?.Invoke(request) ?? new HttpResponseMessage(HttpStatusCode.OK)
        {
            Content = new StringContent("{}"),
        };
        return Task.FromResult(resp);
    }
}

public class NeteaseSearchTests
{
    [Fact]
    public async Task SearchAsync_ParsesSongs()
    {
        var handler = new MockHttpHandler
        {
            Responder = _ => new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StringContent(
                    """{"result":{"songs":[{"id":123,"name":"歌","ar":[{"name":"艺人"}],"al":{"name":"专辑"}}]}}"""),
            }
        };
        var client = new NeteaseClient(new HttpClient(handler));
        var result = await client.SearchAsync("测试");
        Assert.NotEmpty(result.Songs);
    }
}
```

### 14.3 集成测试（tests/NeriPlayer.Data.Tests）

```csharp
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;
using NeriPlayer.Data.Repositories;
using Xunit;

namespace NeriPlayer.Data.Tests;

public class SongRepositoryTests
{
    private static NeriDbContext CreateDb()
    {
        var conn = new SqliteConnection("Data Source=:memory:");
        conn.Open();
        var options = new DbContextOptionsBuilder<NeriDbContext>()
            .UseSqlite(conn).Options;
        var db = new NeriDbContext(options);
        db.Database.EnsureCreated();
        return db;
    }

    [Fact]
    public async Task Upsert_SameStableKey_NoDuplicate()
    {
        await using var db = CreateDb();
        var repo = new SongRepository(db);
        var song = new SongEntity { StableKey = "netease|1", Name = "A", Artist = "B", Album = "C" };
        var id1 = await repo.UpsertAsync(song);
        var id2 = await repo.UpsertAsync(song with { Name = "A2" });
        Assert.Equal(id1, id2);
        Assert.Single(await db.Songs.ToListAsync());
    }
}
```

### 14.4 Avalonia Headless UI 测试

```powershell
dotnet add src/NeriPlayer.UI package Avalonia.Headless.XUnit
```

```csharp
using Avalonia.Headless.XUnit;
using NeriPlayer.UI.Views;
using Xunit;

public class MainWindowTests
{
    [AvaloniaFact]
    public void MainWindow_Initializes()
    {
        var window = new MainWindow();
        Assert.NotNull(window);
    }
}
```
### 14.5 性能基准（Process.md 14.4）

| 场景 | 目标 | 验证方式 |
|------|------|----------|
| 冷启动到可播放 | ≤ 3s | `Stopwatch` 日志埋点 |
| 10,000 首本地扫描 | ≤ 30s / 增量 ≤ 2s | 基准测试脚本 |
| 内存（5000 首歌单） | ≤ 400MB | `dotnet-counters` |
| 连续播放 48h | 无泄漏无崩溃 | 自动化 + 崩溃目录监控 |
| 封面缓存二次加载 | ≤ 100ms | Headless 性能测试 |

```powershell
# 48h 稳定性脚本（PowerShell 计划任务）
$endTime = (Get-Date).AddHours(48)
while ((Get-Date) -lt $endTime) {
  dotnet run --project src/NeriPlayer.App -- --autoplay
  Start-Sleep -Seconds 60
  # 检查 crash 目录是否有新文件 → 失败标记
}
```

**✅ 验收**
- [ ] `dotnet test` 全绿（单元 + 集成 + API）
- [ ] Headless UI 测试可运行
- [ ] 性能基准数据记录在 `docs/benchmark.md`

---

## 十五、打包与发布（第 78-84 天）

### 15.1 便携版发布

```powershell
# 发布到 publish 目录（含 VLC 依赖）
$VLC = 'D:\libs\vlc-3.0.20'
dotnet publish src/NeriPlayer.App -c Release -r win-x64 --self-contained true -o publish

# 复制 VLC 运行库
Copy-Item "$VLC\libvlc.dll" publish\
Copy-Item "$VLC\libvlccore.dll" publish\
Copy-Item "$VLC\plugins" publish\plugins -Recurse

# 验证
.\publish\NeriPlayer.App.exe
```

### 15.2 MSIX 打包（可选）

```powershell
# 使用 Windows Application Packaging 项目（Package.appxmanifest + 图标 + 自动更新）
# 或先用 Inno Setup/NSIS 打传统安装包（Process.md 15.1）
```

### 15.3 GitHub Actions CI/CD

创建 `.github/workflows/build.yml`：

```yaml
name: build

on:
  push:
    tags: ["v*"]
  workflow_dispatch:

jobs:
  windows-build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-dotnet@v4
        with:
          dotnet-version: "8.0.x"
      - name: Download LibVLC
        run: |
          Invoke-WebRequest https://get.videolan.org/vlc/3.0.20/win64/vlc-3.0.20-win64.zip -OutFile vlc.zip
          Expand-Archive vlc.zip .
          Copy-Item "vlc-3.0.20" "$env:LOCALAPPDATA\vlc" -Recurse
      - name: Restore & Build
        run: dotnet build NeriPlayer.Windows.sln -c Release
      - name: Test
        run: dotnet test NeriPlayer.Windows.sln -c Release --no-build
      - name: Publish
        run: |
          dotnet publish src/NeriPlayer.App -c Release -r win-x64 --self-contained true -o publish
          Copy-Item "$env:LOCALAPPDATA\vlc\libvlc.dll" publish\
          Copy-Item "$env:LOCALAPPDATA\vlc\libvlccore.dll" publish\
          Copy-Item "$env:LOCALAPPDATA\vlc\plugins" publish\plugins -Recurse
      - uses: actions/upload-artifact@v4
        with:
          name: NeriPlayer-win-x64
          path: publish
```
### 15.4 版本管理

```powershell
# 版本号：主.次.修订（Git tag 驱动）
$ver = (git describe --tags --abbrev=0).TrimStart('v')
# 写入 Directory.Build.props：<Version>$ver</Version>
# 变更日志：docs/changelog.md
```

**✅ 验收**
- [ ] `publish\NeriPlayer.App.exe` 在干净 Windows 10/11 可直接运行
- [ ] CI 流水线测试通过并产出便携版 + 日志
- [ ] Tag `v1.0.0` 发布后自动产出 release artifact

---

## 总里程碑对照（Process.md 第 16 章）

| 里程碑 | 时间 | 对应章节 | 标志 |
|--------|------|----------|------|
| **M0 骨架** | 第 1-2 周 | 一、二、三 | `dotnet build` + 测试全绿 |
| **M1 可播放** | 第 3-4 周 | 四、五 | 本地文件可播放 + 状态机 |
| **M2 多源在线** | 第 5-6 周 | 六、七 | 三平台搜索/播放/歌词 |
| **M3 数据闭环** | 第 7-8 周 | 八、九 | 下载 + 同步闭环 |
| **M4 桌面体验** | 第 9-11 周 | 十、十一、十二 | 完整 UI + 系统集成 |
| **M5 发布** | 第 12 周 | 十三、十四、十五 | 崩溃防护 + 测试 + 打包 |

## 风险提示（Process.md 第 18 章 摘要）

| 风险 | 缓解 |
|------|------|
| 网易云/B站接口变更 | 加密/签名参数集中在 `NeteaseCrypto`/`WbiSignature`，改动面小 |
| YouTube PoToken 失效 | 多级回退链（NewPipe 兜底）+ 安全模式禁用自研解析 |
| VLC 音效能力受限 | 关键音效走 NAudio WASAPI 独占引擎（ISampleProvider 管道） |
| 3 平台 API 解析工作量大 | 先做搜索+URL+歌词三个最小闭环，其余 P1 |

*start.md · 详细实现过程 · 基于 Analysis.md（24 章）+ Process.md（19 章）编制*
