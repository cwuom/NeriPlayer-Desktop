namespace NeriPlayer.Data.Entities;

/// <summary>
/// 同步元数据实体（对标 Analysis.md 21.2 SyncMetadata / sync_metadata 表）。
/// 记录远端文件 etag / revision，用于增量同步与并发裁决。
/// </summary>
public sealed class SyncMetadataEntity
{
    public required string Key { get; set; }          // 如 "playlists" / "settings"
    public string? Etag { get; set; }
    public string? Revision { get; set; }
    public long UpdatedAt { get; set; }
}
