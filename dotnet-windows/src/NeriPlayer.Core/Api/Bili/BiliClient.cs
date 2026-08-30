using System.Text.Json;
using NeriPlayer.Core.Api.Common;
using NeriPlayer.Core.Logging;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Api.Bili;

/// <summary>Bilibili 音频客户端（对标 Analysis.md 4.2 / Process.md 8.3）</summary>
public sealed class BiliClient : IPlatformClient
{
    private const string BaseUrl = "https://api.bilibili.com";
    private static readonly string[] UserAgents =
    [
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
    ];

    private readonly HttpClient _http;
    private readonly Random _rng = new();

    public BiliClient(HttpClientFactory factory) => _http = factory.Http;
    public BiliClient(HttpClient http) => _http = http;

    public string PlatformId => "bili";
    public bool IsLoggedIn { get; private set; }

    private async Task<JsonElement> GetJsonAsync(string url)
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, url);
        req.Headers.Referrer = new Uri("https://www.bilibili.com");
        using var resp = await _http.SendAsync(req);
        resp.EnsureSuccessStatusCode();
        var body = await resp.Content.ReadAsStringAsync();
        return JsonDocument.Parse(body).RootElement;
    }

    // ── 搜索 ─────────────────────────────────────────────────────

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        try
        {
            var url = $"{BaseUrl}/x/web-interface/search/type?search_type=audio&keyword={Uri.EscapeDataString(keyword)}&page={page}";
            var root = await GetJsonAsync(url);
            var data = root.GetProperty("data").GetProperty("result");
            var songs = new List<SongItem>();
            foreach (var s in data.EnumerateArray())
            {
                var id = s.GetProperty("id").GetInt64();
                var title = s.GetProperty("title").GetString() ?? "";
                var author = s.TryGetProperty("author", out var a) ? a.GetString() ?? "" : "";
                var duration = s.TryGetProperty("duration", out var d) ? ParseBiliDuration(d.GetString()) : 0;
                var coverUrl = s.TryGetProperty("cover", out var c) ? c.GetString() : null;
                songs.Add(new SongItem
                {
                    Id = id, Name = title, Artist = author, Album = "",
                    DurationMs = duration, CoverUrl = coverUrl,
                    ChannelId = "bili", AudioId = id.ToString(),
                });
            }
            return new SearchResponse(songs, data.GetArrayLength() >= 20);
        }
        catch (Exception ex)
        {
            AppLogger.Instance.Warning(ex, "Bilibili search parse failed");
            return new SearchResponse([], false);
        }
    }

    private static long ParseBiliDuration(string? s)
    {
        if (string.IsNullOrEmpty(s)) return 0;
        // Bilibili 音频搜索返回 "mm:ss" 格式
        var parts = s.Split(':');
        if (parts.Length == 2 && int.TryParse(parts[0], out var m) && int.TryParse(parts[1], out var sec))
            return (m * 60 + sec) * 1000;
        return 0;
    }

    // ── 歌曲 URL（音频 v2 接口）────────────────────────────────

    public async Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null)
    {
        try
        {
            // Bilibili 音频 URL：使用 audio/music-service-c/songs/url/v2
            var url = $"{BaseUrl}/audio/music-service-c/songs/url?sid={song.AudioId}&quality=2"; // quality=2: 320kbps
            var root = await GetJsonAsync(url);
            var cdn = root.GetProperty("data").GetProperty("cdns").EnumerateArray().FirstOrDefault();
            var streamUrl = cdn.ValueKind != JsonValueKind.Undefined ? cdn.GetString() : null;
            return new SongUrlResult(!string.IsNullOrEmpty(streamUrl), streamUrl, "high");
        }
        catch (Exception ex)
        {
            AppLogger.Instance.Warning(ex, "Bilibili URL resolve failed");
            return new SongUrlResult(false, null, "high");
        }
    }

    // ── 歌词 ─────────────────────────────────────────────────────

    public async Task<LyricResult?> GetLyricAsync(SongItem song)
    {
        try
        {
            var url = $"{BaseUrl}/audio/music-service-c/songs/info?sid={song.AudioId}";
            var root = await GetJsonAsync(url);
            var lrc = root.TryGetProperty("data", out var data) && data.TryGetProperty("lyric", out var lyric)
                ? lyric.GetString() : null;
            return string.IsNullOrEmpty(lrc) ? null : new LyricResult(lrc, null, "bili");
        }
        catch (Exception ex) { AppLogger.Instance.Warning(ex, "Bilibili lyric fetch failed"); return null; }
    }

    public Task<LoginResult> LoginAsync(LoginMethod method) =>
        Task.FromResult(new LoginResult(false, "Bilibili 登录暂不支持"));
    public Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page = 1) => throw new NotImplementedException();
    public Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId) => throw new NotImplementedException();
    public Task<RecommendationFeed> GetRecommendationsAsync() => throw new NotImplementedException();
}