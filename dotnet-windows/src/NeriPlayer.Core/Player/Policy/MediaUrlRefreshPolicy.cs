namespace NeriPlayer.Core.Player.Policy;

/// <summary>
/// URL 10min 过期 + 10s 冷却防抖（对标 Analysis.md 24.1 MEDIA_URL_STALE_MS / URL_REFRESH_COOLDOWN_MS）
/// </summary>
public sealed class MediaUrlRefreshPolicy
{
    private static readonly TimeSpan Stale = TimeSpan.FromMinutes(10);
    private static readonly TimeSpan Cooldown = TimeSpan.FromSeconds(10);

    // 可替换时钟，便于单测免 Sleep
    private readonly Func<DateTimeOffset> _clock;
    private DateTimeOffset _lastRefreshAt = DateTimeOffset.MinValue;

    public MediaUrlRefreshPolicy(Func<DateTimeOffset>? clock = null)
    {
        _clock = clock ?? (() => DateTimeOffset.UtcNow);
    }

    /// <summary>
    /// 判断是否需要刷新 URL。
    /// </summary>
    /// <param name="urlCreatedAt">上次获取 URL 的时间，null 表示未知（需刷新）</param>
    /// <returns>true = 应调用 API 刷新；false = 可继续使用旧 URL</returns>
    public bool ShouldRefresh(DateTimeOffset? urlCreatedAt)
    {
        var now = _clock();
        if (urlCreatedAt is not null && now - urlCreatedAt < Stale) return false;
        if (now - _lastRefreshAt < Cooldown) return false;
        _lastRefreshAt = now;
        return true;
    }

    /// <summary>上次刷新时间（测试用）</summary>
    public DateTimeOffset LastRefreshAt => _lastRefreshAt;
}
