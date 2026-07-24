import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useHistoryStore } from './history'
import { useToastStore } from './toast'
import {
  MAX_MEDIA_CACHE_SIZE_MB,
  MIN_MEDIA_CACHE_SIZE_MB,
  useSettingsStore,
} from './settings'
import { useDownloadStore } from './download'
import i18n from '@/i18n'
import {
  canonicalizePlaybackTrack,
  getPlaybackSourceKind,
  isRemotePlaybackTrack,
  normalizeBitrateKbps,
  playbackCacheReadCandidates,
  playbackCacheWriteOptions,
  playbackPrefetchCacheId,
  playbackSourceCandidates,
  playbackUrlResolver,
  resolvePlaybackResult,
  type PlaybackAudioSource,
  type PlaybackCacheReadCandidate,
  type PlaybackSourceSettings,
  type PlaybackResolution,
  type ResolvedPlaybackSource,
} from '@/modules/playback/playbackSource'
import { PlaybackPrefetchManager } from '@/modules/playback/playbackPrefetch'
import {
  PlaybackStartupWatchdog,
  resolvePlaybackFailureAdvanceAction,
} from '@/modules/playback/playbackPolicy'
import { resolvePlaybackQueueStartIndex } from '@/modules/playback/playbackQueue'
import {
  isPlaybackSeekCompletionCurrent,
  resolvePlaybackLoadStart,
  shouldDeferPlaybackSeek,
  initialPlaybackPrefetchWindow,
  shouldResolvePlaybackSourceInParallel,
  type DeferredPlaybackSeek,
} from '@/modules/playback/playbackRequest'
import { createLogger } from '@/utils/logger'
import { getTrackCoverUrl } from '@/utils/trackCover'
import {
  buildPersistedPlaybackQueue,
  restorePersistedPlaybackQueue,
} from '@/modules/playback/playerState'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('player')
const uiLog = createLogger('playback-ui')

export interface TrackInfo {
  id: string
  title: string
  artist: string
  album: string
  durationMs: number
  coverUrl: string
  audioUrl: string
  source?: string
  addedAt?: number
  syncPayload?: Record<string, unknown>
  playlistKey?: string
}

/**
 * 将后端返回的 snake_case TrackInfo 映射为前端 camelCase。
 * 前端手动构造的对象已经是 camelCase，此函数同时兼容两种格式
 */
export function normalizeTrack(raw: any): TrackInfo {
  const syncPayload = raw.syncPayload ?? raw.sync_payload
  const coverUrl = getTrackCoverUrl({
    coverUrl: raw.coverUrl,
    cover_url: raw.cover_url,
    syncPayload,
  })
  return canonicalizePlaybackTrack({
    id: raw.id ?? '',
    title: raw.title ?? '',
    artist: raw.artist ?? '',
    album: raw.album ?? '',
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
    coverUrl,
    audioUrl: raw.audioUrl ?? raw.audio_url ?? raw.url ?? '',
    source: raw.source,
    addedAt: raw.addedAt ?? raw.added_at ?? 0,
    syncPayload,
    playlistKey: raw.playlistKey ?? raw.playlist_key,
  })
}

export function tracePlaybackUi(
  stage: string,
  track?: TrackInfo | null,
  detail?: string,
  requestGeneration?: number,
) {
  const source = track ? getPlaybackSourceKind(track) || track.source || 'local' : undefined
  const safeDetail = detail ? summarizeLogError(detail) : undefined
  uiLog.info(
    `stage=${stage}`,
    { generation: requestGeneration, id: track?.id, source, detail: safeDetail },
  )
  if (!import.meta.env.DEV) return
  void invoke<void>('trace_playback_ui', {
    request: {
      stage,
      trackId: track?.id,
      source,
      detail: safeDetail,
      requestGeneration,
    },
  }).catch((error) => {
    uiLog.warn('backend trace unavailable:', error)
  })
}

/** UI 显示用的专辑名：清理 B站 "Bilibili|{cid}" 等内部格式 */
export function displayAlbum(album: string): string {
  if (album.startsWith('Bilibili|') || album === 'Bilibili') return 'Bilibili'
  if (album.startsWith('Netease')) return album.replace(/^Netease/, '').trim() || album
  return album
}

export interface LyricWord {
  startMs: number
  durationMs: number
  text: string
}

export interface LyricLine {
  startMs: number
  durationMs: number
  words: LyricWord[]
  text: string
  translation?: string
}

export type RepeatMode = 'off' | 'all' | 'one'
export type PlaybackCommandSource = 'local' | 'remote_sync'

export interface SeekCommandSnapshot {
  seq: number
  positionMs: number
  source: PlaybackCommandSource
  requestGeneration: number
}

// 当前音频质量信息
export interface AudioInfo {
  bitrate?: number  // kbps
  codec?: string    // e.g. "MP3", "FLAC", "AAC", "Opus"
  format?: string   // 原始格式标识
  source?: PlaybackAudioSource
  qualityKey?: string
  qualityLabel?: string
  qualityOptions?: Array<{ key: string; label: string }>
  mimeType?: string
  sampleRateHz?: number
  bitDepth?: number
  channelCount?: number
  specLabel?: string
}


// 纸面音质名: 与 NowPlaying 音质列表 / 设置页一致, 供缓存命中与 UI 共用
const NETEASE_QUALITY_I18N: Record<string, string> = {
  standard: '标准',
  higher: '较高',
  exhigh: '极高',
  lossless: '无损',
  hires: 'Hi-Res',
  jyeffect: '高清环绕声',
  sky: '沉浸环绕声',
  jymaster: '超清母带',
}
const QQ_QUALITY_I18N: Record<string, string> = {
  standard: '标准',
  high: '高',
  lossless: '无损',
}
const YOUTUBE_QUALITY_I18N: Record<string, string> = {
  low: '低',
  medium: '中',
  high: '高',
  very_high: '最高',
}
const BILI_QUALITY_I18N: Record<string, string> = {
  low: '流畅',
  medium: '标准',
  high: '较好',
  lossless: '无损',
  hires: 'Hi-Res',
  dolby: '杜比全景声',
}

function qualityLabelFromKey(source?: string | null, key?: string | null): string | undefined {
  if (!key) return undefined
  const k = key.trim().toLowerCase()
  if (!k) return undefined
  const map =
    source === 'netease' ? NETEASE_QUALITY_I18N
    : source === 'qq' ? QQ_QUALITY_I18N
    : source === 'youtube' ? YOUTUBE_QUALITY_I18N
    : source === 'bilibili' ? BILI_QUALITY_I18N
    : null
  return map?.[k] ?? key
}

function qualityOptionsFromSource(source?: string | null): Array<{ key: string; label: string }> | undefined {
  const map =
    source === 'netease' ? NETEASE_QUALITY_I18N
    : source === 'qq' ? QQ_QUALITY_I18N
    : source === 'youtube' ? YOUTUBE_QUALITY_I18N
    : source === 'bilibili' ? BILI_QUALITY_I18N
    : null
  if (!map) return undefined
  return Object.entries(map).map(([key, label]) => ({ key, label }))
}

interface PendingSeekState {
  targetMs: number
  issuedAt: number
  expiresAt: number
}

// 均衡器预设（5频段: 60Hz, 230Hz, 910Hz, 3.6kHz, 14kHz，单位 mB）
export const EQ_PRESETS: Record<string, number[]> = {
  flat:           [0, 0, 0, 0, 0],
  acoustic:       [300, 200, 0, 100, 200],
  bass_boost:     [600, 400, 0, 0, 0],
  bass_reduce:    [-600, -400, 0, 0, 0],
  classical:      [400, 200, -100, 200, 300],
  dance:          [500, 200, 100, -100, 200],
  deep:           [500, 300, 100, -100, -200],
  electronic:     [500, 300, 0, 100, 400],
  hip_hop:        [500, 300, 0, 100, 300],
  jazz:           [300, 100, -100, 100, 300],
  latin:          [300, 0, -100, 200, 400],
  loudness:       [500, 200, 0, -100, -200],
  lounge:         [-200, -100, 0, 100, 200],
  piano:          [200, 100, 0, 100, 200],
  pop:            [-100, 200, 400, 200, -100],
  rnb:            [500, 400, 100, -100, 200],
  rock:           [400, 200, -100, 200, 400],
  small_speakers: [400, 200, 100, 200, 400],
  spoken_word:    [-200, 0, 300, 200, -100],
  treble_boost:   [0, 0, 0, 400, 600],
  treble_reduce:  [0, 0, 0, -400, -600],
  vocal_boost:    [-200, 0, 400, 300, 0],
  custom:         [0, 0, 0, 0, 0],
}

// 播放位置插值状态（模块级，rAF 驱动）
let _interpAnchorMs = 0         // 上次后端报告的位置
let _interpAnchorTime = 0       // 对应的 performance.now() 锚点
let _interpRenderedMs = 0       // 上次渲染的插值位置
let _interpSpeed = 1.0          // 当前播放速度快照
let _interpIsPlaying = false    // 当前播放状态快照
let _interpDurationMs = 0       // 当前时长快照
let _interpLoopStarted = false  // rAF 循环是否已启动
const SEEK_EVENT_GUARD_MS = 900
const SEEK_SETTLE_TIMEOUT_MS = 4500
const SEEK_BACKWARD_TOLERANCE_MS = 600
const SEEK_FORWARD_TOLERANCE_MS = 1200
const POSITION_BACKWARD_TOLERANCE_MS = 250
const PAUSE_EVENT_GUARD_MS = 2500
const PAUSE_BACKWARD_TOLERANCE_MS = 250

// 连续失败熔断
let consecutivePlayFailures = 0
const MAX_CONSECUTIVE_FAILURES = 10
let _isAutoSkipping = false

// Shuffle 三栈模型
let shuffleBag: number[] = []       // 未播放索引池
let shuffleHistory: number[] = []   // 已播放栈 (previous 回溯)
let shuffleFuture: number[] = []    // 预排队栈 (next 或 previous 回退)

