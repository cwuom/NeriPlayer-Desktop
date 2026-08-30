namespace NeriPlayer.Data.Entities;

/// <summary>
/// 下载恢复实体（对标 Analysis.md 24.2 download_recovery 索引表）。
/// 记录中断的下载任务（.part 临时文件），用于异常退出后扫描恢复。
/// </summary>
public sealed class DownloadRecoveryEntity
{
    /// <summary>数据库自增主键</summary>
    public long Id { get; set; }

    /// <summary>对应的下载任务 ID</summary>
    public long DownloadId { get; set; }

    /// <summary>.part 临时文件路径</summary>
    public required string PartFilePath { get; set; }

    /// <summary>已下载字节数</summary>
    public long BytesReceived { get; set; }

    /// <summary>恢复状态（0=pending, 1=recovered, 2=abandoned）</summary>
    public int RecoveryStatus { get; set; }

    /// <summary>记录时间（Unix ms）</summary>
    public long RecordedAt { get; set; }
}