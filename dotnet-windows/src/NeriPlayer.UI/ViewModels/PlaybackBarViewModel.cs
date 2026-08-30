using System.Reactive.Linq;
using NeriPlayer.Core.Player;
using ReactiveUI;
using System.Reactive;

namespace NeriPlayer.UI.ViewModels;

public sealed class PlaybackBarViewModel : ReactiveObject
{
    private string _nowPlayingName  = string.Empty;
    private string _nowPlayingArtist = string.Empty;
    private bool   _isPlaying;
    private double _positionMs;
    private double _durationMs;

    public string NowPlayingName   { get => _nowPlayingName;   set => this.RaiseAndSetIfChanged(ref _nowPlayingName, value); }
    public string NowPlayingArtist { get => _nowPlayingArtist;  set => this.RaiseAndSetIfChanged(ref _nowPlayingArtist, value); }
    public bool   IsPlaying        { get => _isPlaying;        set => this.RaiseAndSetIfChanged(ref _isPlaying, value); }
    public double PositionMs       { get => _positionMs;       set => this.RaiseAndSetIfChanged(ref _positionMs, value); }
    public double DurationMs       { get => _durationMs;       set => this.RaiseAndSetIfChanged(ref _durationMs, value); }

    public ReactiveCommand<Unit, Unit> TogglePlayCommand { get; }
    public ReactiveCommand<Unit, Unit> NextCommand      { get; }
    public ReactiveCommand<Unit, Unit> PreviousCommand  { get; }

    private readonly PlayerManager _player;

    public PlaybackBarViewModel(PlayerManager player)
    {
        _player = player;

        TogglePlayCommand = ReactiveCommand.CreateFromTask(async () =>
        {
            if (_player.CurrentState == PlaybackState.Playing)
                await _player.PauseAsync();
            else
                await _player.ResumeAsync();
        });

        NextCommand     = ReactiveCommand.CreateFromTask(() => _player.NextAsync());
        PreviousCommand = ReactiveCommand.CreateFromTask(() => _player.PreviousAsync());

        player.State
            .ObserveOn(RxApp.MainThreadScheduler)
            .Subscribe(s => IsPlaying = s == PlaybackState.Playing);

        player.CurrentSong
            .ObserveOn(RxApp.MainThreadScheduler)
            .Subscribe(song =>
            {
                NowPlayingName   = song?.DisplayName   ?? string.Empty;
                NowPlayingArtist = song?.DisplayArtist  ?? string.Empty;
                DurationMs       = song?.DurationMs    ?? 0;
            });

        player.Position
            .ObserveOn(RxApp.MainThreadScheduler)
            .Subscribe(p => PositionMs = p.TotalMilliseconds);
    }
}
