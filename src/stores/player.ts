import { defineStore } from 'pinia'
import { ref, computed, watch } from 'vue'
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
import { useListenTogetherStore } from './listenTogether'
import i18n from '@/i18n'
import {
  canonicalizePlaybackTrack,
  getPlaybackSourceKind,
  isRemotePlaybackTrack,
  normalizeBitrateKbps,
  playbackCacheReadCandidates,
  playbackCacheWriteOptions,
  selectPlaybackCandidate,
  playbackPrefetchCacheId,
  playbackUrlResolver,
  resolvePlaybackResult,
  isDirectStreamUrl,
  setDecodableCodecs,
  type PlaybackAudioSource,
  type PlaybackCacheReadCandidate,
  type PlaybackSourceSettings,
  type PlaybackResolution,
  type ResolvedPlaybackSource,
} from '@/modules/playback/playbackSource'
import {
  PlaybackFailure,
  playbackFailureMessageKey,
  shouldThrottlePlaybackRefresh,
} from '@/modules/playback/playbackFailure'
import { genericUrlPrefetchTtlMs, playbackPrefetchManager } from '@/modules/playback/playbackPrefetch'
import { recallPlayedQuality, rememberPlayedQuality } from '@/modules/playback/playedQualityMemory'
import {
  shouldRefreshUrlBeforeResume,
  shouldRefreshUrlBeforeSeek,
} from '@/modules/playback/youtubeSeekRefreshPolicy'
import {
  PlaybackStartupWatchdog,
  resolvePlaybackFailureAdvanceAction,
} from '@/modules/playback/playbackPolicy'
import { resolvePlaybackQueueStartIndex } from '@/modules/playback/playbackQueue'
import {
  biliShareQualityOrder,
  decorateLtStreamUrl,
  hasLtStreamQuality,
  LT_SHARE_LIMITS,
  ltChannelForSource,
  neteaseShareQualityGroups,
} from '@/stores/listenTogether/streamQuality'
import {
  LONG_FORM_MIN_DURATION_MS,
  longFormPositionForPersistence,
  resolveLongFormResumePosition,
} from '@/modules/playback/longFormProgress'
import {
  isPlaybackSeekCompletionCurrent,
  resolvePlaybackLoadStart,
  shouldDeferPlaybackSeek,
  initialPlaybackPrefetchWindow,
  shouldResolvePlaybackSourceInParallel,
  type DeferredPlaybackSeek,
} from '@/modules/playback/playbackRequest'
import { createLogger } from '@/utils/logger'
import { formatTimeMs as formatTime } from '@/utils/timeFormat'
import { getTrackCoverUrl } from '@/utils/trackCover'
import {
  buildPersistedPlaybackQueue,
  restorePersistedPlaybackQueue,
} from '@/modules/playback/playerState'
import { summarizeLogError } from '@/utils/logSanitizer'
import {
  finishLegacyPlayerStateCleanup,
  LEGACY_PLAYER_STATE_KEY,
  persistUserData,
  preloadedUserData,
} from '@/modules/persistence/userData'
import { loadLocalAudioInfo } from '@/modules/playback/localAudioInfo'
import { loadPlaybackAudioInfo, type PlaybackAudioProperties } from '@/modules/playback/playbackAudioInfo'

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

export { displayAlbum } from '@/modules/library/albumDisplay'

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
  roman?: string
}

export type RepeatMode = 'off' | 'all' | 'one'

/** 后端 get_decoder_capabilities 的结果 */
export interface DecoderCapabilities {
  ffmpeg: { directory: string; avutil: string; avcodec: string; avformat: string } | null
  ffmpegError?: string | null
  codecs: string[]
}
/** local_safety：睡眠定时、失败跳过、自动推进等内部操作，不受一起听的成员控制限制（对齐 Android LOCAL_SAFETY） */
export type PlaybackCommandSource = 'local' | 'local_safety' | 'remote_sync'

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
  dolby: '多声道（E-AC-3）',
}

