namespace NeriPlayer.Data.Entities;

/// <summary>
/// 核心歌曲实体（对标 Room SongItem / Analysis.md 21.1）。
/// 索引：stable_key 唯一，支持跨版本去重与同步主键。
/// </summary>
public sealed class SongEntity
{
    /// <summary>数据库自增主键</summary>
    public long Id { get; set; }

    /// <summary>跨版本稳定键：local|{path} / netease|{audioId} / bilibili|{audioId}|{subId} / ytm|{videoId}</summary>
    public required string StableKey { get; set; }

    public required string Name { get; set; }
    public required string Artist { get; set; }
    public required string Album { get; set; }
    public long AlbumId { get; set; }
    public long DurationMs { get; set; }

    public string? CoverUrl { get; set; }
    public string? MediaUri { get; set; }
    public string? StreamUrl { get; set; }

    // 平台标识（channelId / audioId / subAudioId）
    public string? ChannelId { get; set; }
    public string? AudioId { get; set; }
    public string? SubAudioId { get; set; }

    // 歌词
    public string? MatchedLyric { get; set; }
    public string? MatchedTranslatedLyric { get; set; }
    public string? MatchedLyricSource { get; set; }
    public long UserLyricOffsetMs { get; set; }

    // 自定义元数据（用户覆盖值，与 Original* 分离）
    public string? CustomName { get; set; }
    public string? CustomArtist { get; set; }
    public string? CustomCoverUrl { get; set; }
    public string? OriginalName { get; set; }
    public string? OriginalArtist { get; set; }

    // 本地文件
    public string? LocalFileName { get; set; }
    public string? LocalFilePath { get; set; }

    /// <summary>添加到库的时间（Unix ms）</summary>
    public long AddedAt { get; set; }
}