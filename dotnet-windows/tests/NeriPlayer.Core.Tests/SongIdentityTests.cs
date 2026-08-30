using NeriPlayer.Core.Player.Model;
using Xunit;

namespace NeriPlayer.Core.Tests;

public class SongIdentityTests
{
    [Fact]
    public void LocalSong_StableKey_IsNormalizedPath()
    {
        var song = new SongItem
        {
            Id = 1, Name = "A", Artist = "B", Album = "C",
            ChannelId = "local", LocalFilePath = @"D:\Music\a\b\c.flac"
        };
        Assert.Equal("local|d:/music/a/b/c.flac", song.StableKey());
    }

    [Theory]
    [InlineData("https://www.youtube.com/watch?v=abcDEF12345")]
    [InlineData("https://youtu.be/abcDEF12345?si=xxx")]
    public void YouTube_ExtractVideoId_Works(string uri)
    {
        var song = new SongItem
        {
            Id = 2, Name = "A", Artist = "B", Album = "C",
            ChannelId = "youtube_music", MediaUri = uri
        };
        Assert.Equal("abcDEF12345", SongIdentity.ExtractYouTubeVideoId(uri));
        Assert.Equal("ytm|abcDEF12345", song.StableKey());
    }

    [Fact]
    public void Netease_StableKey_UsesAudioId()
    {
        var song = new SongItem
        {
            Id = 9, Name = "A", Artist = "B", Album = "C",
            ChannelId = "netease", AudioId = "3456789"
        };
        Assert.Equal("netease|3456789", song.StableKey());
    }

    // ===== 碰撞安全回退测试（CodeRabbit review #20） =====

    [Fact]
    public void LocalSong_WithoutPath_FallsBackToCollisionSafeKey()
    {
        var song = new SongItem
        {
            Id = 10, Name = "歌", Artist = "艺人", Album = "专辑",
            ChannelId = "local", LocalFilePath = null, MediaUri = null
        };
        // 不能返回 "local|"，必须包含 Id 避免与其它无路径歌曲碰撞
        Assert.NotEqual("local|", song.StableKey());
        Assert.StartsWith("id|10|local|", song.StableKey());
    }

    [Fact]
    public void YouTube_ExtractFailure_FallsBackToCollisionSafeKey()
    {
        var song = new SongItem
        {
            Id = 11, Name = "A", Artist = "B", Album = "C",
            ChannelId = "youtube_music", MediaUri = "not-a-youtube-url"
        };
        // 不能返回 "ytm|"，必须回退到包含 Id 的键
        Assert.NotEqual("ytm|", song.StableKey());
        Assert.StartsWith("id|11|ytm|", song.StableKey());
    }

    [Fact]
    public void Bilibili_WithoutAnyId_FallsBackToCollisionSafeKey()
    {
        var song = new SongItem
        {
            Id = 12, Name = "A", Artist = "B", Album = "C",
            ChannelId = "bilibili", AudioId = null, SubAudioId = null
        };
        // 不能返回 "bilibili||"，必须回退到包含 Id 的键
        Assert.NotEqual("bilibili||", song.StableKey());
        Assert.StartsWith("id|12|bilibili|", song.StableKey());
    }

    [Fact]
    public void Bilibili_WithAudioId_StillUsesPlatformKey()
    {
        var song = new SongItem
        {
            Id = 13, Name = "A", Artist = "B", Album = "C",
            ChannelId = "bilibili", AudioId = "12345", SubAudioId = null
        };
        Assert.Equal("bilibili|12345|", song.StableKey());
    }
}
