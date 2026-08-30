using NeriPlayer.Core.Api.Bili;
using Xunit;

namespace NeriPlayer.Api.Tests;

public class WbiSignatureTests
{
    [Fact]
    public void Sign_ReturnsNonEmptyHex()
    {
        // 固定输入验证签名格式
        var imgKey = "7cd084941338484aae1ad9425b84077c";
        var subKey = "4932caff0ff746eab6f01bf08b70ac45";
        var query = new Dictionary<string, string> { ["keyword"] = "test", ["page"] = "1" };

        var sig = WbiSignature.Sign(query, imgKey, subKey);
        Assert.NotEmpty(sig);
        Assert.Equal(32, sig.Length); // MD5 = 32 hex chars
        Assert.True(sig.All(c => "0123456789abcdef".Contains(c)));
    }

    [Fact]
    public void Sign_SameInput_ProducesSameOutput()
    {
        // WBI key 需要至少 64 字符（32+32）
        var imgKey = "7cd084941338484aae1ad9425b84077c";
        var subKey = "4932caff0ff746eab6f01bf08b70ac45";
        var query = new Dictionary<string, string> { ["k"] = "v" };
        var sig1 = WbiSignature.Sign(query, imgKey, subKey);
        var sig2 = WbiSignature.Sign(query, imgKey, subKey);
        Assert.Equal(sig1, sig2);
    }

    [Fact]
    public void Sign_DifferentInput_ProducesDifferentOutput()
    {
        var imgKey = "7cd084941338484aae1ad9425b84077c";
        var subKey = "4932caff0ff746eab6f01bf08b70ac45";
        var sig1 = WbiSignature.Sign(new Dictionary<string, string> { ["k"] = "v1" }, imgKey, subKey);
        var sig2 = WbiSignature.Sign(new Dictionary<string, string> { ["k"] = "v2" }, imgKey, subKey);
        Assert.NotEqual(sig1, sig2);
    }

    [Fact]
    public void ExtractKeys_ParsesCorrectly()
    {
        var (img, sub) = WbiSignature.ExtractKeys(
            "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
            "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png");
        Assert.Equal("7cd084941338484aae1ad9425b84077c", img);
        Assert.Equal("4932caff0ff746eab6f01bf08b70ac45", sub);
    }
}