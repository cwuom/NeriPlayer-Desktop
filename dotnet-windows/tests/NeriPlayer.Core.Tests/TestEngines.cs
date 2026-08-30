using System.Reactive.Linq;
using System.Reactive.Subjects;
using NeriPlayer.Core.Player.Engine;

// ── 测试辅助：内存引擎 ─────────────────────────────────────────────

internal sealed class MemoryPlaybackEngine : IPlaybackEngine
{
    private readonly Subject<PlaybackEngineEvent> _events = new();
    private readonly Subject<TimeSpan> _position = new();

    public IObservable<PlaybackEngineEvent> Events => _events;
    public TimeSpan Duration => TimeSpan.Zero;
    public IObservable<TimeSpan> Position => _position.AsObservable();
    public IObservable<float[]> FftData => Observable.Empty<float[]>();

    public Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options)
    {
        _events.OnNext(new(PlaybackEngineEventKind.Loaded));
        return Task.CompletedTask;
    }

    // ── 测试辅助：手动触发引擎事件 ─────────────────────────────
    public void EmitEnded() => _events.OnNext(new(PlaybackEngineEventKind.Ended));
    public void EmitError(string message = "Simulated engine error")
        => _events.OnNext(new(PlaybackEngineEventKind.Error, message));
    public void EmitPosition(TimeSpan position) => _position.OnNext(position);

    public Task PlayAsync() { _events.OnNext(new(PlaybackEngineEventKind.Playing)); return Task.CompletedTask; }
    public Task PauseAsync() { _events.OnNext(new(PlaybackEngineEventKind.Paused)); return Task.CompletedTask; }
    public Task SeekAsync(TimeSpan position) => Task.CompletedTask;
    public Task SetVolumeAsync(float volume) => Task.CompletedTask;
    public Task SetRateAsync(float speed) => Task.CompletedTask;
    public void ApplyEqualizer(IReadOnlyList<float> gains) { }
    public void ApplyStereoBalance(float balance) { }
    public void ApplyVolumeNormalization(float gainDb) { }
    public void ApplyPitch(float semitones) { }
    public void Dispose() { _events.Dispose(); _position.Dispose(); }
}

internal sealed class FailingPlaybackEngine : IPlaybackEngine
{
    public IObservable<PlaybackEngineEvent> Events => Observable.Empty<PlaybackEngineEvent>();
    public TimeSpan Duration => TimeSpan.Zero;
    public IObservable<TimeSpan> Position => Observable.Empty<TimeSpan>();
    public IObservable<float[]> FftData => Observable.Empty<float[]>();

    public Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options)
        => Task.FromException(new EngineException("Simulated failure"));

    public Task PlayAsync() => Task.CompletedTask;
    public Task PauseAsync() => Task.CompletedTask;
    public Task SeekAsync(TimeSpan position) => Task.CompletedTask;
    public Task SetVolumeAsync(float volume) => Task.CompletedTask;
    public Task SetRateAsync(float speed) => Task.CompletedTask;
    public void ApplyEqualizer(IReadOnlyList<float> gains) { }
    public void ApplyStereoBalance(float balance) { }
    public void ApplyVolumeNormalization(float gainDb) { }
    public void ApplyPitch(float semitones) { }
    public void Dispose() { }
}
