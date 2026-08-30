using System.Net;
using System.Net.Http.Headers;

namespace NeriPlayer.Core.Tests;

/// <summary>
/// DownloadTask 单元测试。
/// 使用自定义 StubHttpHandler 模拟 HTTP 响应。
/// </summary>
public class DownloadTaskTests : IDisposable
{
    private readonly string _testDir;

    public DownloadTaskTests()
    {
        _testDir = Path.Combine(Path.GetTempPath(), $"neri_dl_test_{Guid.NewGuid():N}");
        Directory.CreateDirectory(_testDir);
    }

    public void Dispose()
    {
        if (Directory.Exists(_testDir))
            Directory.Delete(_testDir, true);
    }

    /// <summary>模拟 HTTP 服务端：返回固定字节流，支持/不支持 Range。</summary>
    private sealed class StubHttpHandler(byte[] content, bool supportsRange = true) : HttpMessageHandler
    {
        public HttpRequestMessage? LastRequest { get; private set; }

        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken ct)
        {
            LastRequest = request;
            long start = 0;
            if (supportsRange && request.Headers.Range?.Ranges.Count > 0)
                start = request.Headers.Range.Ranges.First().From ?? 0;

            if (!supportsRange && start > 0)
            {
                // 不支持 Range：返回 200 全量
                return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK)
                {
                    Content = new StreamContent(new MemoryStream(content)),
                });
            }

            if (start > 0)
            {
                var ms = new MemoryStream(content, (int)start, content.Length - (int)start);
                var resp = new HttpResponseMessage(HttpStatusCode.PartialContent)
                {
                    Content = new StreamContent(ms),
                };
                resp.Content.Headers.ContentLength = content.Length - start;
                resp.Content.Headers.ContentRange =
                    new ContentRangeHeaderValue(start, content.Length - 1, content.Length);
                return Task.FromResult(resp);
            }

            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StreamContent(new MemoryStream(content)),
            });
        }
    }

    [Fact]
    public async Task RunAsync_ValidUrl_CompletesAndRenamesPartFile()
    {
        var content = "test audio content"u8.ToArray();
        var handler = new StubHttpHandler(content);
        var http = new HttpClient(handler);
        var targetPath = Path.Combine(_testDir, "song.mp3");

        var task = new Download.DownloadTask(1)
        {
            StableKey = "test|1", Url = "http://example.com/song.mp3", TargetPath = targetPath,
        };

        await task.RunAsync(http);

        Assert.Equal(Download.DownloadStatus.Completed, task.Status);
        Assert.True(File.Exists(targetPath));
        Assert.False(File.Exists(targetPath + ".part"));
        Assert.Equal(content, await File.ReadAllBytesAsync(targetPath));
    }

    [Fact]
    public async Task RunAsync_PartialFileExists_SendsRangeHeader()
    {
        var content = "0123456789ABCDEF"u8.ToArray();
        var partPath = Path.Combine(_testDir, "resume.mp3.part");
        await File.WriteAllBytesAsync(partPath, content[..8]);

        var handler = new StubHttpHandler(content);
        var http = new HttpClient(handler);
        var targetPath = Path.Combine(_testDir, "resume.mp3");

        var task = new Download.DownloadTask(2)
        {
            StableKey = "test|2", Url = "http://example.com/resume.mp3", TargetPath = targetPath,
        };

        await task.RunAsync(http);

        Assert.True(handler.LastRequest?.Headers.Range?.Ranges.Count > 0);
        Assert.Equal(content.Length, task.BytesReceived);
        Assert.Equal(Download.DownloadStatus.Completed, task.Status);
    }

    [Fact]
    public async Task RunAsync_Cancelled_ThrowsAndSetsCancelledStatus()
    {
        var handler = new StubHttpHandler("data"u8.ToArray());
        var http = new HttpClient(handler);
        var targetPath = Path.Combine(_testDir, "cancel.mp3");

        var task = new Download.DownloadTask(3)
        {
            StableKey = "test|3", Url = "http://example.com/cancel.mp3", TargetPath = targetPath,
        };
        task.Cancel();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => task.RunAsync(http));
        Assert.Equal(Download.DownloadStatus.Cancelled, task.Status);
    }

    [Fact]
    public async Task RunAsync_ServerNoRange_RestartsFromBeginning()
    {
        var content = "full content"u8.ToArray();
        var partPath = Path.Combine(_testDir, "restart.mp3.part");
        await File.WriteAllBytesAsync(partPath, content[..4]);

        var handler = new StubHttpHandler(content, supportsRange: false);
        var http = new HttpClient(handler);
        var targetPath = Path.Combine(_testDir, "restart.mp3");

        var task = new Download.DownloadTask(4)
        {
            StableKey = "test|4", Url = "http://example.com/restart.mp3", TargetPath = targetPath,
        };

        await task.RunAsync(http);

        Assert.Equal(Download.DownloadStatus.Completed, task.Status);
        Assert.Equal(content.Length, task.BytesReceived);
    }
}
