using NeriPlayer.Core.Logging;

namespace NeriPlayer.Background.Notifications;

/// <summary>
/// 通知服务（对标 Process.md 12.5 / start.md 11.4）。
///
/// 完整实现需 Windows.UI.Notifications.ToastNotificationManager（要求 TFM 为
/// net8.0-windows10.0.19041.0 且安装完整 Windows SDK）。当前 net8.0 下以
/// AppLogger 记录，接口形状与 WinRT 版本一致，待 SDK 就绪后启用真实系统 Toast。
/// </summary>
public static class ToastNotificationService
{
    /// <summary>显示通知（当前为日志记录，WinRT 启用后为系统 Toast）。</summary>
    public static void Show(string title, string message, string? tag = null)
    {
        AppLogger.Instance.Information("Toast [{Tag}]: {Title} — {Message}", tag ?? "-", title, message);
    }

    public static void ShowNowPlaying(string songTitle, string artist)
        => Show("正在播放", $"{songTitle} - {artist}", "now-playing");

    public static void ShowDownloadCompleted(string songTitle)
        => Show("下载完成", songTitle, "download-completed");

    public static void ShowSyncResult(bool success, string? detail = null)
        => Show(
            success ? "同步成功" : "同步失败",
            detail ?? (success ? "数据已同步" : "请检查网络或配置"),
            "sync-result");
}
