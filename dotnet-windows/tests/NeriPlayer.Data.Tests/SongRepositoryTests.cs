using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;
using NeriPlayer.Data.Repositories;
using Xunit;

namespace NeriPlayer.Data.Tests;

/// <summary>
/// SongRepository 集成测试（对标 start.md 14.3 / 4.x 验收）。
/// 使用 SQLite 内存库（EnsureCreated），测试自包含、不落盘。
/// </summary>
public class SongRepositoryTests
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
    public async Task Upsert_SameStableKey_NoDuplicate()
    {
        await using var db = CreateDb();
        var repo = new SongRepository(db);
        var song = new SongEntity
        {
            StableKey = "netease|1",
            Name = "歌A", Artist = "艺人B", Album = "专辑C"
        };
        var id1 = await repo.UpsertAsync(song);
        song.Name = "歌A2";
        var id2 = await repo.UpsertAsync(song);
        Assert.Equal(id1, id2);
        Assert.Single(await db.Songs.ToListAsync());
        // 验证更新后的值
        var updated = await db.Songs.FindAsync(id1);
        Assert.Equal("歌A2", updated!.Name);
    }

    [Fact]
    public async Task GetByStableKey_ReturnsSong()
    {
        await using var db = CreateDb();
        var repo = new SongRepository(db);
        db.Songs.Add(new SongEntity
        {
            StableKey = "local|d:/music/test.flac",
            Name = "测试", Artist = "测试艺人", Album = "测试专辑"
        });
        await db.SaveChangesAsync();

        var result = await repo.GetByStableKeyAsync("local|d:/music/test.flac");
        Assert.NotNull(result);
        Assert.Equal("测试", result.Name);
        Assert.Equal(1, result.Id);
    }

    [Fact]
    public async Task Upsert_NewSong_ReturnsNewId()
    {
        await using var db = CreateDb();
        var repo = new SongRepository(db);
        var song = new SongEntity
        {
            StableKey = "bilibili|12345|67890",
            Name = "B站", Artist = "UP主", Album = "合集"
        };
        var id = await repo.UpsertAsync(song);
        Assert.True(id > 0);

        // 第二次插入不同 stable_key，得到新 Id
        var song2 = new SongEntity
        {
            StableKey = "bilibili|99999|00000",
            Name = "B站2", Artist = "UP主2", Album = "合集2"
        };
        var id2 = await repo.UpsertAsync(song2);
        Assert.True(id2 > id);
    }

    [Fact]
    public async Task Database_CanCreateAndQueryTables()
    {
        await using var db = CreateDb();
        // 验证 5 张表均可操作（songs, playlists, playlist_members, playback_stats, stat_buckets）
        var song = new SongEntity
        {
            StableKey = "test", Name = "测试", Artist = "测试", Album = "测试"
        };
        db.Songs.Add(song);
        await db.SaveChangesAsync();

        var playlist = new PlaylistEntity { Name = "测试歌单", Kind = "local" };
        db.Playlists.Add(playlist);
        await db.SaveChangesAsync();

        db.PlaylistMembers.Add(new PlaylistMemberEntity
        {
            PlaylistId = playlist.Id, SongId = song.Id, Position = 0
        });
        db.PlaybackStats.Add(new PlaybackStatsEntity
        {
            SongId = song.Id, PlayCount = 1, TotalPlayMs = 30000, LastPlayedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()
        });
        db.StatBuckets.Add(new StatBucketEntity
        {
            SongId = song.Id, DayKey = 20260817, PlayCount = 1, ListenMs = 30000
        });
        await db.SaveChangesAsync();

        Assert.Single(await db.PlaylistMembers.ToListAsync());
        Assert.Single(await db.PlaybackStats.ToListAsync());
        Assert.Single(await db.StatBuckets.ToListAsync());
    }
}