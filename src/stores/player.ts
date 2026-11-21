import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useHistoryStore } from './history'
import { useToastStore } from './toast'
import { normalizePlaybackSpeed, normalizePlaybackPitch, useSettingsStore } from './settings'
import { useListenTogetherStore } from './listenTogether'
import { useBiliVideoSkipStore } from './biliVideoSkip'
import i18n from '@/i18n'
import {
  getPlaybackSourceKind, isDirectStreamUrl, isRemotePlaybackTrack,
} from '@/modules/playback/playbackSource'
import { PlaybackFailure } from '@/modules/playback/playbackFailure'
import { shouldRefreshUrlBeforeResume, shouldRefreshUrlBeforeSeek } from '@/modules/playback/youtubeSeekRefreshPolicy'
import { PlaybackStartupWatchdog } from '@/modules/playback/playbackPolicy'
import {
  isPlaybackSeekCompletionCurrent, resolvePlaybackLoadStart, shouldDeferPlaybackSeek, type DeferredPlaybackSeek,
} from '@/modules/playback/playbackRequest'
import { resolvePlaybackTransition } from '@/modules/playback/playbackTransition'
import { createLogger } from '@/utils/logger'
import { DEFAULT_AUDIO_EFFECTS_SETTINGS, normalizeAudioEffectsSettings } from '@/modules/audioEffects/model'
import { summarizeLogError } from '@/utils/logSanitizer'
import type { AudioInfo } from '@/modules/playback/playbackAudioInfo'
import { normalizeTrack } from '@/modules/playback/trackModel'
import type { LyricLine, LyricWord, RepeatMode, TrackInfo } from '@/types/track'
import {
  BiliVideoSkipTracker, intervalsForBiliVideoSkipPlayback, isBiliVideoSkipTrack, normalizeBiliVideoSkipTarget,
  resolveBiliVideoSkipBvid, resolveBiliVideoSkipCid, resolveBiliVideoSkipTarget, type BiliVideoSkipTarget,
} from '@/modules/playback/biliVideoSkip'
import { createAudioEffects } from './player/audioEffects'
import { createAudioInfoReaders } from './player/audioInfo'
import { createCommandSource } from './player/commandSource'
import { createPlayerEvents } from './player/events'
import { createAudioLevel, createProgressDisplay, createSlowLoadingFlag } from './player/playbackIndicators'
import { createPauseGuard } from './player/pauseGuard'
import { createPlayerPersistence } from './player/persistence'
import { createPlaybackPrefetch } from './player/prefetch'
import { createQueueEditing, createShuffleOrder } from './player/queueEditing'
import { createQueueNavigation } from './player/queueNavigation'
import { createSleepTimer } from './player/sleepTimer'
import {
  audioFilePathKey, claimPlaybackRequest, createPlaybackSourceLoader, downloadedTrackForPlayback, type AudioFileMutation,
} from './player/sourceLoader'
import { createStreamShare } from './player/streamShare'
import { tracePlaybackUi } from './player/trace'
import { createTrackEditing } from './player/trackEditing'
import type { PendingSeekState, PlaybackCommandSource, PlayerCoreState, SeekCommandSnapshot } from './player/types'

const log = createLogger('player')

// 类型与 normalizeTrack 已下沉到 types / modules，这里继续导出以兼容现有调用方
export type { AudioInfo, LyricLine, LyricWord, RepeatMode, TrackInfo }
export { normalizeTrack }
export type { AudioEffectsRuntimeStats, DecoderCapabilities, PlaybackCommandSource, SeekCommandSnapshot } from './player/types'
export { EQ_PRESETS } from '@/modules/audioEffects/eqPresets'
export { tracePlaybackUi }

export { displayAlbum } from '@/modules/library/albumDisplay'

// 播放位置插值状态（模块级，rAF 驱动）
let _interpAnchorMs = 0         // 上次后端报告的位置
let _interpAnchorTime = 0       // 对应的 performance.now() 锚点
let _interpRenderedMs = 0       // 上次渲染的插值位置
let _interpSpeed = 1.0          // 当前播放速度快照
let _interpIsPlaying = false    // 当前播放状态快照
let _interpDurationMs = 0       // 当前时长快照
let _interpLoopStarted = false  // rAF 循环是否已启动
let _interpLastTickTime = 0     // 上一帧 tick 的 performance.now()
// 播放中后端位置与渲染值的偏差在此范围内只做平滑追赶，不硬跳；
// 硬跳的毫秒数在短歌上对应的像素位移更大，会让进度条明显一顿一顿
const POSITION_HARD_RESYNC_MS = 1500
// 平滑追赶的时间常数：偏差约按 e^(-t/τ) 收敛
const POSITION_SLEW_TAU_MS = 300
const SEEK_EVENT_GUARD_MS = 900
const SEEK_SETTLE_TIMEOUT_MS = 4500
const SEEK_BACKWARD_TOLERANCE_MS = 600
const SEEK_FORWARD_TOLERANCE_MS = 1200
const POSITION_BACKWARD_TOLERANCE_MS = 250
// 回退超过该值视为时钟跳变（睡眠唤醒），强制重锚
const CLOCK_JUMP_BACKWARD_MS = 30_000

// playbackRequestToken 防竞态
let playbackRequestToken = Date.now() * 1000

// URL 过期检测 (10min)
let lastUrlResolveTime = 0
const URL_EXPIRY_MS = 10 * 60 * 1000

// Track End 去重
let lastTrackEndedId: string | null = null
let lastTrackEndedTime = 0
let lastTrackEndedRequestToken = -1

// 播放抽象层：解析缓存、预热仲裁、启动看门狗
const playbackStartupWatchdog = new PlaybackStartupWatchdog()

// 恢复后需重新加载标记
let _needsReload = false
let _currentLoadedFromDownloadPath: string | null = null

