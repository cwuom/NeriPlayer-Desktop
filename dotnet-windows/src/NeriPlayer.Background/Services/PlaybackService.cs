using System.Reactive.Linq;
using Microsoft.Extensions.Hosting;
using NeriPlayer.Core.Logging;
using NeriPlayer.Core.Player;

namespace NeriPlayer.Background.Services;

/// <summary>
/// 后台播放服务（对标 AudioPlayerService / Analysis.md 第 20 章 / Process.md 12.4）。
/// 主窗口关闭后保持播放，通过 SMTC 按钮控制。
/// 包含空闲关闭策略（默认 10min 无播放自动停止服务）。
/// </summary>
public sealed class PlaybackService : BackgroundService, IDisposable
{
    private readonly SmtcIntegration _smtc;
    private readonly PlayerManager _player;
    private readonly IDisposable _stateSubscription;
    private readonly IDisposable _songSubscription;
    private readonly IDisposable _positionSubscription;
    private readonly TimeSpan _idleTimeout = TimeSpan.FromMinutes(10);
    private CancellationTokenSource? _idleCts;

    public PlaybackService(SmtcIntegration smtc, PlayerManager player)
    {
        _smtc = smtc;
        _player = player;

        // SMTC 按钮 → PlayerManager 命令
        _smtc.ButtonPressed += OnSmtcButtonPressed;

        // PlayerManager 状态 → SMTC 状态同步
        _stateSubscription = player.State.Subscribe(state =>
        {
            _smtc.UpdatePlaybackStatus(state);
            if (state == PlaybackState.Playing)
                ResetIdleTimer();
        });

        // 歌曲切换 → 更新 SMTC 元数据
        _songSubscription = player.CurrentSong.Subscribe(song =>
        {
            if (song is null)
            {
                _smtc.UpdatePlaybackStatus(PlaybackState.Idle);
                return;
            }
            _smtc.UpdateMetadata(
                song.DisplayName,
                song.DisplayArtist,
                song.Album ?? string.Empty);
        });

        // 进度更新 → SMTC 进度条（节流：每秒更新一次）
        _positionSubscription = player.Position
            .Throttle(TimeSpan.FromSeconds(1))
            .Subscribe(pos =>
            {
                var song = player.NowPlaying;
                var duration = song?.DurationMs > 0
                    ? TimeSpan.FromMilliseconds(song.DurationMs)
                    : TimeSpan.Zero;
                _smtc.UpdateTimeline(pos, duration);
            });
    }

    /// <summary>SMTC 按钮事件 → PlayerManager 命令分发。</summary>
    private void OnSmtcButtonPressed(SmtcButton button)
    {
        switch (button)
        {
            case SmtcButton.Play:
                _ = _player.ResumeAsync();
                break;
            case SmtcButton.Pause:
                _ = _player.PauseAsync();
                break;
            case SmtcButton.Next:
                _ = _player.NextAsync();
                break;
            case SmtcButton.Previous:
                _ = _player.PreviousAsync();
                break;
            case SmtcButton.Stop:
                _ = _player.PauseAsync();
                _smtc.UpdatePlaybackStatus(PlaybackState.Idle);
                break;
        }
    }

    private void ResetIdleTimer()
    {
        _idleCts?.Cancel();
        _idleCts?.Dispose();
        _idleCts = null;
    }

    protected override Task ExecuteAsync(CancellationToken stoppingToken)
        => Task.Delay(Timeout.Infinite, stoppingToken);

    public override void Dispose()
    {
        _smtc.ButtonPressed -= OnSmtcButtonPressed;
        _stateSubscription.Dispose();
        _songSubscription.Dispose();
        _positionSubscription.Dispose();
        _idleCts?.Cancel();
        _idleCts?.Dispose();
        base.Dispose();
    }
}
