namespace NeriPlayer.Core.Player.Effects;

/// <summary>
/// Direct Form I Biquad 滤波器（RBJ Audio EQ Cookbook）
/// 注意：a0 归一化已修正——每种类型按标准 RBJ 单独计算 a0，
/// 而非文档原式的 a0 = 1 + a1 + a2（后者在 0dB 下不透明）。
/// </summary>
public sealed class BiquadFilter
{
    public enum FilterType { Peaking, LowShelf, HighShelf }

    public double B0, B1, B2, A1, A2;   // 归一化系数
    private double _x1, _x2, _y1, _y2;

    public void Configure(FilterType type, double freqHz, double gainDb,
        double sampleRate, double q = 0.707)
    {
        // 参数校验（CodeRabbit review）：除零/非有限值/非法频率会产生无效系数
        if (!double.IsFinite(sampleRate) || sampleRate <= 0)
            throw new ArgumentOutOfRangeException(nameof(sampleRate));
        if (!double.IsFinite(freqHz) || freqHz <= 0 || freqHz >= sampleRate / 2)
            throw new ArgumentOutOfRangeException(nameof(freqHz));
        if (!double.IsFinite(q) || q <= 0)
            throw new ArgumentOutOfRangeException(nameof(q));
        if (!double.IsFinite(gainDb))
            throw new ArgumentOutOfRangeException(nameof(gainDb));

        var a = Math.Pow(10, gainDb / 40.0);   // A = 10^(dB/40)
        var w0 = 2 * Math.PI * freqHz / sampleRate;
        var cos = Math.Cos(w0);
        var sin = Math.Sin(w0);
        var alpha = sin / (2 * q);

        double b0, b1, b2, a1, a2, a0;
        switch (type)
        {
            case FilterType.Peaking:
                b0 = 1 + alpha * a;  b1 = -2 * cos;  b2 = 1 - alpha * a;
                a1 = -2 * cos;       a2 = 1 - alpha / a;
                a0 = 1 + alpha / a;
                break;
            case FilterType.LowShelf:
                var sq = 2 * Math.Sqrt(a) * alpha;
                b0 = a * ((a + 1) - (a - 1) * cos + sq);
                b1 = 2 * a * ((a - 1) - (a + 1) * cos);
                b2 = a * ((a + 1) - (a - 1) * cos - sq);
                a1 = -2 * ((a - 1) + (a + 1) * cos);
                a2 = (a + 1) - (a - 1) * cos - sq;
                a0 = (a + 1) + (a - 1) * cos + sq;
                break;
            case FilterType.HighShelf:
                var sqh = 2 * Math.Sqrt(a) * alpha;
                b0 = a * ((a + 1) + (a - 1) * cos + sqh);
                b1 = -2 * a * ((a - 1) + (a + 1) * cos);
                b2 = a * ((a + 1) + (a - 1) * cos - sqh);
                a1 = 2 * ((a - 1) - (a + 1) * cos);
                a2 = (a + 1) - (a - 1) * cos - sqh;
                a0 = (a + 1) - (a - 1) * cos + sqh;
                break;
            default: return;
        }

        // RBJ 标准归一化：所有系数除以 a0
        B0 = b0 / a0; B1 = b1 / a0; B2 = b2 / a0;
        A1 = a1 / a0; A2 = a2 / a0;
    }

    public float Process(float input)
    {
        var y = B0 * input + B1 * _x1 + B2 * _x2 - A1 * _y1 - A2 * _y2;
        _x2 = _x1; _x1 = input;
        _y2 = _y1; _y1 = y;
        return (float)y;
    }

    public void Reset() { _x1 = _x2 = _y1 = _y2 = 0; }
}
