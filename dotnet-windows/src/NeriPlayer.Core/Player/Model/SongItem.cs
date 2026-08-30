namespace NeriPlayer.Core.Player.Model;

public enum PlaybackSource { Local, Netease, Bilibili, YouTubeMusic }

public sealed record SongItem
{
    public long Id { get; init; }
    public required string Name { get; init; }
    public required string Artist { get; init; }
    public required string Album { get; init; }
    public long AlbumId { get; init; }
    public long DurationMs { get; init; }
    public string? CoverUrl { get; init; }
    public string? MediaUri { get; init; }
    public string? StreamUrl { get; init; }

    public string? ChannelId { get; init; }   // local | netease | bilibili | youtube_music
    public string? AudioId { get; init; }
    public string? SubAudioId { get; init; }

    public string? MatchedLyric { get; init; }
    public string? MatchedTranslatedLyric { get; init; }
    public PlaybackSource? MatchedLyricSource { get; init; }
    public long UserLyricOffsetMs { get; init; }

    public string? CustomName { get; init; }
    public string? CustomArtist { get; init; }
    public string? CustomCoverUrl { get; init; }
    public string? OriginalName { get; init; }
    public string? OriginalArtist { get; init; }

    public string? LocalFileName { get; init; }
    public string? LocalFilePath { get; init; }

    public long AddedAt { get; init; }

    public string DisplayName => CustomName ?? OriginalName ?? Name;
    public string DisplayArtist => CustomArtist ?? OriginalArtist ?? Artist;

    public bool IsLocalSong() =>
        ChannelId == "local" || (!string.IsNullOrEmpty(LocalFilePath) && ChannelId is null);
}
