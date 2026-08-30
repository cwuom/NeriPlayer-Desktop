namespace NeriPlayer.Core.Player.Policy;

/// <summary>
/// 500ms 相邻结束事件去重（对标 Analysis.md 24.1 GUARD_WINDOW）
/// VLC 的 EndReached 有时会重复触发，需在 500ms 窗口内去重。
/// </summary>
public sealed class TrackEndDedupPolicy
{
    private static readonly TimeSpan GuardWindow = TimeSpan.FromMilliseconds(500);

    // 可替换时钟，便于单测免 Sleep（默认使用系统时钟）
    private readonly Func<DateTimeOffset> _clock;
    private DateTimeOffset _lastEndAt;

    public TrackEndDedupPolicy(Func<DateTimeOffset>? clock = null)
    {
        _clock = clock ?? (() => DateTimeOffset.UtcNow);
    }

    /// <summary>
    /// 尝试消费一次结束事件。返回 true 表示允许通过（非重复），false 表示被去重过滤。
    /// </summary>
    public bool TryConsume()
    {
        var now = _clock();
        if (now - _lastEndAt < GuardWindow) return false;
        _lastEndAt = now;
        return true;
    }
}
