# NeriPlayer Windows (.NET)

> 将 NeriPlayer 移植到 Windows 桌面端的 **.NET 8 + Avalonia UI** 实现方案。

## 项目定位

这是 NeriPlayer (Android) 的 Windows 桌面端 **.NET 技术栈**实现，与官方
[NeriPlayer-Desktop](https://github.com/cwuom/NeriPlayer-Desktop)
（Tauri 2 + Rust + Vue 3）为平行独立的两套方案。

- 本仓库是个人开发主仓库（`origin`），同步展示于独立远端
- 同时以 `dotnet-windows/` 子目录的形式提交到上游 PR，作为参考实现对比
- 详细方案见 `Process.md`（移植方案 19 章）、`start.md`（逐步骤执行）、
  `Analysis.md`（上游源码分析 24 章）

## 技术栈

| 组件 | 选型 |
|------|------|
| 运行时 | .NET 8 LTS |
| UI | Avalonia UI 11.x |
| 播放引擎 | LibVLCSharp 3.8.0（VLC 3.0.x） |
| 数据库 | EF Core 8 + SQLite |
| 音效 | NAudio (WASAPI) / Biquad 滤波器 |
| 系统集成 | SMTC / 托盘 / Toast |

## 项目结构

```
src/
├── NeriPlayer.App/         主应用入口（Avalonia Desktop）
├── NeriPlayer.Core/        核心业务层（播放/歌词/下载/策略）
├── NeriPlayer.Data/        数据层（EF Core / 同步）
├── NeriPlayer.UI/          UI 层（Avalonia 视图）
└── NeriPlayer.Background/  后台服务（SMTC / 托盘）
tests/                      单元测试（xunit）
```

## 构建与运行

```powershell
dotnet build NeriPlayer.Windows.sln
dotnet test tests/NeriPlayer.Core.Tests
dotnet run --project src/NeriPlayer.App
```

## 协作流程

每个章节完成后，按 `start.md` 第 0 章「协作与提交流程」执行：
本地提交 → 推送独立远端 → 同步到 fork `dotnet-windows/` → 更新上游 PR。
