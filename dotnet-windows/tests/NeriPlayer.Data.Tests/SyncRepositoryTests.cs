using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;
using NeriPlayer.Data.Repositories;
using Xunit;

namespace NeriPlayer.Data.Tests;

/// <summary>SyncRepository 集成测试（SQLite 内存库，对标 start.md 9.x）</summary>
public class SyncRepositoryTests
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
    public async Task Metadata_UpsertAndGet()
    {
        await using var db = CreateDb();
        var repo = new SyncRepository(db);

        await repo.UpsertMetadataAsync("playlists", "abc123", "rev1");
        var meta = await repo.GetMetadataAsync("playlists");

        Assert.NotNull(meta);
        Assert.Equal("abc123", meta!.Etag);
        Assert.Equal("rev1", meta.Revision);

        // 再次 upsert → 更新 etag
        await repo.UpsertMetadataAsync("playlists", "def456", "rev2");
        meta = await repo.GetMetadataAsync("playlists");
        Assert.Equal("def456", meta!.Etag);
    }

    [Fact]
    public async Task Outbox_EnqueueAndGetPending()
    {
        await using var db = CreateDb();
        var repo = new SyncRepository(db);

        await repo.EnqueueAsync("netease|1", "UPSERT", "{\"name\":\"A\"}");
        await repo.EnqueueAsync("bili|2", "DELETE", null);

        var pending = await repo.GetPendingOutboxAsync();
        Assert.Equal(2, pending.Count);
        Assert.Equal("UPSERT", pending[0].Action);
        Assert.Equal("bili|2", pending[1].RefKey);
    }

    [Fact]
    public async Task Outbox_RemoveAfterSync()
    {
        await using var db = CreateDb();
        var repo = new SyncRepository(db);

        var id1 = await repo.EnqueueAsync("netease|1", "UPSERT", "{}");
        var id2 = await repo.EnqueueAsync("bili|2", "DELETE", null);

        await repo.RemoveOutboxAsync([id1]);

        var pending = await repo.GetPendingOutboxAsync();
        var remaining = Assert.Single(pending);
        Assert.Equal(id2, remaining.Id);
    }

    [Fact]
    public async Task Checkpoint_SetAndGet()
    {
        await using var db = CreateDb();
        var repo = new SyncRepository(db);

        Assert.Null(await repo.GetCheckpointAsync("playlists"));

        await repo.SetCheckpointAsync("playlists", "token-1");
        var cp = await repo.GetCheckpointAsync("playlists");
        Assert.NotNull(cp);
        Assert.Equal("token-1", cp!.Token);

        await repo.SetCheckpointAsync("playlists", "token-2");
        cp = await repo.GetCheckpointAsync("playlists");
        Assert.NotNull(cp);
        Assert.Equal("token-2", cp.Token);
    }
}
