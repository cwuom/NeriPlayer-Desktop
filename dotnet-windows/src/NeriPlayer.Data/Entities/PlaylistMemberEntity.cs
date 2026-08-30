namespace NeriPlayer.Data.Entities;

/// <summary>
/// 歌单成员关联表（一对多，PK(PlaylistId, Position)）。
/// 对标 Room playlist_members / Analysis.md 21.2。
/// </summary>
public sealed class PlaylistMemberEntity
{
    public long PlaylistId { get; set; }
    public long SongId { get; set; }

    /// <summary>歌单内排序位置（0-based）</summary>
    public int Position { get; set; }

    // 导航属性（EF Core fluent API 关联）
    public PlaylistEntity? Playlist { get; set; }
    public SongEntity? Song { get; set; }
}