// 取流失败按原因给出与 Android 一致的提示；其他错误（解码、设备）保留原始信息
function playbackFailureText(reason: PlaybackFailure['reason'] | null, message: string): string {
  const t = (i18n.global as any).t
  if (!reason) return t('player.play_failed', { msg: message })
  if (reason === 'url_error') return t('player.playback_url_error', { msg: summarizeLogError(message) })
  return t(playbackFailureMessageKey(reason))
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
// 回退超过该值视为时钟跳变（睡眠唤醒），强制重锚
const CLOCK_JUMP_BACKWARD_MS = 30_000
const PAUSE_EVENT_GUARD_MS = 2500
const PAUSE_BACKWARD_TOLERANCE_MS = 250

// 连续失败熔断
let consecutivePlayFailures = 0
const MAX_CONSECUTIVE_FAILURES = 10
// 需要登录的曲目直接跳过、不计入失败（对齐 Android）；整个队列都需要登录时停下
let consecutiveLoginSkips = 0
let _isAutoSkipping = false
let _stallRecovering = false
let _lastStallRecovery: { key: string; at: number } | null = null

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
const playbackStartupWatchdog = new PlaybackStartupWatchdog()
let startupRecoveryAttempts = 0
const MAX_STARTUP_RECOVERY_ATTEMPTS = 2
const STARTUP_WATCHDOG_REMOTE_MS = 8_000
const STARTUP_WATCHDOG_YOUTUBE_MS = 12_000

// 状态持久化：正常运行写用户数据库，浏览器开发模式退回 localStorage
const PLAYER_STATE_KEY = LEGACY_PLAYER_STATE_KEY
let _persistDebounceTimer: ReturnType<typeof setTimeout> | null = null
// 上次成功落库的队列序列化结果；队列不变时只更新播放状态行
let _lastPersistedQueueJson: string | null = null
let _progressPersistTime = 0
const PERSIST_DEBOUNCE_MS = 250
const PROGRESS_PERSIST_INTERVAL_MS = 15000
// 恢复后需重新加载标记
let _needsReload = false
let _remoteSyncGuardUntil = 0
let _currentLoadedFromDownloadPath: string | null = null

function audioFilePathKey(path: string | undefined | null) {
  const key = (path || '').replace(/\\/g, '/').trim()
  return /^[a-z]:\//i.test(key) || key.startsWith('//') ? key.toLowerCase() : key
}

export const usePlayerStore = defineStore('player', () => {
  const audioFileMutations = new Map<string, {
    trackId: string | null
    requestToken: number | null
    shouldResume: boolean
    releasesCurrentPlayback: boolean
    positionMs: number
    finished: Promise<void>
  }>()

  function currentAudioFileMutation() {
    return [...audioFileMutations.values()].find(mutation =>
      mutation.trackId === currentTrack.value?.id && mutation.requestToken === playbackRequestToken)
  }
  const settings = useSettingsStore()
  const isPlaying = ref(false)
  const currentTrack = ref<TrackInfo | null>(null)
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
  // 加载持续超过 1s 才置真, 供播放按钮转圈; 短加载不闪 spinner
  const isLoadingAudioSlow = ref(false)
  let loadingSpinnerTimer: number | null = null
  watch(isLoadingAudio, (loading) => {
    if (loading) {
      if (loadingSpinnerTimer === null) {
        loadingSpinnerTimer = window.setTimeout(() => {
          loadingSpinnerTimer = null
          isLoadingAudioSlow.value = isLoadingAudio.value
        }, 1000)
      }
    } else {
      if (loadingSpinnerTimer !== null) {
        clearTimeout(loadingSpinnerTimer)
        loadingSpinnerTimer = null
      }
      isLoadingAudioSlow.value = false
    }
  })
  // 是否存在可见播放上下文；恢复态由 _needsReload 标记后端尚未装载
  const hasPlaybackSession = ref(false)

  // 当前音频质量信息
  const audioInfo = ref<AudioInfo | null>(null)
  let decodedAudioInfo: { requestGeneration: number; properties: PlaybackAudioProperties } | null = null
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
        void pause('local_safety')
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
  // 后端只在播放中推电平；不清零的话暂停后背景律动会定格在最后一拍
  watch(isPlaying, (playing) => {
    if (playing) return
    audioLevel.value = 0
    beatImpulse.value = 0
  })

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
  let _speedInvokeTimer: ReturnType<typeof setTimeout> | null = null

  function markCommandSource(source: PlaybackCommandSource) {
    lastCommandSource.value = source
    if (source === 'remote_sync') {
      _remoteSyncGuardUntil = Date.now() + 3000
    }
  }

  /**
   * 一起听里听众没有控制权（房主离线或关闭了成员控制）时，用户操作在执行前就拦下并提示，
   * 不再先改本地、再被服务端拒绝而和房间分叉（对齐 Android shouldBlockLocalRoomControl）
   */
  function blockedByListenTogether(commandSource: PlaybackCommandSource): boolean {
    if (commandSource !== 'local') return false
    const restriction = useListenTogetherStore().localControlRestriction
    if (!restriction) return false
    useToastStore().error((i18n.global as any).t(restriction === 'controller_offline'
      ? 'listen_together.control_blocked_controller_offline'
      : 'listen_together.control_blocked_member_control'))
    return true
  }

  /** 播放器替用户做的后续动作（恢复、失败跳过）：跟随房间的同步仍按同步处理，其余都不算用户操作 */
  function internalFollowUpSource(commandSource: PlaybackCommandSource): PlaybackCommandSource {
    return commandSource === 'remote_sync' ? 'remote_sync' : 'local_safety'
  }

  /** 一起听期间不能切到本地文件：其它成员没法播放它（对齐 Android shouldBlockLocalSongSwitch） */
  function blocksLocalSongInRoom(track: TrackInfo, commandSource: PlaybackCommandSource): boolean {
    if (commandSource !== 'local' || !useListenTogetherStore().roomId) return false
    if (track.source !== 'local' && !track.id.startsWith('local:')) return false
    // 重新加载当前这首（例如继续播放时需要重载）不是切歌
    if (track.id === currentTrack.value?.id) return false
    useToastStore().error((i18n.global as any).t('listen_together.local_playback_blocked'))
    return true
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

  async function persistPlayerStateToDatabase() {
    const state = persistedPlayerState()
    const queueJson = JSON.stringify(state.queue)
    const queueUnchanged = queueJson === _lastPersistedQueueJson
    try {
      await persistUserData('save_playback_state', {
        state: queueUnchanged ? { ...state, queue: null } : state,
      })
      _lastPersistedQueueJson = queueJson
      finishLegacyPlayerStateCleanup()
      uiLog.info('state persisted', {
        queueSize: queue.value.length,
        queueIndex: queueIndex.value,
        trackId: currentTrack.value?.id,
        queueWritten: !queueUnchanged,
      })
    } catch (error) {
      uiLog.error('state persistence failed:', error)
    }
  }

  /** 退出前立即落盘，避免防抖定时器尚未执行 */
  async function flushPlayerState(): Promise<void> {
    if (_persistDebounceTimer) {
      clearTimeout(_persistDebounceTimer)
      _persistDebounceTimer = null
    }
    if (preloadedUserData()) {
      await persistPlayerStateToDatabase()
      return
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

  /** 保存播放器状态（250ms debounce） */
  function savePlayerState() {
    if (_persistDebounceTimer) clearTimeout(_persistDebounceTimer)
    _persistDebounceTimer = setTimeout(() => {
      void flushPlayerState()
    }, PERSIST_DEBOUNCE_MS)
  }

  /** 启动前预取的数据库快照；没有后端时读取 localStorage */
  function readPersistedPlayerState(): any | null {
    const preloaded = preloadedUserData()
    if (preloaded) return preloaded.playbackState
    const raw = localStorage.getItem(PLAYER_STATE_KEY)
    return raw ? JSON.parse(raw) : null
  }

  /** 恢复播放器状态（store 初始化时调用，不自动播放） */
  function loadPlayerState() {
    try {
      hasPlaybackSession.value = false
      const state = readPersistedPlayerState()
      if (!state) return
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
      persistCurrentLongFormProgress()
    }
  }

  /** 长音频进度写进播放历史，随同步在其它设备续播（对齐 Android PlaybackProgressOwner） */
  function persistLongFormProgress(track: TrackInfo | null, trackPositionMs: number) {
    if (!track) return
    const settings = useSettingsStore()
    const trackDurationMs = Math.max(track.durationMs || 0, currentTrack.value === track ? durationMs.value : 0)
    const remembered = longFormPositionForPersistence(settings.rememberLongFormProgress, trackDurationMs, trackPositionMs)
    if (remembered !== null) useHistoryStore().updateResumePosition(track, remembered)
  }

  function persistCurrentLongFormProgress() {
    persistLongFormProgress(currentTrack.value, currentRenderedPosition())
  }

  function rememberedLongFormStart(track: TrackInfo): number {
    const settings = useSettingsStore()
    if (!settings.rememberLongFormProgress || track.durationMs < LONG_FORM_MIN_DURATION_MS) return 0
    return resolveLongFormResumePosition(true, track.durationMs, 0, useHistoryStore().rememberedPosition(track))
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
      youtubePlaybackSource: settings.youtubePlaybackSource,
      neteaseAutoSourceSwitch: settings.neteaseAutoSourceSwitch,
      neteaseLocalSourceFallback: settings.neteaseLocalSourceFallback,
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
    // 单曲循环不会切到下一首，预取只会浪费请求（对齐 Android）
    if (!queue.value.length || repeatMode.value === 'one') return []

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
    // YouTube 只预取后两首（对齐 Android），更多会在 PoToken 队列里挤占当前请求
    const maxWindow = getPlaybackSourceKindForPrefetch(queue.value[firstIndex]) === 'youtube'
      ? 2
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

  /**
   * 预热后续曲目的解析结果，当前播放需求始终拥有优先级。
   * 预取不带播放代际：切歌不会作废它，按下一首时前台解析直接等它完成（对齐 Android join）。
   */
  function maybePrefetchNext() {
    const tracks = nextPrefetchTracks()
    if (tracks.length === 0) return
    const settings = playbackSourceSettings()
    const ttlMs = genericUrlPrefetchTtlMs(Math.max(durationMs.value, currentTrack.value?.durationMs ?? 0))
    // 已完整缓存的曲目播放时直接离线起播，不必预取地址
    void Promise.all(tracks.map(async track => (
      await hasCompleteCachedAudio(playbackCacheReadCandidates(track, settings)) ? null : track
    ))).then((pending) => {
      const uncached = pending.filter((track): track is TrackInfo => track !== null)
      if (uncached.length === 0) return
      playbackPrefetchManager.prefetchWindow(uncached, settings, playbackUrlResolver, undefined, ttlMs)
    })
  }

  // 鼠标停在哪一行、多久以前：解析回来时它还是最新的意图，才值得预开
  let intentTrackId: string | null = null
  let intentAt = 0
  const INTENT_PREWARM_WINDOW_MS = 10_000

  // 只预开紧接着的下一首和鼠标正停着的那首：首包和到 CDN 的连接提前就位，
  // 播放时不再等冷连接（经代理要 3 秒左右）
  playbackPrefetchManager.onPrefetched = (track, result) => {
    if (result.source === 'local' || result.streamType === 'hls' || !result.url) return
    const isIntent = track.id === intentTrackId && Date.now() - intentAt < INTENT_PREWARM_WINDOW_MS
    if (!isIntent && nextPrefetchTracks()[0]?.id !== track.id) return
    void invoke('prewarm_remote_audio', {
      url: result.url,
      durationHintMs: result.durationMs || track.durationMs || 0,
    }).catch(() => {})
  }

  /** 列表行悬停或聚焦：多半马上要点它，提前解析并预开 */
  function prefetchIntent(track: TrackInfo) {
    if (!isRemotePlaybackTrack(track) || track.id === currentTrack.value?.id) return
    intentTrackId = track.id
    intentAt = Date.now()
    playbackPrefetchManager.prefetchIntent(track, playbackSourceSettings(), playbackUrlResolver)
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
    avoidDirect = false,
  ): Promise<PlaybackResolution> {
    return resolvePlaybackResult(track, playbackSourceSettings(), {
      forceRefresh,
      qualityOverride,
      avoidDirect,
      requestGeneration: playbackRequestToken,
    })
  }

  function takePrefetchedPlaybackUrl(track: TrackInfo): ResolvedPlaybackSource | null {
    return playbackPrefetchManager.take(track, playbackSourceSettings())
  }

  /** 是否已有完整的磁盘缓存（只查标记与长度，不做完整校验） */
  async function hasCompleteCachedAudio(candidates: readonly PlaybackCacheReadCandidate[]): Promise<boolean> {
    if (candidates.length === 0) return false
    try {
      return await invoke<boolean>('has_cached_audio', { cacheKeys: candidates.map(candidate => candidate.cacheKey) })
    } catch {
      return false
    }
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
        const recoverySource = internalFollowUpSource(commandSource)
        if (startupRecoveryAttempts >= MAX_STARTUP_RECOVERY_ATTEMPTS) {
          playbackStartupWatchdog.cancel()
          consecutivePlayFailures += 1
          _isAutoSkipping = true
          void next(true, recoverySource).finally(() => {
            _isAutoSkipping = false
          })
          return
        }
        startupRecoveryAttempts += 1
        const resumePositionMs = currentRenderedPosition()
        void play(track, recoverySource, resumePositionMs, true)
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
    if (!_interpIsPlaying || forceRendered || isClockJump || Math.abs(safePositionMs - renderedMs) > 80) {
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
    _interpSpeed = effectivePlaybackSpeed()
    _interpIsPlaying = true
    clearPauseGuard()
    // 从暂停停表状态恢复播放时重启插值循环
    _startInterpolationLoop()
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
    const speed = _interpIsPlaying ? Math.max(0.25, effectivePlaybackSpeed() || _interpSpeed || 1) : 0
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

  /** 按需启动 rAF 插值循环：暂停且无待确认 seek 时自动停表，状态恢复时重启 */
  function _startInterpolationLoop() {
    if (_interpLoopStarted) return
    _interpLoopStarted = true

    function tick() {
      // 空闲（未播放且无 pendingSeek）时渲染一次最终位置后停表，避免常驻逐帧唤醒
      if (!_interpIsPlaying && !pendingSeek) {
        interpolatedPositionMs.value = Math.round(_interpRenderedMs)
        _interpLoopStarted = false
        return
      }
      requestAnimationFrame(tick)

      // seek 等待后端确认期间冻结进度条，避免「seek 完成仍空转」
      if (pendingSeek) {
        interpolatedPositionMs.value = Math.round(pendingSeek.targetMs)
        _interpRenderedMs = pendingSeek.targetMs
        _interpAnchorMs = pendingSeek.targetMs
        _interpAnchorTime = performance.now()
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
    // 曲中断流: 后端发 stalled 而非 track-ended, 从当前位置重试当前曲, 超限再跳（PB-02）
    listen<{ positionMs: number; requestGeneration: number }>('player:playback-stalled', (e) => {
      if (e.payload.requestGeneration !== playbackRequestToken) return
      const track = currentTrack.value
      if (!track || _stallRecovering) return
      // 同一首在冷却期内再次断流：不再原地重试（否则每次重启都会清零失败计数而无限循环），
      // 按失败处理并跳到下一首（对齐 Android URL_REFRESH_COOLDOWN_MS）
      const now = Date.now()
      if (shouldThrottlePlaybackRefresh(_lastStallRecovery, track.id, now)) {
        _lastStallRecovery = null
        consecutivePlayFailures++
        useToastStore().error((i18n.global as any).t('player.playback_network_error'))
        if (consecutivePlayFailures >= MAX_CONSECUTIVE_FAILURES) {
          void pause('local_safety')
          useToastStore().error((i18n.global as any).t('player.too_many_failures'))
        } else if (!_isAutoSkipping) {
          _isAutoSkipping = true
          void next(true, 'local_safety').finally(() => { _isAutoSkipping = false })
        }
        return
      }
      _lastStallRecovery = { key: track.id, at: now }
      _stallRecovering = true
      const resumeAt = Math.max(0, Math.round(e.payload.positionMs))
      void play(track, 'local_safety', resumeAt, true)
        .catch(() => {
          if (!_isAutoSkipping) {
            _isAutoSkipping = true
            void next(true, 'local_safety').finally(() => { _isAutoSkipping = false })
          }
        })
        .finally(() => { _stallRecovering = false })
    })

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

  function readLocalAudioInfo(path: string, requestGeneration: number) {
    void loadLocalAudioInfo(
      path,
      () => requestGeneration === playbackRequestToken && hasPlaybackSession.value,
      info => {
        // 文件属性探测可能晚于解码器信息返回，保留当前会话的真实参数
        const decoded = decodedAudioInfo?.requestGeneration === requestGeneration
          ? decodedAudioInfo.properties
          : undefined
        if (decoded && typeof info.bitrate === 'number' && Number.isFinite(info.bitrate) && info.bitrate > 0) {
          delete decoded.bitrate
        }
        audioInfo.value = { ...info, ...decoded }
      },
    ).catch(error => log.warn('local audio properties unavailable:', error))
  }

  function readPlaybackAudioInfo(requestGeneration: number) {
    void loadPlaybackAudioInfo(
      requestGeneration,
      () => requestGeneration === playbackRequestToken
        && requestGeneration === loadedPlaybackRequestToken
        && hasPlaybackSession.value && !isLoadingAudio.value && !_needsReload,
      info => {
        const properties = { ...info }
        const existingBitrate = audioInfo.value?.bitrate
        if (typeof existingBitrate === 'number' && Number.isFinite(existingBitrate) && existingBitrate > 0) {
          delete properties.bitrate
        }
        decodedAudioInfo = { requestGeneration, properties }
        audioInfo.value = { ...audioInfo.value, ...properties }
      },
    ).catch(error => log.warn('decoded audio properties unavailable:', error))
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
        await play(track, commandSource, fileMutation.positionMs, forceResolve, allowRememberedPosition)
      }
      return
    }
    if (startPositionMs <= 0 && allowRememberedPosition && !forceResolve && commandSource === 'local') {
      startPositionMs = rememberedLongFormStart(track)
    }
    initEvents()
    markCommandSource(commandSource)
    const token = ++playbackRequestToken
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
    const claimStarted = performance.now()
    void invoke<void>('begin_playback_request', {
      requestGeneration: token,
      trackId: track.id,
      source: getPlaybackSourceKind(track) || track.source || 'local',
      hasCover: !!getTrackCoverUrl(track),
      hasAudioUrl: !!track.audioUrl,
      hasSyncPayload: !!track.syncPayload,
      silencePrevious: !keepsPreviousAudible,
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
    if (previousTrack && (previousTrack.id !== track.id || previousTrack.album !== track.album)) {
      persistLongFormProgress(previousTrack, currentRenderedPosition())
    }
    // 直链只在本次请求确认有效时共享，切歌或强制刷新先清掉旧 URL
    currentStreamUrl.value = !forceResolve && isDirectStreamUrl(track.audioUrl)
      ? track.audioUrl.trim()
      : null
    currentResolvedStreamUrls = currentStreamUrl.value ? [currentStreamUrl.value] : []
    rememberStreamQualities(null)
    const wasPlayingBeforeSwitch = isPlaying.value
    const hadPlaybackSessionBeforeRequest = hasPlaybackSession.value
    const isSwitchingTrack = !!previousTrack && previousTrack.id !== track.id
    const fadeInDurationMs = Math.max(0, Math.round(settings.fadeInDuration))
    const fadeOutDurationMs = Math.max(0, Math.round(settings.fadeOutDuration))
    const overlapFadeInDurationMs = settings.crossfadeNext
      ? Math.max(0, Math.round(settings.crossfadeInDuration))
      : fadeInDurationMs
    const overlapFadeOutDurationMs = settings.crossfadeNext
      ? Math.max(0, Math.round(settings.crossfadeOutDuration))
      : fadeOutDurationMs
    const useOverlapCrossfade = wasPlayingBeforeSwitch && isSwitchingTrack
      && settings.crossfadeNext
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
      // 推送元数据到后端镜像, 供系统媒体会话(SMTC/MPRIS)展示标题/歌手/封面/时长（PB-01）
      void invoke('update_media_metadata', {
        id: track.id,
        title: track.title,
        artist: track.artist,
        album: track.album || '',
        coverUrl: track.coverUrl || null,
        durationMs: track.durationMs || 0,
      }).catch(() => {})
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
    // overlap-crossfade 切歌时旧会话仍在出声直到新会话淡入, 立即置 false 会让播放按钮
    // 闪示暂停态（PB-10）。此场景保持播放态, 仅非交叉淡入切歌才落 false
    if (!(useOverlapCrossfade && wasPlayingBeforeSwitch)) {
      isPlaying.value = false
      _interpIsPlaying = false
    }

    try {
      if (token !== playbackRequestToken) return

      let dur = 0
      let playedFromDownloadedFile = false
      let playedFromPlaybackCache = false
      let playingPreviewClip = false
      playError.value = null
      audioInfo.value = null
      decodedAudioInfo = null
      isPlayingFromDownload.value = false
      isPlayingFromCache.value = false

      const downloaded = useDownloadStore().getDownloadedTrack(track.id)
      if (downloaded?.filePath && !audioFileMutations.has(audioFilePathKey(downloaded.filePath)) && isRemotePlaybackTrack(track)) {
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
        if (downloaded?.filePath) readLocalAudioInfo(downloaded.filePath, token)
        lastUrlResolveTime = 0
      } else if (isRemotePlaybackTrack(track)) {
        _currentLoadedFromDownloadPath = null
        isPlayingFromDownload.value = false
        // 进入在线解析前默认非缓存; 命中缓存时会再置 true
        isPlayingFromCache.value = false
        // 强制重新解析（切换音质、断流恢复）不读缓存、不用预取结果，否则会重播刚要替换掉的那份
        let prefetchedResolution = forceResolve ? null : takePrefetchedPlaybackUrl(track)
        const qualityMemoryKey = playbackCacheReadCandidates(track, playbackSourceSettings())[0]?.cacheKey ?? ''
        const cacheCandidates = forceResolve ? [] : playbackCacheReadCandidates(track, playbackSourceSettings())
        const hasCompleteCache = await hasCompleteCachedAudio(cacheCandidates)
        if (token !== playbackRequestToken) return
        const resolveInParallel = shouldResolvePlaybackSourceInParallel(
          hadPlaybackSessionBeforeRequest,
          !!prefetchedResolution,
          hasCompleteCache,
        )
        const coldResolution = resolveInParallel
          ? resolvePlaybackUrl(track, forceResolve).catch((error): PlaybackResolution => ({
              type: 'failure',
              message: error instanceof Error ? error.message : String(error),
              retryable: true,
            }))
          : null
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
            // 缓存键是首选音质，实际写入的可能是降级流：换成记录下来的实际音质
            void recallPlayedQuality(qualityMemoryKey).then((played) => {
              if (!played?.qualityKey || token !== playbackRequestToken || !audioInfo.value) return
              audioInfo.value = {
                ...audioInfo.value,
                qualityKey: played.qualityKey,
                qualityLabel: qualityLabelFromKey(cached.source, played.qualityKey),
                codec: audioInfo.value.codec ?? played.codecLabel,
                bitrate: audioInfo.value.bitrate ?? played.bitrateKbps,
                mimeType: audioInfo.value.mimeType ?? played.mimeType,
              }
            })
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
                throw new PlaybackFailure('requires_login', resolution.message || 'Playback requires login')
              }
              if (resolution.type === 'waiting_for_authoritative_stream') {
                throw new Error('Waiting for authoritative playback stream')
              }
              throw new PlaybackFailure(resolution.reason ?? 'url_error', resolution.message)
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
            const candidates = [resolved.url, ...resolved.candidateUrls]
              .filter((url, index, values) => values.indexOf(url) === index)
            for (const [candidateIndex, candidateUrl] of candidates.entries()) {
              if (token !== playbackRequestToken) return 0
              const candidateStarted = performance.now()
              try {
                const cacheWrite = playbackCacheWriteOptions(resolved, candidateIndex)
                const selected = selectPlaybackCandidate(resolved, candidateIndex)
                  const startPlan = currentLoadStartPlan()
                  tracePlaybackUi(
                    'backend_stream_start',
                    track,
                    `candidate=${candidateIndex}, startMs=${startPlan.positionMs}, crossfade=${startPlan.useCrossfade}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
                    token,
                  )
                  const duration = selected.source === 'local' ? await playDownloadedFile(
                    candidateUrl,
                    selected.durationMs || track.durationMs,
                    startPlan.useCrossfade,
                    transitionFadeOutMs,
                    transitionFadeInMs,
                    token,
                    startPlan.positionMs,
                  ) : await playRemoteUrl(
                  candidateUrl,
                  selected.durationMs || track.durationMs,
                  startPlan.useCrossfade,
                  transitionFadeOutMs,
                  transitionFadeInMs,
                  token,
                  startPlan.positionMs,
                  cacheWrite.cacheKey,
                  cacheWrite.expectedContentLength,
                  cacheWrite.expectedContentMd5,
                  selected.streamType,
                  )
                  if (token === playbackRequestToken) {
                    currentStreamUrl.value = resolved.source === 'local' ? null : candidateUrl
                    currentResolvedStreamUrls = resolved.source === 'local' ? [] : candidates.slice(candidateIndex)
                    rememberStreamQualities(resolved)
                    result = selectPlaybackCandidate(resolved, candidateIndex)
                  }
                  markLoadStartApplied(startPlan)
                  tracePlaybackUi(
                    'backend_stream_ready',
                    track,
                    `candidate=${candidateIndex}, durationMs=${duration}, candidateMs=${Math.round(performance.now() - candidateStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
                    token,
                  )
                  return duration
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
            throw lastError instanceof Error
              ? lastError
              : new Error(String(lastError || 'All playback candidates failed'))
          }

          try {
            dur = await playResolvedSource(result)
          } catch (firstError) {
            if (token !== playbackRequestToken) return
            playbackUrlResolver.invalidate(track, playbackSourceSettings())
            const refreshed = await resolvePlaybackUrl(
              track,
              true,
            )
            if (refreshed.type !== 'success') throw firstError
            result = refreshed
            try {
              dur = await playResolvedSource(result)
            } catch (refreshError) {
              if (token !== playbackRequestToken) return
              if (getPlaybackSourceKind(track) !== 'youtube' || result.streamType === 'hls') throw refreshError
              const hls = await resolvePlaybackUrl(track, true, undefined, true)
              if (hls.type !== 'success') throw refreshError
              result = hls
              dur = await playResolvedSource(result)
            }
          }
          if (token !== playbackRequestToken) return
          // 直链（一起听）也标记 isPreview 以免进缓存，不能据此提示试听
          playingPreviewClip = result.source === 'netease' && result.isPreview === true && !result.cacheKey.endsWith('|direct')
          if (!result.isPreview && result.source !== 'local') {
            rememberPlayedQuality(qualityMemoryKey, {
              qualityKey: result.audioInfo?.qualityKey ?? result.qualityKey,
              codecLabel: result.audioInfo?.codecLabel ?? result.codec,
              bitrateKbps: result.audioInfo?.bitrateKbps ?? normalizeBitrateKbps(result.bitrate),
              mimeType: result.audioInfo?.mimeType,
            })
          }
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
        readLocalAudioInfo(track.audioUrl, token)
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
      _interpSpeed = effectivePlaybackSpeed()
      _interpIsPlaying = true
      _interpDurationMs = durationMs.value
      _startInterpolationLoop()

      // 重置连续失败计数
      consecutivePlayFailures = 0
      consecutiveLoginSkips = 0
      startupRecoveryAttempts = 0
      if (playingPreviewClip) {
        useToastStore().show((i18n.global as any).t('player.playback_preview_only'), 'info')
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

      const toast = useToastStore()
      const failureReason = e instanceof PlaybackFailure ? e.reason : null
      toast.error(playbackFailureText(failureReason, msg))

      // 连续失败熔断 + 自动 skip；需要登录的曲目只跳过不计失败，但整队列都跳过一轮后停下
      const loginSkip = failureReason === 'requires_login'
      if (loginSkip) consecutiveLoginSkips++
      else consecutivePlayFailures++
      const loginSkipsExhausted = loginSkip && consecutiveLoginSkips >= Math.max(1, queue.value.length)
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
        && !loginSkipsExhausted
        && failureAction !== 'stop'
      ) {
        _isAutoSkipping = true
        try {
          await next(failureAction === 'wrap', internalFollowUpSource(commandSource))
        } finally {
          _isAutoSkipping = false
        }
      } else if (consecutivePlayFailures >= MAX_CONSECUTIVE_FAILURES) {
        toast.error((i18n.global as any).t('player.too_many_failures', '连续播放失败过多，已停止'))
      }
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
        // 恢复播放成功后不再需要 pause seek 兜底
        lastSeekedMs = null
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
      } catch {}
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
    } catch {}
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
    } catch {}
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
    // Track End 去重：200ms ticker 可能重复触发
    const trackId = currentTrack.value?.id ?? null
    if (trackId && trackId === lastTrackEndedId && Date.now() - lastTrackEndedTime < 2000) {
      return
    }
    lastTrackEndedId = trackId
    lastTrackEndedTime = Date.now()
    // 播完即清掉记住的位置
    const finishedTrack = currentTrack.value
    if (finishedTrack) persistLongFormProgress(finishedTrack, Math.max(finishedTrack.durationMs || 0, durationMs.value))

    // 一起听会话激活时听众不本地推进: 暂停并上报 TRACK_FINISHED, 由房主/服务端决定切歌,
    // 避免因流时长差异先于房主播完而反向拖动整个房间（LB-02/LT-08）。
    // 重连途中也一样：上报会走 HTTP 或在重连后补发（对齐 Android）
    const lt = useListenTogetherStore()
    if (lt.roomId && !lt.isController) {
      await pause('remote_sync')
      lt.reportTrackFinished(trackId)
      return
    }

    // 睡眠定时器；以下都是播放器自己的推进，不是用户操作
    const isLast = !shuffleEnabled.value && queueIndex.value >= queue.value.length - 1
    if (sleepTimerMode.value === 'end_of_track') {
      await pause('local_safety')
      cancelSleepTimer()
      return
    }
    if (sleepTimerMode.value === 'end_of_queue') {
      if (isLast && repeatMode.value !== 'all') {
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
      if (shuffleEnabled.value || queueIndex.value < queue.value.length - 1) {
        await next(false, 'local_safety')
      } else {
        // 停止播放但保留队列（对齐 Android stopPlaybackPreservingQueue）
        await pause('local_safety')
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
    if (blockedByListenTogether(commandSource)) return
    markCommandSource(commandSource)
    log.info('next:', { source: commandSource, force, shuffle: shuffleEnabled.value, repeat: repeatMode.value, index: queueIndex.value, queueLen: queue.value.length })
    if (queue.value.length === 0) return
    // 用户手动操作重置失败计数（自动 skip 不重置）
    if (!_isAutoSkipping) {
      consecutivePlayFailures = 0
      consecutiveLoginSkips = 0
    }

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
    await play(queue.value[nextIdx], commandSource, 0, false, false)
  }

  /**
   * 用户手动上一首（对齐 Android previousImpl）
   * - 播放超过 3 秒则回到开头
   * - Shuffle 模式使用 history 栈回溯
   * - 非 shuffle：只有 repeat_all 才回绕到末尾
   */
  async function previous(commandSource: PlaybackCommandSource = 'local') {
    if (blockedByListenTogether(commandSource)) return
    markCommandSource(commandSource)
    log.info('previous:', { source: commandSource, shuffle: shuffleEnabled.value, positionMs: Math.round(positionMs.value) })
    consecutivePlayFailures = 0
    consecutiveLoginSkips = 0
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
        await play(queue.value[prevIdx], commandSource, 0, false, false)
      } else {
        seekTo(0, commandSource) // 无历史，重新开始当前曲目
      }
      return
    }

    // 非 shuffle 模式
    if (queueIndex.value > 0) {
      await play(queue.value[queueIndex.value - 1], commandSource, 0, false, false)
    } else if (repeatMode.value === 'all') {
      await play(queue.value[queue.value.length - 1], commandSource, 0, false, false)
    }
    // else: 已在开头且非列表循环，不动
  }

  async function toggleRepeatMode() {
    // 播放模式在前端本地推进（对齐 Android cycleRepeatModeImpl）, 不再采信后端
    // PlayQueue 独立状态机的返回值: 后端进程启动恒为 Off, 与前端从 localStorage 恢复的
    // 模式不同步, 会导致重启后首次点击跳档/无效（PB-03）
    const modes: RepeatMode[] = ['off', 'all', 'one']
    const idx = modes.indexOf(repeatMode.value)
    repeatMode.value = modes[(idx + 1) % modes.length]
    log.info('repeat mode ->', repeatMode.value)
    savePlayerState()
  }

  async function toggleShuffle() {
    // 同 PB-03: 本地翻转, 不采信后端 toggle_shuffle 返回值
    shuffleEnabled.value = !shuffleEnabled.value
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
  const currentStreamUrl = ref<string | null>(null)
  let currentResolvedStreamUrls: string[] = []
  // 自己解析出的直链对应的一起听频道和音质，分享给听众时据此加音质标记
  let currentStreamChannel: string | null = null
  let currentStreamQualities = new Map<string, string>()
  let listenTogetherSyncRateMultiplier: number | null = null

  function rememberStreamQualities(resolved: ResolvedPlaybackSource | null) {
    // 直链（包括房间给的）和试听片段的实际音质未知，不标
    currentStreamChannel = resolved && !resolved.isPreview ? ltChannelForSource(resolved.source) : null
    currentStreamQualities = new Map()
    if (!resolved || !currentStreamChannel) return
    currentStreamQualities.set(resolved.url, resolved.audioInfo?.qualityKey ?? resolved.qualityKey)
    for (const detail of resolved.candidateDetails ?? []) {
      currentStreamQualities.set(detail.url, detail.audioInfo?.qualityKey ?? detail.qualityKey)
    }
  }

  /** 分享出去的链接带音质标记（对齐 Android decorateListenTogetherStreamUrl）；已带标记的保持原样 */
  function shareableStreamUrl(url: string): string {
    if (!currentStreamChannel || hasLtStreamQuality(url)) return url
    return decorateLtStreamUrl(url, currentStreamChannel, currentStreamQualities.get(url))
  }

  function effectivePlaybackSpeed(): number {
    const multiplier = listenTogetherSyncRateMultiplier ?? 1
    return Math.max(0.25, Math.min(3, playbackSpeed.value * multiplier))
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
      invoke('set_speed', { speed: effectivePlaybackSpeed() })
        .catch(error => log.warn('listen together sync rate not applied:', error))
    }, 80)
  }

  function getCurrentStreamUrl(trackId?: string): string | null {
    if (!currentTrack.value || (trackId && currentTrack.value.id !== trackId)) return null
    return currentStreamUrl.value ? shareableStreamUrl(currentStreamUrl.value) : null
  }

  function getCurrentStreamUrls(trackId?: string): string[] {
    if (!currentTrack.value || (trackId && currentTrack.value.id !== trackId)) return []
    return currentResolvedStreamUrls.map(shareableStreamUrl)
  }

  /**
   * 一起听房主给听众准备可分享的直链：只解析不播放，也不看本地缓存或下载
   * （对齐 Android resolveShareableStreamUrls）。预览片段和 HLS 分享不了，返回空
   */
  async function resolveShareableStreamUrls(track: TrackInfo): Promise<string[]> {
    try {
      const settings = playbackSourceSettings()
      if (getPlaybackSourceKind(track) === 'netease') return await resolveNeteaseShareableStreamUrls(track, settings)
      const result = await resolvePlaybackResult(track, settings)
      if (result.type !== 'success' || result.isPreview || result.streamType === 'hls') return []
      const channelId = ltChannelForSource(result.source)
      if (!channelId) return []
      const candidates = [
        { url: result.url, quality: result.audioInfo?.qualityKey ?? result.qualityKey, bitrate: result.bitrate ?? 0 },
        ...(result.candidateDetails ?? []).filter(detail => detail.streamType !== 'hls').map(detail => ({
          url: detail.url, quality: detail.audioInfo?.qualityKey ?? detail.qualityKey, bitrate: detail.bitrate ?? 0,
        })),
      ].filter((candidate, index, all) =>
        isDirectStreamUrl(candidate.url) && all.findIndex(other => other.url === candidate.url) === index)
      const chosen = result.source === 'bilibili'
        ? pickBiliShareableStreams(candidates, settings.biliQuality)
        : candidates.slice(0, LT_SHARE_LIMITS[channelId] ?? 1)
      return chosen.map(candidate => decorateLtStreamUrl(candidate.url, channelId, candidate.quality))
    } catch (error) {
      log.warn('resolve shareable stream failed:', error)
      return []
    }
  }

  /**
   * 网易云按音质组各取一条（对齐 Android resolveNeteaseListenTogetherShareableStreams）：
   * 组内依次请求，实际音质已经分享过就试下一档，最多三条
   */
  async function resolveNeteaseShareableStreamUrls(track: TrackInfo, settings: PlaybackSourceSettings): Promise<string[]> {
    const urls: string[] = []
    const qualities = new Set<string>()
    for (const group of neteaseShareQualityGroups(settings.neteaseQuality)) {
      if (urls.length >= (LT_SHARE_LIMITS.netease ?? 3)) break
      for (const quality of group) {
        const result = await resolvePlaybackResult(track, settings, { qualityOverride: quality, allowFallback: false })
          .catch(() => null)
        if (!result || result.type !== 'success' || result.isPreview || result.source !== 'netease') continue
        if (!isDirectStreamUrl(result.url)) continue
        const actual = (result.audioInfo?.qualityKey ?? result.qualityKey ?? quality).toLowerCase()
        if (qualities.has(actual)) continue
        qualities.add(actual)
        urls.push(decorateLtStreamUrl(result.url, 'netease', actual))
        break
      }
    }
    return urls
  }

  /** B 站按偏好、高→中→低、无损的顺序各取码率最高的一条，最多两条（对齐 Android selectBiliListenTogetherShareableStreams） */
  function pickBiliShareableStreams<T extends { url: string; quality: string; bitrate: number }>(
    candidates: T[],
    preferredQuality: string,
  ): T[] {
    const byQuality = new Map<string, T>()
    for (const candidate of [...candidates].sort((left, right) => right.bitrate - left.bitrate)) {
      if (!byQuality.has(candidate.quality)) byQuality.set(candidate.quality, candidate)
    }
    return biliShareQualityOrder(preferredQuality, new Set(byQuality.keys()))
      .map(quality => byQuality.get(quality))
      .filter((candidate): candidate is T => !!candidate)
  }

  async function setSpeed(spd: number) {
    const next = Math.max(0.25, Math.min(3, spd))
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
      invoke('set_speed', { speed: effectivePlaybackSpeed() })
        .catch(error => log.warn('playback speed not applied:', error))
    }, 200)
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
    try {
      await invoke('set_loudness_gain', { gainMb: loudnessGainMb.value })
    } catch (error) {
      log.warn('loudness gain not applied:', error)
    }
  }

  // 音量均衡由设置页直接改 settings，这里跟随下发到音频链
  watch(() => settings.normalizeVolume, (enabled) => {
    void invoke('set_normalize_volume', { enabled })
      .catch(error => log.warn('volume normalization not applied:', error))
  })

  // 多声道音轨的动态范围压缩：对之后打开的音轨生效
  watch(() => settings.multichannelDrc, (enabled) => {
    void invoke('set_multichannel_drc', { enabled })
      .catch(error => log.warn('multichannel dynamic range compression not applied:', error))
  })

  // 声道平衡同样在设置页里改，拖动时实时下发，音频链按 30 ms 平滑过渡
  watch(() => settings.volumeBalance, (balance) => {
    void invoke('set_volume_balance', { balance })
      .catch(error => log.warn('channel balance not applied:', error))
  })

  async function setEqualizer(enabled: boolean, bands: number[]) {
    equalizerEnabled.value = enabled
    equalizerBands.value = bands.map(v => Math.round(Math.max(-1500, Math.min(1500, v))))
    settings.equalizerEnabled = enabled
    settings.equalizerBands = [...equalizerBands.value]
    try {
      await invoke('set_equalizer', { enabled, bandLevelsMb: equalizerBands.value })
    } catch (error) {
      log.warn('equalizer not applied:', error)
    }
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
    listenTogetherSyncRateMultiplier = null
    settings.loudnessGainMb = 0
    settings.equalizerEnabled = false
    settings.equalizerPresetId = 'flat'
    settings.equalizerBands = [0, 0, 0, 0, 0]
    settings.playbackSpeed = 1.0
    _interpSpeed = 1.0
    try {
      await invoke('reset_audio_effects')
      await invoke('set_speed', { speed: 1.0 })
    } catch (error) {
      log.warn('audio effects reset not applied:', error)
    }
  }

  // 取流按后端实际能解的编码挑选；查询失败或 FFmpeg 没加载上时只认内置解码器，
  // Opus、E-AC-3 这类流排到后面，选出来的流一定能播
  const decoderCapabilities = ref<DecoderCapabilities | null>(null)
  async function loadDecoderCapabilities() {
    try {
      const capabilities = await invoke<DecoderCapabilities>('get_decoder_capabilities')
      setDecodableCodecs(capabilities.codecs)
      decoderCapabilities.value = capabilities
      if (capabilities.ffmpegError) {
        log.warn('FFmpeg unavailable, streams that need it will be avoided:', capabilities.ffmpegError)
      }
    } catch (error) {
      log.warn('decoder capabilities unavailable, using built-in decoders only:', error)
    }
  }

  async function applyPersistedSettings() {
    volume.value = Math.max(0, Math.min(1, settings.volume))
    playbackSpeed.value = Math.max(0.25, Math.min(3, settings.playbackSpeed))
    loudnessGainMb.value = Math.round(Math.max(0, Math.min(1500, settings.loudnessGainMb)))
    equalizerEnabled.value = settings.equalizerEnabled
    equalizerPresetId.value = settings.equalizerPresetId
    equalizerBands.value = settings.equalizerBands.map(value => Math.round(Math.max(-1500, Math.min(1500, value))))
    _interpSpeed = playbackSpeed.value

    const restored: Array<[string, Promise<unknown>]> = [
      ['volume', invoke('set_volume', { level: volume.value })],
      ['output device', invoke('set_audio_output_device', { name: settings.audioOutputDevice || null })
        .catch((error) => {
          log.warn('preferred output device unavailable, using the system default:', error)
          return invoke('set_audio_output_device', { name: null })
        })],
      ['speed', invoke('set_speed', { speed: effectivePlaybackSpeed() })],
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
  }

  // 批量替换队列，并且只发起一次目标曲目的播放请求
  // 队列从哪个本地歌单开始播：其中的歌计入一次播放时，歌单也计一次（对齐 Android localPlaylistPlaybackSource）
  let localPlaylistSource: { id: string; members: Set<string> } | null = null

  function localPlaylistMemberKey(track: TrackInfo): string {
    return track.playlistKey || track.id
  }

  function setLocalPlaylistSource(tracks: TrackInfo[], localPlaylistId?: string) {
    localPlaylistSource = localPlaylistId
      ? { id: localPlaylistId, members: new Set(tracks.map(localPlaylistMemberKey)) }
      : null
  }

  /** 这首歌属于当前队列的来源本地歌单时返回歌单 id；之后加进队列的别处歌曲不算 */
  function localPlaylistIdFor(track: TrackInfo): string | null {
    return localPlaylistSource?.members.has(localPlaylistMemberKey(track)) ? localPlaylistSource.id : null
  }

  /** @param localPlaylistId 从本地歌单开始播放时传入，用于歌单播放统计 */
  function playAll(
    tracks: TrackInfo[],
    requestedTrackId?: string,
    requestedPlaylistKey?: string,
    localPlaylistId?: string,
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
    shuffleHistory = []
    shuffleFuture = []
    setLocalPlaylistSource(tracks, localPlaylistId)
    // 队列整体替换后 shuffleBag 必须按新队列重建（排除当前索引）；
    // 否则曲目自然结束/手动 next 走 bag 空 -> 提前 return，自动切歌永久卡死
    if (shuffleEnabled.value) {
      rebuildShuffleBag()
    } else {
      shuffleBag = []
    }
    void play(queue.value[startIndex])
  }

  // 洗牌后替换队列并播放
  function shufflePlay(tracks: TrackInfo[], localPlaylistId?: string) {
    if (tracks.length === 0) return
    setLocalPlaylistSource(tracks, localPlaylistId)
    const shuffled = [...tracks]
    for (let i = shuffled.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]]
    }
    queue.value = shuffled
    queueIndex.value = 0
    shuffleHistory = []
    shuffleFuture = []
    if (shuffleEnabled.value) {
      rebuildShuffleBag()
    } else {
      shuffleBag = []
    }
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
      queueIndex.value = -1
      currentTrack.value = null
      shuffleBag = []
      shuffleHistory = []
      shuffleFuture = []
      void pause('local_safety')
      savePlayerState()
      return
    }
    if (index < queueIndex.value) {
      queueIndex.value--
    } else if (wasCurrentTrack) {
      // 被删除的是当前曲目，索引保持（指向下一首），但不超界
      queueIndex.value = Math.min(queueIndex.value, queue.value.length - 1)
      // 同步 currentTrack 到新索引指向的曲目
      currentTrack.value = queue.value[queueIndex.value]
    }
    savePlayerState()
  }

  // 清空队列
  function clearQueue() {
    queue.value = []
    queueIndex.value = -1
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

  async function withReleasedAudioFile<T>(filePath: string, operation: () => Promise<T>): Promise<T> {
    const key = audioFilePathKey(filePath)
    if (audioFileMutations.has(key)) throw new Error('An operation on this audio file is already in progress')
    const track = currentTrack.value
    const usesFile = !!track && (
      (!isLoadingAudio.value && loadedPlaybackRequestToken === playbackRequestToken && audioFilePathKey(_currentLoadedFromDownloadPath) === key)
      || audioFilePathKey(track.audioUrl) === key
      || (isLoadingAudio.value && audioFilePathKey(useDownloadStore().getDownloadedTrack(track.id)?.filePath) === key)
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
    // 换音质只影响本机，一起听里没有控制权时也允许
    await play(track, 'local_safety', pos, true)
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
  void loadDecoderCapabilities()

  return {
    isPlaying, currentTrack, positionMs, durationMs, queue, queueIndex,
    repeatMode, shuffleEnabled, volume, lyrics, playError, isLoadingAudio, isLoadingAudioSlow,
    hasPlaybackSession,
    audioLevel, beatImpulse, audioInfo, isPlayingFromDownload, isPlayingFromCache,
    lastCommandSource, lastSeekCommand, isRemoteSyncGuardActive,
    playbackSpeed, currentStreamUrl, sleepTimerMode, sleepRemainingSeconds,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands, hasActiveEffects,
    progress, interpolatedPositionMs, interpolatedProgress, livePositionMs, effectivePlaybackSpeed,
    currentTimeFormatted, durationFormatted,
    play, togglePlayPause, pause, resume, seekTo, next, previous,
    flushPlayerState,
    toggleRepeatMode, toggleShuffle, cyclePlayMode, applyListenTogetherPlaybackMode,
    playMode, setVolume, setSpeed, setListenTogetherSyncPlaybackRate, getCurrentStreamUrl, getCurrentStreamUrls,
    resolveShareableStreamUrls,
    localPlaylistIdFor,
    setLoudnessGain, setEqualizer, setEqualizerPreset, resetAudioEffects,
    applyPersistedSettings, decoderCapabilities,
    startSleepTimer, startSleepTimerEndOfTrack, startSleepTimerEndOfQueue, cancelSleepTimer,
    playAll, shufflePlay, addToQueueNext, addToQueueEnd, removeFromQueue, clearQueue,
    prefetchPlaybackTracks, prefetchIntent, upcomingTrack: () => nextPrefetchTracks()[0] ?? null,
    updateCurrentTrackInfo, patchCurrentTrackSyncPayload, restoreOriginalTrackInfo, hasOriginalTrackInfo,
    handleDownloadedFileRemoved, withReleasedAudioFile, replayWithQuality,
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
  expectedContentMd5?: string,
  streamType: 'direct' | 'hls' = 'direct',
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
        expectedContentMd5,
        streamType,
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
        expectedContentMd5,
        streamType,
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
      expectedContentMd5,
      streamType,
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
      expectedContentMd5,
      streamType,
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
