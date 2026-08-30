namespace NeriPlayer.Data.Entities;

/// <summary>
/// 下载任务持久化实体（对标 Process.md 9.1 / Analysis.md 24.2 downloads 主目录清单）。
/// 索引：stable_key 唯一，支持按歌曲定位下载记录。
/// </summary>
public sealed class DownloadEntity
{
    /// <summary>数据库自增主键</summary>
    public long Id { get; set; }

    /// <summary>跨版本稳定键（与 SongEntity.StableKey 对应）</summary>
    public required string StableKey { get; set; }

    /// <summary>本地文件路径</summary>
    public required string LocalPath { get; set; }

    /// <summary>下载状态（0=Queued, 1=Downloading, 2=Paused, 3=Completed, 4=Failed, 5=Cancelled）</summary>
    public int Status { get; set; }

    /// <summary>质量标识（如 "exhigh", "lossless"）</summary>
    public string? QualityKey { get; set; }

    /// <summary>已下载字节数（断点续传用）</summary>
    public long BytesReceived { get; set; }

    /// <summary>总字节数（若服务端返回 Content-Length）</summary>
    public long? TotalBytes { get; set; }

    /// <summary>创建时间（Unix ms）</summary>
    public long CreatedAt { get; set; }

    /// <summary>最后更新时间（Unix ms）</summary>
    public long UpdatedAt { get; set; }
}