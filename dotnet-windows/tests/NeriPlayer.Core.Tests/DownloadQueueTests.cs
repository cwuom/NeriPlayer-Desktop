using System.Collections.Concurrent;

namespace NeriPlayer.Core.Tests;

/// <summary>
/// DownloadQueue 单元测试：验证并发控制与任务完成事件。
/// </summary>
public class DownloadQueueTests : IDisposable
{
    private readonly string _testDir;

    public DownloadQueueTests()
    {
        _testDir = Path.Combine(Path.GetTempPath(), $"neri_dq_test_{Guid.NewGuid():N}");
        Directory.CreateDirectory(_testDir);
    }

    public void Dispose()
    {
        if (Directory.Exists(_testDir))
            Directory.Delete(_testDir, true);
    }

    /// <summary>模拟慢速下载：每个任务延迟 delayMs 毫秒。</summary>
    private sealed class SlowHttpHandler(int delayMs = 50) : HttpMessageHandler
    {
        public int ActiveRequests { get; private set; }
        public int MaxConcurrentRequests { get; private set; }
        private readonly object _lock = new();

        protected override async Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken ct)
        {
            lock (_lock)
            {
                ActiveRequests++;
                if (ActiveRequests > MaxConcurrentRequests)
                    MaxConcurrentRequests = ActiveRequests;
            }

            try
            {
                await Task.Delay(delayMs, ct);
                var ms = new MemoryStream("data"u8.ToArray());
                return new HttpResponseMessage(System.Net.HttpStatusCode.OK)
                {
                    Content = new System.Net.Http.StreamContent(ms),
                };
            }
            finally
            {
                lock (_lock) { ActiveRequests--; }
            }
        }
    }

    [Fact]
    public void Enqueue_MultipleTasks_CompletedEventFiredForEach()
    {
        var handler = new SlowHttpHandler(10);
        var http = new HttpClient(handler);
        using var queue = new Download.DownloadQueue(http, concurrency: 3);

        var completedTasks = new ConcurrentBag<Download.DownloadTask>();
        var allDone = new CountdownEvent(4);
        queue.Completed += t => { completedTasks.Add(t); allDone.Signal(); };

        for (long i = 1; i <= 4; i++)
        {
            var task = new Download.DownloadTask(i)
            {
                StableKey = $"test|{i}",
                Url = $"http://example.com/song{i}.mp3",
                TargetPath = Path.Combine(_testDir, $"song{i}.mp3"),
            };
            queue.Enqueue(task);
        }

        // 等待 4 个任务全部完成（最多 10 秒）
        Assert.True(allDone.Wait(TimeSpan.FromSeconds(10)), "Not all tasks completed in time");
        Assert.Equal(4, completedTasks.Count);
        Assert.All(completedTasks, t =>
            Assert.Equal(Download.DownloadStatus.Completed, t.Status));
    }

    [Fact]
    public void Enqueue_ConcurrentLimit_Respected()
    {
        var handler = new SlowHttpHandler(delayMs: 200);
        var http = new HttpClient(handler);
        using var queue = new Download.DownloadQueue(http, concurrency: 2);

        // 等待 4 个任务全部完成（确保 Dispose 前没有运行中的任务占用 .part 文件）
        var allDone = new CountdownEvent(4);
        queue.Completed += _ => allDone.Signal();

        for (long i = 1; i <= 4; i++)
        {
            var task = new Download.DownloadTask(i)
            {
                StableKey = $"test|{i}",
                Url = $"http://example.com/song{i}.mp3",
                TargetPath = Path.Combine(_testDir, $"song{i}.mp3"),
            };
            queue.Enqueue(task);
        }

        Assert.True(allDone.Wait(TimeSpan.FromSeconds(10)), "Not all tasks completed in time");

        // 同时运行数不应超过并发上限 2
        Assert.True(handler.MaxConcurrentRequests <= 2,
            $"Max concurrent: {handler.MaxConcurrentRequests}, expected ≤ 2");
    }
}
