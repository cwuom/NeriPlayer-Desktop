namespace NeriPlayer.Core.Player.Engine;

public enum PlaybackEngineEventKind { Loaded, Playing, Paused, Stopped, Ended, Error, Buffering }

public sealed record PlaybackEngineEvent(PlaybackEngineEventKind Kind, string? Message = null);

public sealed record PlaybackEngineOptions
{
    public float Volume { get; init; } = 1.0f;
    public float Rate { get; init; } = 1.0f;
    public bool FadeOnPlay { get; init; } = true;
}

public interface IPlaybackEngine : IDisposable
{
    Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options);
    Task PlayAsync();
    Task PauseAsync();
    Task SeekAsync(TimeSpan position);
    Task SetVolumeAsync(float volume);
    Task SetRateAsync(float speed);
    IObservable<PlaybackEngineEvent> Events { get; }

    void ApplyEqualizer(IReadOnlyList<float> gains);   // 10-band
    void ApplyStereoBalance(float balance);            // -1.0 ~ 1.0
    void ApplyVolumeNormalization(float gainDb);
    void ApplyPitch(float semitones);

    TimeSpan Duration { get; }
    IObservable<TimeSpan> Position { get; }
    IObservable<float[]> FftData { get; }
}