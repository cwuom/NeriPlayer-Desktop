using System.Text;
using System.Text.Json;
using NeriPlayer.Core.Api.Common;
using NeriPlayer.Core.Logging;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Api.Netease;

/// <summary>网易云音乐客户端（对标 Analysis.md 4.1 / Process.md 8.2）</summary>
public sealed class NeteaseClient : IPlatformClient
{
    private const string BaseUrl = "https://music.163.com/weapi/";
    private const int MaxResponseBytes = 4 * 1024 * 1024;

    private readonly HttpClient _http;

    public NeteaseClient(HttpClientFactory factory) => _http = factory.Http;
    public NeteaseClient(HttpClient http) => _http = http;

    public string PlatformId => "netease";
    public bool IsLoggedIn { get; private set; }

    private async Task<string> PostWeapiAsync(string path, Dictionary<string, object> payload)
    {
        var form = NeteaseCrypto.Weapi(payload);
        using var content = new FormUrlEncodedContent(form);
        using var resp = await _http.PostAsync(BaseUrl + path, content);
        resp.EnsureSuccessStatusCode();
        var body = await resp.Content.ReadAsByteArrayAsync();
        if (body.Length > MaxResponseBytes)
            throw new InvalidOperationException($"Response too large ({body.Length} > {MaxResponseBytes})");
        return Encoding.UTF8.GetString(body);
    }

    // ── 搜索 ─────────────────────────────────────────────────────

    public async Task<SearchResponse> SearchAsync(string keyword, int page = 1)
    {
        var json = await PostWeapiAsync("search/get", new Dictionary<string, object>
        {
            ["s"] = keyword, ["type"] = 1, ["limit"] = 30, ["offset"] = (page - 1) * 30,
        });
        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement.GetProperty("result");
            var songs = new List<SongItem>();
            foreach (var s in root.GetProperty("songs").EnumerateArray())
            {
                var id = s.GetProperty("id").GetInt64();
                var artists = s.GetProperty("ar").EnumerateArray()
                    .Select(a => a.GetProperty("name").GetString() ?? "").ToArray();
                var album = s.GetProperty("al").GetProperty("name").GetString() ?? "";
                var coverUrl = s.TryGetProperty("al", out var alEl) && alEl.TryGetProperty("picUrl", out var pic)
                    ? pic.GetString() : null;
                songs.Add(new SongItem
                {
                    Id = id, Name = s.GetProperty("name").GetString() ?? "",
                    Artist = string.Join("/", artists), Album = album,
                    DurationMs = s.GetProperty("dt").GetInt64(), CoverUrl = coverUrl,
                    ChannelId = "netease", AudioId = id.ToString(),
                });
            }
            return new SearchResponse(songs, root.TryGetProperty("hasMore", out var hm) && hm.GetBoolean());
        }
        catch (Exception ex)
        {
            AppLogger.Instance.Warning(ex, "Netease search parse failed");
            return new SearchResponse([], false);
        }
    }

    // ── 歌曲 URL（音质降级链）─────────────────────────────────────

    public async Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null)
    {
        var level = qualityKey switch { "lossless"=>"lossless","hires"=>"hires","high"=>"exhigh", _=>"standard" };
        var json = await PostWeapiAsync("song/enhance/player/url/v1", new Dictionary<string, object>
        {
            ["ids"] = new[] { song.AudioId ?? song.Id.ToString() }, ["level"] = level, ["encodeType"] = "mp3",
        });
        try
        {
            using var doc = JsonDocument.Parse(json);
            var data = doc.RootElement.GetProperty("data").EnumerateArray().FirstOrDefault();
            var url = data.TryGetProperty("url", out var urlEl) ? urlEl.GetString() : null;
            if (string.IsNullOrEmpty(url) && level != "standard")
            {
                AppLogger.Instance.Debug("Netease URL empty for {Level}, retrying standard", level);
                return await ResolveSongUrlAsync(song, "standard");
            }
            return new SongUrlResult(!string.IsNullOrEmpty(url), url, level);
        }
        catch (Exception ex)
        {
            AppLogger.Instance.Warning(ex, "Netease URL parse failed");
            return new SongUrlResult(false, null, level);
        }
    }

    // ── 歌词 ─────────────────────────────────────────────────────

    public async Task<LyricResult?> GetLyricAsync(SongItem song)
    {
        try
        {
            var json = await PostWeapiAsync("song/lyric", new Dictionary<string, object>
            {
                ["id"] = song.AudioId ?? song.Id.ToString(), ["lv"] = -1, ["tv"] = -1,
            });
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            var lrc = root.TryGetProperty("lrc", out var lrcEl) ? lrcEl.GetProperty("lyric").GetString() : null;
            var tlrc = root.TryGetProperty("tlyric", out var tlyricEl) ? tlyricEl.GetProperty("lyric").GetString() : null;
            return string.IsNullOrEmpty(lrc) ? null : new LyricResult(lrc, tlrc, "netease");
        }
        catch (Exception ex) { AppLogger.Instance.Warning(ex, "Netease lyric fetch failed"); return null; }
    }

    // ── 二维码登录（骨架，待 CredentialStore 接入）──────────────────

    public async Task<LoginResult> LoginAsync(LoginMethod method)
    {
        if (method != LoginMethod.QrCode) return new LoginResult(false, "仅支持二维码登录");
        try
        {
            var keyJson = await PostWeapiAsync("login/qrcode/unikey", new Dictionary<string, object> { ["type"] = 1 });
            using var keyDoc = JsonDocument.Parse(keyJson);
            var key = keyDoc.RootElement.GetProperty("unikey").GetString();
            if (string.IsNullOrEmpty(key)) return new LoginResult(false, "无法获取二维码 key");
            var qrUrl = $"https://music.163.com/login?code_key={key}";
            AppLogger.Instance.Information("Netease QR url: {Url}", qrUrl);
            for (var i = 0; i < 60; i++)
            {
                await Task.Delay(2000);
                try
                {
                    var checkJson = await PostWeapiAsync("login/qrcode/client/login", new Dictionary<string, object>
                    { ["key"] = key, ["type"] = 1 });
                    using var checkDoc = JsonDocument.Parse(checkJson);
                    var code = checkDoc.RootElement.GetProperty("code").GetInt32();
                    if (code == 803) { IsLoggedIn = true; return new LoginResult(true, "登录成功"); }
                    if (code == 800) return new LoginResult(false, "二维码已过期");
                }
                catch { /* 801/802 继续轮询 */ }
            }
            return new LoginResult(false, "登录超时");
        }
        catch (Exception ex) { return new LoginResult(false, ex.Message); }
    }

    public Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page = 1) => throw new NotImplementedException();
    public Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId) => throw new NotImplementedException();
    public Task<RecommendationFeed> GetRecommendationsAsync() => throw new NotImplementedException();
}