using System.Text.Json;
using NeriPlayer.Core.Api.Search;
using NeriPlayer.Core.Api.Common;
using NeriPlayer.Core.Player.Model;
using Xunit;

namespace NeriPlayer.Api.Tests;

public class SearchManagerTests
{
    private static SongItem MakeSong(long id, string channel, string audioId)
        => new() { Id = id, Name = $"Song{id}", Artist = "Test", Album = "Album",
                   ChannelId = channel, AudioId = audioId };

    [Fact]
    public async Task SearchAsync_MergesAndDeduplicates()
    {
        var mock1 = new MockPlatform("netease", [MakeSong(1, "netease", "100"), MakeSong(2, "netease", "200")]);
        var mock2 = new MockPlatform("bili", [MakeSong(1, "netease", "100"), MakeSong(3, "bili", "300")]);
        var manager = new SearchManager([mock1, mock2]);

        var result = await manager.SearchAsync("test");
        // song 100 重复，应只保留一条；song 200 和 300 各一条 → 共 3 条
        Assert.Equal(3, result.Songs.Count);
    }

    [Fact]
    public async Task SearchAsync_CachesResults()
    {
        var mock = new MockPlatform("netease", [MakeSong(1, "netease", "100")]);
        var manager = new SearchManager([mock]);
        var r1 = await manager.SearchAsync("test");
        var r2 = await manager.SearchAsync("test");
        Assert.Same(r1, r2); // 缓存返回同一引用
        Assert.Equal(1, mock.CallCount); // 只调用一次
    }

    private sealed class MockPlatform : IPlatformClient
    {
        private readonly SearchResponse _resp;
        public int CallCount { get; private set; }
        public MockPlatform(string platform, IReadOnlyList<SongItem> songs) =>
            (_resp, PlatformId) = (new SearchResponse(songs, false), platform);

        public string PlatformId { get; }
        public bool IsLoggedIn => false;
        public Task<SearchResponse> SearchAsync(string keyword, int page = 1)
        { CallCount++; return Task.FromResult(_resp); }
        public Task<LoginResult> LoginAsync(LoginMethod method) => throw new NotImplementedException();
        public Task<IReadOnlyList<RemotePlaylist>> GetFeaturedPlaylistsAsync(int page = 1) => throw new NotImplementedException();
        public Task<RemotePlaylistDetail> GetPlaylistAsync(string playlistId) => throw new NotImplementedException();
        public Task<SongUrlResult> ResolveSongUrlAsync(SongItem song, string? qualityKey = null) => throw new NotImplementedException();
        public Task<LyricResult?> GetLyricAsync(SongItem song) => throw new NotImplementedException();
        public Task<RecommendationFeed> GetRecommendationsAsync() => throw new NotImplementedException();
    }
}