using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Api.Common;

// ── 类型契约（对标 start.md 7.1） ─────────────────────────────────────

public enum LoginMethod { QrCode, Cookie, Token }

public sealed record LoginResult(bool Success, string? Message, string? QrUrl = null);
public sealed record SongUrlResult(bool Success, string? Url, string? QualityKey);
public sealed record LyricResult(string Lrc, string? TranslatedLrc, string Source);
public sealed record RemotePlaylist(string Id, string Name, string? CoverUrl);
public sealed record RemotePlaylistDetail(string Id, string Name, IReadOnlyList<SongItem> Songs);
public sealed record RecommendationFeed(IReadOnlyList<SongItem> Songs, IReadOnlyList<RemotePlaylist> Playlists);
public sealed record SearchResponse(IReadOnlyList<SongItem> Songs, bool HasMore);

// ── 平台统一接口 ─────────────────────────────────────────────────────

public interface IPlatformClient
{
    string PlatformId { get; }            // "netease" | "bili" | "youtube_music"
    bool IsLoggedIn { get; }
    Task<LoginResult> LoginAsync(LoginMethod method);

    // 搜索
    Task<SearchResponse> SearchAsync(string keyword, int page = 1);

    // 歌单
    Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page = 1);
    Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId);

    // 歌曲播放
    Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null);

    // 歌词
    Task<LyricResult?> GetLyricAsync(SongItem song);

    // 推荐
    Task<RecommendationFeed> GetRecommendationsAsync();
}