// playbackRequestToken 防竞态
let playbackRequestToken = Date.now() * 1000

// URL 过期检测 (10min)
let lastUrlResolveTime = 0
const URL_EXPIRY_MS = 10 * 60 * 1000

// Track End 去重
let lastTrackEndedId: string | null = null
let lastTrackEndedTime = 0

// 播放抽象层：解析缓存、预热仲裁、启动看门狗
const playbackPrefetchManager = new PlaybackPrefetchManager()
const playbackStartupWatchdog = new PlaybackStartupWatchdog()
let startupRecoveryAttempts = 0
const MAX_STARTUP_RECOVERY_ATTEMPTS = 2
const STARTUP_WATCHDOG_REMOTE_MS = 8_000
const STARTUP_WATCHDOG_YOUTUBE_MS = 12_000

// 状态持久化
const PLAYER_STATE_KEY = 'neri:player-state'
let _persistDebounceTimer: ReturnType<typeof setTimeout> | null = null
let _progressPersistTime = 0
const PERSIST_DEBOUNCE_MS = 250
const PROGRESS_PERSIST_INTERVAL_MS = 15000
// 恢复后需重新加载标记
let _needsReload = false
let _remoteSyncGuardUntil = 0
let _currentLoadedFromDownloadPath: string | null = null

