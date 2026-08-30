using NeriPlayer.Core.Player.Model;
using NeriPlayer.Data.Sync;
using Xunit;

namespace NeriPlayer.Data.Tests;

/// <summary>同步合并策略测试（对标 start.md 9.4 / Analysis.md 9.3）</summary>
public class SyncMergeStrategyTests
{
    private static SongItem MakeSong(long id, string channel, string row, long addedAt) => new()
    {
        Id = id, Name = $"{channel}-{row}", Artist = "A", Album = "B",
        ChannelId = channel, AudioId = row, AddedAt = addedAt,
    };

    // ── 因果 Token 冲突裁决 ─────────────────────────────────────────

    [Fact]
    public void Compare_LocalAncestorOfRemote_TakesRemote()
    {
        var a = new SyncToken("s1", "", "op1");
        var b = new SyncToken("s1", "op1", "op2");   // b 基于 a
        // a 是 b 的祖先 → b 胜出（返回正数）
        Assert.True(SyncMergeStrategy.Compare(a, b, 100, 200) > 0);
    }

    [Fact]
    public void Compare_RemoteAncestorOfLocal_TakesLocal()
    {
        var a = new SyncToken("s1", "op2", "op1");   // a 基于 b=op2
        var b = new SyncToken("s1", "", "op2");
        // b 是 a 的祖先 → a 胜出（返回负数的反向由调用方解释：Compare(b,a) 应 >0）
        Assert.True(SyncMergeStrategy.Compare(b, a, 200, 100) > 0);
    }

    [Fact]
    public void Compare_NoCausalRelation_FallsBackToTimestamp()
    {
        var a = new SyncToken("s1", "", "op1");
        var b = new SyncToken("s1", "", "op2");      // 无因果关系
        // 时间戳裁决：a(200) > b(100) → a 胜出
        Assert.True(SyncMergeStrategy.Compare(a, b, 200, 100) > 0);
        Assert.True(SyncMergeStrategy.Compare(b, a, 100, 200) < 0);
    }

    // ── 歌单合并 ─────────────────────────────────────────────────────

    [Fact]
    public void MergePlaylists_DeduplicatesByStableKey()
    {
        var local = new List<SongItem> { MakeSong(1, "netease", "100", 100) };
        var remote = new List<SongItem> { MakeSong(2, "netease", "100", 50) };  // 同 key，更旧
        var merged = SyncMergeStrategy.MergePlaylists(local, remote);
        Assert.Single(merged);
        Assert.Equal("netease-100", merged[0].Name);
    }

    [Fact]
    public void MergePlaylists_NewerWins_OnConflict()
    {
        var local = new List<SongItem> { MakeSong(1, "netease", "100", 100) };
        var remote = new List<SongItem> { MakeSong(2, "netease", "100", 300) };  // 同 key，更新
        var merged = SyncMergeStrategy.MergePlaylists(local, remote);
        Assert.Single(merged);
        Assert.Equal(300, merged[0].AddedAt);
    }

    [Fact]
    public void MergePlaylists_UnionsDistinctSongs()
    {
        var local = new List<SongItem> { MakeSong(1, "netease", "100", 100) };
        var remote = new List<SongItem> { MakeSong(2, "bili", "200", 50) };
        var merged = SyncMergeStrategy.MergePlaylists(local, remote);
        Assert.Equal(2, merged.Count);
    }

    // ── 计数合并 ─────────────────────────────────────────────────────

    [Fact]
    public void MergeCount_TakesMax()
    {
        Assert.Equal(50, SyncMergeStrategy.MergeCount(10, 50));
        Assert.Equal(10, SyncMergeStrategy.MergeCount(10, 5));
    }
}
