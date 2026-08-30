using System.Text.RegularExpressions;

namespace NeriPlayer.Core.Player.Model;

public static partial class SongIdentity
{
    /// <summary>生成跨版本稳定的歌曲标识：去重、同步、持久化主键（对标 SongIdentity.kt）</summary>
    public static string StableKey(this SongItem song)
    {
        if (song.IsLocalSong())
        {
            var path = song.LocalFilePath ?? song.MediaUri;
            if (!string.IsNullOrEmpty(path))
                return $"local|{NormalizePath(path)}";
            // 本地歌曲无路径：回退到包含 Id 的碰撞安全键，避免所有无路径歌曲生成相同的 "local|"
            return $"id|{song.Id}|local|{song.Name}|{song.Artist}";
        }

        return song.ChannelId switch
        {
            "netease" => $"netease|{song.AudioId ?? song.Id.ToString()}",
            "bilibili" => !string.IsNullOrEmpty(song.AudioId) || !string.IsNullOrEmpty(song.SubAudioId)
                ? $"bilibili|{song.AudioId}|{song.SubAudioId}"
                : $"id|{song.Id}|bilibili|{song.Name}|{song.Artist}",   // 双 ID 均缺失 → 碰撞安全回退
            "youtube_music" when !string.IsNullOrEmpty(ExtractYouTubeVideoId(song.MediaUri)) =>
                $"ytm|{ExtractYouTubeVideoId(song.MediaUri)}",
            "youtube_music" =>
                $"id|{song.Id}|ytm|{song.Name}|{song.Artist}",           // 视频 ID 提取失败 → 碰撞安全回退
            _ => $"id|{song.Id}|{song.Album}|{song.MediaUri}"
        };
    }

    private static string NormalizePath(string p) =>
        p.Replace('\\', '/').TrimEnd('/').ToLowerInvariant();

    /// <summary>从 YouTube 链接/播放列表 URI 提取视频 ID</summary>
    public static string ExtractYouTubeVideoId(string? uri)
    {
        if (string.IsNullOrEmpty(uri)) return "";
        var m = YoutubeVideoIdRegex().Match(uri);
        return m.Success ? m.Groups[1].Value : "";
    }

    [GeneratedRegex(@"(?:v=|youtu\.be/|/shorts/)([A-Za-z0-9_-]{11})")]
    private static partial Regex YoutubeVideoIdRegex();
}
