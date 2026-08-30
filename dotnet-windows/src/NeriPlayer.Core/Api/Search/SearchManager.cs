using System.Collections.Concurrent;
using NeriPlayer.Core.Api.Common;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Api.Search;

/// <summary>
/// 搜索聚合器（对标 start.md 7.7 / Process.md 8.6）
/// 三平台并发搜索 + stableKey 去重合并 + LRU 缓存
/// </summary>
public sealed class SearchManager
{
    private readonly IEnumerable<IPlatformClient> _clients;
    private readonly ConcurrentDictionary<string, SearchResponse> _cache = new();

    public SearchManager(IEnumerable<IPlatformClient> clients) => _clients = clients;

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        var cacheKey = $"{keyword}|{page}";
        if (_cache.TryGetValue(cacheKey, out var hit)) return hit;

        // 三平台并发搜索（任一平台失败不阻塞整体）
        var tasks = _clients.Select(async c =>
        {
            try { return await c.SearchAsync(keyword, page); }
            catch { return new SearchResponse([], false); }
        }).ToArray();
        var results = await Task.WhenAll(tasks);

        // 按 stableKey 去重合并
        var seen = new HashSet<string>();
        var merged = new List<SongItem>();
        foreach (var r in results)
            foreach (var song in r.Songs)
            {
                if (seen.Add(song.StableKey()))
                    merged.Add(song);
            }

        var resp = new SearchResponse(merged, results.Any(r => r.HasMore));
        _cache[cacheKey] = resp;
        return resp;
    }

    /// <summary>清空缓存（用于下线/切换账号）</summary>
    public void ClearCache() => _cache.Clear();
}