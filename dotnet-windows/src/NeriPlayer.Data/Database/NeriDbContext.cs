namespace NeriPlayer.Data.Database;

using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Entities;

/// <summary>
/// 应用数据库上下文（对标 Room NeriUserDataDatabase / Analysis.md 21.2）。
/// 核心 5 表：songs / playlists / playlist_members / playback_stats / stat_buckets。
/// 后续章节补充：PlayHistory / PlaybackQueue / QueueState / Downloads /
/// DownloadSnapshots / SyncMetadata / SyncOutbox / SyncCheckpoints /
/// TrafficStats / CoverUrlMapping / Settings / CookieCredentials。
/// </summary>
public sealed class NeriDbContext(DbContextOptions<NeriDbContext> options) : DbContext(options)
{
    public DbSet<SongEntity> Songs => Set<SongEntity>();
    public DbSet<PlaylistEntity> Playlists => Set<PlaylistEntity>();
    public DbSet<PlaylistMemberEntity> PlaylistMembers => Set<PlaylistMemberEntity>();
    public DbSet<PlaybackStatsEntity> PlaybackStats => Set<PlaybackStatsEntity>();
    public DbSet<StatBucketEntity> StatBuckets => Set<StatBucketEntity>();

    // 第八章：下载索引（对标 Analysis.md 24.2）
    public DbSet<DownloadEntity> Downloads => Set<DownloadEntity>();
    public DbSet<DownloadSnapshotEntity> DownloadSnapshots => Set<DownloadSnapshotEntity>();
    public DbSet<DownloadRecoveryEntity> DownloadRecoveries => Set<DownloadRecoveryEntity>();
    public DbSet<DownloadQueueEntity> DownloadQueues => Set<DownloadQueueEntity>();

    // 第九章：同步（对标 Analysis.md 21.2 / Process.md 5.3）
    public DbSet<SyncMetadataEntity> SyncMetadata => Set<SyncMetadataEntity>();
    public DbSet<SyncOutboxEntity> SyncOutbox => Set<SyncOutboxEntity>();
    public DbSet<SyncCheckpointEntity> SyncCheckpoints => Set<SyncCheckpointEntity>();

    protected override void OnModelCreating(ModelBuilder b)
    {
        b.Entity<SongEntity>(e =>
        {
            e.ToTable("songs");
            e.HasKey(x => x.Id);
            e.HasIndex(x => x.StableKey).IsUnique();
        });

        b.Entity<PlaylistEntity>(e =>
        {
            e.ToTable("playlists");
            e.HasKey(x => x.Id);
            e.HasMany(p => p.Members)
             .WithOne(m => m.Playlist!)
             .HasForeignKey(m => m.PlaylistId)
             .OnDelete(DeleteBehavior.Cascade);
        });

        b.Entity<PlaylistMemberEntity>(e =>
        {
            e.ToTable("playlist_members");
            e.HasKey(x => new { x.PlaylistId, x.Position });
            e.HasOne(m => m.Song)
             .WithMany()
             .HasForeignKey(m => m.SongId)
             .OnDelete(DeleteBehavior.Cascade);
        });

        b.Entity<PlaybackStatsEntity>(e =>
        {
            e.ToTable("playback_stats");
            e.HasKey(x => x.SongId);
        });

        b.Entity<StatBucketEntity>(e =>
        {
            e.ToTable("stat_buckets");
            e.HasKey(x => new { x.SongId, x.DayKey });
        });

        // 第八章：下载索引（对标 Analysis.md 24.2）
        b.Entity<DownloadEntity>(e =>
        {
            e.ToTable("downloads");
            e.HasKey(x => x.Id);
            e.HasIndex(x => x.StableKey).IsUnique();
        });

        b.Entity<DownloadSnapshotEntity>(e =>
        {
            e.ToTable("download_snapshots");
            e.HasKey(x => new { x.RootKey, x.Bucket, x.EntryKey });
        });

        b.Entity<DownloadRecoveryEntity>(e =>
        {
            e.ToTable("download_recovery");
            e.HasKey(x => x.Id);
            e.HasIndex(x => x.DownloadId);
        });

        b.Entity<DownloadQueueEntity>(e =>
        {
            e.ToTable("download_queue");
            e.HasKey(x => x.Id);
            e.HasIndex(x => x.StableKey);
        });

        // 第九章：同步（对标 Analysis.md 21.2 / Process.md 5.3）
        b.Entity<SyncMetadataEntity>(e =>
        {
            e.ToTable("sync_metadata");
            e.HasKey(x => x.Key);
        });

        b.Entity<SyncOutboxEntity>(e =>
        {
            e.ToTable("sync_outbox");
            e.HasKey(x => x.Id);
            e.HasIndex(x => x.RefKey);
        });

        b.Entity<SyncCheckpointEntity>(e =>
        {
            e.ToTable("sync_checkpoints");
            e.HasKey(x => x.Scope);
        });
    }
}