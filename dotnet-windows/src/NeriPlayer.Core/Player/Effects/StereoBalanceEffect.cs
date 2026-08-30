namespace NeriPlayer.Core.Player.Effects;

/// <summary>对标 StereoBalanceAudioProcessor.kt：(L+R) 混音权重</summary>
public sealed class StereoBalanceEffect
{
    private float _balance;   // -1.0 全左 ~ 0 平衡 ~ +1.0 全右

    public float Balance => _balance;

    public void SetBalance(float balance) => _balance = Math.Clamp(balance, -1f, 1f);

    /// <summary>输入交错立体声 buffer，原地处理（若长度为奇数，忽略末尾样本避免越界）</summary>
    public void Process(Span<float> interleaved)
    {
        if (_balance == 0) return;
        var lw = 1f - Math.Max(0, _balance);          // 左权重
        var rw = 1f + Math.Min(0, _balance);          // 右权重
        var end = interleaved.Length - 1;             // 保护：奇数长度时忽略最后单样本
        for (var i = 0; i < end; i += 2)
        {
            var l = interleaved[i];
            var r = interleaved[i + 1];
            interleaved[i] = l * lw + r * (1 - lw);
            interleaved[i + 1] = r * rw + l * (1 - rw);
        }
    }
}
