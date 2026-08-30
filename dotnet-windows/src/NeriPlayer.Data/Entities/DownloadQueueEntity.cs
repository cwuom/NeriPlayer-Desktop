namespace NeriPlayer.Data.Entities;

/// <summary>
/// 下载队列持久化实体（对标 Analysis.md 24.2 download_queue 索引表）。
/// 待下载任务持久化，支持应用重启后恢复队列。
/// </summary>
public sealed class DownloadQueueEntity
{
    /// <summary>数据库自增主键</summary>
    public long Id { get; set; }

    /// <summary>对应的歌曲 StableKey</summary>
    public required string StableKey { get; set; }

    /// <summary>歌曲名称（冗余，用于 UI 展示）</summary>
    public required string SongName { get; set; }

    /// <summary>质量标识</summary>
    public string? QualityKey { get; set; }

    /// <summary>目标路径</summary>
    public required string TargetPath { get; set; }

    /// <summary>优先级（0=普通, 1=高优先）</summary>
    public int Priority { get; set; }

    /// <summary>状态（0=pending, 1=downloading, 2=completed, 3=failed）</summary>
    public int Status { get; set; }

    /// <summary>入队时间（Unix ms）</summary>
    public long EnqueuedAt { get; set; }
}