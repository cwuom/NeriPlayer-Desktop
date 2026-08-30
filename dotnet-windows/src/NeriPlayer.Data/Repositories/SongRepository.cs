namespace NeriPlayer.Data.Repositories;

using Microsoft.EntityFrameworkCore;
using NeriPlayer.Data.Database;
using NeriPlayer.Data.Entities;

/// <summary>
/// 歌曲仓储（对标 start.md 4.5 / Process.md 5.1）。
/// Upsert 语义：按 StableKey 查询，存在则更新、不存在则插入。
/// </summary>
public sealed class SongRepository(NeriDbContext db)
{
    /// <summary>按 StableKey 查询单首歌曲</summary>
    public async Task<SongEntity?> GetByStableKeyAsync(string stableKey) =>
        await db.Songs.FirstOrDefaultAsync(s => s.StableKey == stableKey);

    /// <summary>插入或更新歌曲，返回持久化后的 Id（StableKey 唯一约束保护）</summary>
    public async Task<long> UpsertAsync(SongEntity song)
    {
        var existing = await GetByStableKeyAsync(song.StableKey);
        if (existing is not null)
        {
            db.Entry(existing).CurrentValues.SetValues(song);
            await db.SaveChangesAsync();
            return existing.Id;
        }
        db.Songs.Add(song);
        await db.SaveChangesAsync();
        return song.Id;
    }

    /// <summary>获取全部歌曲（同步合并用）</summary>
    public async Task<List<SongEntity>> GetAllAsync() => await db.Songs.ToListAsync();

    /// <summary>
    /// 批量 upsert（同步合并用）。返回受影响（新增或更新）的记录。
    /// </summary>
    public async Task<List<SongEntity>> UpsertBatchAsync(IReadOnlyList<SongEntity> songs, int estimatedRemote)
    {
        var existing = await db.Songs.ToListAsync();
        var map = existing.ToDictionary(s => s.StableKey);

        var changed = new List<SongEntity>();
        foreach (var s in songs)
        {
            if (map.TryGetValue(s.StableKey, out var e))
            {
                // 仅在字段有变化时更新，避免无谓写放大
                if (e.Name != s.Name || e.Artist != s.Artist || e.Album != s.Album
                    || e.CoverUrl != s.CoverUrl || e.StreamUrl != s.StreamUrl
                    || e.ChannelId != s.ChannelId || e.AudioId != s.AudioId)
                {
                    db.Entry(e).CurrentValues.SetValues(s);
                    changed.Add(s);
                }
            }
            else
            {
                map[s.StableKey] = s;
                db.Songs.Add(s);
                changed.Add(s);
            }
        }

        await db.SaveChangesAsync();
        return changed;
    }
}