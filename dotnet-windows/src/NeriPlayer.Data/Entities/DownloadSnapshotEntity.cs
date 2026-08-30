namespace NeriPlayer.Data.Entities;

/// <summary>
/// 下载快照实体（对标 Analysis.md 24.2 download_snapshots 索引表）。
/// 复合主键：(RootKey, Bucket, EntryKey)，用于同步时比对增量。
/// </summary>
public sealed class DownloadSnapshotEntity
{
    /// <summary>根键（如 "netease", "bilibili"）</summary>
    public required string RootKey { get; set; }

    /// <summary>分区键（如歌单 ID）</summary>
    public required string Bucket { get; set; }

    /// <summary>条目键（如歌曲 StableKey）</summary>
    public required string EntryKey { get; set; }

    /// <summary>本地文件路径</summary>
    public required string LocalPath { get; set; }

    /// <summary>文件哈希（SHA256）</summary>
    public string? FileHash { get; set; }

    /// <summary>快照时间（Unix ms）</summary>
    public long SnapshotAt { get; set; }
}