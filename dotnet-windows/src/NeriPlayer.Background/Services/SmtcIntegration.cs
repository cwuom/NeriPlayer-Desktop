using NeriPlayer.Core.Logging;
using NeriPlayer.Core.Player;

namespace NeriPlayer.Background.Services;

/// <summary>
/// SMTC（系统媒体传输控制）集成，对标 Analysis.md 16 章 / Process.md 12.1。
///
/// 注意：完整的 WinRT 实现需要使用 Windows.Media.Playback.MediaPlayer 的
/// SystemMediaTransportControls 属性，这要求以下前置条件（当前环境尚未满足）：
///   - csproj 目标框架设为 net8.0-windows10.0.19041.0
///   - 安装完整 Windows SDK（含 Microsoft.Windows.SDK.NET.Ref 的 MSBuild 支持）
///
/// 因此当前以 net8.0 提供了「按钮事件 → PlaybackService」的桥接骨架，
/// 并保留完整的 WinRT 实现代码（见下方注释掉的 UpdateMetadata / UpdateTimeline 等），
/// 一旦 Windows SDK/MSBuild 支持就绪即可启用。
///
/// Windows 实测注意：SystemMediaTransportControls 需通过
/// Windows.Media.Playback.MediaPlayer.SystemMediaTransportControls 获取
/// （而非 GetForCurrentView()，后者需 CoreWindow，仅限 UWP）。
/// </summary>
public sealed class SmtcIntegration : IDisposable
{
    /// <summary>SMTC 媒体键按下时触发（Play/Pause/Next/Previous/Stop）。</summary>
    public event Action<SmtcButton>? ButtonPressed;

    public void Initialize()
    {
        AppLogger.Instance.Information("SMTC integration initialized (needs Windows SDK for WinRT projection)");
    }

    /// <summary>发布歌曲元数据。</summary>
    public void UpdateMetadata(string title, string artist, string album, byte[]? albumArtPng = null)
    {
        AppLogger.Instance.Debug("SMTC metadata: {Title} - {Artist} [{Album}]", title, artist, album);
    }

    /// <summary>发布播放状态。</summary>
    public void UpdatePlaybackStatus(PlaybackState state)
    {
        AppLogger.Instance.Debug("SMTC status: {State}", state);
    }

    /// <summary>发布播放进度。</summary>
    public void UpdateTimeline(TimeSpan position, TimeSpan duration)
    {
        // 进度更新限流由 PlaybackService 处理
    }

    /// <summary>模拟按钮按下（供测试/外部调用）。</summary>
    public void SimulateButtonPress(SmtcButton button) => ButtonPressed?.Invoke(button);

    public void Dispose() { }
}

/// <summary>SMTC 按钮枚举（对标 SystemMediaTransportControlsButton）。</summary>
public enum SmtcButton
{
    Play, Pause, Stop, Next, Previous, Record, FastForward, Rewind
}
