using System.Reflection;
using System.Reactive.Linq;
using NeriPlayer.Core.Player;
using NeriPlayer.Core.Player.Engine;
using NeriPlayer.Core.Player.Model;

namespace NeriPlayer.Core.Tests;

/// <summary>
/// PlayerManager 状态机测试（用 MemoryPlaybackEngine 替代真实 VLC）
/// </summary>
public class PlayerManagerTests
{
    private static SongItem MakeLocalSong(long id, string path = @"D:\Music\test.mp3") => new()
    {
        Id = id, Name = $"Song{id}", Artist = "Test", Album = "Album",
        ChannelId = "local", LocalFilePath = path,
        MediaUri = $"file:///{path.Replace('\\', '/')}"
    };

    private static SongItem MakeRemoteSong(long id, string streamUrl = "https://old.example/song.mp3") => new()
    {
        Id = id, Name = $"Remote{id}", Artist = "Test", Album = "Album",
        ChannelId = "netease", AudioId = id.ToString(), StreamUrl = streamUrl,
    };

    /// <summary>通过反射将保鲜时间戳设为给定值，触发/抑制 URL 保鲜判定。</summary>
    private static void SetLastRefresh(PlayerManager pm, DateTimeOffset value) =>
        typeof(PlayerManager).GetField("_lastUrlRefreshAt", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(pm, value);

    [Fact]
    public async Task PlayAsync_TransitionsToPlaying()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        var states = new List<PlaybackState>();
        pm.State.Subscribe(states.Add);

        await pm.PlayAsync([MakeLocalSong(1)], 0);

        Assert.Contains(PlaybackState.Loading, states);
        Assert.Equal(PlaybackState.Playing, pm.CurrentState);
        Assert.NotNull(pm.NowPlaying);
        Assert.Equal("Song1", pm.NowPlaying!.Name);
    }

