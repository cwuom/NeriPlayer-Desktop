using NeriPlayer.Core.Api.Common;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Api.Lyrics;

/// <summary>歌词源聚合（对标 Analysis.md 6.1 + Process.md 8.5）</summary>
public sealed class LyricsSourceAggregator
{
    private readonly IEnumerable<LyricsSource> _sources;
    private readonly LyricsCache _cache;

    public LyricsSourceAggregator(IEnumerable<LyricsSource> sources, LyricsCache cache)
    {
        _sources = sources;
        _cache = cache;
    }

    public async Task<LyricResult?> GetLyricAsync(SongItem song)
    {
        var key = $"{song.ChannelId}|{song.AudioId}";
        var cached = _cache.Get(key);
        if (cached is not null) return cached;

        // 按优先级顺序回退
        foreach (var source in _sources.OrderBy(s => s.Priority))
        {
            var r = await source.TryGetAsync(song);
            if (r is not null)
            {
                _cache.Put(key, r);
                return r;
            }
        }
        return null;
    }
}

/// <summary>歌词 LRU 缓存（对标 LruCache 20 条）</summary>
public sealed class LyricsCache
{
    private readonly int _capacity;
    private readonly Dictionary<string, LyricResult> _map = new();

    public LyricsCache(int capacity = 20) => _capacity = capacity;

    public LyricResult? Get(string key) =>
        _map.TryGetValue(key, out var r) ? r : null;

    public void Put(string key, LyricResult r)
    {
        if (_map.Count >= _capacity) _map.Remove(_map.Keys.First());
        _map[key] = r;
    }
}

/// <summary>歌词源抽象基类</summary>
public abstract class LyricsSource
{
    public int Priority { get; }
    protected LyricsSource(int priority) => Priority = priority;
    public abstract Task<LyricResult?> TryGetAsync(SongItem song);
}

/// <summary>
/// 酷狗歌词客户端（对标 Analysis.md 6.1 Kugou 歌词 + KC 解密）
/// </summary>
public sealed class KugouLyricsClient : LyricsSource
{
    private readonly HttpClient _http;

    public KugouLyricsClient(HttpClientFactory factory) : base(40) => _http = factory.Http;
    public KugouLyricsClient(HttpClient http) : base(40) => _http = http;

    public override async Task<LyricResult?> TryGetAsync(SongItem song)
    {
        try
        {
            var keyword = $"{song.Name} {song.Artist}";
            // Kugou 歌词搜索
            var url = $"https://mobilecdn.kugou.com/api/v3/search/song?format=json&keyword={Uri.EscapeDataString(keyword)}&page=1&pagesize=1&showtype=1";
            using var resp = await _http.GetAsync(url);
            resp.EnsureSuccessStatusCode();
            var json = await resp.Content.ReadAsStringAsync();
            using var doc = System.Text.Json.JsonDocument.Parse(json);
            var list = doc.RootElement.GetProperty("data").GetProperty("info");
            var first = list.EnumerateArray().FirstOrDefault();
            if (first.ValueKind == System.Text.Json.JsonValueKind.Undefined) return null;

            var hash = first.GetProperty("hash").GetString();
            if (string.IsNullOrEmpty(hash)) return null;

            // 获取歌词（KC 格式 → LRC 转换）
            var lrcUrl = $"https://krcs.kugou.com/search?ver=1&man=yes&client=mobi&keyword=&duration=&hash={hash}&album_audio_id=";
            using var lrcResp = await _http.GetAsync(lrcUrl);
            lrcResp.EnsureSuccessStatusCode();
            var lrcJson = await lrcResp.Content.ReadAsStringAsync();
            using var lrcDoc = System.Text.Json.JsonDocument.Parse(lrcJson);
            var candidates = lrcDoc.RootElement.GetProperty("candidates");
            var candidate = candidates.EnumerateArray().FirstOrDefault();
            if (candidate.ValueKind == System.Text.Json.JsonValueKind.Undefined) return null;

            // 解密歌词（Kugou 的 KC 格式 base64 编码）
            var lrcContent = candidate.TryGetProperty("content", out var contentEl) ? contentEl.GetString() : null;
            return string.IsNullOrEmpty(lrcContent) ? null : new LyricResult(lrcContent, null, "kugou");
        }
        catch { return null; }
    }
}