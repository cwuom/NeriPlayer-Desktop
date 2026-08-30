using NeriPlayer.Core.Player.Effects;

namespace NeriPlayer.Core.Tests;

public class FftAnalyzerTests
{
    [Fact]
    public void OfSine_ReturnsNonZeroPeak()
    {
        var fft = new FftAnalyzer(1024);
        var samples = new float[1024];
        for (var i = 0; i < 1024; i++)
            samples[i] = MathF.Sin(2 * MathF.PI * 440f * i / 44100f);   // 440Hz
        var bands = fft.Compute(samples);
        Assert.True(bands.Max() > 0f);
    }

    [Fact]
    public void OfSine_PeaksInLowerBands()
    {
        // 440Hz 落在 20Hz~20kHz 对数刻度的中低频段（约 band 28）
        var fft = new FftAnalyzer(1024);
        var samples = new float[1024];
        for (var i = 0; i < 1024; i++)
            samples[i] = MathF.Sin(2 * MathF.PI * 440f * i / 44100f);
        var bands = fft.Compute(samples);
        var peak = Array.IndexOf(bands, bands.Max());
        Assert.InRange(peak, 20, 40);
    }

    [Fact]
    public void Silence_IsZero()
    {
        var fft = new FftAnalyzer(1024);
        var bands = fft.Compute(new float[1024]);
        Assert.All(bands, b => Assert.Equal(0f, b));
    }

    // ── 参数校验（CodeRabbit review） ────────────────────────────────

    [Theory]
    [InlineData(1000)]   // 非 2 的幂
    [InlineData(0)]
    [InlineData(-512)]
    [InlineData(1)]      // 不大于 1
    public void Size_Invalid_Throws(int size)
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => _ = new FftAnalyzer(size));
    }

    [Theory]
    [InlineData(0f)]
    [InlineData(-44_100f)]
    [InlineData(float.NaN)]
    public void SampleRate_Invalid_Throws(float sampleRate)
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => _ = new FftAnalyzer(1024, sampleRate));
    }

    [Fact]
    public void DifferentSampleRate_StillDetectsPeak()
    {
        // 48kHz 采样率下频带映射也应正确（基于真实 Nyquist = 24kHz）
        var fft = new FftAnalyzer(1024, 48_000f);
        var samples = new float[1024];
        for (var i = 0; i < 1024; i++)
            samples[i] = MathF.Sin(2 * MathF.PI * 440f * i / 48_000f);   // 440Hz
        var bands = fft.Compute(samples);
        var peak = Array.IndexOf(bands, bands.Max());
        Assert.True(bands.Max() > 0f);
        Assert.InRange(peak, 20, 40);
    }
}