    [Fact]
    public async Task PauseAsync_TransitionsToPaused()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        await pm.PlayAsync([MakeLocalSong(1)], 0);
        await pm.PauseAsync();
        Assert.Equal(PlaybackState.Paused, pm.CurrentState);
    }

    [Fact]
    public async Task ResumeAsync_BackToPlaying()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        await pm.PlayAsync([MakeLocalSong(1)], 0);
        await pm.PauseAsync();
        await pm.ResumeAsync();
        Assert.Equal(PlaybackState.Playing, pm.CurrentState);
    }

    [Fact]
    public async Task NextAsync_MovesToNextSong()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        var playlist = new[] { MakeLocalSong(1), MakeLocalSong(2), MakeLocalSong(3) };
        await pm.PlayAsync(playlist, 0);
        Assert.Equal("Song1", pm.NowPlaying!.Name);

        await pm.NextAsync();
        Assert.Equal("Song2", pm.NowPlaying!.Name);

        await pm.NextAsync();
        Assert.Equal("Song3", pm.NowPlaying!.Name);
    }

    [Fact]
    public async Task PreviousAsync_WrapsAround()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        var playlist = new[] { MakeLocalSong(1), MakeLocalSong(2) };
        await pm.PlayAsync(playlist, 0);

        await pm.PreviousAsync();
        Assert.Equal("Song2", pm.NowPlaying!.Name); // index 0 → prev wraps to Count-1
    }

    [Fact]
    public async Task NextAsync_EndOfQueue_Stops()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        await pm.PlayAsync([MakeLocalSong(1)], 0);
        await pm.NextAsync();
        Assert.Equal(PlaybackState.Stopped, pm.CurrentState);
    }

    [Fact]
    public async Task RepeatModeAll_LoopsQueue()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        pm.SetRepeatMode(RepeatMode.All);
        await pm.PlayAsync([MakeLocalSong(1)], 0);

        await pm.NextAsync();
        Assert.Equal("Song1", pm.NowPlaying!.Name); // 回绕到第一首
        Assert.Equal(PlaybackState.Playing, pm.CurrentState);
    }

    // ── 引擎事件集成（Ended 自动切歌 / 去重 / Position 桥接） ─────────

    [Fact]
    public async Task Ended_Event_TriggersAutoNext()
    {
        var engine = new MemoryPlaybackEngine();
        using var pm = new PlayerManager(engine);
        var playlist = new[] { MakeLocalSong(1), MakeLocalSong(2) };
        await pm.PlayAsync(playlist, 0);
        Assert.Equal("Song1", pm.NowPlaying!.Name);

        engine.EmitEnded();
        await Task.Delay(50);

        Assert.Equal("Song2", pm.NowPlaying!.Name);
        Assert.Equal(PlaybackState.Playing, pm.CurrentState);
    }

    [Fact]
    public async Task Ended_Within500ms_IsDeduped()
    {
        var engine = new MemoryPlaybackEngine();
        using var pm = new PlayerManager(engine);
        var playlist = new[] { MakeLocalSong(1), MakeLocalSong(2), MakeLocalSong(3) };
        await pm.PlayAsync(playlist, 0);

        engine.EmitEnded(); // 第一次 → 切到 Song2
        engine.EmitEnded(); // 第二次（500ms 窗口内）→ 被 TrackEndDedupPolicy 过滤
        await Task.Delay(50);

        Assert.Equal("Song2", pm.NowPlaying!.Name); // 不会重复切到 Song3
    }

    [Fact]
    public async Task EngineError_IsForwardedToFailurePolicy()
    {
        var engine = new MemoryPlaybackEngine();
        using var pm = new PlayerManager(engine);
        await pm.PlayAsync([MakeLocalSong(1)], 0);
        Assert.Equal(PlaybackState.Playing, pm.CurrentState);

        engine.EmitError("boom");
        await Task.Delay(50);

        // 队列只有 1 首，自动 Next 后队列结束 → Stopped
        Assert.Equal(PlaybackState.Stopped, pm.CurrentState);
    }

    [Fact]
    public async Task Position_IsForwardedFromEngine()
    {
        var engine = new MemoryPlaybackEngine();
        using var pm = new PlayerManager(engine);
        var positions = new List<TimeSpan>();
        pm.Position.Subscribe(positions.Add);
        await pm.PlayAsync([MakeLocalSong(1)], 0);

        engine.EmitPosition(TimeSpan.FromSeconds(30));
        await Task.Delay(50);

        Assert.Contains(TimeSpan.FromSeconds(30), positions);
    }

    [Fact]
    public async Task ConsecutiveFailures_ReachesError()
    {
        var failingEngine = new FailingPlaybackEngine();
        using var pm = new PlayerManager(failingEngine);

        // 10 首歌，每首都失败，连续失败链会 fire-and-forget 执行直到 Error
        var playlist = Enumerable.Range(1, 10)
            .Select(i => MakeLocalSong(i, $"file:///fail{i}.mp3"))
            .ToArray();

        await pm.PlayAsync(playlist, 0);
        // fire-and-forget 链需要小延迟完成
        await Task.Delay(200);

        Assert.Equal(PlaybackState.Error, pm.CurrentState);
    }

    [Fact]
    public async Task EmptyPlaylist_DoesNothing()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        await pm.PlayAsync([], 0);
        Assert.Equal(PlaybackState.Idle, pm.CurrentState);
    }

    [Fact]
    public void ToggleShuffle_TogglesState()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        Assert.False(pm.IsShuffle);
        pm.ToggleShuffle();
        Assert.True(pm.IsShuffle);
        pm.ToggleShuffle();
        Assert.False(pm.IsShuffle);
    }

    [Fact]
    public void SetRepeatMode_UpdatesProperty()
    {
        using var pm = new PlayerManager(new MemoryPlaybackEngine());
        pm.SetRepeatMode(RepeatMode.One);
        Assert.Equal(RepeatMode.One, pm.RepeatMode);
    }

    // ── A1：在线歌曲 URL 保鲜 ─────────────────────────────────────────

    [Fact]
    public async Task UrlRefresh_WhenStale_ReplacesStreamUrlAndUpdatesQueue()
    {
        var engine = new MemoryPlaybackEngine();
        var calls = 0;
        var pm = new PlayerManager(engine,
            song => { calls++; return Task.FromResult<string?>($"https://new.example/{song.Id}.mp3"); });

        SetLastRefresh(pm, DateTimeOffset.UtcNow.AddMinutes(-11));   // 视为已过期

        var queue = new[] { MakeRemoteSong(1), MakeRemoteSong(2) };
        await pm.PlayAsync(queue, 0);
        await Task.Delay(30);

        Assert.Equal(1, calls);
        Assert.Equal("https://new.example/1.mp3", pm.NowPlaying!.StreamUrl);
    }

    [Fact]
    public async Task UrlRefresh_WithinFreshWindow_DoesNotRefresh()
    {
        var engine = new MemoryPlaybackEngine();
        var calls = 0;
        var pm = new PlayerManager(engine,
            song => { calls++; return Task.FromResult<string?>($"https://new.example/{song.Id}.mp3"); });

        SetLastRefresh(pm, DateTimeOffset.UtcNow);   // 刚刷新过 → 保鲜期内

        await pm.PlayAsync([MakeRemoteSong(1)], 0);
        await Task.Delay(30);

        Assert.Equal(0, calls);
        Assert.Equal("https://old.example/song.mp3", pm.NowPlaying!.StreamUrl);
    }

    [Fact]
    public async Task UrlRefresh_Fails_FallsBackToCachedUrl()
    {
        var engine = new MemoryPlaybackEngine();
        var pm = new PlayerManager(engine,
            song => Task.FromException<string?>(new InvalidOperationException("network down")));

        SetLastRefresh(pm, DateTimeOffset.UtcNow.AddMinutes(-11));

        await pm.PlayAsync([MakeRemoteSong(1)], 0);
        await Task.Delay(30);

        Assert.Equal("https://old.example/song.mp3", pm.NowPlaying!.StreamUrl);
    }
}
