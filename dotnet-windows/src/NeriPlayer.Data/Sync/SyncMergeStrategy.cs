using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Data.Sync;

/// <summary>同步记录携带因果 Token（对标 SyncCausalToken / Analysis.md 9.3 / Process.md 10.3）</summary>
public sealed record SyncToken(string SongId, string BaseVersion, string OperationId);

/// <summary>合并裁决结果</summary>
public sealed record SyncMergeDecision(bool TakeLocal, string? Reason);

/// <summary>
/// 同步合并策略（对标 Analysis.md 9.3 各 MergePolicy）。
/// 冲突裁决：因果序优先，其次时间戳。
/// </summary>
public static class SyncMergeStrategy
{
    /// <summary>
    /// 因果冲突裁决：返回正数表示 a 胜出，负数表示 b 胜出，0 表示相等。
    /// </summary>
    public static int Compare(SyncToken a, SyncToken b, long aTimestamp, long bTimestamp)
    {
        // a 是 b 的祖先（a 的操作被 b 基于）→ b 胜出
        if (a.OperationId == b.BaseVersion) return 1;
        // b 是 a 的祖先 → a 胜出
        if (b.OperationId == a.BaseVersion) return -1;
        // 无因果关系 → 时间戳裁决
        return aTimestamp.CompareTo(bTimestamp);
    }

    /// <summary>歌单合并：按 stableKey 去重 + updated_at 冲突裁决（对标 SyncPlaylistSongMergePolicy）</summary>
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

    /// <summary>播放统计合并：取较大计数（对标 SyncPlaybackStatsMergePolicy）</summary>
    public static long MergeCount(long local, long remote) => Math.Max(local, remote);

    /// <summary>歌曲元数据合并：有自定义字段者胜出，否则取近者（对标 SyncSongMetadataMergePolicy）</summary>
    public static SongItem MergeSong(SongItem local, SongItem remote)
    {
        if (!string.IsNullOrEmpty(remote.CustomName) || !string.IsNullOrEmpty(remote.CustomArtist))
            return remote;
        if (!string.IsNullOrEmpty(local.CustomName) || !string.IsNullOrEmpty(local.CustomArtist))
            return local;
        return remote.AddedAt > local.AddedAt ? remote : local;
    }
}
