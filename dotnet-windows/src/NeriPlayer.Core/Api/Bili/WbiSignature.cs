using System.Security.Cryptography;
using System.Text;

namespace NeriPlayer.Core.Api.Bili;

/// <summary>WBI 签名（对标 Analysis.md 4.2 / 22.4）</summary>
public static class WbiSignature
{
    // MixinKeyEncTab（Bilibili 标准混淆表，固定不变）
    private static readonly int[] MixinKeyEncTab =
        [46,47,18,2,53,8,23,32,15,50,10,31,58,3,45,35,27,43,5,49,
         33,9,42,19,29,28,14,39,12,38,41,13,37,48,7,16,24,55,40,61,
         26,17,0,1,60,51,30,4,22,25,54,21,56,59,6,63,57,62,11,36,20,34,44,52];

    /// <summary>
    /// 签名计算：w_rid = MD5(mixedKey + wts + sorted_params)
    /// </summary>
    /// <param name="query">业务参数（不含 w_rid 和 wts）</param>
    /// <param name="imgKey">从 x/web-interface/nav 获取的 img_url 的 key 部分</param>
    /// <param name="subKey">从 x/web-interface/nav 获取的 sub_url 的 key 部分</param>
    public static string Sign(Dictionary<string, string> query, string imgKey, string subKey)
    {
        var raw = imgKey + subKey;
        var mixed = new string(MixinKeyEncTab.Select(i => raw[i]).ToArray());
        var joined = string.Join("&", query.OrderBy(kv => kv.Key).Select(kv => $"{kv.Key}={kv.Value}"));
        var m = MD5.HashData(Encoding.UTF8.GetBytes(mixed + joined));
        return Convert.ToHexString(m).ToLowerInvariant();
    }

    /// <summary>提取 img_url / sub_url 的 key 部分（取最后一段，去掉扩展名）</summary>
    public static (string imgKey, string subKey) ExtractKeys(string imgUrl, string subUrl)
    {
        var imgKey = imgUrl.Split('/').Last().Split('.').First();
        var subKey = subUrl.Split('/').Last().Split('.').First();
        return (imgKey, subKey);
    }
}