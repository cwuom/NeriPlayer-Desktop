import { useHistoryStore } from '@/stores/history'
import { useSettingsStore } from '@/stores/settings'
import {
  LONG_FORM_MIN_DURATION_MS,
  longFormPositionForPersistence,
  resolveLongFormResumePosition,
} from '@/modules/playback/longFormProgress'
import {
  buildPersistedPlaybackQueue,
  restorePersistedPlaybackQueue,
  type PersistedPlaybackQueue,
} from '@/modules/playback/playerState'
import {
  finishLegacyPlayerStateCleanup,
  LEGACY_PLAYER_STATE_KEY,
  persistUserData,
  preloadedUserData,
} from '@/modules/persistence/userData'
import { normalizeTrack } from '@/modules/playback/trackModel'
import { createLogger } from '@/utils/logger'
import type { RepeatMode, TrackInfo } from '@/types/track'
import type { PlayerCoreState } from './types'

const uiLog = createLogger('playback-ui')

// 状态持久化：正常运行写用户数据库，浏览器开发模式退回 localStorage
const PLAYER_STATE_KEY = LEGACY_PLAYER_STATE_KEY
let _persistDebounceTimer: ReturnType<typeof setTimeout> | null = null
// 上次成功落库的队列序列化结果；队列不变时只更新播放状态行
let _lastPersistedQueueJson: string | null = null
let _progressPersistTime = 0
const PERSIST_DEBOUNCE_MS = 250
const PROGRESS_PERSIST_INTERVAL_MS = 15000

type PersistedPlayerState = PersistedPlaybackQueue<TrackInfo> & {
  volume: number
  positionMs?: number
  repeatMode?: RepeatMode
  shuffleEnabled?: boolean
}

export interface PlayerPersistenceContext extends PlayerCoreState {
  currentRenderedPosition: () => number
  setRenderedPosition: (position: number, durationOverride?: number) => number
  rebuildShuffleBag: () => void
  /** 恢复出的播放上下文还没装进后端，首次播放需要重新加载 */
  markNeedsReload: (needsReload: boolean) => void
}

/** 播放状态持久化与恢复，以及长音频的续播进度 */
export function createPlayerPersistence(ctx: PlayerPersistenceContext) {
  const {
    queue, queueIndex, currentTrack, volume, repeatMode, shuffleEnabled, durationMs, positionMs,
    hasPlaybackSession, currentRenderedPosition, setRenderedPosition, rebuildShuffleBag, markNeedsReload,
  } = ctx

  // 状态持久化函数
  function persistedPlayerState(compact = false): PersistedPlayerState {
    const settings = useSettingsStore()
    const persistedQueue = buildPersistedPlaybackQueue(
      queue.value,
      queueIndex.value,
      currentTrack.value,
      compact,
    )
    const state: PersistedPlayerState = {
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
  function readPersistedPlayerState(): Record<string, unknown> | null {
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
      markNeedsReload(restored.hasPlaybackSession)
      // 恢复的是可见播放上下文；音频后端会在用户再次点击播放时装载
      hasPlaybackSession.value = restored.hasPlaybackSession

      if (settings.keepProgress && typeof state.positionMs === 'number') {
        setRenderedPosition(state.positionMs)
      }

      if (settings.keepPlaybackMode) {
        if (typeof state.repeatMode === 'string' && ['off', 'all', 'one'].includes(state.repeatMode)) {
          repeatMode.value = state.repeatMode as RepeatMode
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

  return {
    flushPlayerState, savePlayerState, loadPlayerState, maybePersistProgress,
    persistLongFormProgress, persistCurrentLongFormProgress, rememberedLongFormStart,
  }
}
