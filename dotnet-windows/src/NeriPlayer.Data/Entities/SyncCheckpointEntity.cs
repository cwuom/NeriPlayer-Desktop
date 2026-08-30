namespace NeriPlayer.Data.Entities;

/// <summary>
/// 同步检查点实体（对标 Analysis.md 21.2 SyncCheckpoint / sync_checkpoints 表）。
/// 记录每个 scope 的增量游标，避免全量拉取。
/// </summary>
public sealed class SyncCheckpointEntity
{
    public required string Scope { get; set; }        // 主键：如 "playlists"
    public string? Token { get; set; }
    public long UpdatedAt { get; set; }
}
