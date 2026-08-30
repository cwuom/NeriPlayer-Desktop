namespace NeriPlayer.Data.Entities;

/// <summary>
/// 播放统计主记录（PK song_id）。
/// 对标 Room PlaybackStatsEntity / Analysis.md 13.3 / Process.md 5.3。
/// </summary>
public sealed class PlaybackStatsEntity
{
    /// <summary>歌曲 ID（主键）</summary>
    public long SongId { get; set; }

    /// <summary>累计播放次数</summary>
    public long PlayCount { get; set; }

    /// <summary>累计播放时长（毫秒）</summary>
    public long TotalPlayMs { get; set; }

    /// <summary>最后一次播放时间（Unix ms）</summary>
    public long LastPlayedAt { get; set; }
}

/// <summary>
/// 每日播放统计分片桶（PK(SongId, DayKey)）。
/// 对标 Room PlaybackStatDailyCounterShardEntity，缓解高频写放大（Analysis.md 21.2）。
/// </summary>
public sealed class StatBucketEntity
{
    public long SongId { get; set; }

    /// <summary>日期键，格式 yyyyMMdd</summary>
    public long DayKey { get; set; }

    /// <summary>当日播放次数</summary>
    public long PlayCount { get; set; }

    /// <summary>当日播放时长（毫秒）</summary>
    public long ListenMs { get; set; }
}