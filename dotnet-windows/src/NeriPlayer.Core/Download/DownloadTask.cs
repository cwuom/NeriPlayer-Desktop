using System.Reactive.Subjects;

namespace NeriPlayer.Core.Download;

/// <summary>
/// 下载任务状态枚举（对标 start.md 8.1 / Analysis.md 24.2）。
/// </summary>
public enum DownloadStatus
{
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// <summary>
/// 下载进度（对标 start.md 8.1）。
/// </summary>
public sealed record DownloadProgress(
    long TaskId,
    long BytesReceived,
    long? TotalBytes,
    double Percent,
    DownloadStatus Status);

/// <summary>
/// 单曲下载任务（对标 start.md 8.1）。
/// 支持断点续传（Range header + .part 临时文件）。
/// </summary>
public sealed class DownloadTask
{
    public long TaskId { get; }
    public required string StableKey { get; init; }
    public required string Url { get; init; }
    public required string TargetPath { get; init; }
    public DownloadStatus Status { get; private set; } = DownloadStatus.Queued;
    public long BytesReceived { get; private set; }

    private readonly Subject<DownloadProgress> _progress = new();

    /// <summary>进度流（订阅获取实时进度）</summary>
    public IObservable<DownloadProgress> Progress => _progress;

    private CancellationTokenSource _cts = new();

    public DownloadTask(long taskId)
    {
        TaskId = taskId;
    }

    /// <summary>
    /// 执行下载（对标 start.md 8.1 RunAsync）。
    /// 断点续传：BytesReceived > 0 时发送 Range: bytes=N-
    /// 下载到 .part 临时文件，完成后重命名为最终路径。
    /// </summary>
    public async Task RunAsync(HttpClient http, CancellationToken externalCt = default)
    {
        using var linkedCts = CancellationTokenSource.CreateLinkedTokenSource(_cts.Token, externalCt);
        var ct = linkedCts.Token;

        var partPath = TargetPath + ".part";
        Directory.CreateDirectory(Path.GetDirectoryName(TargetPath)!);

        // 断点续传：检查已存在的 .part 文件大小
        if (File.Exists(partPath))
        {
            BytesReceived = new FileInfo(partPath).Length;
        }

        Status = DownloadStatus.Downloading;
        ReportProgress();

        try
        {
            using var request = new HttpRequestMessage(HttpMethod.Get, Url);
            if (BytesReceived > 0)
            {
                request.Headers.Range = new System.Net.Http.Headers.RangeHeaderValue(BytesReceived, null);
            }

            using var response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, ct);
            response.EnsureSuccessStatusCode();

            // 服务端不支持 Range（返回 200 而非 206）且已下载部分 → 全量重下（Process.md 9.3）
            if (BytesReceived > 0 && response.StatusCode != System.Net.HttpStatusCode.PartialContent)
            {
                BytesReceived = 0;
                File.Delete(partPath);
            }

            var totalBytes = response.Content.Headers.ContentLength;
            if (totalBytes.HasValue) totalBytes += BytesReceived;

            // 流式写入 .part 文件（嵌套 using 确保在 Move 前关闭）
            using (var contentStream = await response.Content.ReadAsStreamAsync(ct))
            using (var fileStream = new FileStream(
                partPath,
                BytesReceived > 0 ? FileMode.Append : FileMode.Create,
                FileAccess.Write,
                FileShare.None,
                bufferSize: 81920,
                useAsync: true))
            {
                var buffer = new byte[81920];
                int bytesRead;
                while ((bytesRead = await contentStream.ReadAsync(buffer, ct)) > 0)
                {
                    await fileStream.WriteAsync(buffer.AsMemory(0, bytesRead), ct);
                    BytesReceived += bytesRead;
                    ReportProgress(totalBytes);
                }
                await fileStream.FlushAsync(ct);
            }

            // 完成：删除旧文件（若有），重命名 .part → 最终路径
            if (File.Exists(TargetPath)) File.Delete(TargetPath);
            File.Move(partPath, TargetPath);

            Status = DownloadStatus.Completed;
            ReportProgress(totalBytes);
        }
        catch (OperationCanceledException)
        {
            Status = DownloadStatus.Cancelled;
            ReportProgress();
            throw;
        }
        catch
        {
            Status = DownloadStatus.Failed;
            ReportProgress();
            throw;
        }
    }

    /// <summary>取消下载</summary>
    public void Cancel()
    {
        _cts.Cancel();
    }

    private void ReportProgress(long? totalBytes = null)
    {
        var percent = totalBytes is > 0
            ? (double)BytesReceived / totalBytes.Value * 100
            : 0;
        _progress.OnNext(new DownloadProgress(TaskId, BytesReceived, totalBytes, percent, Status));
    }
}