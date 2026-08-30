namespace NeriPlayer.Data.Repositories;

using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;

/// <summary>
/// 下载仓储（对标 start.md 8.4 / Analysis.md 24.2 下载索引）。
/// 管理 downloads / download_snapshots / download_recovery / download_queue 四张表。
/// </summary>
public sealed class DownloadRepository(NeriDbContext db) : RepositoryBase<NeriDbContext>(db)
{
    // ── downloads 主表 ──────────────────────────────────────────────

    /// <summary>按 StableKey 查找下载记录</summary>
    public Task<DownloadEntity?> GetByStableKeyAsync(string stableKey)
        => Db.Downloads.FirstOrDefaultAsync(x => x.StableKey == stableKey);

    /// <summary>获取所有下载记录</summary>
    public Task<List<DownloadEntity>> GetAllAsync()
        => Db.Downloads.OrderByDescending(x => x.CreatedAt).ToListAsync();

    /// <summary>获取指定状态的下载记录</summary>
    public Task<List<DownloadEntity>> GetByStatusAsync(int status)
        => Db.Downloads.Where(x => x.Status == status)
            .OrderByDescending(x => x.CreatedAt).ToListAsync();

    /// <summary>新增或更新下载记录（按 StableKey 去重）</summary>
    public async Task<long> UpsertAsync(DownloadEntity entity)
    {
        return await RunAsync(async db =>
        {
            var existing = await db.Downloads.FirstOrDefaultAsync(x => x.StableKey == entity.StableKey);
            if (existing is not null)
            {
                existing.LocalPath = entity.LocalPath;
                existing.Status = entity.Status;
                existing.QualityKey = entity.QualityKey;
                existing.BytesReceived = entity.BytesReceived;
                existing.TotalBytes = entity.TotalBytes;
                existing.UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
                await db.SaveChangesAsync();
                return existing.Id;
            }

            entity.CreatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            entity.UpdatedAt = entity.CreatedAt;
            db.Downloads.Add(entity);
            await db.SaveChangesAsync();
            return entity.Id;
        });
    }

    /// <summary>更新下载进度</summary>
    public async Task UpdateProgressAsync(string stableKey, long bytesReceived, long? totalBytes, int status)
    {
        var entity = await Db.Downloads.FirstOrDefaultAsync(x => x.StableKey == stableKey);
        if (entity is null) return;

        entity.BytesReceived = bytesReceived;
        entity.TotalBytes = totalBytes;
        entity.Status = status;
        entity.UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        await Db.SaveChangesAsync();
    }

    // ── download_snapshots 快照表 ───────────────────────────────────

    /// <summary>新增或更新快照</summary>
    public async Task UpsertSnapshotAsync(DownloadSnapshotEntity snapshot)
    {
        var existing = await Db.DownloadSnapshots.FirstOrDefaultAsync(
            x => x.RootKey == snapshot.RootKey &&
                 x.Bucket == snapshot.Bucket &&
                 x.EntryKey == snapshot.EntryKey);

        if (existing is not null)
        {
            existing.LocalPath = snapshot.LocalPath;
            existing.FileHash = snapshot.FileHash;
            existing.SnapshotAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        }
        else
        {
            snapshot.SnapshotAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            Db.DownloadSnapshots.Add(snapshot);
        }

        await Db.SaveChangesAsync();
    }

    /// <summary>按 RootKey + Bucket 获取快照列表</summary>
    public Task<List<DownloadSnapshotEntity>> GetSnapshotsAsync(string rootKey, string bucket)
        => Db.DownloadSnapshots.Where(x => x.RootKey == rootKey && x.Bucket == bucket).ToListAsync();

    // ── download_recovery 恢复表 ────────────────────────────────────

    /// <summary>记录待恢复任务</summary>
    public async Task<long> AddRecoveryAsync(DownloadRecoveryEntity recovery)
    {
        recovery.RecordedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        Db.DownloadRecoveries.Add(recovery);
        await Db.SaveChangesAsync();
        return recovery.Id;
    }

    /// <summary>获取所有 pending 恢复任务</summary>
    public Task<List<DownloadRecoveryEntity>> GetPendingRecoveriesAsync()
        => Db.DownloadRecoveries.Where(x => x.RecoveryStatus == 0)
            .OrderBy(x => x.RecordedAt).ToListAsync();

    /// <summary>更新恢复状态</summary>
    public async Task UpdateRecoveryStatusAsync(long recoveryId, int status)
    {
        var entity = await Db.DownloadRecoveries.FindAsync(recoveryId);
        if (entity is null) return;
        entity.RecoveryStatus = status;
        await Db.SaveChangesAsync();
    }

    // ── download_queue 队列表 ────────────────────────────────────────

    /// <summary>入队待下载任务</summary>
    public async Task<long> EnqueueAsync(DownloadQueueEntity item)
    {
        item.Status = 0; // pending
        item.EnqueuedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        Db.DownloadQueues.Add(item);
        await Db.SaveChangesAsync();
        return item.Id;
    }

    /// <summary>获取所有 pending 队列项（按优先级+入队时间排序）</summary>
    public Task<List<DownloadQueueEntity>> GetPendingQueueAsync()
        => Db.DownloadQueues.Where(x => x.Status == 0)
            .OrderByDescending(x => x.Priority).ThenBy(x => x.EnqueuedAt)
            .ToListAsync();

    /// <summary>更新队列项状态</summary>
    public async Task UpdateQueueStatusAsync(long queueId, int status)
    {
        var entity = await Db.DownloadQueues.FindAsync(queueId);
        if (entity is null) return;
        entity.Status = status;
        await Db.SaveChangesAsync();
    }
}