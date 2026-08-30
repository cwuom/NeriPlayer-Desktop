namespace NeriPlayer.Core.Player.Effects;

/// <summary>
/// 10 段图形均衡器（首段 LowShelf / 末段 HighShelf / 中间 Peaking）
/// </summary>
public sealed class EqualizerEffect
{
    public static readonly double[] BandsHz =
        { 31.25, 62.5, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000 };

    public static readonly IReadOnlyDictionary<string, double[]> Presets =
        new Dictionary<string, double[]>
        {
            ["默认"] = [0,0,0,0,0,0,0,0,0,0],
            ["流行"] = [-1,0,1,2,3,2,0,1,2,1],
            ["摇滚"] = [3,2,0,-1,1,2,3,2,1,0],
            ["爵士"] = [2,1,0,1,2,2,0,0,1,2],
            ["古典"] = [2,1,0,0,-1,-1,0,1,2,3],
            ["电子"] = [2,2,1,0,-1,0,1,2,3,3],
            ["人声"] = [-2,-1,0,1,2,3,3,2,1,-1],
        };

    private readonly BiquadFilter[] _filters;

    public EqualizerEffect(double sampleRate = 44100)
    {
        _filters = BandsHz.Select(_ => new BiquadFilter()).ToArray();
        SampleRate = sampleRate;
    }

    public double SampleRate { get; }

    public void ApplyGains(IReadOnlyList<double> gainsDb)
    {
        // 参数校验（CodeRabbit review）：每个频段必须有恰好一个增益值
        if (gainsDb.Count != _filters.Length)
            throw new ArgumentException(
                $"Expected {_filters.Length} gain values for {_filters.Length} bands, but got {gainsDb.Count}.",
                nameof(gainsDb));

        for (var i = 0; i < _filters.Length; i++)
        {
            var type = i == 0 ? BiquadFilter.FilterType.LowShelf
                      : i == _filters.Length - 1 ? BiquadFilter.FilterType.HighShelf
                      : BiquadFilter.FilterType.Peaking;
            _filters[i].Configure(type, BandsHz[i],
                Math.Clamp(gainsDb[i], -20, 20), SampleRate);
        }
    }

    public float Process(float sample)
    {
        foreach (var f in _filters) sample = f.Process(sample);
        return sample;
    }
}
