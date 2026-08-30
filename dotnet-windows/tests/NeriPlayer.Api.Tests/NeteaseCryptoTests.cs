using NeriPlayer.Core.Api.Netease;
using Xunit;

namespace NeriPlayer.Api.Tests;

public class NeteaseCryptoTests
{
    [Fact]
    public void Weapi_ReturnsParamsAndEncSecKey()
    {
        var payload = new Dictionary<string, object> { ["s"] = "test", ["type"] = 1 };
        var result = NeteaseCrypto.Weapi(payload);

        Assert.True(result.ContainsKey("params"));
        Assert.True(result.ContainsKey("encSecKey"));
        Assert.NotEmpty(result["params"]);
        Assert.NotEmpty(result["encSecKey"]);
    }

    [Fact]
    public void Weapi_DifferentInputs_ProduceDifferentParams()
    {
        var r1 = NeteaseCrypto.Weapi(new Dictionary<string, object> { ["s"] = "a" });
        var r2 = NeteaseCrypto.Weapi(new Dictionary<string, object> { ["s"] = "b" });
        // params 应该不同（因为 payload 不同）
        Assert.NotEqual(r1["params"], r2["params"]);
    }

    [Fact]
    public void Weapi_EncSecKey_IsConsistentLength()
    {
        var result = NeteaseCrypto.Weapi(new Dictionary<string, object> { ["s"] = "x" });
        // RSA 加密后 encSecKey 长度固定（1024 bit = 256 hex chars）
        Assert.Equal(256, result["encSecKey"].Length);
    }

    [Fact]
    public void Random16Hex_Returns32Chars()
    {
        var hex = NeteaseCrypto.Random16Hex();
        Assert.Equal(32, hex.Length);
        Assert.True(hex.All(c => "0123456789abcdef".Contains(c)));
    }
}