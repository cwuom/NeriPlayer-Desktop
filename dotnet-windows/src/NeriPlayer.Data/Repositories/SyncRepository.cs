using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;

namespace NeriPlayer.Data.Repositories;

/// <summary>
/// 同步仓储（对标 start.md 9.x / Analysis.md 21.2 sync 三表）。
/// 管理 sync_metadata / sync_outbox / sync_checkpoints，支撑增量同步与断点续传。
/// </summary>
public sealed class SyncRepository(NeriDbContext db) : RepositoryBase<NeriDbContext>(db)
{
    // ── sync_metadata 元数据 ─────────────────────────────────────────

    public async Task<SyncMetadataEntity?> GetMetadataAsync(string key)
        => await Db.SyncMetadata.FirstOrDefaultAsync(x => x.Key == key);

    public async Task UpsertMetadataAsync(string key, string? etag, string? revision)
    {
        var entity = await Db.SyncMetadata.FirstOrDefaultAsync(x => x.Key == key);
        if (entity is null)
        {
            Db.SyncMetadata.Add(new SyncMetadataEntity
            {
                Key = key, Etag = etag, Revision = revision,
                UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
            });
        }
        else
        {
            entity.Etag = etag;
            entity.Revision = revision;
            entity.UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        }
        await Db.SaveChangesAsync();
    }

    // ── sync_outbox 发件箱 ───────────────────────────────────────────

    public async Task<long> EnqueueAsync(string refKey, string action, string? payloadJson)
    {
        var entity = new SyncOutboxEntity
        {
            RefKey = refKey,
            Action = action,
            PayloadJson = payloadJson,
            CreatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        };
        Db.SyncOutbox.Add(entity);
        await Db.SaveChangesAsync();
        return entity.Id;
    }

    public async Task<List<SyncOutboxEntity>> GetPendingOutboxAsync(int limit = 500)
        => await Db.SyncOutbox.OrderBy(x => x.CreatedAt).Take(limit).ToListAsync();

    public async Task RemoveOutboxAsync(IEnumerable<long> ids)
    {
        var list = ids.ToList();
        if (list.Count == 0) return;
        var rows = await Db.SyncOutbox.Where(x => list.Contains(x.Id)).ToListAsync();
        Db.SyncOutbox.RemoveRange(rows);
        await Db.SaveChangesAsync();
    }

    // ── sync_checkpoints 检查点 ──────────────────────────────────────

    public async Task<SyncCheckpointEntity?> GetCheckpointAsync(string scope)
        => await Db.SyncCheckpoints.FirstOrDefaultAsync(x => x.Scope == scope);

    public async Task SetCheckpointAsync(string scope, string? token)
    {
        var entity = await Db.SyncCheckpoints.FirstOrDefaultAsync(x => x.Scope == scope);
        if (entity is null)
        {
            Db.SyncCheckpoints.Add(new SyncCheckpointEntity
            {
                Scope = scope, Token = token,
                UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
            });
        }
        else
        {
            entity.Token = token;
            entity.UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        }
        await Db.SaveChangesAsync();
    }
}
