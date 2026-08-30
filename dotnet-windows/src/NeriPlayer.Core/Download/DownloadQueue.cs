namespace NeriPlayer.Core.Download;

/// <summary>
/// 并发下载队列（对标 start.md 8.2 / Analysis.md 24.2）。
/// Semaphore 控制并发（默认 6，最大 8），取消后 5000ms 稳定期。
/// </summary>
public sealed class DownloadQueue : IDisposable
{
    /// <summary>默认并发数（对标 Analysis.md 24.2）</summary>
    public const int DefaultConcurrency = 6;

    /// <summary>最大并发数</summary>
    public const int MaxConcurrency = 8;

    /// <summary>取消后稳定期（ms）（对标 Analysis.md 24.2 DOWNLOAD_CANCEL_SETTLE_TIMEOUT_MS）</summary>
    public const int CancelSettleTimeoutMs = 5000;

    private readonly SemaphoreSlim _semaphore;
    private readonly Queue<DownloadTask> _pending = new();
    private readonly List<Task> _running = [];
    private readonly HttpClient _http;
    private readonly object _lock = new();
    private bool _disposed;

    /// <summary>任务完成时触发（无论成功/失败/取消）</summary>
    public event Action<DownloadTask>? Completed;

    public DownloadQueue(HttpClient http, int concurrency = DefaultConcurrency)
    {
        _http = http;
        _semaphore = new SemaphoreSlim(concurrency, MaxConcurrency);
    }

    /// <summary>入队下载任务</summary>
    public void Enqueue(DownloadTask task)
    {
        lock (_lock)
        {
            _pending.Enqueue(task);
        }
        _ = PumpAsync();
    }

    /// <summary>信号量泵：控制并发</summary>
    private async Task PumpAsync()
    {
        while (true)
        {
            DownloadTask? task;
            lock (_lock)
            {
                if (_pending.Count == 0 || _semaphore.CurrentCount == 0) return;
                task = _pending.Dequeue();
            }

            await _semaphore.WaitAsync();
            var runTask = RunTaskAsync(task);
            lock (_lock)
            {
                _running.Add(runTask);
            }
        }
    }

    private async Task RunTaskAsync(DownloadTask task)
    {
        try
        {
            await task.RunAsync(_http);
        }
        catch (OperationCanceledException)
        {
            // 取消后等待稳定期（对标 Analysis.md 24.2）
            await Task.Delay(CancelSettleTimeoutMs);
        }
        catch
        {
            // 失败由 task.Status 标记
        }
        finally
        {
            _semaphore.Release();
            lock (_lock)
            {
                _running.RemoveAll(t => t.IsCompleted);
            }
            Completed?.Invoke(task);
            // 重启 pump：信号量释放后处理队列中剩余任务
            _ = PumpAsync();
        }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        _semaphore.Dispose();
    }
}