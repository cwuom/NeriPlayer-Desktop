using System.Text;
using System.Text.Json;
using NeriPlayer.Core.Player.Model;
using NeriPlayer.Data.Entities;
using NeriPlayer.Data.Repositories;
using NeriPlayer.Data.Sync;

namespace NeriPlayer.Data.Sync;

/// <summary>
/// 同步协调器（对标 Analysis.md 9.4 SyncCoordinator / Process.md 10.1）。
/// 串起 Provider、合并策略、仓储（outbox/checkpoint），互斥锁避免并发同步。
/// </summary>
public sealed class SyncCoordinator
{
    private readonly ISyncProvider _provider;
    private readonly SongRepository _songRepo;
    private readonly SyncRepository _syncRepo;

    private readonly SemaphoreSlim _mutex = new(1, 1);

    public SyncCoordinator(ISyncProvider provider, SongRepository songRepo, SyncRepository syncRepo)
    {
        _provider = provider;
        _songRepo = songRepo;
        _syncRepo = syncRepo;
    }

    public string ProviderName => _provider.ProviderName;

    /// <summary>
    /// 执行一次同步：收集 outbox → 发射 → 拉取远端 → 合并回写 → 更新 checkpoint。
    /// </summary>
    public async Task<SyncResult> SyncAsync(string scope)
    {
        if (!await _mutex.WaitAsync(0)) return new SyncResult(false, 0, "sync already in progress");
        try
        {
            // 1) 前置校验
            if (!await _provider.TestConnectionAsync())
                return new SyncResult(false, 0, $"provider '{_provider.ProviderName}' connection failed");

            // 2) 发射本地 outbox 变更
            var outbox = await _syncRepo.GetPendingOutboxAsync();
            var consumed = new List<long>();
            foreach (var item in outbox)
            {
                var ok = await PushOutboxItemAsync(scope, item);
                if (ok) consumed.Add(item.Id);
            }
            if (consumed.Count > 0) await _syncRepo.RemoveOutboxAsync(consumed);

            // 3) 拉取远端歌单并合并（SongEntity → SongItem 投影，统一到 Core 模型再合并）
            var remote = await DownloadPlaylistsAsync(scope);
            var local = (await _songRepo.GetAllAsync()).Select(ToItem).ToList();
            var merged = SyncMergeStrategy.MergePlaylists(local, remote)
                .Select(ToEntity)
                .ToList();
            var changed = await _songRepo.UpsertBatchAsync(merged, remote.Count);

            // 4) 更新 checkpoint
            await _syncRepo.SetCheckpointAsync(scope, DateTimeOffset.UtcNow.Ticks.ToString());

            return new SyncResult(true, changed.Count, $"merged {merged.Count} songs ({remote.Count} remote)");
        }
        finally
        {
            _mutex.Release();
        }
    }

    private async Task<bool> PushOutboxItemAsync(string scope, SyncOutboxEntity item)
    {
        try
        {
            if (item.Action == "DELETE")
                return await _provider.DeleteAsync(scope, item.RefKey);

            if (item.PayloadJson is null) return true;
            var bytes = Encoding.UTF8.GetBytes(item.PayloadJson);
            var existing = await _provider.DownloadAsync(scope, item.RefKey);
            return await _provider.UploadAsync(scope, item.RefKey, bytes, existing?.Etag);
        }
        catch
        {
            return false;
        }
    }

    private async Task<List<SongItem>> DownloadPlaylistsAsync(string scope)
    {
        var songs = new List<SongItem>();
        try
        {
            var files = await _provider.ListAsync(scope);
            foreach (var f in files)
            {
                if (!f.Name.EndsWith(".json", StringComparison.OrdinalIgnoreCase)) continue;
                var file = await _provider.DownloadAsync(scope, f.Name);
                if (file is null) continue;
                try
                {
                    var list = JsonSerializer.Deserialize<List<SongItem>>(file.Content);
                    if (list is not null) songs.AddRange(list);
                }
                catch { /* 跳过损坏文件 */ }
            }
        }
        catch { /* 远端缺失目录等 → 空列表 */ }
        return songs;
    }

    private static SongEntity ToEntity(SongItem s) => new()
    {
        StableKey = s.StableKey(),
        Name = s.Name,
        Artist = s.Artist,
        Album = s.Album,
        AlbumId = s.AlbumId,
        DurationMs = s.DurationMs,
        CoverUrl = s.CoverUrl,
        MediaUri = s.MediaUri,
        StreamUrl = s.StreamUrl,
        ChannelId = s.ChannelId,
        AudioId = s.AudioId,
        SubAudioId = s.SubAudioId,
        AddedAt = s.AddedAt,
    };

    private static SongItem ToItem(SongEntity e) => new()
    {
        Id = e.Id,
        Name = e.Name,
        Artist = e.Artist,
        Album = e.Album,
        AlbumId = e.AlbumId,
        DurationMs = e.DurationMs,
        CoverUrl = e.CoverUrl,
        MediaUri = e.MediaUri,
        StreamUrl = e.StreamUrl,
        ChannelId = e.ChannelId,
        AudioId = e.AudioId,
        SubAudioId = e.SubAudioId,
        AddedAt = e.AddedAt,
    };
}
