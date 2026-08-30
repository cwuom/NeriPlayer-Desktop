using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace NeriPlayer.Core.Api.Netease;

/// <summary>
/// weapi 加密：AES-128-CBC + RSA + 随机 secretKey（对标 Analysis.md 22.4 / NeteaseCrypto.kt）
/// RSA 公钥来源于 Tauri 版 crypto.rs（NeriPlayer-Desktop/src-tauri/src/api/netease/crypto.rs:20-24）
/// </summary>
public static class NeteaseCrypto
{
    private const string AesKey = "0CoJUm6Qyw8W8jud";   // 固定 key（weapi 标准）
    private const string AesIv  = "0102030405060708";
    private const string RsaExponent = "010001";
    // 1024-bit RSA 公钥（去掉前导 00，C# RSAParameters.Modulus 用 128 字节）
    private const string RsaModulus =
        "e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b72" +
        "5152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312" +
        "ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d" +
        "813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";

    private static readonly RandomNumberGenerator Rng = RandomNumberGenerator.Create();

    /// <summary>
    /// 加密请求体，返回 FormUrlEncodedContent 所需的 (params, encSecKey) 字典。
    /// </summary>
    public static Dictionary<string, string> Weapi(Dictionary<string, object> payload)
    {
        var text = JsonSerializer.Serialize(payload);
        var secretKey = Random16Hex();
        var params1 = AesEncrypt(text, AesKey);   // 第一层：固定 key
        var params2 = AesEncrypt(params1, secretKey); // 第二层：随机 key
        var encSecKey = RsaEncrypt(secretKey);

        return new Dictionary<string, string>
        {
            ["params"] = params2,
            ["encSecKey"] = encSecKey,
        };
    }

    /// <summary>生成 16 字节随机 hex 小写字符串</summary>
    public static string Random16Hex()
    {
        var bytes = new byte[16];
        Rng.GetBytes(bytes);
        return Convert.ToHexString(bytes).ToLowerInvariant();
    }

    private static string AesEncrypt(string input, string key)
    {
        using var aes = Aes.Create();
        aes.Key  = Encoding.UTF8.GetBytes(key);
        aes.IV   = Encoding.UTF8.GetBytes(AesIv);
        aes.Mode = CipherMode.CBC;
        aes.Padding = PaddingMode.PKCS7;
        using var enc = aes.CreateEncryptor();
        var bytes = Encoding.UTF8.GetBytes(input);
        var outBytes = enc.TransformFinalBlock(bytes, 0, bytes.Length);
        return Convert.ToHexString(outBytes).ToLowerInvariant();
    }

    private static string RsaEncrypt(string input)
    {
        using var rsa = RSA.Create();
        var exponent = Convert.FromHexString(RsaExponent);
        var modulus  = Convert.FromHexString(RsaModulus);
        rsa.ImportParameters(new RSAParameters { Exponent = exponent, Modulus = modulus });

        // 逆序输入字节再加密（网易云 weapi 标准）
        var text = Encoding.UTF8.GetBytes(input).Reverse().ToArray();
        var encrypted = rsa.Encrypt(text, RSAEncryptionPadding.Pkcs1);
        return Convert.ToHexString(encrypted).ToLowerInvariant();
    }
}