using NeriPlayer.Core.Player.Effects;

namespace NeriPlayer.Core.Tests;

/// <summary>
/// 立体声平衡端点行为测试。
/// 说明：balance=-1.0 全左（双声道=原左信号），+1.0 全右（双声道=原右信号）。
/// </summary>
public class StereoBalanceEffectTests
{
    [Fact]
    public void LeftEndpoint_MixesToLeftChannel()
    {
        var sb = new StereoBalanceEffect();
        sb.SetBalance(-1.0f);   // 全左
        var data = new float[] { 1f, -1f, 1f, -1f };   // L=1, R=-1
        sb.Process(data);
        Assert.Equal(1f, data[0], 3);   // 左 = 原左
        Assert.Equal(1f, data[1], 3);   // 右也变成原左信号
        Assert.Equal(1f, data[2], 3);
        Assert.Equal(1f, data[3], 3);
    }

    [Fact]
    public void RightEndpoint_MixesToRightChannel()
    {
        var sb = new StereoBalanceEffect();
        sb.SetBalance(1.0f);   // 全右
        var data = new float[] { 1f, -1f, 1f, -1f };   // L=1, R=-1
        sb.Process(data);
        Assert.Equal(-1f, data[0], 3);   // 左变成原右信号
        Assert.Equal(-1f, data[1], 3);   // 右 = 原右
        Assert.Equal(-1f, data[2], 3);
        Assert.Equal(-1f, data[3], 3);
    }

    [Fact]
    public void Center_IsPassthrough()
    {
        var sb = new StereoBalanceEffect();
        sb.SetBalance(0f);   // 平衡
        var data = new float[] { 1f, -1f, 1f, -1f };
        sb.Process(data);
        Assert.Equal(1f, data[0]);
        Assert.Equal(-1f, data[1]);
        Assert.Equal(1f, data[2]);
        Assert.Equal(-1f, data[3]);
    }

    [Fact]
    public void Balance_IsClampedToUnitRange()
    {
        var sb = new StereoBalanceEffect();
        sb.SetBalance(99f);
        Assert.Equal(1f, sb.Balance);
        sb.SetBalance(-99f);
        Assert.Equal(-1f, sb.Balance);
    }

    [Fact]
    public void OddLengthBuffer_DoesNotThrow()
    {
        var sb = new StereoBalanceEffect();
        sb.SetBalance(0.5f);
        var data = new float[] { 1f, -1f, 1f };   // 奇数长度（异常数据）
        sb.Process(data);   // 不应越界/抛异常，末尾样本忽略
        Assert.True(float.IsFinite(data[0]));
        Assert.True(float.IsFinite(data[1]));
    }
}
