namespace NeriPlayer.Data.Sync;

/// <summary>同步文件记录（对标 start.md 9.1 / Analysis.md 9.1 SyncFile）</summary>
public sealed record SyncFile(string Name, byte[] Content, string? Etag, DateTimeOffset ModifiedAt);

/// <summary>同步执行结果（对标 start.md 9.1 SyncResult）</summary>
public sealed record SyncResult(bool Success, int ChangedCount, string? Message);

/// <summary>
/// 同步后端抽象：GitHub 仓库 或 WebDAV 目录（对标 start.md 9.1 / Process.md 10.1 ISyncProvider）。
/// </summary>
public interface ISyncProvider
{
    string ProviderName { get; }

    /// <summary>验证连接/凭据是否有效</summary>
    Task<bool> TestConnectionAsync();

    /// <summary>列出指定 scope 下的所有同步文件</summary>
    Task<IReadOnlyList<SyncFile>> ListAsync(string scope);

    /// <summary>下载指定文件；不存在返回 null</summary>
    Task<SyncFile?> DownloadAsync(string scope, string name);

    /// <summary>上传文件；etag 非空时做并发保护（乐观锁）</summary>
    Task<bool> UploadAsync(string scope, string name, byte[] content, string? etag);

    /// <summary>删除指定文件</summary>
    Task<bool> DeleteAsync(string scope, string name);
}
