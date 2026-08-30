namespace NeriPlayer.Core.Api.YouTube;

/// <summary>
/// YouTube player.js 缓存（对标 Analysis.md 4.3 YouTubePlayerScriptStore）
/// 本地缓存 player.js，48h 过期刷新。
/// </summary>
public sealed class YouTubePlayerScriptStore
{
    private static readonly TimeSpan CacheExpiry = TimeSpan.FromHours(48);
    private readonly string _cacheDir;

    public YouTubePlayerScriptStore(string? cacheDir = null)
    {
        _cacheDir = cacheDir ?? Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "NeriPlayer", "yt_cache");
        Directory.CreateDirectory(_cacheDir);
    }

    private string ScriptPath => Path.Combine(_cacheDir, "player.js");
    private string MetaPath => Path.Combine(_cacheDir, "player_meta.json");

    /// <summary>获取缓存的 player.js（48h 内有效返回 true，否则返回 null）</summary>
    public (string script, bool valid)? GetCached()
    {
        if (!File.Exists(ScriptPath) || !File.Exists(MetaPath)) return null;
        try
        {
            var meta = System.Text.Json.JsonSerializer.Deserialize<Meta>(
                File.ReadAllText(MetaPath));
            if (meta is null || DateTimeOffset.UtcNow - meta.CachedAt > CacheExpiry) return null;
            return (File.ReadAllText(ScriptPath), true);
        }
        catch { return null; }
    }

    /// <summary>写入缓存</summary>
    public void Save(string script)
    {
        File.WriteAllText(ScriptPath, script);
        File.WriteAllText(MetaPath,
            System.Text.Json.JsonSerializer.Serialize(new Meta(DateTimeOffset.UtcNow)));
    }

    /// <summary>强制失效缓存</summary>
    public void Invalidate()
    {
        if (File.Exists(ScriptPath)) File.Delete(ScriptPath);
        if (File.Exists(MetaPath)) File.Delete(MetaPath);
    }

    private sealed record Meta(DateTimeOffset CachedAt);
}