using System.Reactive.Linq;
using System.Reactive.Subjects;
using LibVLCSharp.Shared;
using NeriPlayer.Core.Logging;

namespace NeriPlayer.Core.Player.Engine;

/// <summary>
/// LibVLCSharp 播放引擎实现（对标 start.md 5.2 / Analysis.md 第 15 章）
/// </summary>
public sealed class VlcPlaybackEngine : IPlaybackEngine
{
    private static readonly LibVLC? _libVlc;
    private readonly MediaPlayer _player;
    private readonly Subject<PlaybackEngineEvent> _events = new();
    private readonly Subject<TimeSpan> _position = new();
    private readonly Subject<float[]> _fft = new();

    static VlcPlaybackEngine()
    {
        // 优先环境变量，其次默认路径（与 1.6 appsettings.json 约定一致）
        var vlcDir = Environment.GetEnvironmentVariable("NERIPLAYER_VLC_DIR")
                     ?? @"D:\libs\vlc-3.0.20";
        try
        {
            // 用全限定名避免与 NeriPlayer.Core 命名空间冲突
            LibVLCSharp.Shared.Core.Initialize(vlcDir);
            _libVlc = new LibVLC("--no-video", "--no-video-title-show");
            AppLogger.Instance.Information("VLC initialized from {Dir}", vlcDir);
        }
        catch (Exception ex)
        {
            AppLogger.Instance.Warning(ex, "VLC initialization failed, playback will be unavailable");
        }
    }

    public VlcPlaybackEngine()
    {
        if (_libVlc is null)
            throw new EngineException("VLC not initialized — check libvlc.dll path");

        _player = new MediaPlayer(_libVlc);
        _player.TimeChanged += (_, e) => _position.OnNext(TimeSpan.FromMilliseconds(e.Time));
        _player.EndReached += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Ended));
        _player.Playing += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Playing));
        _player.Paused += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Paused));
        _player.Stopped += (_, _) => _events.OnNext(new(PlaybackEngineEventKind.Stopped));
        _player.EncounteredError += (_, _) =>
            _events.OnNext(new(PlaybackEngineEventKind.Error, "VLC EncounteredError"));
        _player.Buffering += (_, e) =>
            _events.OnNext(new(PlaybackEngineEventKind.Buffering, e.Cache.ToString()));
    }

    public IObservable<PlaybackEngineEvent> Events => _events;
    public IObservable<TimeSpan> Position => _position.AsObservable();
    public IObservable<float[]> FftData => _fft.AsObservable();
    public TimeSpan Duration =>
        _player.Length > 0 ? TimeSpan.FromMilliseconds(_player.Length) : TimeSpan.Zero;

    public Task LoadAsync(Uri mediaUri, PlaybackEngineOptions options)
    {
        using var media = new Media(_libVlc!, mediaUri);
        if (!_player.Play(media))
            return Task.FromException(new EngineException("VLC Play failed"));
        _player.Volume = (int)(options.Volume * 100);
        return Task.CompletedTask;
    }

    public Task PlayAsync() { _player.Play(); return Task.CompletedTask; }
    public Task PauseAsync() { _player.Pause(); return Task.CompletedTask; }
    public Task SeekAsync(TimeSpan position) { _player.Time = (long)position.TotalMilliseconds; return Task.CompletedTask; }
    public Task SetVolumeAsync(float volume) { _player.Volume = Math.Clamp((int)(volume * 100), 0, 200); return Task.CompletedTask; }
    public Task SetRateAsync(float speed) { _player.SetRate(speed); return Task.CompletedTask; }

    public void ApplyEqualizer(IReadOnlyList<float> gains)
    {
        var eq = new Equalizer();
        var bandFreqs = new uint[] { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9 }; // 10-band indices
        for (var i = 0; i < Math.Min(gains.Count, 10); i++)
            eq.SetAmp(Math.Clamp(gains[i], -20, 20) * 100f, bandFreqs[i]);
        _player.SetEqualizer(eq);
    }

    public void ApplyStereoBalance(float balance)
    {
        // StereoBalanceEffect 已实现于 Core/Player/Effects（第六章）；
        // 接入 WasapiOut 软件渲染路径待后续章节
    }

    public void ApplyVolumeNormalization(float gainDb)
    {
        // EBU R128 音量归一化见 Process.md 7.1，尚未接入渲染管线
    }

    public void ApplyPitch(float semitones)
    {
        // 变速变调（SpeedEffect/PitchEffect）见 Process.md 7.1，SoundTouch 实现待后续章节
    }

    public void Dispose()
    {
        _player.Stop();
        _player.Dispose();
        _events.Dispose();
        _position.Dispose();
        _fft.Dispose();
    }
}
