using System.Reactive.Linq;
using System.Reactive.Subjects;
using NeriPlayer.Core.Logging;
using NeriPlayer.Core.Player.Engine;
using NeriPlayer.Core.Player.Model;
using NeriPlayer.Core.Player.Policy;

namespace NeriPlayer.Core.Player;

public enum PlaybackState { Idle, Loading, Playing, Paused, Stopped, Error }
public enum RepeatMode { Off, All, One }
public enum PlaybackCommandSource { Local, Smtc, Shortcut, Auto }

/// <summary>
/// 播放器总控（对标 Analysis.md 第 3 章 + Process.md 4.1）
/// 常量全部对标 Analysis.md 24.1 节速查表。
/// </summary>
public sealed class PlayerManager : IDisposable
{
    // ── 常量（对标 Analysis.md 24.1） ───────────────────────────────────
    private static readonly TimeSpan MediaUrlStale = TimeSpan.FromMinutes(10);       // MEDIA_URL_STALE_MS
    private static readonly TimeSpan UrlRefreshCooldown = TimeSpan.FromSeconds(10);  // URL_REFRESH_COOLDOWN_MS（MediaUrlRefreshPolicy 用）
    private const int MaxConsecutiveFailures = 10;                                   // MAX_CONSECUTIVE_FAILURES
    private static readonly TimeSpan StatePersistInterval = TimeSpan.FromSeconds(15);     // 预留：状态持久化（后续章节接入）
    private static readonly TimeSpan DefaultFadeDuration = TimeSpan.FromMilliseconds(500); // 预留：淡入（后续章节接入）
    private static readonly TimeSpan ProgressThrottle = TimeSpan.FromMilliseconds(80);    // 预留：进度节流（后续章节接入）
    private const long MinListenMsForPlayCount = 30_000;                               // 预留：听满 30s 计次（后续章节接入）

    // ── 依赖 ──────────────────────────────────────────────────────────
    private readonly IPlaybackEngine _engine;
    private readonly Func<SongItem, Task<string?>>? _urlRefresher;
    private readonly PlaybackFailurePolicy _failurePolicy = new();
    private readonly TrackEndDedupPolicy _endDedupPolicy = new();
    private readonly IDisposable _eventSub;
    private readonly IDisposable _positionSub;

    // ── 响应式流 ─────────────────────────────────────────────────────
    private readonly Subject<PlaybackState> _state = new();
    private readonly Subject<SongItem?> _currentSong = new();
    private readonly Subject<TimeSpan> _position = new();

    // ── 内部状态 ─────────────────────────────────────────────────────
    private List<SongItem> _queue = [];
    private int _index;
    private int _consecutiveFailures;
    private RepeatMode _repeatMode = RepeatMode.Off;
    private bool _shuffle;
    private DateTimeOffset _lastUrlRefreshAt = DateTimeOffset.MinValue;

    // ── 公开 Observable ──────────────────────────────────────────────
    public IObservable<PlaybackState> State => _state.AsObservable();
    public IObservable<SongItem?> CurrentSong => _currentSong.AsObservable();
    public IObservable<TimeSpan> Position => _position.AsObservable();
    public PlaybackState CurrentState { get; private set; } = PlaybackState.Idle;
    public SongItem? NowPlaying { get; private set; }

    // ── 构造 ─────────────────────────────────────────────────────────
    public PlayerManager(IPlaybackEngine engine, Func<SongItem, Task<string?>>? urlRefresher = null)
    {
        _engine = engine;
        _urlRefresher = urlRefresher;
        // 订阅引擎事件：Ended → 去重后自动下一首；Error → 走失败策略
        _eventSub = _engine.Events.Subscribe(OnEngineEvent);
        // 桥接引擎进度 → 对外 Position 流
        _positionSub = _engine.Position.Subscribe(_position);
    }

    public PlayerManager() : this(new VlcPlaybackEngine()) { }

    // ── 播放歌单（对标 playPlaylistImpl） ─────────────────────────────
    public async Task PlayAsync(IReadOnlyList<SongItem> playlist, int startIndex,
        PlaybackCommandSource source = PlaybackCommandSource.Local)
    {
        if (playlist.Count == 0) return;
        _queue = playlist.ToList();
        _index = Math.Clamp(startIndex, 0, _queue.Count - 1);
        _consecutiveFailures = 0;
        _failurePolicy.RecordSuccess();
        SetState(PlaybackState.Loading);
        await PlayAtIndexAsync();
    }


    private async Task PlayAtIndexAsync()
    {
        var song = _queue[_index];

        // 在线歌曲 URL 保鲜（A1）：流 URL 过期时重新解析并回写队列
        var candidate = await MaybeRefreshStreamUrlAsync(song);
        if (!ReferenceEquals(candidate, song))
        {
            _queue[_index] = candidate;
            song = candidate;
        }

        var url = ResolveMediaUri(song);
        if (string.IsNullOrEmpty(url))
        {
            HandlePlayFailure(song, "No media URI available");
            return;
        }

        try
        {
            await _engine.LoadAsync(new Uri(url), new PlaybackEngineOptions { Volume = 1.0f });
            await _engine.PlayAsync();
            _failurePolicy.RecordSuccess();
            _consecutiveFailures = 0;
            NowPlaying = song;
            _currentSong.OnNext(song);
            SetState(PlaybackState.Playing);
            AppLogger.Instance.Information("Now playing: {Name} - {Artist}",
                song.DisplayName, song.DisplayArtist);
        }
        catch (Exception ex)
        {
            HandlePlayFailure(song, ex.Message);
        }
    }