export const usePlayerStore = defineStore('player', () => {
  const settings = useSettingsStore()
  const isPlaying = ref(false)
  const currentTrack = ref<TrackInfo | null>(null)
  const currentResolvedStreamUrl = ref<string | null>(null)
  const positionMs = ref(0)
  const durationMs = ref(0)
  const queue = ref<TrackInfo[]>([])
  const queueIndex = ref(-1)
  const repeatMode = ref<RepeatMode>('off')
  const shuffleEnabled = ref(false)
  const volume = ref(settings.volume)
  const lyrics = ref<LyricLine[]>([])

  // 播放错误信息（供 UI 展示）
  const playError = ref<string | null>(null)
  // 是否正在加载音频（下载/解码中）
  const isLoadingAudio = ref(false)
  // 是否存在可见播放上下文；恢复态由 _needsReload 标记后端尚未装载
  const hasPlaybackSession = ref(false)

  // 当前音频质量信息
  const audioInfo = ref<AudioInfo | null>(null)
  const isPlayingFromDownload = ref(false)
  // 当前会话是否命中播放缓存 (非下载文件)
  const isPlayingFromCache = ref(false)

  // 睡眠定时器
  const sleepTimerEndMs = ref(0) // 0 = 未启用
  const sleepTimerMode = ref<'countdown' | 'end_of_track' | 'end_of_queue' | null>(null)
  const sleepTimerNowMs = ref(Date.now())
  let _sleepTimerInterval: ReturnType<typeof setInterval> | null = null

  /** 剩余睡眠时间（秒） */
  const sleepRemainingSeconds = computed(() => {
    if (!sleepTimerMode.value || sleepTimerEndMs.value <= 0) return 0
    if (sleepTimerMode.value === 'end_of_track') return -1 // 特殊标记
    return Math.max(0, Math.ceil((sleepTimerEndMs.value - sleepTimerNowMs.value) / 1000))
  })

  function startSleepTimer(minutes: number) {
    cancelSleepTimer()
    const now = Date.now()
    sleepTimerMode.value = 'countdown'
    sleepTimerNowMs.value = now
    sleepTimerEndMs.value = now + minutes * 60 * 1000
    _sleepTimerInterval = setInterval(() => {
      const now = Date.now()
      sleepTimerNowMs.value = now
      if (now >= sleepTimerEndMs.value) {
        pause()
        cancelSleepTimer()
      }
    }, 1000)
  }

  function startSleepTimerEndOfTrack() {
    cancelSleepTimer()
    sleepTimerMode.value = 'end_of_track'
    sleepTimerEndMs.value = 1 // 非零表示启用
  }

  function startSleepTimerEndOfQueue() {
    cancelSleepTimer()
    sleepTimerMode.value = 'end_of_queue'
    sleepTimerEndMs.value = 1
  }

  function cancelSleepTimer() {
    if (_sleepTimerInterval) {
      clearInterval(_sleepTimerInterval)
      _sleepTimerInterval = null
    }
    sleepTimerMode.value = null
    sleepTimerEndMs.value = 0
    sleepTimerNowMs.value = Date.now()
  }

  // 音频分析数据
  const audioLevel = ref(0)
  const beatImpulse = ref(0)

  // 插值后的播放位置（rAF 驱动，60fps 平滑）
  const interpolatedPositionMs = ref(0)
  const interpolatedProgress = computed(() =>
    durationMs.value > 0 ? interpolatedPositionMs.value / durationMs.value : 0
  )

  const progress = computed(() =>
    durationMs.value > 0 ? positionMs.value / durationMs.value : 0
  )
  const currentTimeFormatted = computed(() => formatTime(interpolatedPositionMs.value))
  const durationFormatted = computed(() => formatTime(durationMs.value))

  // 是否已初始化事件监听
  let eventsInitialized = false
  // seek 后忽略 position 事件的时间窗口
  let seekGuardUntil = 0
  // 记住最后 seek 的位置，用于 resume 时重新 seek（防止后端丢失 seek-while-paused）
  let lastSeekedMs: number | null = null
  // seek 后等待后端位置收敛，防止刚开播的旧 position 把进度条拉回去
  let pendingSeek: PendingSeekState | null = null
  let pauseGuardUntil = 0
  let pauseFrozenMs: number | null = null
  const lastCommandSource = ref<PlaybackCommandSource>('local')
  const lastSeekCommand = ref<SeekCommandSnapshot>({
    seq: 0,
    positionMs: 0,
    source: 'local',
    requestGeneration: 0,
  })
  let loadedPlaybackRequestToken = 0
  let deferredPlaybackSeek: DeferredPlaybackSeek | null = null

  function markCommandSource(source: PlaybackCommandSource) {
    lastCommandSource.value = source
    if (source === 'remote_sync') {
      _remoteSyncGuardUntil = Date.now() + 3000
    }
  }

  function isRemoteSyncGuardActive() {
    return Date.now() < _remoteSyncGuardUntil
  }

  // 状态持久化函数
  function persistedPlayerState(compact = false): Record<string, any> {
    const settings = useSettingsStore()
    const persistedQueue = buildPersistedPlaybackQueue(
      queue.value,
      queueIndex.value,
      currentTrack.value,
      compact,
    )
    const state: Record<string, any> = {
      ...persistedQueue,
      volume: volume.value,
    }
    if (settings.keepProgress) {
      state.positionMs = currentRenderedPosition()
    }
    if (settings.keepPlaybackMode) {
      state.repeatMode = repeatMode.value
      state.shuffleEnabled = shuffleEnabled.value
    }
    return state
  }

  /** 退出前立即落盘，避免防抖定时器尚未执行 */
  function flushPlayerState() {
    if (_persistDebounceTimer) {
      clearTimeout(_persistDebounceTimer)
      _persistDebounceTimer = null
    }
    try {
      localStorage.setItem(PLAYER_STATE_KEY, JSON.stringify(persistedPlayerState()))
      uiLog.info('state persisted', {
        queueSize: queue.value.length,
        queueIndex: queueIndex.value,
        trackId: currentTrack.value?.id,
        positionMs: currentRenderedPosition(),
      })
    } catch (fullStateError) {
      try {
        const compactState = persistedPlayerState(true)
        localStorage.setItem(PLAYER_STATE_KEY, JSON.stringify(compactState))
        uiLog.warn('full queue persistence failed, compact state saved:', {
          queueSize: queue.value.length,
          trackId: currentTrack.value?.id,
          error: fullStateError,
        })
      } catch (compactStateError) {
        uiLog.error('state persistence failed:', {
          fullStateError,
          compactStateError,
        })
      }
    }
  }

  /** 保存播放器状态到 localStorage（250ms debounce） */
  function savePlayerState() {
    if (_persistDebounceTimer) clearTimeout(_persistDebounceTimer)
    _persistDebounceTimer = setTimeout(() => {
      flushPlayerState()
    }, PERSIST_DEBOUNCE_MS)
  }

  /** 从 localStorage 恢复播放器状态（store 初始化时调用，不自动播放） */
  function loadPlayerState() {
    try {
      hasPlaybackSession.value = false
      const raw = localStorage.getItem(PLAYER_STATE_KEY)
      if (!raw) return
      const state = JSON.parse(raw)
      const settings = useSettingsStore()

      const restored = restorePersistedPlaybackQueue(
        state.queue,
        state.queueIndex,
        state.hasPlaybackSession,
        state.currentTrackId,
        state.currentTrackPlaylistKey,
        normalizeTrack,
        track => !!track.id && (!!track.audioUrl || !track.id.startsWith('local:')),
      )
      queue.value = restored.queue
      queueIndex.value = restored.queueIndex
      currentTrack.value = restored.currentTrack
      _needsReload = restored.hasPlaybackSession
      // 恢复的是可见播放上下文；音频后端会在用户再次点击播放时装载
      hasPlaybackSession.value = restored.hasPlaybackSession

      if (settings.keepProgress && typeof state.positionMs === 'number') {
        setRenderedPosition(state.positionMs)
      }

      if (settings.keepPlaybackMode) {
        if (state.repeatMode && ['off', 'all', 'one'].includes(state.repeatMode)) {
          repeatMode.value = state.repeatMode
        }
        if (typeof state.shuffleEnabled === 'boolean') {
          shuffleEnabled.value = state.shuffleEnabled
          if (state.shuffleEnabled && queue.value.length > 1) {
            rebuildShuffleBag()
          }
        }
      }

      // 恢复 durationMs 以便 UI 显示进度条
      if (currentTrack.value && currentTrack.value.durationMs > 0) {
        durationMs.value = currentTrack.value.durationMs
      }
      uiLog.info('state restored', {
        queueSize: queue.value.length,
        queueIndex: queueIndex.value,
        trackId: currentTrack.value?.id,
        positionMs: positionMs.value,
        miniPlayerVisible: hasPlaybackSession.value,
      })
    } catch (error) {
      uiLog.warn('state restore failed:', error)
    }
  }

  /** 节流保存进度（每 15s，对齐 Android scheduleStatePersist） */
  function maybePersistProgress() {
    const now = Date.now()
    if (now - _progressPersistTime >= PROGRESS_PERSIST_INTERVAL_MS) {
      _progressPersistTime = now
      savePlayerState()
    }
  }

  // Shuffle 三栈辅助函数
  /** Fisher-Yates 洗牌重建 shuffleBag，排除当前索引 */
  function rebuildShuffleBag() {
    shuffleBag = []
    for (let i = 0; i < queue.value.length; i++) {
      if (i !== queueIndex.value) shuffleBag.push(i)
    }
    for (let i = shuffleBag.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffleBag[i], shuffleBag[j]] = [shuffleBag[j], shuffleBag[i]]
    }
  }

  /** 队列插入后更新 shuffle 索引 */
  function shiftShuffleIndicesForInsert(insertIdx: number) {
    shuffleBag = shuffleBag.map(i => i >= insertIdx ? i + 1 : i)
    shuffleHistory = shuffleHistory.map(i => i >= insertIdx ? i + 1 : i)
    shuffleFuture = shuffleFuture.map(i => i >= insertIdx ? i + 1 : i)
    shuffleBag.push(insertIdx) // 新曲目加入未播放池
  }

  /** 队列移除后更新 shuffle 索引 */
  function shiftShuffleIndicesForRemove(removeIdx: number) {
    shuffleBag = shuffleBag.filter(i => i !== removeIdx).map(i => i > removeIdx ? i - 1 : i)
    shuffleHistory = shuffleHistory.filter(i => i !== removeIdx).map(i => i > removeIdx ? i - 1 : i)
    shuffleFuture = shuffleFuture.filter(i => i !== removeIdx).map(i => i > removeIdx ? i - 1 : i)
  }

  // URL 预热
  function playbackSourceSettings(): PlaybackSourceSettings {
    return {
      neteaseQuality: settings.neteaseQuality,
      qqMusicQuality: settings.qqMusicQuality,
      biliQuality: settings.biliQuality,
      youtubeQuality: settings.youtubeQuality,
      neteaseAutoSourceSwitch: settings.neteaseAutoSourceSwitch,
    }
  }

  function playbackDemandKey(track: TrackInfo | null): string | null {
    if (!track || !isRemotePlaybackTrack(track)) return null
    return playbackPrefetchCacheId(track, playbackSourceSettings())
  }

  function replacePlaybackDemand(track: TrackInfo | null): void {
    playbackPrefetchManager.replacePlaybackDemand(playbackDemandKey(track))
  }

  function nextPrefetchTracks(): TrackInfo[] {
    if (!queue.value.length) return []

    const firstIndex = (() => {
      if (shuffleEnabled.value) {
        if (shuffleFuture.length > 0) return shuffleFuture[shuffleFuture.length - 1]
        if (shuffleBag.length > 0) return shuffleBag[0]
        return -1
      }
      const nextIndex = queueIndex.value + 1
      if (nextIndex < queue.value.length) return nextIndex
      return repeatMode.value === 'all' ? 0 : -1
    })()
    if (firstIndex < 0) return []

    const tracks: TrackInfo[] = []
    const maxWindow = getPlaybackSourceKindForPrefetch(queue.value[firstIndex]) === 'youtube'
      ? 6
      : 1
    let index = firstIndex
    for (let count = 0; count < maxWindow; count += 1) {
      const track = queue.value[index]
      if (track && isRemotePlaybackTrack(track)) tracks.push(track)
      if (maxWindow === 1) break
      index += 1
      if (index >= queue.value.length) {
        if (repeatMode.value !== 'all') break
        index = 0
      }
      if (index === queueIndex.value) break
    }
    return tracks
  }

  function getPlaybackSourceKindForPrefetch(track?: TrackInfo): string | null {
    return track ? getPlaybackSourceKind(track) : null
  }

  /** 预热后续曲目的解析结果，当前播放需求始终拥有优先级 */
  function maybePrefetchNext() {
    const tracks = nextPrefetchTracks()
    if (tracks.length === 0) return

    playbackPrefetchManager.prefetchWindow(
      tracks,
      playbackSourceSettings(),
      playbackUrlResolver,
    )
  }

  function prefetchPlaybackTracks(tracks: readonly TrackInfo[]) {
    const candidates = initialPlaybackPrefetchWindow(tracks)
      .filter(track => isRemotePlaybackTrack(track))
    if (candidates.length === 0) return
    playbackPrefetchManager.prefetchWindow(
      [...candidates],
      playbackSourceSettings(),
      playbackUrlResolver,
    )
  }

  async function resolvePlaybackUrl(
    track: TrackInfo,
    forceRefresh = false,
    qualityOverride?: string,
  ): Promise<PlaybackResolution> {
    return resolvePlaybackResult(track, playbackSourceSettings(), {
      forceRefresh,
      qualityOverride,
      requestGeneration: playbackRequestToken,
    })
  }

  async function resolveCurrentStreamUrl(): Promise<string | null> {
    const track = currentTrack.value
    const requestToken = playbackRequestToken
    const existingUrl = currentResolvedStreamUrl.value?.trim()
    if (existingUrl && /^https?:\/\//i.test(existingUrl)) return existingUrl
    if (!track || !isRemotePlaybackTrack(track)) return null

    try {
      const resolution = await resolvePlaybackUrl(track)
      if (requestToken !== playbackRequestToken || currentTrack.value !== track) return null
      if (resolution.type !== 'success') return null
      const streamUrl = resolution.url?.trim()
      if (!streamUrl || !/^https?:\/\//i.test(streamUrl)) return null
      currentResolvedStreamUrl.value = streamUrl
      return streamUrl
    } catch {
      return null
    }
  }

  function takePrefetchedPlaybackUrl(track: TrackInfo): ResolvedPlaybackSource | null {
    return playbackPrefetchManager.take(track, playbackSourceSettings())
  }

  function schedulePlaybackStartupWatchdog(
    token: number,
    track: TrackInfo,
    startPositionMs: number,
    commandSource: PlaybackCommandSource,
  ): void {
    if (!isRemotePlaybackTrack(track)) return
    const sourceKind = getPlaybackSourceKind(track)
    const timeoutMs = sourceKind === 'youtube'
      ? STARTUP_WATCHDOG_YOUTUBE_MS
      : STARTUP_WATCHDOG_REMOTE_MS
    playbackStartupWatchdog.schedule({
      timeoutMs,
      startPositionMs,
      getPositionMs: () => positionMs.value,
      isActive: () => token === playbackRequestToken
        && currentTrack.value?.id === track.id
        && isPlaying.value
        && !isLoadingAudio.value,
      onStall: () => {
        if (token !== playbackRequestToken) return
        if (startupRecoveryAttempts >= MAX_STARTUP_RECOVERY_ATTEMPTS) {
          playbackStartupWatchdog.cancel()
          consecutivePlayFailures += 1
          _isAutoSkipping = true
          void next(true, commandSource).finally(() => {
            _isAutoSkipping = false
          })
          return
        }
        startupRecoveryAttempts += 1
        const resumePositionMs = currentRenderedPosition()
        void play(track, commandSource, resumePositionMs, true)
      },
    })
  }

  function commitBackendPosition(nextPositionMs: number, nextDurationMs?: number, forceRendered = false) {
    const safeDurationMs = nextDurationMs || durationMs.value || currentTrack.value?.durationMs || 0
    const safePositionMs = safeDurationMs > 0
      ? Math.min(Math.max(0, nextPositionMs), safeDurationMs)
      : Math.max(0, nextPositionMs)

    if (typeof nextDurationMs === 'number' && nextDurationMs > 0) {
      durationMs.value = nextDurationMs
    }

    const renderedMs = clampPlaybackPosition(_interpRenderedMs)
    // 倍速播放时后端时钟与插值都应按速度前进；仍拒绝明显回跳
    if (_interpIsPlaying && !forceRendered && safePositionMs < renderedMs - POSITION_BACKWARD_TOLERANCE_MS) {
      // 仅忽略回跳，不把锚点锁死在旧渲染值（否则倍速歌词会落后）
      return
    }

    positionMs.value = safePositionMs
    _interpAnchorMs = safePositionMs
    _interpAnchorTime = performance.now()
    _interpDurationMs = Math.max(durationMs.value || _interpDurationMs, safePositionMs)
    // 播放中也允许后端时钟校准插值，保证歌词与倍速同步
    if (!_interpIsPlaying || forceRendered || Math.abs(safePositionMs - renderedMs) > 80) {
      _interpRenderedMs = safePositionMs
      interpolatedPositionMs.value = safePositionMs
    }
  }

  function clampPlaybackPosition(position: number, durationOverride?: number): number {
    const safeDurationMs = durationOverride && durationOverride > 0
      ? durationOverride
      : durationMs.value || currentTrack.value?.durationMs || _interpDurationMs || 0
    const safePositionMs = Number.isFinite(position) ? Math.max(0, Math.round(position)) : 0
    return safeDurationMs > 0 ? Math.min(safePositionMs, safeDurationMs) : safePositionMs
  }

  function clearPauseGuard(): void {
    pauseFrozenMs = null
    pauseGuardUntil = 0
  }

  function armPendingSeek(targetMs: number): void {
    const now = Date.now()
    pendingSeek = {
      targetMs,
      issuedAt: now,
      expiresAt: now + SEEK_SETTLE_TIMEOUT_MS,
    }
    seekGuardUntil = now + SEEK_EVENT_GUARD_MS
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
    options: { bumpSeq?: boolean; durationMs?: number } = {},
  ): number {
    const safeTargetMs = setRenderedPosition(targetMs, options.durationMs)
    lastSeekCommand.value = {
      seq: options.bumpSeq === false ? lastSeekCommand.value.seq : lastSeekCommand.value.seq + 1,
      positionMs: safeTargetMs,
      source: commandSource,
      requestGeneration: playbackRequestToken,
    }
    lastSeekedMs = safeTargetMs
    clearPauseGuard()
    armPendingSeek(safeTargetMs)
    return safeTargetMs
  }

  function freezeRenderedPosition(): number {
    const frozenMs = setRenderedPosition(currentRenderedPosition())
    pauseFrozenMs = frozenMs
    pauseGuardUntil = Date.now() + PAUSE_EVENT_GUARD_MS
    return frozenMs
  }

  function startInterpolationFromRenderedPosition(): void {
    const startMs = setRenderedPosition(currentRenderedPosition())
    _interpAnchorMs = startMs
    _interpAnchorTime = performance.now()
    _interpRenderedMs = startMs
    _interpSpeed = playbackSpeed.value
    _interpIsPlaying = true
    clearPauseGuard()
  }

  function normalizePositionAfterPause(nextPositionMs: number): number | null {
    if (pauseFrozenMs === null) return nextPositionMs

    const now = Date.now()
    if (now >= pauseGuardUntil) {
      clearPauseGuard()
      return nextPositionMs
    }

    if (nextPositionMs < pauseFrozenMs - PAUSE_BACKWARD_TOLERANCE_MS) {
      return null
    }

    return pauseFrozenMs
  }

  function shouldAcceptPositionAfterSeek(nextPositionMs: number, nextDurationMs?: number): boolean {
    if (!pendingSeek) return true

    const now = Date.now()
    if (now >= pendingSeek.expiresAt) {
      pendingSeek.expiresAt = now + SEEK_SETTLE_TIMEOUT_MS
      seekGuardUntil = 0
    }

    const elapsedMs = Math.max(0, now - pendingSeek.issuedAt)
    const speed = _interpIsPlaying ? Math.max(0.25, playbackSpeed.value || _interpSpeed || 1) : 0
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

  /** 启动 rAF 插值循环（仅调用一次） */
  function _startInterpolationLoop() {
    if (_interpLoopStarted) return
    _interpLoopStarted = true

    function tick() {
      requestAnimationFrame(tick)

      // seek 等待后端确认期间冻结进度条，避免「seek 完成仍空转」
      if (pendingSeek) {
        interpolatedPositionMs.value = Math.round(pendingSeek.targetMs)
        _interpRenderedMs = pendingSeek.targetMs
        _interpAnchorMs = pendingSeek.targetMs
        _interpAnchorTime = performance.now()
        return
      }

      if (!_interpIsPlaying) {
        interpolatedPositionMs.value = Math.round(_interpRenderedMs)
        return
      }

      const now = performance.now()
      const elapsed = (now - _interpAnchorTime) * _interpSpeed
      const predicted = _interpAnchorMs + elapsed
      const clamped = Math.max(0, Math.min(predicted, _interpDurationMs))

      // 普通进度同步不允许把 UI 时间轴拉回，真正回跳只走 seek
      if (clamped >= _interpRenderedMs - 24) {
        _interpRenderedMs = Math.max(_interpRenderedMs, clamped)
      }

      interpolatedPositionMs.value = Math.round(_interpRenderedMs)
    }

    requestAnimationFrame(tick)
  }

  function initEvents() {
    if (eventsInitialized) return
    eventsInitialized = true
    _startInterpolationLoop()

    // 监听后端播放位置更新
    listen<{
      positionMs: number
      durationMs: number
      requestGeneration: number
    }>('player:position', (e) => {
      if (e.payload.requestGeneration !== playbackRequestToken) return
      // seek 后时间窗口内忽略旧位置事件
      if (Date.now() < seekGuardUntil) return
      if (!shouldAcceptPositionAfterSeek(e.payload.positionMs, e.payload.durationMs)) return
      const normalizedPositionMs = normalizePositionAfterPause(e.payload.positionMs)
      if (normalizedPositionMs === null) return
      commitBackendPosition(normalizedPositionMs, e.payload.durationMs)

      // 节流保存播放进度（每 15s）
      if (_interpIsPlaying) {
        maybePersistProgress()
      }
    })

    // 监听音频电平
    listen<{ level: number; beat: number }>('player:audio-level', (e) => {
      audioLevel.value = e.payload.level
      beatImpulse.value = e.payload.beat
    })

    // 监听播放完成（对齐 Android handleTrackEnded）
    listen<{ requestGeneration: number }>('player:track-ended', (e) => {
      if (e.payload.requestGeneration !== playbackRequestToken) return
      handleTrackEnded()
    })

    // 系统媒体键事件（SMTC / MPRIS，来自 Rust 后端）
    listen('media:play', () => {
      void resume()
    })

    listen('media:pause', () => {
      void pause()
    })

    listen('media:toggle', () => {
      void togglePlayPause()
    })

    listen('media:next', () => {
      next()
    })

    listen('media:previous', () => {
      previous()
    })

    listen<{ positionMs: number }>('media:seek-requested', (e) => {
      void seekTo(e.payload.positionMs)
    })
  }

  async function play(
    track: TrackInfo,
    commandSource: PlaybackCommandSource = 'local',
    startPositionMs = 0,
    forceResolve = false,
  ) {
    initEvents()
    markCommandSource(commandSource)
    const token = ++playbackRequestToken
    currentResolvedStreamUrl.value = null
    const requestStarted = performance.now()
    tracePlaybackUi(
      'store_play_enter',
      track,
      `command=${commandSource}, startMs=${Math.max(0, Math.round(startPositionMs))}, force=${forceResolve}`,
      token,
    )
    const claimStarted = performance.now()
    void invoke<void>('begin_playback_request', {
      requestGeneration: token,
      trackId: track.id,
      source: getPlaybackSourceKind(track) || track.source || 'local',
      hasCover: !!getTrackCoverUrl(track),
      hasAudioUrl: !!track.audioUrl,
      hasSyncPayload: !!track.syncPayload,
    }).then(() => {
      if (token !== playbackRequestToken) return
      tracePlaybackUi(
        'backend_request_claimed',
        track,
        `invokeMs=${Math.round(performance.now() - claimStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
        token,
      )
    }).catch((error) => {
      if (token === playbackRequestToken) {
        log.warn('playback request preclaim failed:', error)
      }
    })
    replacePlaybackDemand(track)
    playbackStartupWatchdog.cancel()
    const previousTrack = currentTrack.value
    const wasPlayingBeforeSwitch = isPlaying.value
    const hadPlaybackSessionBeforeRequest = hasPlaybackSession.value
    const isSwitchingTrack = !!previousTrack && previousTrack.id !== track.id
    const settings = useSettingsStore()
    const fadeInDurationMs = Math.max(0, Math.round(settings.fadeInDuration))
    const fadeOutDurationMs = Math.max(0, Math.round(settings.fadeOutDuration))
    const overlapFadeInDurationMs = settings.crossfadeNext
      ? Math.max(0, Math.round(settings.crossfadeInDuration))
      : fadeInDurationMs
    const overlapFadeOutDurationMs = settings.crossfadeNext
      ? Math.max(0, Math.round(settings.crossfadeOutDuration))
      : fadeOutDurationMs
    const useOverlapCrossfade = wasPlayingBeforeSwitch && isSwitchingTrack
      && (settings.crossfadeNext || settings.crossfade)
      && overlapFadeOutDurationMs > 0
      && overlapFadeInDurationMs > 0
    const useTrackSwitchFadeIn = settings.fadeIn
      && wasPlayingBeforeSwitch
      && isSwitchingTrack
      && !useOverlapCrossfade
      && fadeInDurationMs > 0
    let trackCommitted = false
    const requestedStartMs = startPositionMs > 1000
      ? track.durationMs > 0
        ? clampPlaybackPosition(startPositionMs, track.durationMs)
        : Math.max(0, Math.round(startPositionMs))
      : 0
    const useInitialFadeIn = settings.fadeIn
      && !wasPlayingBeforeSwitch
      && fadeInDurationMs > 0
      && requestedStartMs === 0
    const transitionFadeInMs = useTrackSwitchFadeIn || useInitialFadeIn
      ? fadeInDurationMs
      : overlapFadeInDurationMs
    const wantsCrossfade = useOverlapCrossfade || useTrackSwitchFadeIn || useInitialFadeIn
    const transitionFadeOutMs = useOverlapCrossfade ? overlapFadeOutDurationMs : 0
    let appliedLoadSeekSeq: number | null = null
    let appliedLoadPositionMs = 0

    if (requestedStartMs > 0) {
      markOptimisticSeek(requestedStartMs, commandSource, { durationMs: track.durationMs })
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

    function commitTrack() {
      if (trackCommitted) return
      currentTrack.value = track
      queueIndex.value = resolvePlaybackQueueStartIndex(
        queue.value,
        track.id,
        track.playlistKey,
      )
      trackCommitted = true
    }

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
    isPlaying.value = false
    _interpIsPlaying = false

    try {
      if (token !== playbackRequestToken) return

      let dur = 0
      let playedFromDownloadedFile = false
      let playedFromPlaybackCache = false
      playError.value = null
      audioInfo.value = null
      isPlayingFromDownload.value = false
      isPlayingFromCache.value = false

      const downloaded = useDownloadStore().getDownloadedTrack(track.id)
      if (downloaded?.filePath && isRemotePlaybackTrack(track)) {
        try {
          const startPlan = currentLoadStartPlan()
          dur = await playDownloadedFile(
            downloaded.filePath,
            downloaded.durationMs || track.durationMs,
            startPlan.useCrossfade,
            transitionFadeOutMs,
            transitionFadeInMs,
            token,
            startPlan.positionMs,
          )
          if (token !== playbackRequestToken) return
          markLoadStartApplied(startPlan)
          playedFromDownloadedFile = true
          _currentLoadedFromDownloadPath = downloaded.filePath
          isPlayingFromDownload.value = true
        } catch (e) {
          if (token !== playbackRequestToken) return
          log.warn('downloaded file unavailable, falling back to online source:', e)
          _currentLoadedFromDownloadPath = null
          isPlayingFromDownload.value = false
        }
      }

      if (playedFromDownloadedFile) {
        // 本地下载：不展示 Local/download 占位；格式从扩展名推断，码率有则显示
        const ext = downloaded?.filePath
          ?.split(/[\\/]/)
          .pop()
          ?.split('.')
          .pop()
          ?.toLowerCase()
        const formatFromExt = ext && ext.length <= 5 ? ext : undefined
        audioInfo.value = {
          // 不写 codec: Local，避免进度条下出现 Local · download
          format: formatFromExt,
          // getPlaybackSourceKind 已覆盖远程源；仅当明确是 local 时回退
          source: getPlaybackSourceKind(track)
            ?? (track.source === 'local' ? 'local' : undefined),
          qualityKey: undefined,
        }
        lastUrlResolveTime = 0
      } else if (isRemotePlaybackTrack(track)) {
        _currentLoadedFromDownloadPath = null
        isPlayingFromDownload.value = false
        // 进入在线解析前默认非缓存; 命中缓存时会再置 true
        isPlayingFromCache.value = false
        let prefetchedResolution = takePrefetchedPlaybackUrl(track)
        const resolveInParallel = shouldResolvePlaybackSourceInParallel(
          hadPlaybackSessionBeforeRequest,
          !!prefetchedResolution,
        )
        const coldResolution = resolveInParallel
          ? resolvePlaybackUrl(track, forceResolve).catch((error): PlaybackResolution => ({
              type: 'failure',
              message: error instanceof Error ? error.message : String(error),
              retryable: true,
            }))
          : null
        const cacheCandidates = playbackCacheReadCandidates(track, playbackSourceSettings())
        tracePlaybackUi(
          'remote_pipeline_start',
          track,
          `cacheCandidates=${cacheCandidates.length}, prefetched=${!!prefetchedResolution}, parallelResolve=${resolveInParallel}`,
          token,
        )
        try {
          const startPlan = currentLoadStartPlan()
          const cacheLookupStarted = performance.now()
          tracePlaybackUi(
            'cache_lookup_start',
            track,
            `candidates=${cacheCandidates.length}, startMs=${startPlan.positionMs}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
            token,
          )
          const cached = await playCachedRemoteAudioCandidates(
            cacheCandidates,
            track.durationMs,
            startPlan.useCrossfade,
            transitionFadeOutMs,
            transitionFadeInMs,
            token,
            startPlan.positionMs,
          )
          if (token !== playbackRequestToken) return
          if (cached) {
            markLoadStartApplied(startPlan)
            dur = cached.durationMs
            playedFromPlaybackCache = true
            isPlayingFromCache.value = true
            audioInfo.value = {
              source: cached.source,
              qualityKey: cached.qualityKey,
              // 缓存命中也要展示纸面音质名 (最高/极高…), 不能只塞 raw key
              qualityLabel: qualityLabelFromKey(cached.source, cached.qualityKey),
              qualityOptions: qualityOptionsFromSource(cached.source),
            }
            tracePlaybackUi(
              'cache_lookup_hit',
              track,
              `quality=${cached.qualityKey}, durationMs=${cached.durationMs}, lookupMs=${Math.round(performance.now() - cacheLookupStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
              token,
            )
          } else {
            tracePlaybackUi(
              'cache_lookup_miss',
              track,
              `lookupMs=${Math.round(performance.now() - cacheLookupStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
              token,
            )
          }
        } catch (error) {
          if (token !== playbackRequestToken) return
          log.warn('persistent cache unavailable, resolving online source:', error)
        }

        if (!playedFromPlaybackCache) {
          let result = prefetchedResolution
          prefetchedResolution = null
          if (!result) {
            const resolveStarted = performance.now()
            tracePlaybackUi(
              'source_resolve_wait',
              track,
              `parallel=${!!coldResolution}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
              token,
            )
            const resolution = await (coldResolution ?? resolvePlaybackUrl(track, forceResolve))
            tracePlaybackUi(
              'source_resolve_returned',
              track,
              `type=${resolution.type}, resolveMs=${Math.round(performance.now() - resolveStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
              token,
            )
            if (resolution.type !== 'success') {
              if (resolution.type === 'requires_login') {
                throw new Error(resolution.message || 'Playback requires login')
              }
              if (resolution.type === 'waiting_for_authoritative_stream') {
                throw new Error('Waiting for authoritative playback stream')
              }
              throw new Error(resolution.message)
            }
            result = resolution
          }
          tracePlaybackUi(
            'source_resolved',
            track,
            `quality=${result.qualityKey}, candidates=${result.candidateUrls?.length ?? 0}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
            token,
          )
          if (token !== playbackRequestToken) return

          const playResolvedSource = async (resolved: ResolvedPlaybackSource) => {
            let lastError: unknown = null
            for (const sourceCandidate of playbackSourceCandidates(resolved)) {
              const candidates = [sourceCandidate.url, ...sourceCandidate.candidateUrls]
                .filter((url, index, values) => values.indexOf(url) === index)
              for (const [candidateIndex, candidateUrl] of candidates.entries()) {
                if (token !== playbackRequestToken) {
                  return { duration: 0, resolved: sourceCandidate, streamUrl: null }
                }
                const candidateStarted = performance.now()
                try {
                  const cacheWrite = playbackCacheWriteOptions(
                    sourceCandidate,
                    candidateIndex,
                    candidateUrl,
                  )
                  const startPlan = currentLoadStartPlan()
                  tracePlaybackUi(
                    'backend_stream_start',
                    track,
                    `candidate=${candidateIndex}, startMs=${startPlan.positionMs}, crossfade=${startPlan.useCrossfade}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
                    token,
                  )
                  const duration = await playRemoteUrl(
                    candidateUrl,
                    track.durationMs,
                    startPlan.useCrossfade,
                    transitionFadeOutMs,
                    transitionFadeInMs,
                    token,
                    startPlan.positionMs,
                    cacheWrite.cacheKey,
                    cacheWrite.expectedContentLength,
                  )
                  markLoadStartApplied(startPlan)
                  tracePlaybackUi(
                    'backend_stream_ready',
                    track,
                    `candidate=${candidateIndex}, durationMs=${duration}, candidateMs=${Math.round(performance.now() - candidateStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
                    token,
                  )
                  return { duration, resolved: sourceCandidate, streamUrl: candidateUrl }
                } catch (error) {
                  lastError = error
                  tracePlaybackUi(
                    'backend_stream_failed',
                    track,
                    `candidate=${candidateIndex}, candidateMs=${Math.round(performance.now() - candidateStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}, error=${summarizeLogError(error)}`,
                    token,
                  )
                }
              }
            }
            throw lastError instanceof Error
              ? lastError
              : new Error(String(lastError || 'All playback candidates failed'))
          }

          try {
            const played = await playResolvedSource(result)
            if (token !== playbackRequestToken) return
            currentResolvedStreamUrl.value = played.streamUrl
            dur = played.duration
            result = played.resolved
          } catch (firstError) {
            if (token !== playbackRequestToken) return
            playbackUrlResolver.invalidate(track, playbackSourceSettings())
            const refreshed = await resolvePlaybackUrl(
              track,
              true,
              getPlaybackSourceKind(track) === 'youtube' ? 'high' : undefined,
            )
            if (refreshed.type !== 'success') throw firstError
            const played = await playResolvedSource(refreshed)
            if (token !== playbackRequestToken) return
            currentResolvedStreamUrl.value = played.streamUrl
            dur = played.duration
            result = played.resolved
          }
          if (token !== playbackRequestToken) return
          {
            const qKey = result.audioInfo?.qualityKey ?? result.qualityKey
            const rawLabel = result.audioInfo?.qualityLabel
            const paperLabel =
              (rawLabel && rawLabel !== qKey && !/kbps/i.test(rawLabel))
                ? rawLabel
                : qualityLabelFromKey(result.source, qKey)
            audioInfo.value = {
              bitrate: result.audioInfo?.bitrateKbps
                ?? normalizeBitrateKbps(result.bitrate),
              codec: result.audioInfo?.codecLabel ?? result.codec,
              format: result.format || result.audioInfo?.mimeType,
              source: result.source,
              qualityKey: qKey,
              qualityLabel: paperLabel,
              qualityOptions: result.audioInfo?.qualityOptions
                ?? qualityOptionsFromSource(result.source),
              mimeType: result.audioInfo?.mimeType,
              sampleRateHz: result.audioInfo?.sampleRateHz,
              bitDepth: result.audioInfo?.bitDepth,
              channelCount: result.audioInfo?.channelCount,
              // 过滤 kbps, 只保留纸面规格
              specLabel: result.audioInfo?.specLabel
                ?.split('|')
                .map(s => s.trim())
                .filter(s => s && !/kbps/i.test(s))
                .join(' | ') || undefined,
            }
          }
        }
      } else {
        _currentLoadedFromDownloadPath = null
        isPlayingFromDownload.value = false
        isPlayingFromCache.value = false
        // 本地文件
        const startPlan = currentLoadStartPlan()
        tracePlaybackUi(
          'backend_file_start',
          track,
          `startMs=${startPlan.positionMs}, crossfade=${startPlan.useCrossfade}`,
          token,
        )
        if (startPlan.useCrossfade) {
          dur = await invoke<number>('crossfade_file', {
            path: track.audioUrl,
            durationHintMs: track.durationMs,
            fadeOutMs: transitionFadeOutMs, fadeInMs: transitionFadeInMs,
            requestGeneration: token,
          })
        } else {
          dur = await invoke<number>('play_file', {
            path: track.audioUrl,
            durationHintMs: track.durationMs,
            startPositionMs: startPlan.positionMs,
            requestGeneration: token,
          })
        }
        if (token !== playbackRequestToken) return
        markLoadStartApplied(startPlan)
      }

      commitTrack()
      durationMs.value = dur || track.durationMs
      loadedPlaybackRequestToken = token
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
            lastSeekedMs = null
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
        lastSeekedMs = startMs
        armPendingSeek(startMs)
      } else {
        setRenderedPosition(0)
        pendingSeek = null
        seekGuardUntil = 0
        clearPauseGuard()
        lastSeekedMs = null
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
      _interpSpeed = playbackSpeed.value
      _interpIsPlaying = true
      _interpDurationMs = durationMs.value

      // 重置连续失败计数
      consecutivePlayFailures = 0
      startupRecoveryAttempts = 0
      // 记录 URL 解析时间（用于过期检测）
      if (playedFromPlaybackCache) {
        lastUrlResolveTime = 0
      } else if (!_currentLoadedFromDownloadPath) {
        lastUrlResolveTime = Date.now()
      }
      // 标记已加载（取消 restore 重载标记）
      _needsReload = false

      // 记录播放历史
      if (commandSource === 'local') {
        const history = useHistoryStore()
        history.record(track)
      }

      // 持久化状态 + 预热下一首
      savePlayerState()
      maybePrefetchNext()
      schedulePlaybackStartupWatchdog(token, track, startMs, commandSource)
    } catch (e) {
      if (token !== playbackRequestToken) return // 竞态过期请求，静默忽略

      playbackStartupWatchdog.cancel()
      ++playbackRequestToken
      currentResolvedStreamUrl.value = null

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
      const shouldRestorePreviousPlaybackState = useOverlapCrossfade && wasPlayingBeforeSwitch && !trackCommitted
      if (shouldRestorePreviousPlaybackState) {
        try {
          const state = await invoke<{ is_playing?: boolean }>('get_player_state')
          isPlaying.value = !!state?.is_playing
          _interpIsPlaying = isPlaying.value
        } catch {
          isPlaying.value = true
          _interpIsPlaying = true
        }
      } else {
        isPlaying.value = false
        _interpIsPlaying = false
      }
      isLoadingAudio.value = false

      const toast = useToastStore()
      toast.error((i18n.global as any).t('player.play_failed', { msg }))

      // 连续失败熔断 + 自动 skip
      consecutivePlayFailures++
      const failureAction = resolvePlaybackFailureAdvanceAction({
        currentIndex: queueIndex.value,
        queueSize: queue.value.length,
        repeatAll: repeatMode.value === 'all',
        shuffle: shuffleEnabled.value,
        shuffleFutureSize: shuffleFuture.length,
        shuffleBagSize: shuffleBag.length,
      })
      if (
        consecutivePlayFailures < MAX_CONSECUTIVE_FAILURES
        && failureAction !== 'stop'
      ) {
        _isAutoSkipping = true
        try {
          await next(failureAction === 'wrap', commandSource)
        } finally {
          _isAutoSkipping = false
        }
      } else if (consecutivePlayFailures >= MAX_CONSECUTIVE_FAILURES) {
        toast.error((i18n.global as any).t('player.too_many_failures', '连续播放失败过多，已停止'))
      }
    }
  }

  async function togglePlayPause(commandSource: PlaybackCommandSource = 'local') {
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
      await play(currentTrack.value, commandSource, savedPos)
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
    }

    try {
      const settings = useSettingsStore()
      if (optimistic) {
        if (settings.fadeIn && settings.fadeInDuration > 0) {
          await invoke('resume_with_fade', { durationMs: settings.fadeInDuration })
        } else {
          await invoke('resume')
        }
        // 恢复播放成功后不再需要 pause seek 兜底
        lastSeekedMs = null
      } else if (settings.fadeIn && settings.fadeOutDuration > 0) {
        playbackStartupWatchdog.cancel()
        await invoke('pause_with_fade', { durationMs: settings.fadeOutDuration })
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
    markCommandSource(commandSource)
    playbackStartupWatchdog.cancel()
    // 乐观更新
    isPlaying.value = false
    _interpIsPlaying = false
    freezeRenderedPosition()
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
      try {
        await invoke<void>('begin_playback_request', {
          requestGeneration: cancellationToken,
          trackId: currentTrack.value?.id,
          source: currentTrack.value
            ? getPlaybackSourceKind(currentTrack.value) || currentTrack.value.source || 'local'
            : 'local',
        })
      } catch {}
      savePlayerState()
      return
    }
    try {
      const settings = useSettingsStore()
      if (settings.fadeIn && settings.fadeOutDuration > 0) {
        await invoke('pause_with_fade', { durationMs: settings.fadeOutDuration })
      } else {
        await invoke('pause')
      }
    } catch {}
    savePlayerState()
  }

  async function resume(commandSource: PlaybackCommandSource = 'local') {
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
      await play(currentTrack.value, commandSource, savedPos)
      return
    }

    // URL 过期检测（10min）：在线来源 URL 过期后需重新解析
    const isOnlineSource = isRemotePlaybackTrack(currentTrack.value)
    if (isOnlineSource && lastUrlResolveTime > 0
      && Date.now() - lastUrlResolveTime > URL_EXPIRY_MS) {
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
        await invoke('resume_with_fade', { durationMs: settings.fadeInDuration })
      } else {
        await invoke('resume')
      }
    } catch {}
  }

  async function seekTo(ms: number, commandSource: PlaybackCommandSource = 'local') {
    markCommandSource(commandSource)
    const roundedMs = Math.max(0, Math.round(ms))
    const maxSeekMs = durationMs.value || currentTrack.value?.durationMs || 0
    const posMs = maxSeekMs > 0 ? Math.min(roundedMs, maxSeekMs) : roundedMs
    const safePosMs = markOptimisticSeek(posMs, commandSource, { durationMs: maxSeekMs })
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
    }).catch((e) => {
      if (!isPlaybackSeekCompletionCurrent(
        requestGeneration,
        playbackRequestToken,
        seekSeq,
        lastSeekCommand.value.seq,
      )) return
      pendingSeek = null
      seekGuardUntil = 0
      // seek 失败：停止乐观插值，回到后端真实位置，避免「进度条在走但无声」
      _interpIsPlaying = isPlaying.value
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
    // Track End 去重：200ms ticker 可能重复触发
    const trackId = currentTrack.value?.id ?? null
    if (trackId && trackId === lastTrackEndedId && Date.now() - lastTrackEndedTime < 2000) {
      return
    }
    lastTrackEndedId = trackId
    lastTrackEndedTime = Date.now()

    // 睡眠定时器
    const isLast = !shuffleEnabled.value && queueIndex.value >= queue.value.length - 1
    if (sleepTimerMode.value === 'end_of_track') {
      ++playbackRequestToken
      currentResolvedStreamUrl.value = null
      await pause()
      cancelSleepTimer()
      return
    }
    if (sleepTimerMode.value === 'end_of_queue') {
      if (isLast && repeatMode.value !== 'all') {
        ++playbackRequestToken
        currentResolvedStreamUrl.value = null
        await pause()
        cancelSleepTimer()
        return
      }
    }

    if (repeatMode.value === 'one') {
      // 单曲循环：重新播放当前曲目
      if (currentTrack.value) {
        await play(currentTrack.value)
      }
    } else if (repeatMode.value === 'all') {
      // 列表循环：强制推进到下一首（到末尾回到开头）
      await next(true)
    } else {
      // 顺序播放：还有下一首则推进，否则停止
      if (shuffleEnabled.value || queueIndex.value < queue.value.length - 1) {
        await next(false)
      } else {
        // 停止播放但保留队列（对齐 Android stopPlaybackPreservingQueue）
        ++playbackRequestToken
        currentResolvedStreamUrl.value = null
        await pause()
        positionMs.value = 0
      }
    }
  }

  /**
   * 用户手动下一首（对齐 Android nextImpl）
   * - 不管 repeat_one，始终推进
   * - force=true 时列表末尾回绕
   * - Shuffle 模式使用三栈模型
   */
  async function next(force: boolean = false, commandSource: PlaybackCommandSource = 'local') {
    markCommandSource(commandSource)
    log.info('next:', { source: commandSource, force, shuffle: shuffleEnabled.value, repeat: repeatMode.value, index: queueIndex.value, queueLen: queue.value.length })
    if (queue.value.length === 0) return
    // 用户手动操作重置失败计数（自动 skip 不重置）
    if (!_isAutoSkipping) consecutivePlayFailures = 0

    let nextIdx: number
    if (shuffleEnabled.value) {
      // Shuffle 三栈模型
      if (shuffleFuture.length > 0) {
        // 优先从 future 栈弹出（previous 回退过的）
        shuffleHistory.push(queueIndex.value)
        nextIdx = shuffleFuture.pop()!
      } else if (shuffleBag.length > 0) {
        // 从未播放池随机取
        shuffleHistory.push(queueIndex.value)
        const bagIdx = Math.floor(Math.random() * shuffleBag.length)
        nextIdx = shuffleBag[bagIdx]
        shuffleBag.splice(bagIdx, 1)
      } else {
        // bag 已空
        if (force || repeatMode.value === 'all') {
          rebuildShuffleBag()
          if (shuffleBag.length > 0) {
            shuffleHistory.push(queueIndex.value)
            const bagIdx = Math.floor(Math.random() * shuffleBag.length)
            nextIdx = shuffleBag[bagIdx]
            shuffleBag.splice(bagIdx, 1)
          } else {
            return
          }
        } else {
          return // 顺序播放结束
        }
      }
    } else {
      if (queueIndex.value < queue.value.length - 1) {
        nextIdx = queueIndex.value + 1
      } else {
        if (force || repeatMode.value === 'all') {
          nextIdx = 0
        } else {
          // 已在末尾，不动
          return
        }
      }
    }
    await play(queue.value[nextIdx], commandSource)
  }

  /**
   * 用户手动上一首（对齐 Android previousImpl）
   * - 播放超过 3 秒则回到开头
   * - Shuffle 模式使用 history 栈回溯
   * - 非 shuffle：只有 repeat_all 才回绕到末尾
   */
  async function previous(commandSource: PlaybackCommandSource = 'local') {
    markCommandSource(commandSource)
    log.info('previous:', { source: commandSource, shuffle: shuffleEnabled.value, positionMs: Math.round(positionMs.value) })
    consecutivePlayFailures = 0
    if (queue.value.length === 0) return

    // 播放超过 3 秒则回到开头
    if (positionMs.value > 3000) {
      seekTo(0, commandSource)
      return
    }

    if (shuffleEnabled.value) {
      // Shuffle：从 history 栈回溯
      if (shuffleHistory.length > 0) {
        shuffleFuture.push(queueIndex.value)
        const prevIdx = shuffleHistory.pop()!
        await play(queue.value[prevIdx], commandSource)
      } else {
        seekTo(0, commandSource) // 无历史，重新开始当前曲目
      }
      return
    }

    // 非 shuffle 模式
    if (queueIndex.value > 0) {
      await play(queue.value[queueIndex.value - 1], commandSource)
    } else if (repeatMode.value === 'all') {
      await play(queue.value[queue.value.length - 1], commandSource)
    }
    // else: 已在开头且非列表循环，不动
  }

  async function toggleRepeatMode() {
    try {
      const mode = await invoke<string>('cycle_repeat')
      repeatMode.value = mode as RepeatMode
    } catch {
      const modes: RepeatMode[] = ['off', 'all', 'one']
      const idx = modes.indexOf(repeatMode.value)
      repeatMode.value = modes[(idx + 1) % modes.length]
    }
    log.info('repeat mode ->', repeatMode.value)
    savePlayerState()
  }

  async function toggleShuffle() {
    try {
      const enabled = await invoke<boolean>('toggle_shuffle')
      shuffleEnabled.value = enabled
    } catch {
      shuffleEnabled.value = !shuffleEnabled.value
    }
    // Shuffle 三栈管理
    if (shuffleEnabled.value) {
      rebuildShuffleBag()
      shuffleHistory = []
      shuffleFuture = []
    } else {
      shuffleBag = []
      shuffleHistory = []
      shuffleFuture = []
    }
    log.info('shuffle ->', shuffleEnabled.value)
    savePlayerState()
  }

  /**
   * 统一播放模式循环切换：顺序播放 -> 列表循环 -> 单曲循环 -> 随机播放 -> 顺序播放
   * 合并 repeat + shuffle 为一个按钮的逻辑
   */
  type PlayMode = 'sequential' | 'repeat_all' | 'repeat_one' | 'shuffle'

  const playMode = computed<PlayMode>(() => {
    if (shuffleEnabled.value) return 'shuffle'
    if (repeatMode.value === 'all') return 'repeat_all'
    if (repeatMode.value === 'one') return 'repeat_one'
    return 'sequential'
  })

  async function cyclePlayMode() {
    const current = playMode.value
    switch (current) {
      case 'sequential':
        // -> 列表循环
        if (shuffleEnabled.value) await toggleShuffle()
        repeatMode.value = 'all'
        try { await invoke<string>('cycle_repeat') } catch {}
        break
      case 'repeat_all':
        // -> 单曲循环
        repeatMode.value = 'one'
        try { await invoke<string>('cycle_repeat') } catch {}
        break
      case 'repeat_one':
        // -> 随机播放
        repeatMode.value = 'off'
        try { await invoke<string>('cycle_repeat') } catch {}
        if (!shuffleEnabled.value) await toggleShuffle()
        break
      case 'shuffle':
        // -> 顺序播放
        if (shuffleEnabled.value) await toggleShuffle()
        repeatMode.value = 'off'
        break
    }
    savePlayerState()
  }

  /**
   * 一起听远端模式应用 (对齐 Android applyListenTogetherPlaybackMode)
   * 仅本地落状态, 不反向上报, 由调用方负责 suppress watch
   */
  function applyListenTogetherPlaybackMode(options: {
    repeatMode?: number | null
    shuffleEnabled?: boolean | null
  }) {
    let changed = false
    if (options.repeatMode !== null && options.repeatMode !== undefined) {
      const mapped =
        options.repeatMode === 1 ? 'one'
          : options.repeatMode === 2 ? 'all'
            : options.repeatMode === 0 ? 'off'
              : null
      if (mapped && repeatMode.value !== mapped) {
        repeatMode.value = mapped
        changed = true
      }
    }
    if (typeof options.shuffleEnabled === 'boolean' && shuffleEnabled.value !== options.shuffleEnabled) {
      shuffleEnabled.value = options.shuffleEnabled
      if (shuffleEnabled.value) {
        rebuildShuffleBag()
        shuffleHistory = []
        shuffleFuture = []
      } else {
        shuffleBag = []
        shuffleHistory = []
        shuffleFuture = []
      }
      changed = true
    }
    if (changed) savePlayerState()
  }

  async function setVolume(vol: number) {
    volume.value = Math.max(0, Math.min(1, vol))
    settings.volume = volume.value
    try { await invoke('set_volume', { level: volume.value }) } catch {}
    savePlayerState()
  }

  // 播放速度
  const playbackSpeed = ref(settings.playbackSpeed)
  async function setSpeed(spd: number) {
    const next = Math.max(0.25, Math.min(3, spd))
    const wasPlaying = _interpIsPlaying
    // 先锚定当前渲染位置，立刻切换插值速度，歌词/进度同步跟手
    const nowMs = currentRenderedPosition()
    playbackSpeed.value = next
    settings.playbackSpeed = next
    _interpSpeed = next
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
    try { await invoke('set_speed', { speed: next }) } catch {}
  }

  // 音效参数（响度增益 + 均衡器）
  const loudnessGainMb = ref(settings.loudnessGainMb)
  const equalizerEnabled = ref(settings.equalizerEnabled)
  const equalizerPresetId = ref(settings.equalizerPresetId)
  const equalizerBands = ref([...settings.equalizerBands]) // 5 bands, mB values

  /** 是否有任何非默认音效 */
  const hasActiveEffects = computed(() =>
    playbackSpeed.value !== 1.0
    || loudnessGainMb.value !== 0
    || equalizerEnabled.value
  )

  async function setLoudnessGain(mb: number) {
    loudnessGainMb.value = Math.round(Math.max(0, Math.min(1500, mb)))
    settings.loudnessGainMb = loudnessGainMb.value
    try { await invoke('set_loudness_gain', { gainMb: loudnessGainMb.value }) } catch {}
  }

  async function setEqualizer(enabled: boolean, bands: number[]) {
    equalizerEnabled.value = enabled
    equalizerBands.value = bands.map(v => Math.round(Math.max(-1500, Math.min(1500, v))))
    settings.equalizerEnabled = enabled
    settings.equalizerBands = [...equalizerBands.value]
    try { await invoke('set_equalizer', { enabled, bandLevelsMb: equalizerBands.value }) } catch {}
  }

  async function setEqualizerPreset(presetId: string) {
    equalizerPresetId.value = presetId
    settings.equalizerPresetId = presetId
    const bands = EQ_PRESETS[presetId] || [0, 0, 0, 0, 0]
    await setEqualizer(presetId !== 'flat', [...bands])
  }

  async function resetAudioEffects() {
    loudnessGainMb.value = 0
    equalizerEnabled.value = false
    equalizerPresetId.value = 'flat'
    equalizerBands.value = [0, 0, 0, 0, 0]
    playbackSpeed.value = 1.0
    settings.loudnessGainMb = 0
    settings.equalizerEnabled = false
    settings.equalizerPresetId = 'flat'
    settings.equalizerBands = [0, 0, 0, 0, 0]
    settings.playbackSpeed = 1.0
    _interpSpeed = 1.0
    try {
      await invoke('reset_audio_effects')
      await invoke('set_speed', { speed: 1.0 })
    } catch {}
  }

  async function applyPersistedSettings() {
    volume.value = Math.max(0, Math.min(1, settings.volume))
    playbackSpeed.value = Math.max(0.25, Math.min(3, settings.playbackSpeed))
    loudnessGainMb.value = Math.round(Math.max(0, Math.min(1500, settings.loudnessGainMb)))
    equalizerEnabled.value = settings.equalizerEnabled
    equalizerPresetId.value = settings.equalizerPresetId
    equalizerBands.value = settings.equalizerBands.map(value => Math.round(Math.max(-1500, Math.min(1500, value))))
    _interpSpeed = playbackSpeed.value

    await Promise.allSettled([
      invoke('set_volume', { level: volume.value }),
      invoke('set_speed', { speed: playbackSpeed.value }),
      invoke('set_loudness_gain', { gainMb: loudnessGainMb.value }),
      invoke('set_equalizer', { enabled: equalizerEnabled.value, bandLevelsMb: equalizerBands.value }),
    ])
  }

  // 批量替换队列，并且只发起一次目标曲目的播放请求
  function playAll(
    tracks: TrackInfo[],
    requestedTrackId?: string,
    requestedPlaylistKey?: string,
  ) {
    const startIndex = resolvePlaybackQueueStartIndex(
      tracks,
      requestedTrackId,
      requestedPlaylistKey,
    )
    if (startIndex < 0) {
      tracePlaybackUi(
        'queue_start_rejected',
        undefined,
        `tracks=${tracks.length}, requestedId=${requestedTrackId || '-'}, requestedKey=${requestedPlaylistKey || '-'}`,
      )
      return
    }
    tracePlaybackUi(
      'queue_start_resolved',
      tracks[startIndex],
      `index=${startIndex}, tracks=${tracks.length}, requestedId=${requestedTrackId || '-'}`,
    )
    queue.value = [...tracks]
    queueIndex.value = startIndex
    shuffleBag = []
    shuffleHistory = []
    shuffleFuture = []
    void play(queue.value[startIndex])
  }

  // 洗牌后替换队列并播放
  function shufflePlay(tracks: TrackInfo[]) {
    if (tracks.length === 0) return
    const shuffled = [...tracks]
    for (let i = shuffled.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]]
    }
    queue.value = shuffled
    queueIndex.value = 0
    shuffleBag = []
    shuffleHistory = []
    shuffleFuture = []
    play(shuffled[0])
  }

  // 插入到当前曲目之后
  function addToQueueNext(track: TrackInfo) {
    const existing = queue.value.findIndex(t => t.id === track.id)
    if (existing !== -1) {
      if (shuffleEnabled.value) shiftShuffleIndicesForRemove(existing)
      queue.value.splice(existing, 1)
      if (existing < queueIndex.value) queueIndex.value--
    }
    const idx = queueIndex.value + 1
    queue.value.splice(idx, 0, track)
    if (shuffleEnabled.value) shiftShuffleIndicesForInsert(idx)
    savePlayerState()
  }

  // 追加到队列末尾
  function addToQueueEnd(track: TrackInfo) {
    if (!queue.value.find(t => t.id === track.id)) {
      queue.value.push(track)
      if (shuffleEnabled.value) {
        shuffleBag.push(queue.value.length - 1)
      }
    }
    savePlayerState()
  }

  // 从队列移除指定索引
  function removeFromQueue(index: number) {
    if (index < 0 || index >= queue.value.length) return
    const wasCurrentTrack = index === queueIndex.value

    if (shuffleEnabled.value) shiftShuffleIndicesForRemove(index)

    queue.value.splice(index, 1)
    if (queue.value.length === 0) {
      ++playbackRequestToken
      queueIndex.value = -1
      currentTrack.value = null
      currentResolvedStreamUrl.value = null
      shuffleBag = []
      shuffleHistory = []
      shuffleFuture = []
      pause()
      savePlayerState()
      return
    }
    if (index < queueIndex.value) {
      queueIndex.value--
    } else if (wasCurrentTrack) {
      // 被删除的是当前曲目，索引保持（指向下一首），但不超界
      queueIndex.value = Math.min(queueIndex.value, queue.value.length - 1)
      // 同步 currentTrack 到新索引指向的曲目
      ++playbackRequestToken
      currentTrack.value = queue.value[queueIndex.value]
      currentResolvedStreamUrl.value = null
    }
    savePlayerState()
  }

  // 清空队列
  function clearQueue() {
    ++playbackRequestToken
    queue.value = []
    queueIndex.value = -1
    currentResolvedStreamUrl.value = null
    shuffleBag = []
    shuffleHistory = []
    shuffleFuture = []
    savePlayerState()
  }

  // 编辑当前曲目信息
  let originalTrackInfo: TrackInfo | null = null

  function updateCurrentTrackInfo(patch: Partial<TrackInfo>) {
    if (!currentTrack.value) return
    if (!originalTrackInfo) {
      originalTrackInfo = { ...currentTrack.value }
    }
    currentTrack.value = { ...currentTrack.value, ...patch }
  }

  /** 更新当前曲目 syncPayload (合并字段), 并同步到队列中同 id 曲目 */
  function patchCurrentTrackSyncPayload(
    nextPayload: Record<string, unknown> | null | undefined,
  ) {
    if (!currentTrack.value) return
    const payload = nextPayload ? { ...nextPayload } : undefined
    updateCurrentTrackInfo({ syncPayload: payload })
    const trackId = currentTrack.value.id
    if (!trackId) return
    queue.value = queue.value.map((item) =>
      item.id === trackId ? { ...item, syncPayload: payload } : item,
    )
  }

  function restoreOriginalTrackInfo() {
    if (originalTrackInfo && currentTrack.value) {
      currentTrack.value = { ...originalTrackInfo }
      originalTrackInfo = null
    }
  }

  function hasOriginalTrackInfo() {
    return originalTrackInfo !== null
  }

  function handleDownloadedFileRemoved(trackId: string, filePath?: string) {
    if (!currentTrack.value || currentTrack.value.id !== trackId) return
    if (filePath && _currentLoadedFromDownloadPath && _currentLoadedFromDownloadPath !== filePath) return

    _currentLoadedFromDownloadPath = null
    isPlayingFromDownload.value = false
    lastUrlResolveTime = 0
    if (!isPlaying.value) {
      _needsReload = true
    }
  }

  // 用指定音质重新播放当前曲目（保持进度）
  async function replayWithQuality() {
    const track = currentTrack.value
    if (!track) return
    const pos = positionMs.value
    const wasPlaying = isPlaying.value
    const sourceSettings = playbackSourceSettings()
    playbackPrefetchManager.clearForTrack(track, sourceSettings)
    playbackUrlResolver.invalidate(track, sourceSettings)
    lastUrlResolveTime = 0
    playError.value = null
    await play(track, 'local', pos, true)
    if (playError.value) {
      throw new Error(playError.value)
    }
    if (!wasPlaying && currentTrack.value?.id === track.id) {
      await pause('local')
    }
  }

  // 初始化：恢复持久化状态
  loadPlayerState()
  void applyPersistedSettings()

  return {
    isPlaying, currentTrack, currentResolvedStreamUrl, positionMs, durationMs, queue, queueIndex,
    repeatMode, shuffleEnabled, volume, lyrics, playError, isLoadingAudio,
    hasPlaybackSession,
    audioLevel, beatImpulse, audioInfo, isPlayingFromDownload, isPlayingFromCache,
    lastCommandSource, lastSeekCommand, isRemoteSyncGuardActive,
    playbackSpeed, sleepTimerMode, sleepRemainingSeconds,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands, hasActiveEffects,
    progress, interpolatedPositionMs, interpolatedProgress,
    currentTimeFormatted, durationFormatted,
    play, togglePlayPause, pause, resume, seekTo, next, previous,
    resolveCurrentStreamUrl,
    flushPlayerState,
    toggleRepeatMode, toggleShuffle, cyclePlayMode, applyListenTogetherPlaybackMode, playMode, setVolume, setSpeed,
    setLoudnessGain, setEqualizer, setEqualizerPreset, resetAudioEffects,
    applyPersistedSettings,
    startSleepTimer, startSleepTimerEndOfTrack, startSleepTimerEndOfQueue, cancelSleepTimer,
    playAll, shufflePlay, addToQueueNext, addToQueueEnd, removeFromQueue, clearQueue,
    prefetchPlaybackTracks,
    updateCurrentTrackInfo, patchCurrentTrackSyncPayload, restoreOriginalTrackInfo, hasOriginalTrackInfo,
    handleDownloadedFileRemoved, replayWithQuality,
  }
})

async function playDownloadedFile(
  path: string,
  durationHintMs: number,
  useCrossfade: boolean,
  fadeOutMs: number,
  fadeInMs: number,
  requestGeneration: number,
  startPositionMs = 0,
): Promise<number> {
  if (useCrossfade) {
    return invoke<number>('crossfade_file', {
      path,
      durationHintMs,
      fadeOutMs,
      fadeInMs,
      requestGeneration,
    })
  }
  return invoke<number>('play_file', {
    path,
    durationHintMs,
    startPositionMs: Math.max(0, Math.round(startPositionMs)),
    requestGeneration,
  })
}

async function playRemoteUrl(
  url: string,
  durationHintMs: number,
  useCrossfade: boolean,
  fadeOutMs: number,
  fadeInMs: number,
  requestGeneration: number,
  startPositionMs = 0,
  cacheKey?: string,
  expectedContentLength?: number,
): Promise<number> {
  const safeStartMs = Math.max(0, Math.round(startPositionMs))
  const cacheLimitBytes = playbackCacheLimitBytes()
  if (useCrossfade) {
    try {
      return await invoke<number>('crossfade_url_streaming', {
        url,
        durationHintMs,
        fadeOutMs,
        fadeInMs,
        cacheKey,
        cacheLimitBytes,
        expectedContentLength,
        requestGeneration,
      })
    } catch (streamError) {
      if (requestGeneration !== playbackRequestToken) throw streamError
      log.warn(
        'streaming playback failed, falling back to temp-file playback:',
        summarizeLogError(streamError),
      )
      return invoke<number>('crossfade_url_fast', {
        url,
        durationHintMs,
        fadeOutMs,
        fadeInMs,
        cacheKey,
        cacheLimitBytes,
        expectedContentLength,
        requestGeneration,
      })
    }
  }

  try {
    return await invoke<number>('play_url_streaming', {
      url,
      durationHintMs,
      startPositionMs: safeStartMs,
      cacheKey,
      cacheLimitBytes,
      expectedContentLength,
      requestGeneration,
    })
  } catch (streamError) {
    if (requestGeneration !== playbackRequestToken) throw streamError
    log.warn(
      'streaming playback failed, falling back to temp-file playback:',
      summarizeLogError(streamError),
    )
    return invoke<number>('play_url_fast', {
      url,
      durationHintMs,
      startPositionMs: safeStartMs,
      cacheKey,
      cacheLimitBytes,
      expectedContentLength,
      requestGeneration,
    })
  }
}

interface CachedRemoteAudioResult {
  durationMs: number
  source: 'netease' | 'qq' | 'bilibili' | 'youtube'
  qualityKey: string
}

async function playCachedRemoteAudioCandidates(
  candidates: PlaybackCacheReadCandidate[],
  durationHintMs: number,
  useCrossfade: boolean,
  fadeOutMs: number,
  fadeInMs: number,
  requestGeneration: number,
  startPositionMs = 0,
): Promise<CachedRemoteAudioResult | null> {
  if (candidates.length === 0) return null
  return invoke<CachedRemoteAudioResult | null>('play_cached_audio_candidates', {
    request: {
      candidates,
      durationHintMs,
      startPositionMs: Math.max(0, Math.round(startPositionMs)),
      useCrossfade,
      fadeOutMs,
      fadeInMs,
      cacheLimitBytes: playbackCacheLimitBytes(),
      requestGeneration,
    },
  })
}

function playbackCacheLimitBytes(): number {
  const settings = useSettingsStore()
  const cacheSizeMb = Math.min(
    MAX_MEDIA_CACHE_SIZE_MB,
    Math.max(MIN_MEDIA_CACHE_SIZE_MB, settings.maxCacheSize),
  )
  return Math.round(cacheSizeMb * 1024 * 1024)
}

function formatTime(ms: number): string {
  const totalSeconds = Math.floor(Math.max(0, ms) / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}