export const usePlayerStore = defineStore('player', () => {
  const audioFileMutations = new Map<string, AudioFileMutation>()

  function currentAudioFileMutation() {
    return [...audioFileMutations.values()].find(mutation =>
      mutation.trackId === currentTrack.value?.id && mutation.requestToken === playbackRequestToken)
  }
  const settings = useSettingsStore()
  const isPlaying = ref(false)
  const currentTrack = ref<TrackInfo | null>(null)
  const currentBiliVideoSkipTarget = ref<BiliVideoSkipTarget | null>(null)
  const biliVideoSkipTracker = new BiliVideoSkipTracker()
  let biliVideoSkipRequestToken = 0
  const positionMs = ref(0)
  const durationMs = ref(0)
  const queue = ref<TrackInfo[]>([])
  const queueIndex = ref(-1)
  const queueSelectionNeedsPlayback = ref(false)
  const repeatMode = ref<RepeatMode>('off')
  const shuffleEnabled = ref(false)
  const shuffleBag = ref<number[]>([])
  const shuffleHistory = ref<number[]>([])
  const shuffleFuture = ref<number[]>([])
  const volume = ref(settings.volume)
  const lyrics = ref<LyricLine[]>([])

  // 播放错误信息（供 UI 展示）
  const playError = ref<string | null>(null)
  // 是否正在加载音频（下载/解码中）
  const isLoadingAudio = ref(false)
  // 加载持续超过 1s 才置真, 供播放按钮转圈; 短加载不闪 spinner
  const isLoadingAudioSlow = createSlowLoadingFlag(isLoadingAudio)
  // 是否存在可见播放上下文；恢复态由 _needsReload 标记后端尚未装载
  const hasPlaybackSession = ref(false)

  // 当前音频质量信息
  const audioInfo = ref<AudioInfo | null>(null)
  const isPlayingFromDownload = ref(false)
  // 当前会话是否命中播放缓存 (非下载文件)
  const isPlayingFromCache = ref(false)

  const core: PlayerCoreState = {
    settings, isPlaying, currentTrack, positionMs, durationMs, queue, queueIndex, queueSelectionNeedsPlayback,
    repeatMode, shuffleEnabled, shuffleBag, shuffleHistory, shuffleFuture, volume, isLoadingAudio, hasPlaybackSession,
  }

  // 睡眠定时器
  const {
    sleepTimerMode, sleepRemainingSeconds,
    startSleepTimer, startSleepTimerEndOfTrack, startSleepTimerEndOfQueue, cancelSleepTimer,
  } = createSleepTimer(pause)

  const { audioLevel, beatImpulse } = createAudioLevel(isPlaying)
  const {
    interpolatedPositionMs, interpolatedProgress, progress, currentTimeFormatted, durationFormatted,
  } = createProgressDisplay(positionMs, durationMs)

  // seek 后忽略 position 事件的时间窗口
  let seekGuardUntil = 0
  // seek 后等待后端位置收敛，防止刚开播的旧 position 把进度条拉回去
  let pendingSeek: PendingSeekState | null = null
  const lastSeekCommand = ref<SeekCommandSnapshot>({
    seq: 0,
    positionMs: 0,
    source: 'local',
    requestGeneration: 0,
  })
  let loadedPlaybackRequestToken = 0
  // 队列选中项可能先于音频变更，记录后端时长实际属于哪首歌
  const loadedPlaybackTrackKey = ref<string | null>(null)
  let deferredPlaybackSeek: DeferredPlaybackSeek | null = null
  let _speedInvokeTimer: ReturnType<typeof setTimeout> | null = null
  let _pitchInvokeTimer: ReturnType<typeof setTimeout> | null = null

  /** 供拆出的切片判断异步结果是否过期；核心流程直接比较 playbackRequestToken */
  function isCurrentRequest(token: number) {
    return token === playbackRequestToken
  }

  const { clearPauseGuard, freezeRenderedPosition, normalizePositionAfterPause } =
    createPauseGuard(currentRenderedPosition, setRenderedPosition)
  const {
    lastCommandSource, lastCommandSequence, markCommandSource, blockedByListenTogether,
    internalFollowUpSource, blocksLocalSongInRoom, isRemoteSyncGuardActive,
  } = createCommandSource(currentTrack)
  const { rebuildShuffleBag, shiftShuffleIndicesForPlayNext, shiftShuffleIndicesForRemove } = createShuffleOrder(core)
  const {
    flushPlayerState, savePlayerState, loadPlayerState, maybePersistProgress,
    persistLongFormProgress, persistCurrentLongFormProgress, rememberedLongFormStart,
  } = createPlayerPersistence({
    ...core, currentRenderedPosition, setRenderedPosition, rebuildShuffleBag,
    markNeedsReload: needsReload => { _needsReload = needsReload },
  })
  const {
    playbackSourceSettings, replacePlaybackDemand, nextPrefetchTracks, maybePrefetchNext,
    prefetchIntent, prefetchPlaybackTracks, takePrefetchedPlaybackUrl, hasCompleteCachedAudio,
  } = createPlaybackPrefetch(core)
  const {
    currentStreamUrl, setResolvedStreamUrls, rememberStreamQualities,
    getCurrentStreamUrl, getCurrentStreamUrls, resolveShareableStreamUrls,
  } = createStreamShare(currentTrack, playbackSourceSettings)
  const { resetDecodedAudioInfo, readLocalAudioInfo, readPlaybackAudioInfo } = createAudioInfoReaders({
    audioInfo, hasPlaybackSession, isLoadingAudio, isCurrentRequest,
    isLoadedRequest: token => token === loadedPlaybackRequestToken,
    needsReload: () => _needsReload,
  })
  const {
    next, previous, toggleShuffle, pendingReplayIndex, addToQueueNext, discardSupersededReplay,
    schedulePlaybackStartupWatchdog, resetPlaybackFailures, handlePlaybackFailure, recoverStalledPlayback,
  } = createQueueNavigation({
    ...core, play, pause, seekTo, markCommandSource, blockedByListenTogether, internalFollowUpSource,
    rebuildShuffleBag, shiftShuffleIndicesForPlayNext, shiftShuffleIndicesForRemove, savePlayerState,
    currentRenderedPosition, isCurrentRequest, playbackStartupWatchdog,
  })
  const {
    queueDisplayIndices, queueDisplayCurrentIndex, createTrackCommit,
    toggleRepeatMode, playMode, cyclePlayMode, applyListenTogetherPlaybackMode, localPlaylistIdFor,
    playAll, shufflePlay, playQueueItem, addToQueueEnd, moveQueueItem, removeFromQueue, clearQueue,
  } = createQueueEditing({
    ...core, play, pause, savePlayerState, maybePrefetchNext, blockedByListenTogether, blocksLocalSongInRoom,
    rebuildShuffleBag, shiftShuffleIndicesForRemove, toggleShuffle,
  })
  const {
    updateCurrentTrackInfo, patchCurrentTrackSyncPayload, restoreOriginalTrackInfo, hasOriginalTrackInfo,
  } = createTrackEditing(currentTrack, queue)
  const { loadPlaybackSource, replayWithQuality } = createPlaybackSourceLoader({
    currentTrack, positionMs, isPlaying, play, pause,
    playError, audioInfo, isPlayingFromDownload, isPlayingFromCache, currentStreamUrl, audioFileMutations,
    setLoadedFromDownloadPath: path => { _currentLoadedFromDownloadPath = path },
    resetUrlResolveTime: () => { lastUrlResolveTime = 0 },
    setResolvedStreamUrls, rememberStreamQualities, resetDecodedAudioInfo, readLocalAudioInfo,
    applyResolvedBiliVideoSkipTarget, takePrefetchedPlaybackUrl, playbackSourceSettings, hasCompleteCachedAudio,
    currentRequestGeneration: () => playbackRequestToken,
  })
  const { initEvents, nativeTrackEndCount } = createPlayerEvents({
    currentTrack, isPlaying, audioLevel, beatImpulse, isCurrentRequest,
    isLoadedRequest: token => token === loadedPlaybackRequestToken,
    hasPendingSeek: () => pendingSeek !== null,
    isWithinSeekGuard: () => Date.now() < seekGuardUntil,
    isInterpolating: () => _interpIsPlaying,
    _startInterpolationLoop, shouldAcceptPositionAfterSeek, normalizePositionAfterPause, commitBackendPosition,
    maybeAutoSkipBiliVideoInterval, maybePersistProgress, recoverStalledPlayback, handleTrackEnded,
    resume, pause, togglePlayPause, next, previous, seekTo,
  })

  function commitBackendPosition(nextPositionMs: number, nextDurationMs?: number, forceRendered = false) {
    const safeDurationMs = nextDurationMs || durationMs.value || currentTrack.value?.durationMs || 0
    const safePositionMs = safeDurationMs > 0
      ? Math.min(Math.max(0, nextPositionMs), safeDurationMs)
      : Math.max(0, nextPositionMs)

    if (typeof nextDurationMs === 'number' && nextDurationMs > 0) {
      durationMs.value = nextDurationMs
    }

    const renderedMs = clampPlaybackPosition(_interpRenderedMs)
    // 后端位置比渲染值回退超过 30s 视为时钟跳变（睡眠唤醒后 performance.now 大跳
    // 把渲染值冲到曲末），此时必须强制重锚而非忽略，否则进度永久卡死
    const isClockJump = _interpIsPlaying && !forceRendered
      && renderedMs - safePositionMs > CLOCK_JUMP_BACKWARD_MS
    // 倍速播放时后端时钟与插值都应按速度前进；仍拒绝明显回跳
    if (_interpIsPlaying && !forceRendered && !isClockJump
      && safePositionMs < renderedMs - POSITION_BACKWARD_TOLERANCE_MS) {
      // 后端在缓冲（时钟停了）而插值还在走：渲染值不往回拉，但锚点要跟上后端，
      // 插值随之停住、等声音追上再走；只丢弃事件的话锚点不动，歌词会按累计卡顿时长永久领先
      positionMs.value = safePositionMs
      _interpAnchorMs = safePositionMs
      _interpAnchorTime = performance.now()
      return
    }

    positionMs.value = safePositionMs
    _interpAnchorMs = safePositionMs
    _interpAnchorTime = performance.now()
    _interpDurationMs = Math.max(durationMs.value || _interpDurationMs, safePositionMs)
    // 播放中也允许后端时钟校准插值，保证歌词与倍速同步
    if (!_interpIsPlaying || forceRendered || isClockJump
      || Math.abs(safePositionMs - renderedMs) > POSITION_HARD_RESYNC_MS) {
      _interpRenderedMs = safePositionMs
      interpolatedPositionMs.value = safePositionMs
    }
    // 播放中位置事件到达即确保插值循环在跑（停表后的兜底重启点）
    if (_interpIsPlaying) _startInterpolationLoop()
  }

  function clampPlaybackPosition(position: number, durationOverride?: number): number {
    const safeDurationMs = durationOverride && durationOverride > 0
      ? durationOverride
      : durationMs.value || currentTrack.value?.durationMs || _interpDurationMs || 0
    const safePositionMs = Number.isFinite(position) ? Math.max(0, Math.round(position)) : 0
    return safeDurationMs > 0 ? Math.min(safePositionMs, safeDurationMs) : safePositionMs
  }

  function armPendingSeek(targetMs: number): void {
    const now = Date.now()
    pendingSeek = {
      targetMs,
      issuedAt: now,
      expiresAt: now + SEEK_SETTLE_TIMEOUT_MS,
    }
    seekGuardUntil = now + SEEK_EVENT_GUARD_MS
    // 停表状态下发起 seek 也要恢复插值循环，等待后端确认
    _startInterpolationLoop()
  }

  function currentRenderedPosition(): number {
    return clampPlaybackPosition(_interpRenderedMs || interpolatedPositionMs.value || positionMs.value)
  }

  function setRenderedPosition(position: number, durationOverride?: number): number {
    const safePositionMs = clampPlaybackPosition(position, durationOverride)
    positionMs.value = safePositionMs
    _interpAnchorMs = safePositionMs
    _interpAnchorTime = performance.now()
    _interpRenderedMs = safePositionMs
    _interpDurationMs = Math.max(
      durationOverride || durationMs.value || currentTrack.value?.durationMs || _interpDurationMs,
      safePositionMs,
    )
    interpolatedPositionMs.value = safePositionMs
    return safePositionMs
  }

  function markOptimisticSeek(
    targetMs: number,
    commandSource: PlaybackCommandSource,
    options: { bumpSeq?: boolean; durationMs?: number; fromPlaybackStart?: boolean } = {},
  ): number {
    const safeTargetMs = setRenderedPosition(targetMs, options.durationMs)
    lastSeekCommand.value = {
      seq: options.bumpSeq === false ? lastSeekCommand.value.seq : lastSeekCommand.value.seq + 1,
      positionMs: safeTargetMs,
      source: commandSource,
      requestGeneration: playbackRequestToken,
      fromPlaybackStart: options.fromPlaybackStart === true,
    }
    clearPauseGuard()
    armPendingSeek(safeTargetMs)
    return safeTargetMs
  }

  function startInterpolationFromRenderedPosition(): void {
    const startMs = setRenderedPosition(currentRenderedPosition())
    _interpAnchorMs = startMs
    _interpAnchorTime = performance.now()
    _interpRenderedMs = startMs
    _interpSpeed = effectivePlaybackSpeed()
    _interpIsPlaying = true
    clearPauseGuard()
    // 从暂停停表状态恢复播放时重启插值循环
    _startInterpolationLoop()
  }

  function shouldAcceptPositionAfterSeek(nextPositionMs: number, nextDurationMs?: number): boolean {
    if (!pendingSeek) return true

    const now = Date.now()
    if (now >= pendingSeek.expiresAt) {
      pendingSeek.expiresAt = now + SEEK_SETTLE_TIMEOUT_MS
      seekGuardUntil = 0
    }

    const elapsedMs = Math.max(0, now - pendingSeek.issuedAt)
    const speed = _interpIsPlaying ? Math.max(0.1, effectivePlaybackSpeed() || _interpSpeed || 1) : 0
    const durationLimit = nextDurationMs || durationMs.value || currentTrack.value?.durationMs || 0
    const lowerBound = Math.max(0, pendingSeek.targetMs - SEEK_BACKWARD_TOLERANCE_MS)
    const upperCandidate = pendingSeek.targetMs + elapsedMs * speed + SEEK_FORWARD_TOLERANCE_MS
    const upperBound = durationLimit > 0 ? Math.min(durationLimit, upperCandidate) : upperCandidate
    const accepted = nextPositionMs >= lowerBound && nextPositionMs <= upperBound
    if (accepted) {
      pendingSeek = null
      seekGuardUntil = 0
    }
    return accepted
  }

  /// 此刻的播放位置，按插值锚点现算
  ///
  /// interpolatedPositionMs 由 rAF 推进，窗口最小化后页面不可见、rAF 暂停，它会停在最后一帧；
  /// 桌面歌词这类窗口不可见时仍要跟着走的地方用这个。
  function livePositionMs(): number {
    if (pendingSeek) return Math.round(pendingSeek.targetMs)
    if (!_interpIsPlaying) return Math.round(_interpRenderedMs)
    const predicted = _interpAnchorMs + (performance.now() - _interpAnchorTime) * _interpSpeed
    const clamped = Math.max(0, Math.min(predicted, _interpDurationMs))
    return Math.round(Math.max(_interpRenderedMs, clamped))
  }

  // seek 待确认时播放状态不变，但进度时钟必须和主窗口一起停在目标位置
  function isPlaybackClockAdvancing(): boolean {
    return _interpIsPlaying && !pendingSeek
  }

  /** 按需启动 rAF 插值循环：暂停且无待确认 seek 时自动停表，状态恢复时重启 */
  function _startInterpolationLoop() {
    if (_interpLoopStarted) return
    _interpLoopStarted = true
    _interpLastTickTime = 0

    function tick() {
      // 空闲（未播放且无 pendingSeek）时渲染一次最终位置后停表，避免常驻逐帧唤醒
      if (!_interpIsPlaying && !pendingSeek) {
        interpolatedPositionMs.value = Math.round(_interpRenderedMs)
        _interpLoopStarted = false
        return
      }
      requestAnimationFrame(tick)

      const now = performance.now()
      const dt = _interpLastTickTime ? now - _interpLastTickTime : 0
      _interpLastTickTime = now

      // seek 等待后端确认期间冻结进度条，避免「seek 完成仍空转」
      if (pendingSeek) {
        interpolatedPositionMs.value = Math.round(pendingSeek.targetMs)
        _interpRenderedMs = pendingSeek.targetMs
        _interpAnchorMs = pendingSeek.targetMs
        _interpAnchorTime = now
        return
      }

      const predicted = _interpAnchorMs + (now - _interpAnchorTime) * _interpSpeed
      const clamped = Math.max(0, Math.min(predicted, _interpDurationMs))

      // 按自然速度推进，同时把与后端预测值的偏差按指数平滑收敛：
      // 落后时略快、超前时略慢（超前太多则停住等后端追上），从不往回拉
      // 先扣除本帧自然推进量，避免后台恢复时把同一段经过时间重复算进校正
      const naturalStep = dt * _interpSpeed
      const error = clamped - (_interpRenderedMs + naturalStep)
      const correction = error * Math.min(1, dt / POSITION_SLEW_TAU_MS)
      const step = Math.max(0, naturalStep + correction)
      _interpRenderedMs = Math.min(_interpRenderedMs + step, Math.max(_interpRenderedMs, _interpDurationMs))

      interpolatedPositionMs.value = Math.round(_interpRenderedMs)
    }

    requestAnimationFrame(tick)
  }

  function prepareBiliVideoSkipTrack(track: TrackInfo, requestGeneration: number) {
    biliVideoSkipRequestToken = requestGeneration
    currentBiliVideoSkipTarget.value = resolveBiliVideoSkipTarget(track)
    biliVideoSkipTracker.reset()
    if (currentBiliVideoSkipTarget.value) return
    const bvid = resolveBiliVideoSkipBvid(track)
    if (!bvid) return
    const cid = resolveBiliVideoSkipCid(track)
    // 缓存和下载会绕过取流解析，也要补全实际分 P 身份
    void invoke<BiliVideoSkipTarget[]>('get_bili_video_skip_targets', { bvid }).then(options => {
      if (requestGeneration !== playbackRequestToken || requestGeneration !== biliVideoSkipRequestToken
        || currentTrack.value?.id !== track.id || currentBiliVideoSkipTarget.value) return
      const option = cid ? options.find(option => option.cid === cid) : options[0]
      const target = normalizeBiliVideoSkipTarget(option)
      if (!target || target.bvid !== bvid) return
      applyResolvedBiliVideoSkipTarget(track, target, requestGeneration)
      maybeAutoSkipBiliVideoInterval(positionMs.value, durationMs.value)
    }).catch(error => {
      if (requestGeneration === playbackRequestToken) log.warn('Bilibili skip target resolution failed:', error)
    })
  }

  function applyResolvedBiliVideoSkipTarget(
    track: TrackInfo,
    target: BiliVideoSkipTarget,
    requestGeneration: number,
  ) {
    if (requestGeneration !== playbackRequestToken || requestGeneration !== biliVideoSkipRequestToken || track.id !== currentTrack.value?.id) return
    const normalized = normalizeBiliVideoSkipTarget(target)
    if (!normalized) return
    const previous = currentBiliVideoSkipTarget.value
    if (previous?.bvid !== normalized.bvid || previous.cid !== normalized.cid) biliVideoSkipTracker.reset()
    currentBiliVideoSkipTarget.value = normalized
    if (!isBiliVideoSkipTrack(track)) return
    // 房间曲目身份来自权威状态，本机解析的 BV/CID 只用于当前播放
    if (useListenTogetherStore().roomId) return
    // 保存真正播放的分 P，缓存或下载播放时仍能恢复同一条区间规则
    track.album = `Bilibili|${normalized.cid}|${normalized.bvid}`
    if (track.syncPayload) {
      const sourceAudioId = track.syncPayload.audioId ?? track.syncPayload.audio_id
      const audioId = typeof sourceAudioId === 'string' && sourceAudioId.trim()
        ? sourceAudioId.trim()
        : typeof sourceAudioId === 'number' && Number.isSafeInteger(sourceAudioId) && sourceAudioId > 0
          ? String(sourceAudioId) : normalized.bvid
      track.syncPayload = {
        ...track.syncPayload,
        channelId: 'bilibili',
        audioId,
        subAudioId: String(normalized.cid),
        bvid: normalized.bvid,
        album: track.album,
      }
    }
    savePlayerState()
  }

  function maybeAutoSkipBiliVideoInterval(currentPositionMs: number, currentDurationMs: number): boolean {
    const track = currentTrack.value
    if (!track || !isBiliVideoSkipTrack(track) || !isPlaying.value || isLoadingAudio.value || _needsReload || pendingSeek
      || biliVideoSkipRequestToken !== playbackRequestToken || loadedPlaybackRequestToken !== playbackRequestToken
      || useListenTogetherStore().roomId) return false
    const intervals = intervalsForBiliVideoSkipPlayback(
      useBiliVideoSkipStore().rules,
      currentBiliVideoSkipTarget.value,
      resolveBiliVideoSkipCid(track),
      resolveBiliVideoSkipBvid(track),
    )
    const skipEndMs = biliVideoSkipTracker.nextSkipPosition(intervals, currentPositionMs, currentDurationMs)
    if (skipEndMs === null) return false
    if (currentDurationMs > 0 && skipEndMs >= currentDurationMs) {
      // 解码器不一定接受精确 EOF 跳转，曲尾区间复用正常结束时的循环和睡眠逻辑
      void handleTrackEnded().catch(error => log.warn('Bilibili interval advancement failed:', error))
    } else {
      void seekTo(skipEndMs, 'local_safety').catch(error => log.warn('Bilibili interval skip failed:', error))
    }
    return true
  }

  /**
   * @param allowRememberedPosition 用户点播时长音频从记住的位置继续；上一首/下一首、
   *   恢复会话等调用方传 false（对齐 Android allowRememberedLongFormPosition）
   */
  async function play(
    track: TrackInfo,
    commandSource: PlaybackCommandSource = 'local',
    startPositionMs = 0,
    forceResolve = false,
    allowRememberedPosition = true,
    requestedQueueIndex?: number,
    shuffleNavigation = false,
  ) {
    if (blockedByListenTogether(commandSource) || blocksLocalSongInRoom(track, commandSource)) return
    const fileMutation = !isRemotePlaybackTrack(track) && audioFileMutations.get(audioFilePathKey(track.audioUrl))
    if (fileMutation) {
      const waitingToken = ++playbackRequestToken
      fileMutation.requestToken = waitingToken
      fileMutation.trackId = currentTrack.value?.id || track.id
      fileMutation.shouldResume = true
      fileMutation.positionMs = Math.max(0, Math.round(startPositionMs))
      playbackStartupWatchdog.cancel()
      await fileMutation.finished
      if (waitingToken === playbackRequestToken && fileMutation.shouldResume) {
        await play(track, commandSource, fileMutation.positionMs, forceResolve, allowRememberedPosition, requestedQueueIndex, shuffleNavigation)
      }
      return
    }
    // An internal reload of the same track keeps a queued replay; a different track
    // supersedes it.
    discardSupersededReplay(track.id)
    if (startPositionMs <= 0 && allowRememberedPosition && !forceResolve && commandSource === 'local') {
      startPositionMs = rememberedLongFormStart(track)
    }
    initEvents()
    markCommandSource(commandSource)
    const token = ++playbackRequestToken
    const isCurrent = () => token === playbackRequestToken
    prepareBiliVideoSkipTrack(track, token)
    const requestStarted = performance.now()
    tracePlaybackUi(
      'store_play_enter',
      track,
      `command=${commandSource}, startMs=${Math.max(0, Math.round(startPositionMs))}, force=${forceResolve}`,
      token,
    )
    const settings = useSettingsStore()
    // 换歌时一发起请求就静音上一首，新音源解析、缓冲再慢也不会卡着旧声音；切歌交叉淡化要让它继续出声。
    // 同一首歌重新请求（断流恢复、换地址、音质切换）时旧会话在新会话就绪前继续出声，避免一断一断
    const isTrackChange = !!currentTrack.value && currentTrack.value.id !== track.id
    const keepsPreviousAudible = !isTrackChange || (isPlaying.value
      && settings.crossfadeNext
      && Math.round(settings.crossfadeInDuration) > 0
      && Math.round(settings.crossfadeOutDuration) > 0)
    claimPlaybackRequest(track, token, keepsPreviousAudible, requestStarted, isCurrent)
    replacePlaybackDemand(track)
    playbackStartupWatchdog.cancel()
    const previousTrack = currentTrack.value
    if (previousTrack && (previousTrack.id !== track.id || previousTrack.album !== track.album)) {
      persistLongFormProgress(previousTrack, currentRenderedPosition())
    }
    // 直链只在本次请求确认有效时共享，切歌或强制刷新先清掉旧 URL
    currentStreamUrl.value = !forceResolve && isDirectStreamUrl(track.audioUrl)
      ? track.audioUrl.trim()
      : null
    setResolvedStreamUrls(currentStreamUrl.value ? [currentStreamUrl.value] : [])
    rememberStreamQualities(null)
    const wasPlayingBeforeSwitch = isPlaying.value
    const hadPlaybackSessionBeforeRequest = hasPlaybackSession.value
    const isSwitchingTrack = !!previousTrack && previousTrack.id !== track.id
    const requestedStartMs = startPositionMs > 1000
      ? track.durationMs > 0
        ? clampPlaybackPosition(startPositionMs, track.durationMs)
        : Math.max(0, Math.round(startPositionMs))
      : 0
    const { useOverlapCrossfade, wantsCrossfade, transitionFadeInMs, transitionFadeOutMs } =
      resolvePlaybackTransition(settings, wasPlayingBeforeSwitch, isSwitchingTrack, requestedStartMs)
    let appliedLoadSeekSeq: number | null = null
    let appliedLoadPositionMs = 0

    if (requestedStartMs > 0) {
      markOptimisticSeek(requestedStartMs, commandSource, { durationMs: track.durationMs, fromPlaybackStart: true })
      deferredPlaybackSeek = {
        requestGeneration: token,
        positionMs: requestedStartMs,
        seekSeq: lastSeekCommand.value.seq,
      }
    } else {
      deferredPlaybackSeek = null
      pendingSeek = null
      seekGuardUntil = 0
      setRenderedPosition(0, track.durationMs)
    }

    function currentLoadStartPlan() {
      const start = resolvePlaybackLoadStart(
        token,
        requestedStartMs,
        deferredPlaybackSeek,
      )
      return {
        ...start,
        useCrossfade: wantsCrossfade && start.positionMs === 0,
      }
    }

    function markLoadStartApplied(start: { positionMs: number; seekSeq: number | null }) {
      appliedLoadPositionMs = start.positionMs
      if (start.seekSeq !== null) appliedLoadSeekSeq = start.seekSeq
    }

    const commitTrack = createTrackCommit(track, requestedQueueIndex, shuffleNavigation)

    // 加入队列
    if (!queue.value.find(t => t.id === track.id)) {
      queue.value.push(track)
    }
    // 当前选择代表用户的最新播放意图，必须在任何异步解析前提交
    commitTrack()
    isLoadingAudio.value = true
    hasPlaybackSession.value = true
    // 网络和解码尚未完成时也保存用户的最新播放意图
    savePlayerState()
    // overlap-crossfade 切歌时旧会话仍在出声直到新会话淡入, 立即置 false 会让播放按钮
    // 闪示暂停态（PB-10）。此场景保持播放态, 仅非交叉淡入切歌才落 false
    if (!(useOverlapCrossfade && wasPlayingBeforeSwitch)) {
      isPlaying.value = false
      _interpIsPlaying = false
    }

    try {
      if (token !== playbackRequestToken) return

      const loaded = await loadPlaybackSource({
        track, token, forceResolve, hadPlaybackSessionBeforeRequest, requestStarted,
        transitionFadeOutMs, transitionFadeInMs, currentLoadStartPlan, markLoadStartApplied, isCurrent,
      })
      if (!loaded) return
      const { dur, playedFromPlaybackCache, playingPreviewClip } = loaded

      commitTrack()
      durationMs.value = dur || track.durationMs
      loadedPlaybackRequestToken = token
      loadedPlaybackTrackKey.value = JSON.stringify([track.playlistKey ?? null, track.id])
      const deferredSeek = deferredPlaybackSeek?.requestGeneration === token
        ? deferredPlaybackSeek
        : null
      let deferredSeekFailed = false
      if (deferredSeek && deferredSeek.seekSeq !== appliedLoadSeekSeq) {
        try {
          await invoke<void>('seek', {
            positionMs: deferredSeek.positionMs,
            requestGeneration: token,
          })
          if (token !== playbackRequestToken) return
          if (isPlaybackSeekCompletionCurrent(
            token,
            playbackRequestToken,
            deferredSeek.seekSeq,
            lastSeekCommand.value.seq,
          )) {
            appliedLoadSeekSeq = deferredSeek.seekSeq
          }
        } catch (error) {
          if (isPlaybackSeekCompletionCurrent(
            token,
            playbackRequestToken,
            deferredSeek.seekSeq,
            lastSeekCommand.value.seq,
          )) {
            deferredSeekFailed = true
            pendingSeek = null
            seekGuardUntil = 0
            log.warn('deferred seek failed after media load:', error)
          }
        }
      }
      if (deferredPlaybackSeek?.requestGeneration === token) {
        deferredPlaybackSeek = null
      }
      const startMs = !deferredSeekFailed
        && lastSeekCommand.value.requestGeneration === token
        ? clampPlaybackPosition(lastSeekCommand.value.positionMs)
        : clampPlaybackPosition(appliedLoadPositionMs)
      if (startMs > 0) {
        setRenderedPosition(startMs)
        armPendingSeek(startMs)
      } else {
        setRenderedPosition(0)
        pendingSeek = null
        seekGuardUntil = 0
        clearPauseGuard()
      }
      isPlaying.value = true
      hasPlaybackSession.value = true
      isLoadingAudio.value = false
      tracePlaybackUi(
        'session_committed',
        track,
        `durationMs=${durationMs.value}, startMs=${startMs}, totalMs=${Math.round(performance.now() - requestStarted)}`,
        token,
      )

      // 重置插值状态
      _interpAnchorMs = startMs
      _interpAnchorTime = performance.now()
      _interpRenderedMs = startMs
      _interpSpeed = effectivePlaybackSpeed()
      _interpIsPlaying = true
      _interpDurationMs = durationMs.value
      _startInterpolationLoop()

      // 重置连续失败计数
      resetPlaybackFailures()
      if (playingPreviewClip) {
        useToastStore().show(i18n.global.t('player.playback_preview_only'), 'info')
      }
      // 记录 URL 解析时间（用于过期检测）
      if (playedFromPlaybackCache) {
        lastUrlResolveTime = 0
      } else if (!_currentLoadedFromDownloadPath) {
        lastUrlResolveTime = Date.now()
      }
      // 标记已加载（取消 restore 重载标记）
      _needsReload = false
      readPlaybackAudioInfo(token)

      // 记录播放历史
      // 跟随房间的远端同步不记历史；用户操作和自动推进都记
      if (commandSource !== 'remote_sync') {
        const history = useHistoryStore()
        history.record(track)
      }

      // 持久化状态 + 预热下一首
      savePlayerState()
      maybePrefetchNext()
      schedulePlaybackStartupWatchdog(token, track, startMs, commandSource)
      maybeAutoSkipBiliVideoInterval(startMs, durationMs.value)
    } catch (e) {
      if (token !== playbackRequestToken) return // 竞态过期请求，静默忽略

      playbackStartupWatchdog.cancel()

      if (deferredPlaybackSeek?.requestGeneration === token) {
        deferredPlaybackSeek = null
      }
      if (lastSeekCommand.value.requestGeneration === token) {
        pendingSeek = null
        seekGuardUntil = 0
      }

      const msg = e instanceof Error ? e.message : String(e)
      log.error('Play failed:', summarizeLogError(msg))
      tracePlaybackUi(
        'play_failed',
        track,
        `${summarizeLogError(msg)}, totalMs=${Math.round(performance.now() - requestStarted)}`,
        token,
      )
      playError.value = summarizeLogError(msg)
      isPlayingFromDownload.value = false
      isPlayingFromCache.value = false
      hasPlaybackSession.value = false
      // crossfade 失败时旧后端会话可能仍在出声（PCM ring 最多 4s 已解码帧），
      // 一律回查后端真实状态决定 isPlaying，而非武断置 false。原 `!trackCommitted`
      // 条件恒为 false（commitTrack 在 try 前已置位），使该恢复分支永为死代码（PB-05）
      const shouldRestorePreviousPlaybackState = useOverlapCrossfade && wasPlayingBeforeSwitch
      if (shouldRestorePreviousPlaybackState) {
        try {
          const state = await invoke<{ is_playing?: boolean }>('get_player_state')
          if (token !== playbackRequestToken) return
          isPlaying.value = !!state?.is_playing
          _interpIsPlaying = isPlaying.value
        } catch {
          if (token !== playbackRequestToken) return
          isPlaying.value = true
          _interpIsPlaying = true
        }
        if (_interpIsPlaying) _startInterpolationLoop()
      } else {
        isPlaying.value = false
        _interpIsPlaying = false
      }
      isLoadingAudio.value = false

      await handlePlaybackFailure(e instanceof PlaybackFailure ? e.reason : null, msg, commandSource)
    }
  }

  async function togglePlayPause(commandSource: PlaybackCommandSource = 'local') {
    const mutation = currentAudioFileMutation()
    if (mutation) {
      markCommandSource(commandSource)
      mutation.shouldResume = !isPlaying.value
      isPlaying.value = mutation.shouldResume
      if (!mutation.shouldResume && !mutation.releasesCurrentPlayback) {
        _interpIsPlaying = false
        freezeRenderedPosition()
        await invoke('pause')
      }
      return
    }
    markCommandSource(commandSource)
    log.info('togglePlayPause:', { source: commandSource, wasPlaying: isPlaying.value, trackId: currentTrack.value?.id })
    if (isLoadingAudio.value && shouldDeferPlaybackSeek(
      playbackRequestToken,
      loadedPlaybackRequestToken,
    )) {
      await pause(commandSource)
      return
    }
    // 恢复后首次播放：需要重新加载曲目
    if (!isPlaying.value && currentTrack.value && _needsReload) {
      _needsReload = false
      const savedPos = positionMs.value
      await play(currentTrack.value, commandSource, savedPos, false, false)
      return
    }

    // 乐观更新：立即翻转 UI 状态，消除 IPC 延迟感
    const previousPlaying = isPlaying.value
    const optimistic = !isPlaying.value
    isPlaying.value = optimistic
    if (optimistic) {
      startInterpolationFromRenderedPosition()
    } else {
      _interpIsPlaying = false
      freezeRenderedPosition()
      persistCurrentLongFormProgress()
    }

    try {
      const settings = useSettingsStore()
      if (optimistic) {
        if (settings.fadeIn && settings.fadeInDuration > 0) {
          await invoke('resume_with_fade', { durationMs: Math.round(settings.fadeInDuration) })
        } else {
          await invoke('resume')
        }
      } else if (settings.fadeIn && settings.fadeOutDuration > 0) {
        playbackStartupWatchdog.cancel()
        await invoke('pause_with_fade', { durationMs: Math.round(settings.fadeOutDuration) })
      } else {
        playbackStartupWatchdog.cancel()
        await invoke('pause')
      }
    } catch {
      isPlaying.value = previousPlaying
      if (previousPlaying) {
        startInterpolationFromRenderedPosition()
      } else {
        _interpIsPlaying = false
        freezeRenderedPosition()
      }
    }
  }

  async function pause(commandSource: PlaybackCommandSource = 'local') {
    if (blockedByListenTogether(commandSource)) return
    const mutation = currentAudioFileMutation()
    if (mutation) {
      markCommandSource(commandSource)
      mutation.shouldResume = false
      isPlaying.value = false
      if (!mutation.releasesCurrentPlayback) {
        _interpIsPlaying = false
        freezeRenderedPosition()
        await invoke('pause')
      }
      return
    }
    markCommandSource(commandSource)
    playbackStartupWatchdog.cancel()
    // 乐观更新
    isPlaying.value = false
    _interpIsPlaying = false
    freezeRenderedPosition()
    persistCurrentLongFormProgress()
    if (isLoadingAudio.value && shouldDeferPlaybackSeek(
      playbackRequestToken,
      loadedPlaybackRequestToken,
    )) {
      const cancellationToken = ++playbackRequestToken
      deferredPlaybackSeek = null
      isLoadingAudio.value = false
      hasPlaybackSession.value = false
      _needsReload = !!currentTrack.value
      replacePlaybackDemand(null)
      // 立即暂停旧会话输出：切歌加载中按暂停时，旧后端会话的 PCM ring（~4s 已解码帧）
      // 会继续出声而 UI 已显示暂停。fire-and-forget，不阻塞取消流程（PB-06）
      void invoke('pause').catch(() => {})
      try {
        await invoke<void>('begin_playback_request', {
          requestGeneration: cancellationToken,
          trackId: currentTrack.value?.id,
          source: currentTrack.value
            ? getPlaybackSourceKind(currentTrack.value) || currentTrack.value.source || 'local'
            : 'local',
        })
      } catch {
        // 预占只为让后端丢弃在途加载；本地请求已作废，失败无需回滚
      }
      savePlayerState()
      return
    }
    try {
      const settings = useSettingsStore()
      if (settings.fadeIn && settings.fadeOutDuration > 0) {
        await invoke('pause_with_fade', { durationMs: Math.round(settings.fadeOutDuration) })
      } else {
        await invoke('pause')
      }
    } catch {
      // 暂停是乐观更新，后端失败时界面保持暂停，与本地意图一致
    }
    savePlayerState()
  }

  async function resume(commandSource: PlaybackCommandSource = 'local') {
    if (blockedByListenTogether(commandSource)) return
    const mutation = currentAudioFileMutation()
    if (mutation) {
      markCommandSource(commandSource)
      mutation.shouldResume = true
      isPlaying.value = true
      return
    }
    markCommandSource(commandSource)
    if (!currentTrack.value) return
    if (isLoadingAudio.value && shouldDeferPlaybackSeek(
      playbackRequestToken,
      loadedPlaybackRequestToken,
    )) {
      return
    }

    if (_needsReload) {
      _needsReload = false
      const savedPos = positionMs.value
      await play(currentTrack.value, commandSource, savedPos, false, false)
      return
    }

    // URL 过期检测（10min）：在线来源 URL 过期后需重新解析；
    // YouTube 另按地址本身判断（缺 PoToken、签名即将过期，对齐 Android shouldRefreshUrlBeforeResume）
    const isOnlineSource = isRemotePlaybackTrack(currentTrack.value)
    const youtubeNeedsRefresh = shouldRefreshUrlBeforeResume(
      getPlaybackSourceKind(currentTrack.value) === 'youtube',
      currentStreamUrl.value,
    )
    if (isOnlineSource && (youtubeNeedsRefresh || (lastUrlResolveTime > 0
      && Date.now() - lastUrlResolveTime > URL_EXPIRY_MS))) {
      // URL 已过期，重新解析
      await play(currentTrack.value, commandSource, currentRenderedPosition(), true)
      return
    }

    // 乐观更新
    isPlaying.value = true
    startInterpolationFromRenderedPosition()
    try {
      const settings = useSettingsStore()
      if (settings.fadeIn && settings.fadeInDuration > 0) {
        await invoke('resume_with_fade', { durationMs: Math.round(settings.fadeInDuration) })
      } else {
        await invoke('resume')
      }
    } catch {
      // 恢复是乐观更新，后端失败时保留界面状态，由位置事件和启动看门狗兜底
    }
  }

  async function seekTo(ms: number, commandSource: PlaybackCommandSource = 'local') {
    if (blockedByListenTogether(commandSource)) return
    markCommandSource(commandSource)
    const roundedMs = Math.max(0, Math.round(ms))
    const maxSeekMs = durationMs.value || currentTrack.value?.durationMs || 0
    const posMs = maxSeekMs > 0 ? Math.min(roundedMs, maxSeekMs) : roundedMs
    const safePosMs = markOptimisticSeek(posMs, commandSource, { durationMs: maxSeekMs })
    const mutation = currentAudioFileMutation()
    if (mutation) {
      mutation.positionMs = safePosMs
      return
    }
    persistLongFormProgress(currentTrack.value, safePosMs)
    const seekSeq = lastSeekCommand.value.seq
    const requestGeneration = playbackRequestToken

    if (shouldDeferPlaybackSeek(requestGeneration, loadedPlaybackRequestToken)) {
      deferredPlaybackSeek = {
        requestGeneration,
        positionMs: safePosMs,
        seekSeq,
      }
      return
    }

    // YouTube 直链缺 PoToken、快过期或无法按范围读取时，先换新地址再从目标位置起播（对齐 Android）；
    // 下方 403 的被动重解析保留为兜底
    const seekTrack = currentTrack.value
    if (seekTrack && shouldRefreshUrlBeforeSeek(getPlaybackSourceKind(seekTrack) === 'youtube', currentStreamUrl.value)) {
      log.info('Refreshing YouTube stream URL before seek')
      void play(seekTrack, commandSource, safePosMs, true)
      return
    }

    // Fire-and-forget：不阻塞 UI，后端异步执行 seek
    invoke('seek', { positionMs: safePosMs, requestGeneration }).then(() => {
      if (!isPlaybackSeekCompletionCurrent(
        requestGeneration,
        playbackRequestToken,
        seekSeq,
        lastSeekCommand.value.seq,
      )) return
      // 后端确认后才钉死目标位置；失败时 catch 会回滚
      setRenderedPosition(safePosMs)
      seekGuardUntil = Math.max(seekGuardUntil, Date.now() + SEEK_EVENT_GUARD_MS)
      // 长时间停留后跳转，之前的下一首预取可能已过期：跳转成功后重新预热（对齐 Android STATE_READY 重新预取）
      maybePrefetchNext()
    }).catch((e) => {
      if (!isPlaybackSeekCompletionCurrent(
        requestGeneration,
        playbackRequestToken,
        seekSeq,
        lastSeekCommand.value.seq,
      )) return
      // 远端对目标区间返回 403（googlevideo 直链对远跳 range 的风控）：
      // 直链本身已不可用于该偏移，唯一出路是重新解析 URL 并从目标位置起播
      // （对齐 Android YouTubeSeekRefreshPolicy 的 seek 前刷新语义）
      const message = String((e as Error)?.message ?? e ?? '')
      if (message.includes('remote range request forbidden')
        && currentTrack.value && isRemotePlaybackTrack(currentTrack.value)) {
        log.warn('Seek hit forbidden range, re-resolving stream URL:', message)
        void play(currentTrack.value, commandSource, safePosMs, true)
        return
      }
      pendingSeek = null
      seekGuardUntil = 0
      // seek 失败：停止乐观插值，回到后端真实位置，避免「进度条在走但无声」
      _interpIsPlaying = isPlaying.value
      if (_interpIsPlaying) _startInterpolationLoop()
      void invoke<{
        is_playing?: boolean
        position_ms?: number
        duration_ms?: number
      }>('get_player_state').then((state) => {
        if (!isPlaybackSeekCompletionCurrent(
          requestGeneration,
          playbackRequestToken,
          seekSeq,
          lastSeekCommand.value.seq,
        )) return
        if (typeof state?.position_ms === 'number') {
          setRenderedPosition(state.position_ms, state.duration_ms)
        }
        if (typeof state?.is_playing === 'boolean') {
          isPlaying.value = state.is_playing
          _interpIsPlaying = state.is_playing
          if (_interpIsPlaying) _startInterpolationLoop()
        }
      }).catch(() => {})
      log.error('Seek failed:', e)
    })
  }

  /**
   * 播放结束自动触发（对齐 Android handleTrackEnded）
   * - repeat_one: 重新播放当前
   * - repeat_all: next(force=true) 强制推进
   * - off: 还有下一首则推进，否则停止播放但保留队列
   */
  async function handleTrackEnded() {
    const finishedRequestToken = playbackRequestToken
    // Track End 去重：200ms ticker 可能重复触发
    const trackId = currentTrack.value?.id ?? null
    if (trackId && trackId === lastTrackEndedId && finishedRequestToken === lastTrackEndedRequestToken
      && Date.now() - lastTrackEndedTime < 2000) {
      return
    }
    lastTrackEndedId = trackId
    lastTrackEndedTime = Date.now()
    lastTrackEndedRequestToken = finishedRequestToken
    // 播完即清掉记住的位置
    const finishedTrack = currentTrack.value
    if (finishedTrack) persistLongFormProgress(finishedTrack, Math.max(finishedTrack.durationMs || 0, durationMs.value))

    // 一起听会话激活时听众不本地推进: 暂停并上报 TRACK_FINISHED, 由房主/服务端决定切歌,
    // 避免因流时长差异先于房主播完而反向拖动整个房间（LB-02/LT-08）。
    // 重连途中也一样：上报会走 HTTP 或在重连后补发（对齐 Android）
    const lt = useListenTogetherStore()
    if (lt.roomId && !lt.isController) {
      const finishedRoomId = lt.roomId
      await pause('remote_sync')
      // 暂停确认可能晚于切歌或换房，旧会话的结束通知不能推进新会话
      if (finishedRequestToken !== playbackRequestToken || lt.roomId !== finishedRoomId || lt.isController) return
      lt.reportTrackFinished(trackId)
      return
    }

    // 睡眠定时器；以下都是播放器自己的推进，不是用户操作
    const isLast = !shuffleEnabled.value && queueIndex.value >= queue.value.length - 1
    // A queued replay of the current track is a valid reason to advance even at the
    // physical end of the queue with repeat off.
    const hasPendingReplay = !shuffleEnabled.value && pendingReplayIndex() >= 0
    if (sleepTimerMode.value === 'end_of_track') {
      await pause('local_safety')
      cancelSleepTimer()
      return
    }
    if (sleepTimerMode.value === 'end_of_queue') {
      if (isLast && !hasPendingReplay && repeatMode.value !== 'all') {
        await pause('local_safety')
        cancelSleepTimer()
        return
      }
    }

    if (repeatMode.value === 'one') {
      // 单曲循环：重新播放当前曲目
      if (currentTrack.value) {
        await play(currentTrack.value, 'local_safety')
      }
    } else if (repeatMode.value === 'all') {
      // 列表循环：强制推进到下一首（到末尾回到开头）
      await next(true, 'local_safety')
    } else {
      // 顺序播放：还有下一首则推进，否则停止
      if (shuffleEnabled.value || queueIndex.value < queue.value.length - 1 || hasPendingReplay) {
        await next(false, 'local_safety')
      } else {
        // 停止播放但保留队列（对齐 Android stopPlaybackPreservingQueue）
        await pause('local_safety')
        positionMs.value = 0
      }
    }
  }

  // 播放速度
  const playbackSpeed = ref(normalizePlaybackSpeed(settings.playbackSpeed))
  const playbackPitch = ref(normalizePlaybackPitch(settings.playbackPitch))
  const vinylEffectEnabled = ref(settings.vinylEffectEnabled === true)
  let listenTogetherSyncRateMultiplier: number | null = null

  function effectivePlaybackSpeed(): number {
    const multiplier = listenTogetherSyncRateMultiplier ?? 1
    return Math.max(0.1, Math.min(4, playbackSpeed.value * multiplier))
  }

  function setListenTogetherSyncPlaybackRate(multiplier: number | null) {
    listenTogetherSyncRateMultiplier = multiplier === null
      ? null
      : Math.max(0.9, Math.min(1.1, multiplier))
    const nowMs = currentRenderedPosition()
    _interpSpeed = effectivePlaybackSpeed()
    _interpAnchorMs = nowMs
    _interpAnchorTime = performance.now()
    _interpRenderedMs = nowMs
    interpolatedPositionMs.value = nowMs
    if (_speedInvokeTimer) clearTimeout(_speedInvokeTimer)
    _speedInvokeTimer = setTimeout(() => {
      _speedInvokeTimer = null
      void applyPlaybackParameter('speed')
    }, 80)
  }

  async function setSpeed(spd: number) {
    const next = normalizePlaybackSpeed(spd)
    const wasPlaying = _interpIsPlaying
    // 先锚定当前渲染位置，立刻切换插值速度，歌词/进度同步跟手
    const nowMs = currentRenderedPosition()
    playbackSpeed.value = next
    settings.playbackSpeed = next
    _interpSpeed = effectivePlaybackSpeed()
    _interpAnchorMs = nowMs
    _interpAnchorTime = performance.now()
    _interpRenderedMs = nowMs
    interpolatedPositionMs.value = nowMs
    // 通知歌词组件强制 seek 到当前点，避免倍速后视觉落后
    if (wasPlaying) {
      lastSeekCommand.value = {
        seq: lastSeekCommand.value.seq + 1,
        positionMs: nowMs,
        source: 'local',
        requestGeneration: playbackRequestToken,
      }
    }
    // 去抖后再下发：滑条 @input 每 tick 都会触发，只需送出最后的值。
    // 后端在输出端保持音调地实时变速，UI/插值已即时更新，音频落地晚 200ms 无感
    if (_speedInvokeTimer) clearTimeout(_speedInvokeTimer)
    _speedInvokeTimer = setTimeout(() => {
      _speedInvokeTimer = null
      void applyPlaybackParameter('speed')
    }, 200)
  }

  async function setPitch(pitch: number) {
    playbackPitch.value = normalizePlaybackPitch(pitch)
    settings.playbackPitch = playbackPitch.value
    // 音调不参与媒体时钟和歌词插值，只改变 DSP 的重采样倍率
    if (_pitchInvokeTimer) clearTimeout(_pitchInvokeTimer)
    _pitchInvokeTimer = setTimeout(() => {
      _pitchInvokeTimer = null
      void applyPlaybackParameter('pitch')
    }, 80)
  }

  let vinylEffectRevision = 0
  let vinylEffectApplyChain: Promise<void> = Promise.resolve()
  function applyPlaybackParameter(parameter: 'speed' | 'pitch' | 'loudness'): Promise<void> {
    vinylEffectApplyChain = Promise.all([audioEffectsApplyChain, vinylEffectApplyChain]).then(async () => {
      if (parameter === 'speed') await invoke('set_speed', { speed: effectivePlaybackSpeed() })
      else if (parameter === 'pitch') await invoke('set_pitch', { pitch: playbackPitch.value })
      else await invoke('set_loudness_gain', { gainMb: loudnessGainMb.value })
    }).catch(error => log.warn(`playback ${parameter} not applied:`, error))
    return vinylEffectApplyChain
  }
  async function setVinylEffect(enabled: boolean) {
    vinylEffectEnabled.value = enabled
    settings.vinylEffectEnabled = enabled
    // 切换时保留手动音调，并把尚未下发的滑条值一并应用
    if (_speedInvokeTimer) clearTimeout(_speedInvokeTimer)
    if (_pitchInvokeTimer) clearTimeout(_pitchInvokeTimer)
    _speedInvokeTimer = null
    _pitchInvokeTimer = null
    const revision = ++vinylEffectRevision
    vinylEffectApplyChain = Promise.all([audioEffectsApplyChain, vinylEffectApplyChain]).then(async () => {
      if (revision !== vinylEffectRevision) return
      await invoke('set_pitch', { pitch: playbackPitch.value })
      await invoke('set_speed', { speed: effectivePlaybackSpeed() })
      await invoke('set_vinyl_effect', { enabled })
    }).catch(error => log.warn('vinyl effect not applied:', error))
    await vinylEffectApplyChain
  }

  let audioEffectsRevision = 0
  let audioEffectsTimer: ReturnType<typeof setTimeout> | null = null
  let audioEffectsApplyChain: Promise<void> = Promise.resolve()

  function applyNativeAudioEffects(): Promise<void> {
    if (audioEffectsTimer) clearTimeout(audioEffectsTimer)
    audioEffectsTimer = null
    const revision = ++audioEffectsRevision
    const snapshot = normalizeAudioEffectsSettings(settings.audioEffects)
    // 请求串行，已排队的旧配置不能覆盖更新的滑条或重置结果
    audioEffectsApplyChain = audioEffectsApplyChain.then(async () => {
      if (revision !== audioEffectsRevision) return
      await invoke('set_audio_effects', { settings: snapshot, outputDevice: audioEffectsOutputDevice.value })
    }).catch(error => log.warn('native audio effects not applied:', error))
    return audioEffectsApplyChain
  }

  function scheduleNativeAudioEffects() {
    ++audioEffectsRevision
    if (audioEffectsTimer) clearTimeout(audioEffectsTimer)
    audioEffectsTimer = setTimeout(() => { void applyNativeAudioEffects() }, 48)
  }

  const {
    setVolume, effectivePlaybackPitch,
    audioEffects, audioEffectsOutputDevice, audioEffectsProfile, listeningTuning,
    updateAudioEffects, updateAudioEffectsProfile, updateAudioEffectsSound, setAudioEffectsPreset,
    setListeningTuning, resetListeningTuning, resetAudioEffectsSection, saveAudioEffectsPreset,
    deleteAudioEffectsPreset, exportAudioEffectsPreset, importAudioEffectsPreset, importAutoEq, refreshAudioEffectsStats,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands, hasActiveEffects,
    setLoudnessGain, setEqualizer, setEqualizerPreset, decoderCapabilities, loadDecoderCapabilities,
  } = createAudioEffects({
    settings, volume, playbackSpeed, playbackPitch, vinylEffectEnabled, savePlayerState,
    effectivePlaybackSpeed, applyPlaybackParameter, scheduleNativeAudioEffects, setSpeed, setPitch, setVinylEffect,
  })

  async function resetAudioEffects() {
    if (_speedInvokeTimer) clearTimeout(_speedInvokeTimer)
    if (_pitchInvokeTimer) clearTimeout(_pitchInvokeTimer)
    _speedInvokeTimer = null
    _pitchInvokeTimer = null
    if (audioEffectsTimer) clearTimeout(audioEffectsTimer)
    audioEffectsTimer = null
    ++audioEffectsRevision
    ++vinylEffectRevision
    settings.audioEffects = normalizeAudioEffectsSettings({ ...DEFAULT_AUDIO_EFFECTS_SETTINGS,
      userPresets: settings.audioEffects.userPresets })
    loudnessGainMb.value = 0
    equalizerEnabled.value = false
    equalizerPresetId.value = 'flat'
    equalizerBands.value = [0, 0, 0, 0, 0]
    playbackSpeed.value = 1.0
    playbackPitch.value = 1.0
    vinylEffectEnabled.value = false
    settings.loudnessGainMb = 0
    settings.equalizerEnabled = false
    settings.equalizerPresetId = 'flat'
    settings.equalizerBands = [0, 0, 0, 0, 0]
    settings.playbackSpeed = 1.0
    settings.playbackPitch = 1.0
    settings.vinylEffectEnabled = false
    const nowMs = currentRenderedPosition()
    _interpSpeed = effectivePlaybackSpeed()
    _interpAnchorMs = nowMs
    _interpAnchorTime = performance.now()
    _interpRenderedMs = nowMs
    interpolatedPositionMs.value = nowMs
    const resetRequest = Promise.all([audioEffectsApplyChain, vinylEffectApplyChain]).then(async () => {
      await invoke('reset_audio_effects')
      await invoke('set_speed', { speed: effectivePlaybackSpeed() })
    })
    audioEffectsApplyChain = resetRequest.catch(error => log.warn('native audio effects reset not applied:', error))
    const results = await Promise.allSettled([
      resetRequest,
    ])
    for (const result of results) {
      if (result.status === 'rejected') log.warn('audio effects reset not applied:', result.reason)
    }
  }

  async function applyPersistedSettings() {
    if (_speedInvokeTimer) clearTimeout(_speedInvokeTimer)
    if (_pitchInvokeTimer) clearTimeout(_pitchInvokeTimer)
    _speedInvokeTimer = null
    _pitchInvokeTimer = null
    volume.value = Math.max(0, Math.min(1, settings.volume))
    playbackSpeed.value = normalizePlaybackSpeed(settings.playbackSpeed)
    playbackPitch.value = normalizePlaybackPitch(settings.playbackPitch)
    vinylEffectEnabled.value = settings.vinylEffectEnabled === true
    loudnessGainMb.value = Math.round(Math.max(0, Math.min(1500, settings.loudnessGainMb)))
    equalizerEnabled.value = settings.equalizerEnabled
    equalizerPresetId.value = settings.equalizerPresetId
    equalizerBands.value = settings.equalizerBands.map(value => Math.round(Math.max(-1500, Math.min(1500, value))))
    _interpSpeed = effectivePlaybackSpeed()

    const restored: Array<[string, Promise<unknown>]> = [
      ['volume', invoke('set_volume', { level: volume.value })],
      ['output device', invoke('set_audio_output_device', { name: settings.audioOutputDevice || null })
        .catch((error) => {
          log.warn('preferred output device unavailable, using the system default:', error)
          return invoke('set_audio_output_device', { name: null })
        })],
      ['speed', invoke('set_speed', { speed: effectivePlaybackSpeed() })],
      ['pitch', invoke('set_pitch', { pitch: playbackPitch.value })],
      ['vinyl effect', setVinylEffect(vinylEffectEnabled.value)],
      ['loudness gain', invoke('set_loudness_gain', { gainMb: loudnessGainMb.value })],
      ['volume normalization', invoke('set_normalize_volume', { enabled: settings.normalizeVolume })],
      ['multichannel dynamic range compression', invoke('set_multichannel_drc', { enabled: settings.multichannelDrc })],
      ['channel balance', invoke('set_volume_balance', { balance: settings.volumeBalance })],
      ['equalizer', invoke('set_equalizer', { enabled: equalizerEnabled.value, bandLevelsMb: equalizerBands.value })],
    ]
    const results = await Promise.allSettled(restored.map(([, request]) => request))
    results.forEach((result, index) => {
      if (result.status === 'rejected') log.warn(`persisted ${restored[index][0]} not applied:`, result.reason)
    })
    await refreshAudioEffectsStats().catch(error => log.warn('audio effects output unavailable:', error))
    await applyNativeAudioEffects()
  }

  async function withReleasedAudioFile<T>(filePath: string, operation: () => Promise<T>): Promise<T> {
    const key = audioFilePathKey(filePath)
    if (audioFileMutations.has(key)) throw new Error('An operation on this audio file is already in progress')
    const track = currentTrack.value
    const usesFile = !!track && (
      (!isLoadingAudio.value && loadedPlaybackRequestToken === playbackRequestToken && audioFilePathKey(_currentLoadedFromDownloadPath) === key)
      || audioFilePathKey(track.audioUrl) === key
      || (isLoadingAudio.value && audioFilePathKey(downloadedTrackForPlayback(track)?.filePath) === key)
    )
    let finishMutation!: () => void
    const mutation = {
      trackId: usesFile ? track?.id || null : null,
      requestToken: null as number | null,
      shouldResume: usesFile && (isPlaying.value || isLoadingAudio.value),
      releasesCurrentPlayback: usesFile,
      positionMs: positionMs.value,
      finished: new Promise<void>(resolve => { finishMutation = resolve }),
    }
    audioFileMutations.set(key, mutation)
    let requestToken: number | null = null
    try {
      if (usesFile && track) {
        requestToken = ++playbackRequestToken
        mutation.requestToken = requestToken
        playbackStartupWatchdog.cancel()
        replacePlaybackDemand(null)
        _interpIsPlaying = false
        freezeRenderedPosition()
        mutation.positionMs = positionMs.value
        isPlaying.value = false
        isLoadingAudio.value = false
        _needsReload = true
        deferredPlaybackSeek = null
        pendingSeek = null
        seekGuardUntil = 0
        await invoke('begin_playback_request', {
          requestGeneration: requestToken, trackId: track.id,
          source: getPlaybackSourceKind(track) || track.source || 'local',
        })
      }
      await invoke<boolean>('release_audio_file', { path: filePath })
      if (requestToken !== null && requestToken === playbackRequestToken) {
        _currentLoadedFromDownloadPath = null
        isPlayingFromDownload.value = false
        isPlayingFromCache.value = false
        audioInfo.value = null
        loadedPlaybackRequestToken = 0
      }
      return await operation()
    } finally {
      audioFileMutations.delete(key)
      finishMutation()
      const resumeTrack = currentTrack.value
      if (requestToken !== null && requestToken === playbackRequestToken && resumeTrack && track && resumeTrack.id === track.id) {
        _needsReload = true
        savePlayerState()
        if (mutation.shouldResume && (isRemotePlaybackTrack(resumeTrack) || resumeTrack.audioUrl)) {
          try {
            await play(resumeTrack, 'local_safety', mutation.positionMs, true)
          } catch (error) {
            log.warn('Resume after audio file operation failed:', error)
          }
        } else {
          isPlaying.value = false
        }
      }
    }
  }

  function handleDownloadedFileRemoved(trackId: string, filePath?: string) {
    const key = audioFilePathKey(filePath)
    if (key) {
      queue.value = queue.value.map(track => audioFilePathKey(track.audioUrl) === key ? { ...track, audioUrl: '' } : track)
      if (currentTrack.value && audioFilePathKey(currentTrack.value.audioUrl) === key) {
        currentTrack.value = { ...currentTrack.value, audioUrl: '' }
      }
    }
    if (!currentTrack.value || currentTrack.value.id !== trackId) return
    if (filePath && _currentLoadedFromDownloadPath && _currentLoadedFromDownloadPath !== filePath) return

    _currentLoadedFromDownloadPath = null
    isPlayingFromDownload.value = false
    lastUrlResolveTime = 0
    if (!isPlaying.value) {
      _needsReload = true
    }
  }

  // 初始化：恢复持久化状态
  loadPlayerState()
  void applyPersistedSettings()
  void loadDecoderCapabilities()

  return {
    isPlaying, currentTrack, currentBiliVideoSkipTarget, positionMs, durationMs, queue, queueIndex,
    queueDisplayIndices, queueDisplayCurrentIndex,
    repeatMode, shuffleEnabled, volume, lyrics, playError, isLoadingAudio, isLoadingAudioSlow,
    hasPlaybackSession, loadedPlaybackTrackKey,
    audioLevel, beatImpulse, audioInfo, isPlayingFromDownload, isPlayingFromCache,
    lastCommandSource, lastCommandSequence, lastSeekCommand, isRemoteSyncGuardActive, nativeTrackEndCount,
    playbackSpeed, playbackPitch, vinylEffectEnabled, effectivePlaybackPitch, setVinylEffect,
    currentStreamUrl, sleepTimerMode, sleepRemainingSeconds,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands, hasActiveEffects,
    audioEffects, audioEffectsProfile, audioEffectsOutputDevice, updateAudioEffects, updateAudioEffectsProfile,
    listeningTuning, setListeningTuning, resetListeningTuning,
    updateAudioEffectsSound, setAudioEffectsPreset, resetAudioEffectsSection, saveAudioEffectsPreset,
    deleteAudioEffectsPreset, exportAudioEffectsPreset, importAudioEffectsPreset, importAutoEq, refreshAudioEffectsStats,
    progress, interpolatedPositionMs, interpolatedProgress, livePositionMs, isPlaybackClockAdvancing, effectivePlaybackSpeed,
    currentTimeFormatted, durationFormatted,
    initEvents, play, togglePlayPause, pause, resume, seekTo, next, previous,
    flushPlayerState,
    toggleRepeatMode, toggleShuffle, cyclePlayMode, applyListenTogetherPlaybackMode,
    playMode, setVolume, setSpeed, setPitch, setListenTogetherSyncPlaybackRate, getCurrentStreamUrl, getCurrentStreamUrls,
    resolveShareableStreamUrls,
    localPlaylistIdFor,
    setLoudnessGain, setEqualizer, setEqualizerPreset, resetAudioEffects,
    applyPersistedSettings, decoderCapabilities,
    startSleepTimer, startSleepTimerEndOfTrack, startSleepTimerEndOfQueue, cancelSleepTimer,
    playAll, shufflePlay, playQueueItem, addToQueueNext, addToQueueEnd, moveQueueItem, removeFromQueue, clearQueue,
    prefetchPlaybackTracks, prefetchIntent, upcomingTrack: () => nextPrefetchTracks()[0] ?? null,
    updateCurrentTrackInfo, patchCurrentTrackSyncPayload, restoreOriginalTrackInfo, hasOriginalTrackInfo,
    handleDownloadedFileRemoved, withReleasedAudioFile, replayWithQuality, downloadedTrackForPlayback,
  }
})
