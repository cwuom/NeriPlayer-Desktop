using System.Runtime.Versioning;
using System.Security.Cryptography;
using System.Text;

namespace NeriPlayer.Data.Auth;

/// <summary>
/// 凭据加密存储（对标 Process.md 13.1 / start.md 13.1 DPAPI）。
/// 用 DPAPI（CurrentUser）加密 Cookie / 刷新 Token / 同步凭据，写入安全目录。
/// </summary>
[SupportedOSPlatform("windows")]
public sealed class CredentialStore
{
    private readonly string _dir;

    public CredentialStore(string? baseDir = null)
    {
        _dir = baseDir ?? Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "NeriPlayer", "secure");
        Directory.CreateDirectory(_dir);
    }

    private string PathFor(string key) => Path.Combine(_dir, key);

    /// <summary>加密并保存明文</summary>
    public void Save(string key, string plaintext) => Save(key, Encoding.UTF8.GetBytes(plaintext));

    public void Save(string key, byte[] plaintext)
    {
        var encrypted = ProtectedData.Protect(plaintext, null, DataProtectionScope.CurrentUser);
        File.WriteAllBytes(PathFor(key), encrypted);
    }

    /// <summary>读取并解密；不存在或解密失败返回 null（密钥失效时自动清除）</summary>
    public byte[]? LoadBytes(string key)
    {
        var path = PathFor(key);
        if (!File.Exists(path)) return null;
        try
        {
            return ProtectedData.Unprotect(File.ReadAllBytes(path), null, DataProtectionScope.CurrentUser);
        }
        catch (CryptographicException)
        {
            File.Delete(path); // 密钥失效 → 清除
            return null;
        }
    }

    public string? Load(string key)
    {
        var bytes = LoadBytes(key);
        return bytes is null ? null : Encoding.UTF8.GetString(bytes);
    }

    public void Delete(string key)
    {
        var path = PathFor(key);
        if (File.Exists(path)) File.Delete(path);
    }

    public bool Exists(string key) => File.Exists(PathFor(key));
}
