using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;
using NeriPlayer.Data.Repositories;
using Xunit;

namespace NeriPlayer.Data.Tests;

/// <summary>
/// DownloadRepository 集成测试（对标 SongRepositoryTests 模式）。
/// 使用 SQLite 内存库（EnsureCreated），测试自包含、不落盘。
/// </summary>
public class DownloadRepositoryTests
{
    private static NeriDbContext CreateDb()
    {
        var conn = new SqliteConnection("Data Source=:memory:");
        conn.Open();
        var options = new DbContextOptionsBuilder<NeriDbContext>()
            .UseSqlite(conn).Options;
        var db = new NeriDbContext(options);
        db.Database.EnsureCreated();
        return db;
    }

    [Fact]
    public async Task Upsert_SameStableKey_UpdatesExisting()
    {
        await using var db = CreateDb();
        var repo = new DownloadRepository(db);
        var entity = new DownloadEntity
        {
            StableKey = "netease|1",
            LocalPath = @"D:\Music\song1.mp3",
            Status = 0,
        };

        var id1 = await repo.UpsertAsync(entity);
        Assert.True(id1 > 0);

        // 同一 StableKey 再次 upsert → 更新
        entity.LocalPath = @"D:\Music\song1_v2.mp3";
        entity.Status = 3; // Completed
        var id2 = await repo.UpsertAsync(entity);

        Assert.Equal(id1, id2);
        var all = await repo.GetAllAsync();
        Assert.Single(all);
        Assert.Equal("song1_v2.mp3", Path.GetFileName(all[0].LocalPath));
    }

    [Fact]
    public async Task GetByStatus_ReturnsCorrectSubset()
    {
        await using var db = CreateDb();
        var repo = new DownloadRepository(db);

        await repo.UpsertAsync(new DownloadEntity
        {
            StableKey = "netease|1", LocalPath = "/a", Status = 3, // Completed
        });
        await repo.UpsertAsync(new DownloadEntity
        {
            StableKey = "netease|2", LocalPath = "/b", Status = 0, // Queued
        });
        await repo.UpsertAsync(new DownloadEntity
        {
            StableKey = "netease|3", LocalPath = "/c", Status = 3,
        });

        var completed = await repo.GetByStatusAsync(3);
        Assert.Equal(2, completed.Count);

        var queued = await repo.GetByStatusAsync(0);
        Assert.Single(queued);
    }

    [Fact]
    public async Task UpdateProgress_UpdatesBytesAndStatus()
    {
        await using var db = CreateDb();
        var repo = new DownloadRepository(db);
        await repo.UpsertAsync(new DownloadEntity
        {
            StableKey = "bilibili|123|456", LocalPath = "/d", Status = 0,
        });

        await repo.UpdateProgressAsync("bilibili|123|456", 1024, 4096, 1);

        var entity = await repo.GetByStableKeyAsync("bilibili|123|456");
        Assert.NotNull(entity);
        Assert.Equal(1024, entity.BytesReceived);
        Assert.Equal(4096, entity.TotalBytes);
        Assert.Equal(1, entity.Status);
    }

    [Fact]
    public async Task Snapshot_UpsertAndQuery()
    {
        await using var db = CreateDb();
        var repo = new DownloadRepository(db);

        await repo.UpsertSnapshotAsync(new DownloadSnapshotEntity
        {
            RootKey = "netease", Bucket = "playlist1",
            EntryKey = "song1", LocalPath = "/a/song1.flac",
        });

        var snapshots = await repo.GetSnapshotsAsync("netease", "playlist1");
        Assert.Single(snapshots);
        Assert.Equal("song1", snapshots[0].EntryKey);

        // 更新同一快照
        await repo.UpsertSnapshotAsync(new DownloadSnapshotEntity
        {
            RootKey = "netease", Bucket = "playlist1",
            EntryKey = "song1", LocalPath = "/a/song1_v2.flac",
        });
        snapshots = await repo.GetSnapshotsAsync("netease", "playlist1");
        Assert.Single(snapshots);
        Assert.Equal("song1_v2.flac", Path.GetFileName(snapshots[0].LocalPath));
    }

    [Fact]
    public async Task Queue_EnqueueAndQueryPending()
    {
        await using var db = CreateDb();
        var repo = new DownloadRepository(db);

        await repo.EnqueueAsync(new DownloadQueueEntity
        {
            StableKey = "ytm|abc", SongName = "Song A",
            TargetPath = "/a.mp3", Priority = 0,
        });
        await repo.EnqueueAsync(new DownloadQueueEntity
        {
            StableKey = "ytm|def", SongName = "Song B",
            TargetPath = "/b.mp3", Priority = 1,
        });

        var pending = await repo.GetPendingQueueAsync();
        Assert.Equal(2, pending.Count);
        // 高优先级在前
        Assert.Equal(1, pending[0].Priority);
        Assert.Equal("Song B", pending[0].SongName);
    }

    [Fact]
    public async Task Recovery_AddAndQueryPending()
    {
        await using var db = CreateDb();
        var repo = new DownloadRepository(db);

        var recoveryId = await repo.AddRecoveryAsync(new DownloadRecoveryEntity
        {
            DownloadId = 1,
            PartFilePath = "/download/song.mp3.part",
            BytesReceived = 512,
        });

        var pending = await repo.GetPendingRecoveriesAsync();
        Assert.Single(pending);
        Assert.Equal(recoveryId, pending[0].Id);

        // 更新状态
        await repo.UpdateRecoveryStatusAsync(recoveryId, 1); // recovered
        pending = await repo.GetPendingRecoveriesAsync();
        Assert.Empty(pending);
    }
}