    private void HandlePlayFailure(SongItem song, string reason)
    {
        AppLogger.Instance.Warning("Play failed for {Name}: {Reason}", song.DisplayName, reason);
        _failurePolicy.RecordFailure(out var shouldStop);
        _consecutiveFailures++;
        if (shouldStop)
        {
            SetState(PlaybackState.Error);
            AppLogger.Instance.Error("Consecutive failures reached {Count}, entering Error state",
                MaxConsecutiveFailures);
            return;
        }
        _ = NextAsync(auto: true);
    }

    /// <summary>
    /// 引擎事件处理：Ended 去重后自动下一首；Error 走失败策略。
    /// </summary>
    private void OnEngineEvent(PlaybackEngineEvent e)
    {
        switch (e.Kind)
        {
            case PlaybackEngineEventKind.Ended:
                if (_endDedupPolicy.TryConsume())
                {
                    AppLogger.Instance.Debug("Track ended, auto-advancing");
                    _ = NextAsync(auto: true);
                }
                else
                {
                    AppLogger.Instance.Debug("Ended event deduplicated (within 500ms)");
                }
                break;

            case PlaybackEngineEventKind.Error:
                if (NowPlaying is not null)
                    HandlePlayFailure(NowPlaying, e.Message ?? "Engine error");
                break;
        }
    }

    /// <summary>
    /// 在线歌曲 URL 保鲜（A1，对标 start.md 5.4 MediaUrlRefreshPolicy）：
    /// 流 URL 超过保鲜期时调用注入的 UrlRefresher 重新解析，成功则返回换新 URL 的副本；
    /// 未过期、无 refresher、本地歌曲或刷新失败则原样返回。冷却由 MediaUrlStale 覆盖，避免频繁重取。
    /// </summary>
    private async Task<SongItem> MaybeRefreshStreamUrlAsync(SongItem song)
    {
        if (song.IsLocalSong() || _urlRefresher is null || string.IsNullOrEmpty(song.StreamUrl))
            return song;

        var sinceLast = DateTimeOffset.UtcNow - _lastUrlRefreshAt;
        // 首次（MinValue）或超过保鲜期 → 需要刷新；否则命中保鲜期，保持现 URL（UrlRefreshCooldown 被 MediaUrlStale 覆盖）
        var shouldRefresh = _lastUrlRefreshAt == DateTimeOffset.MinValue || sinceLast >= MediaUrlStale;
        if (!shouldRefresh) return song;

        _lastUrlRefreshAt = DateTimeOffset.UtcNow;   // 先记录，避免并发/重复刷新
        AppLogger.Instance.Debug("Stream URL stale ({Age}ms), refreshing for {Name}",
            sinceLast.TotalMilliseconds, song.DisplayName);
        try
        {
            var newUrl = await _urlRefresher(song);
            if (!string.IsNullOrEmpty(newUrl) && newUrl != song.StreamUrl)
            {
                AppLogger.Instance.Information("Stream URL refreshed for {Name} - {Artist}",
                    song.DisplayName, song.DisplayArtist);
                return song with { StreamUrl = newUrl };
            }
        }
        catch (Exception ex)
        {
            AppLogger.Instance.Warning(ex, "Stream URL refresh failed for {Name}, using cached URL", song.DisplayName);
        }
        return song;
    }

    /// <summary>
    /// 解析用于播放的媒体 URI：本地歌曲优先 MediaUri，回退到 LocalFilePath；远程歌曲用 StreamUrl。
    /// </summary>
    private string? ResolveMediaUri(SongItem song)
    {
        if (song.IsLocalSong())
        {
            // 优先 MediaUri，回退到 LocalFilePath（转为 file:// URI）
            if (!string.IsNullOrEmpty(song.MediaUri)) return song.MediaUri;
            if (!string.IsNullOrEmpty(song.LocalFilePath))
                return $"file:///{song.LocalFilePath.Replace('\\', '/')}";
            return null;
        }
        if (!string.IsNullOrEmpty(song.StreamUrl)) return song.StreamUrl;
        return song.MediaUri;
    }

    // ── 播放控制 ─────────────────────────────────────────────────────
    public async Task PauseAsync()
    {
        await _engine.PauseAsync();
        SetState(PlaybackState.Paused);
    }

    public async Task ResumeAsync()
    {
        await _engine.PlayAsync();
        SetState(PlaybackState.Playing);
    }

    public async Task NextAsync(bool auto = false)
    {
        if (_shuffle && _queue.Count > 1)
        {
            var rng = new Random();
            int next;
            do { next = rng.Next(_queue.Count); } while (next == _index);
            _index = next;
        }
        else if (_index >= _queue.Count - 1)
        {
            if (_repeatMode == RepeatMode.All) _index = 0;
            else { SetState(PlaybackState.Stopped); return; }
        }
        else { _index++; }
        await PlayAtIndexAsync();
    }

    public async Task PreviousAsync()
    {
        _index = _index > 0 ? _index - 1 : _queue.Count - 1;
        await PlayAtIndexAsync();
    }

    public async Task SeekAsync(TimeSpan position) => await _engine.SeekAsync(position);
    public async Task SetVolumeAsync(float volume) => await _engine.SetVolumeAsync(volume);

    // ── 模式切换 ─────────────────────────────────────────────────────
    public void SetRepeatMode(RepeatMode mode) => _repeatMode = mode;
    public void ToggleShuffle() => _shuffle = !_shuffle;
    public RepeatMode RepeatMode => _repeatMode;
    public bool IsShuffle => _shuffle;

    // ── 辅助 ─────────────────────────────────────────────────────────
    private void SetState(PlaybackState s) { CurrentState = s; _state.OnNext(s); }

    public void Dispose()
    {
        _eventSub.Dispose();
        _positionSub.Dispose();
        _engine.Dispose();
        _state.Dispose();
        _currentSong.Dispose();
        _position.Dispose();
    }
}
