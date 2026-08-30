using NeriPlayer.Core.Player.Engine;

namespace NeriPlayer.Core.Tests;

/// <summary>
/// 手动播放验证（临时，仅手动执行，不进 CI）
/// 用法：在测试项目中运行 ManualPlay.RunAsync(new Uri(@"D:\Music\demo.flac"), TimeSpan.FromSeconds(10))
/// </summary>
public static class ManualPlay
{
    public static async Task RunAsync(Uri uri, TimeSpan seconds)
    {
        using var engine = new VlcPlaybackEngine();
        await engine.LoadAsync(uri, new PlaybackEngineOptions());
        await engine.PlayAsync();
        await Task.Delay(seconds);
        await engine.PauseAsync();
    }
}
