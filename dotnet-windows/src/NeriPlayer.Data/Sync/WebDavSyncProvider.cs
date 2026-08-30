using WebDav;

namespace NeriPlayer.Data.Sync;

/// <summary>
/// WebDAV 同步后端：远程目录（对标 start.md 9.3 / Analysis.md 9.2 WebDavApiClient）。
/// etag 即 resource ETag，上传时带 If-Match 做并发保护。
/// </summary>
public sealed class WebDavSyncProvider : ISyncProvider
{
    private readonly WebDavClient _client;
    private readonly string _root;

    public WebDavSyncProvider(Uri server, string user, string password, string root = "neriplayer")
    {
        _client = new WebDavClient(new WebDavClientParams
        {
            BaseAddress = server,
            Credentials = new System.Net.NetworkCredential(user, password),
        });
        _root = root.Trim('/');
    }

    public string ProviderName => "webdav";

    public async Task<bool> TestConnectionAsync()
    {
        try
        {
            var result = await _client.Propfind(_root);
            return result.IsSuccessful;
        }
        catch
        {
            return false;
        }
    }

    public async Task<IReadOnlyList<SyncFile>> ListAsync(string scope)
    {
        var result = await _client.Propfind($"{_root}/{scope}");
        return result.Resources
            .Where(r => !r.IsCollection)
            .Select(r => new SyncFile(
                r.Uri?.Split('/').Last() ?? r.DisplayName ?? "",
                Array.Empty<byte>(),
                r.ETag,
                r.LastModifiedDate ?? DateTimeOffset.MinValue))
            .ToList();
    }

    public async Task<SyncFile?> DownloadAsync(string scope, string name)
    {
        var resp = await _client.GetRawFile($"{_root}/{scope}/{name}");
        if (!resp.IsSuccessful) return null;

        using var ms = new MemoryStream();
        await resp.Stream.CopyToAsync(ms);
        return new SyncFile(name, ms.ToArray(), null, DateTimeOffset.UtcNow);
    }

    public async Task<bool> UploadAsync(string scope, string name, byte[] content, string? etag)
    {
        var path = $"{_root}/{scope}/{name}";
        using var ms = new MemoryStream(content);
        try
        {
            WebDavResponse resp;
            if (etag is not null)
            {
                // 用 PutFileParameters.Headers 携带 If-Match 做并发保护
                var parameters = new PutFileParameters
                {
                    Headers = new List<KeyValuePair<string, string>>
                    {
                        new("If-Match", etag),
                    },
                };
                resp = await _client.PutFile(path, ms, parameters);
            }
            else
            {
                resp = await _client.PutFile(path, ms);
            }
            return resp.IsSuccessful;
        }
        catch
        {
            return false;
        }
    }

    public async Task<bool> DeleteAsync(string scope, string name)
    {
        try
        {
            var resp = await _client.Delete($"{_root}/{scope}/{name}");
            return resp.IsSuccessful;
        }
        catch
        {
            return false;
        }
    }
}

