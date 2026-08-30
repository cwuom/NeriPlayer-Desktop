namespace NeriPlayer.Data.Entities;

/// <summary>
/// 同步发件箱实体（对标 Analysis.md 21.2 sync_outbox 表）。
/// 记录待同步的变更操作（UPSERT / DELETE），断网缓存、下次同步重放。
/// </summary>
public sealed class SyncOutboxEntity
{
    public long Id { get; set; }

    /// <summary>变更作用对象的关键字（如稳定键 / 歌单 ID）</summary>
    public required string RefKey { get; set; }

    /// <summary>操作类型：UPSERT / DELETE</summary>
    public required string Action { get; set; }

    /// <summary>变更负载（JSON 序列化）</summary>
    public string? PayloadJson { get; set; }

    /// <summary>入队时间（Unix ms）</summary>
    public long CreatedAt { get; set; }
}
