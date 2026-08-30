using NeriPlayer.Core.Player.Policy;

namespace NeriPlayer.Core.Tests;

public class PolicyTests
{
    // ── PlaybackFailurePolicy ────────────────────────────────────────

    [Fact]
    public void FailurePolicy_NineFailures_ShouldNotStop()
    {
        var policy = new PlaybackFailurePolicy();
        for (int i = 0; i < 8; i++)
            policy.RecordFailure(out _);

        policy.RecordFailure(out var shouldStop); // 第 9 次
        Assert.False(shouldStop);
    }

    [Fact]
    public void FailurePolicy_TenFailures_ShouldStop()
    {
        var policy = new PlaybackFailurePolicy();
        for (int i = 0; i < 9; i++)
            policy.RecordFailure(out _);

        policy.RecordFailure(out var shouldStop); // 第 10 次
        Assert.True(shouldStop);
    }

    [Fact]
    public void FailurePolicy_RecordSuccess_ResetsCounter()
    {
        var policy = new PlaybackFailurePolicy();
        for (int i = 0; i < 8; i++)
            policy.RecordFailure(out _);

        policy.RecordSuccess();
        policy.RecordFailure(out var shouldStop);
        Assert.False(shouldStop);
        Assert.Equal(1, policy.FailureCount);
    }

    // ── TrackEndDedupPolicy ──────────────────────────────────────────

    [Fact]
    public void DedupPolicy_FirstCall_Allows()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new TrackEndDedupPolicy(() => now);
        Assert.True(policy.TryConsume());
    }

    [Fact]
    public void DedupPolicy_ImmediateSecondCall_Blocks()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new TrackEndDedupPolicy(() => now);
        policy.TryConsume();
        Assert.False(policy.TryConsume()); // 同一时间戳 → 被去重
    }

    [Fact]
    public void DedupPolicy_After500ms_Allows()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new TrackEndDedupPolicy(() => now);
        policy.TryConsume();

        now = now.AddMilliseconds(501);
        Assert.True(policy.TryConsume());
    }

    [Fact]
    public void DedupPolicy_Within500ms_Blocks()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new TrackEndDedupPolicy(() => now);
        policy.TryConsume();

        now = now.AddMilliseconds(499);
        Assert.False(policy.TryConsume());
    }

    // ── MediaUrlRefreshPolicy ────────────────────────────────────────

    [Fact]
    public void UrlRefresh_NullCreatedAt_ShouldRefresh()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new MediaUrlRefreshPolicy(() => now);
        Assert.True(policy.ShouldRefresh(null));
    }

    [Fact]
    public void UrlRefresh_FreshUrl_ShouldNotRefresh()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new MediaUrlRefreshPolicy(() => now);
        var urlCreated = now.AddMinutes(-5); // 5 min ago, within 10min stale window
        Assert.False(policy.ShouldRefresh(urlCreated));
    }

    [Fact]
    public void UrlRefresh_StaleUrl_ShouldRefresh()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new MediaUrlRefreshPolicy(() => now);
        var urlCreated = now.AddMinutes(-11); // 11 min ago, beyond 10min stale window
        Assert.True(policy.ShouldRefresh(urlCreated));
    }

    [Fact]
    public void UrlRefresh_CooldownBlocks_RapidCalls()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new MediaUrlRefreshPolicy(() => now);
        var stale = now.AddMinutes(-15);

        Assert.True(policy.ShouldRefresh(stale));  // 首次 → 刷新
        Assert.False(policy.ShouldRefresh(stale)); // 10s 冷却中 → 不刷新
    }

    [Fact]
    public void UrlRefresh_AfterCooldown_AllowsRefresh()
    {
        var now = DateTimeOffset.UtcNow;
        var policy = new MediaUrlRefreshPolicy(() => now);
        var stale = now.AddMinutes(-15);

        policy.ShouldRefresh(stale); // 首次 → 刷新，记录 _lastRefreshAt = now

        now = now.AddSeconds(11); // 11s 后
        Assert.True(policy.ShouldRefresh(stale)); // 冷却已过 → 允许刷新
    }
}
