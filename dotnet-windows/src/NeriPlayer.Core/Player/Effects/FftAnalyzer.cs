namespace NeriPlayer.Core.Player.Effects;

/// <summary>Cooley-Tukey 基 2 FFT + Hann 窗，输出 64 频带对数刻度</summary>
public sealed class FftAnalyzer
{
    private readonly int _size;
    private readonly float _sampleRate;
    private readonly float[] _window;

    public FftAnalyzer(int size = 1024, float sampleRate = 44_100f)
    {
        // 参数校验（CodeRabbit review）：size 须为大于 1 的 2 的幂；sampleRate 须为正有限值
        if (size <= 1 || (size & (size - 1)) != 0)
            throw new ArgumentOutOfRangeException(nameof(size),
                "FFT size must be a power of two greater than 1.");
        if (!float.IsFinite(sampleRate) || sampleRate <= 0)
            throw new ArgumentOutOfRangeException(nameof(sampleRate));

        _size = size;
        _sampleRate = sampleRate;
        _window = Enumerable.Range(0, size)
            .Select(i => 0.5f * (1 - MathF.Cos(2 * MathF.PI * i / (size - 1))))  // Hann
            .ToArray();
    }

    public int Size => _size;

    public float[] Compute(Span<float> samples)
    {
        var re = new float[_size];
        var im = new float[_size];
        for (var i = 0; i < Math.Min(samples.Length, _size); i++)
        {
            re[i] = samples[i] * _window[i];
            im[i] = 0;
        }
        Fft(re, im);

        const int bands = 64;
        var result = new float[bands];
        var nyquist = _sampleRate / 2;
        var maxFrequency = MathF.Min(20_000f, nyquist);   // 人耳范围上限
        var logMin = MathF.Log10(20);
        var logMax = MathF.Log10(maxFrequency);

        for (var b = 0; b < bands; b++)
        {
            var f0 = MathF.Pow(10, logMin + (logMax - logMin) * b / bands);
            var f1 = MathF.Pow(10, logMin + (logMax - logMin) * (b + 1) / bands);
            var i0 = Math.Clamp((int)(f0 / nyquist * (_size / 2)), 0, _size / 2 - 1);
            var i1 = Math.Clamp((int)(f1 / nyquist * (_size / 2)), i0 + 1, _size / 2);
            var sum = 0f;
            for (var i = i0; i < i1; i++)
                sum += MathF.Sqrt(re[i] * re[i] + im[i] * im[i]);
            result[b] = sum / MathF.Max(1, i1 - i0);
        }
        return result;
    }

    private static void Fft(Span<float> re, Span<float> im)
    {
        var n = re.Length;
        for (int i = 1, j = 0; i < n; i++)
        {
            var bit = n >> 1;
            for (; (j & bit) != 0; bit >>= 1) j ^= bit;
            j ^= bit;
            if (i < j) (re[i], re[j]) = (re[j], re[i]);
        }
        for (var len = 2; len <= n; len <<= 1)
        {
            var ang = -2 * MathF.PI / len;
            var wRe = MathF.Cos(ang);
            var wIm = MathF.Sin(ang);
            for (var i = 0; i < n; i += len)
            {
                var curRe = 1f; var curIm = 0f;
                for (var k = 0; k < len / 2; k++)
                {
                    var uRe = re[i + k]; var uIm = im[i + k];
                    var vRe = re[i + k + len/2] * curRe - im[i + k + len/2] * curIm;
                    var vIm = re[i + k + len/2] * curIm + im[i + k + len/2] * curRe;
                    re[i + k] = uRe + vRe;       im[i + k] = uIm + vIm;
                    re[i + k + len/2] = uRe - vRe;  im[i + k + len/2] = uIm - vIm;
                    (curRe, curIm) = (curRe*wRe - curIm*wIm, curRe*wIm + curIm*wRe);
                }
            }
        }
    }
}
