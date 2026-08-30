using NeriPlayer.Core.Api.YouTube;
using Xunit;

namespace NeriPlayer.Api.Tests;

public class YouTubeMusicClientTests
{
    [Fact]
    public void PlayerScriptStore_CacheAndInvalidate()
    {
        var tmpDir = Path.Combine(Path.GetTempPath(), $"neri_test_{Guid.NewGuid():N}");
        try
        {
            var store = new YouTubePlayerScriptStore(tmpDir);
            Assert.Null(store.GetCached());

            store.Save("// fake player.js content");
            var cached = store.GetCached();
            Assert.NotNull(cached);
            Assert.Equal("// fake player.js content", cached.Value.script);
            Assert.True(cached.Value.valid);

            store.Invalidate();
            Assert.Null(store.GetCached());
        }
        finally
        {
            if (Directory.Exists(tmpDir)) Directory.Delete(tmpDir, true);
        }
    }

    [Fact]
    public async Task SearchAsync_MockReturnsEmptyResult()
    {
        var handler = new MockHttpHandler();
        handler.Register("youtubei/v1/search",
            """{"contents":{"tabbedSearchResultsRenderer":{"tabs":[{"tabRenderer":{"content":{"sectionListRenderer":{"contents":[{"musicShelfRenderer":{"contents":[]}}]}}}}]}}}""");
        var http = new HttpClient(handler);
        var store = new YouTubePlayerScriptStore();
        var client = new YouTubeMusicClient(http, store);

        var result = await client.SearchAsync("nonexistent");
        Assert.NotNull(result);
        Assert.Empty(result.Songs);
    }
}