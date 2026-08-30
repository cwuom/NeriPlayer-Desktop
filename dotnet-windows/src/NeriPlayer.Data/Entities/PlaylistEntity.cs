namespace NeriPlayer.Data.Entities;

/// <summary>
/// 歌单实体（对标 Room LocalPlaylist / Analysis.md 21.2）。
/// kind：local（用户创建）| favorite（我喜欢）| system（系统歌单）
/// </summary>
public sealed class PlaylistEntity
{
    public long Id { get; set; }

    public required string Name { get; set; }

    /// <summary>歌单类型：local / favorite / system</summary>
    public string Kind { get; set; } = "local";

    /// <summary>远端平台标识（同步用）</summary>
    public string? RemotePlatform { get; set; }

    /// <summary>远端歌单 ID（同步用）</summary>
    public string? RemoteId { get; set; }

    /// <summary>创建时间（Unix ms）</summary>
    public long CreatedAt { get; set; }

    /// <summary>最后更新时间（Unix ms）</summary>
    public long UpdatedAt { get; set; }

    /// <summary>歌单成员（一对多，级联删除）</summary>
    public List<PlaylistMemberEntity> Members { get; set; } = [];
}