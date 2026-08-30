using System.Text;
using Octokit;

namespace NeriPlayer.Data.Sync;

/// <summary>
/// GitHub 同步后端：仓库内 JSON 文件（对标 start.md 9.2 / Analysis.md 9.1 GitHubApiClient）。
/// 用 Git Data API 提交正文，etag 即文件 SHA，用于并发保护。
/// </summary>
public sealed class GitHubSyncProvider : ISyncProvider
{
    private readonly GitHubClient _client;
    private readonly string _owner;
    private readonly string _repo;
    private readonly string _pathPrefix;

    public GitHubSyncProvider(string token, string owner, string repo, string pathPrefix = "neriplayer")
    {
        _client = new GitHubClient(new ProductHeaderValue("NeriPlayer.Windows"))
        {
            Credentials = new Credentials(token)
        };
        _owner = owner;
        _repo = repo;
        _pathPrefix = pathPrefix.Trim('/');
    }

    public string ProviderName => "github";

    public async Task<bool> TestConnectionAsync()
    {
        try
        {
            var user = await _client.User.Current();
            return !string.IsNullOrEmpty(user.Login);
        }
        catch
        {
            return false;
        }
    }

    public async Task<IReadOnlyList<SyncFile>> ListAsync(string scope)
    {
        var items = await _client.Repository.Content.GetAllContents(_owner, _repo,
            $"{_pathPrefix}/{scope}");
        return items.Select(i => new SyncFile(i.Name, Array.Empty<byte>(), i.Sha,
            DateTimeOffset.UtcNow)).ToList();
    }

    public async Task<SyncFile?> DownloadAsync(string scope, string name)
    {
        var items = await _client.Repository.Content.GetAllContents(_owner, _repo,
            $"{_pathPrefix}/{scope}/{name}");
        var item = items.FirstOrDefault();
        if (item is null) return null;

        // GitHub Content API 返回 Base64 编码，用 EncodedContent 解码
        var content = item.EncodedContent ?? item.Content ?? "";
        byte[] bytes;
        try { bytes = Convert.FromBase64String(content); }
        catch { bytes = Encoding.UTF8.GetBytes(content); }
        return new SyncFile(name, bytes, item.Sha, DateTimeOffset.UtcNow);
    }

    public async Task<bool> UploadAsync(string scope, string name, byte[] content, string? etag)
    {
        var path = $"{_pathPrefix}/{scope}/{name}";
        var text = Encoding.UTF8.GetString(content);
        try
        {
            if (etag is not null)
                await _client.Repository.Content.UpdateFile(_owner, _repo, path,
                    new UpdateFileRequest($"sync {name}", text, etag));
            else
                await _client.Repository.Content.CreateFile(_owner, _repo, path,
                    new CreateFileRequest($"sync {name}", text));
            return true;
        }
        catch (NotFoundException)
        {
            // 等值冲突 / 文件被对端删除：尝试重建（乐观锁失败时允许重新创建）
            await _client.Repository.Content.CreateFile(_owner, _repo, path,
                new CreateFileRequest($"sync {name}", text));
            return true;
        }
        catch (Exception)
        {
            return false;
        }
    }

    public async Task<bool> DeleteAsync(string scope, string name)
    {
        var path = $"{_pathPrefix}/{scope}/{name}";
        var items = await _client.Repository.Content.GetAllContents(_owner, _repo, path);
        var item = items.FirstOrDefault();
        if (item is null) return true; // 已不存在视为成功
        try
        {
            await _client.Repository.Content.DeleteFile(_owner, _repo, path,
                new DeleteFileRequest($"delete {name}", item.Sha));
            return true;
        }
        catch
        {
            return false;
        }
    }
}
