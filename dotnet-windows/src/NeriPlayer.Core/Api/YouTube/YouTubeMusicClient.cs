using System.Net.Http.Json;
using System.Text.Json;
using NeriPlayer.Core.Api.Common;
using NeriPlayer.Core.Logging;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Api.YouTube;

/// <summary>InnerTube WEB_REMIX 客户端（对标 Analysis.md 4.3 / Process.md 8.4）</summary>
public sealed class YouTubeMusicClient : IPlatformClient
{
    private const string InnerTubeApi = "https://music.youtube.com/youtubei/v1/";
    private const string ClientName = "WEB_REMIX";
    private const int ClientVersion = 67;

    private readonly HttpClient _http;
    private readonly YouTubePlayerScriptStore _scriptStore;

    public YouTubeMusicClient(HttpClientFactory factory, YouTubePlayerScriptStore scriptStore)
    { _http = factory.Http; _scriptStore = scriptStore; }
    public YouTubeMusicClient(HttpClient http, YouTubePlayerScriptStore scriptStore)
    { _http = http; _scriptStore = scriptStore; }

    public string PlatformId => "youtube_music";
    public bool IsLoggedIn { get; private set; }

    private async Task<JsonDocument> PostInnerTubeAsync(string endpoint, Dictionary<string, object> payload)
    {
        var body = new Dictionary<string, object>
        {
            ["context"] = new Dictionary<string, object>
            {
                ["client"] = new Dictionary<string, object>
                {
                    ["clientName"] = ClientName,
                    ["clientVersion"] = ClientVersion.ToString(),
                    ["hl"] = "zh-Hans", ["gl"] = "CN",
                }
            },
        };
        foreach (var kv in payload) body[kv.Key] = kv.Value;
        using var resp = await _http.PostAsJsonAsync(InnerTubeApi + endpoint, body);
        resp.EnsureSuccessStatusCode();
        return await JsonDocument.ParseAsync(await resp.Content.ReadAsStreamAsync());
    }

    // ── 搜索 ─────────────────────────────────────────────────────

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        try
        {
            using var doc = await PostInnerTubeAsync("search", new Dictionary<string, object>
            { ["query"] = keyword, ["params"] = "EgWKAQIIAWoKEAoQCRADEAA%3D" });
            var contents = doc.RootElement.GetProperty("contents").GetProperty("tabbedSearchResultsRenderer")
                .GetProperty("tabs").EnumerateArray().FirstOrDefault()
                .GetProperty("tabRenderer").GetProperty("content")
                .GetProperty("sectionListRenderer").GetProperty("contents")
                .EnumerateArray().FirstOrDefault()
                .GetProperty("musicShelfRenderer").GetProperty("contents");
            var songs = new List<SongItem>();
            foreach (var item in contents.EnumerateArray())
            {
                var renderer = item.GetProperty("musicResponsiveListItemRenderer");
                var title = renderer.GetProperty("flexColumns")[0]
                    .GetProperty("musicResponsiveListItemFlexColumnRenderer")
                    .GetProperty("text").GetProperty("runs").EnumerateArray()
                    .FirstOrDefault().GetProperty("text").GetString() ?? "";
                var videoId = renderer.GetProperty("playlistItemData").GetProperty("videoId").GetString() ?? "";
                songs.Add(new SongItem { Id = 0, Name = title, Artist = "", Album = "",
                    DurationMs = 0, ChannelId = "youtube_music", MediaUri = $"https://music.youtube.com/watch?v={videoId}" });
            }
            return new SearchResponse(songs, true);
        }
        catch (Exception ex) { AppLogger.Instance.Warning(ex, "YouTube Music search parse failed"); return new SearchResponse([], false); }
    }

    // ── 歌曲 URL（PoToken 多级回退骨架）────────────────────────

    public async Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null)
    {
        try
        {
            var videoId = ExtractVideoId(song.MediaUri);
            if (string.IsNullOrEmpty(videoId)) return new SongUrlResult(false, null, qualityKey);
            using var doc = await PostInnerTubeAsync("player", new Dictionary<string, object> { ["videoId"] = videoId });
            var formats = doc.RootElement.GetProperty("streamingData").GetProperty("adaptiveFormats");
            var best = formats.EnumerateArray()
                .Where(f => f.GetProperty("mimeType").GetString()?.StartsWith("audio/") == true)
                .OrderByDescending(f => f.GetProperty("bitrate").GetInt32())
                .FirstOrDefault();
            var url = best.ValueKind != JsonValueKind.Undefined && best.TryGetProperty("url", out var u) ? u.GetString() : null;
            return new SongUrlResult(!string.IsNullOrEmpty(url), url, qualityKey);
        }
        catch (Exception ex) { AppLogger.Instance.Warning(ex, "YouTube Music URL resolve failed"); return new SongUrlResult(false, null, qualityKey); }
    }

    private static string? ExtractVideoId(string? uri)
    {
        if (string.IsNullOrEmpty(uri)) return null;
        var idx = uri.IndexOf("v=");
        if (idx >= 0) return uri[(idx + 2)..].Split('&', '?').FirstOrDefault();
        idx = uri.LastIndexOf('/');
        return idx >= 0 ? uri[(idx + 1)..].Split('?', '&').FirstOrDefault() : null;
    }

    public Task<LyricResult?> GetLyricAsync(SongItem song) => Task.FromResult<LyricResult?>(null);
    public Task<LoginResult> LoginAsync(LoginMethod method) =>
        Task.FromResult(new LoginResult(false, "YouTube Music Cookie 登录暂未实现"));
    public Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page = 1) => throw new NotImplementedException();
    public Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId) => throw new NotImplementedException();
    public Task<RecommendationFeed> GetRecommendationsAsync() => throw new NotImplementedException();
}