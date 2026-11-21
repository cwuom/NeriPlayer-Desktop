import { ref, type Ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { createLogger } from '@/utils/logger'
import type { TrackInfo } from '@/types/track'
import type { PlaybackCommandSource, PlaybackControl, SeekControl } from './types'

const log = createLogger('player')

export interface PlayerEventsContext {
  currentTrack: Ref<TrackInfo | null>
  isPlaying: Ref<boolean>
  audioLevel: Ref<number>
  beatImpulse: Ref<number>
  /** token 仍是最新的播放请求 */
  isCurrentRequest: (token: number) => boolean
  /** 后端当前装载的正是 token 这次请求 */
  isLoadedRequest: (token: number) => boolean
  /** 有 seek 还在等后端位置收敛 */
  hasPendingSeek: () => boolean
  /** seek 刚发出，后端还可能报告旧位置 */
  isWithinSeekGuard: () => boolean
  /** 进度插值正在按播放推进 */
  isInterpolating: () => boolean
  _startInterpolationLoop: () => void
  shouldAcceptPositionAfterSeek: (nextPositionMs: number, nextDurationMs?: number) => boolean
  normalizePositionAfterPause: (nextPositionMs: number) => number | null
  commitBackendPosition: (nextPositionMs: number, nextDurationMs?: number, forceRendered?: boolean) => void
  maybeAutoSkipBiliVideoInterval: (currentPositionMs: number, currentDurationMs: number) => boolean
  maybePersistProgress: () => void
  recoverStalledPlayback: (stalledPositionMs: number) => void
  handleTrackEnded: () => Promise<void>
  resume: PlaybackControl
  pause: PlaybackControl
  togglePlayPause: PlaybackControl
  next: (force?: boolean, commandSource?: PlaybackCommandSource) => Promise<void>
  previous: PlaybackControl
  seekTo: SeekControl
}

/** 后端播放事件与系统媒体键的监听（首次播放时才注册），以及原生曲终通知的去重 */
export function createPlayerEvents(ctx: PlayerEventsContext) {
  const {
    currentTrack, isPlaying, audioLevel, beatImpulse, isCurrentRequest, isLoadedRequest, hasPendingSeek,
    isWithinSeekGuard, isInterpolating, _startInterpolationLoop, shouldAcceptPositionAfterSeek,
    normalizePositionAfterPause, commitBackendPosition, maybeAutoSkipBiliVideoInterval, maybePersistProgress,
    recoverStalledPlayback, handleTrackEnded, resume, pause, togglePlayPause, next, previous, seekTo,
  } = ctx

  // 是否已初始化事件监听
  let eventsInitialized = false
  let lastNativeEnd: { requestGeneration: number; transition: number } | null = null
  const nativeTrackEndCount = ref(0)

  function acceptNativeTrackEnd(requestGeneration: number, transition: number, position: number, duration: number) {
    if (!isCurrentRequest(requestGeneration)
      || !isLoadedRequest(requestGeneration) || !isPlaying.value) return
    if (lastNativeEnd?.requestGeneration === requestGeneration && transition <= lastNativeEnd.transition) return
    // 刚发出的 seek 可能与旧的曲终通知在途交错，等位置收敛后再接受自然结束
    if (hasPendingSeek() && (isWithinSeekGuard() || !shouldAcceptPositionAfterSeek(position, duration))) return
    lastNativeEnd = { requestGeneration, transition }
    nativeTrackEndCount.value++
    void handleTrackEnded().catch(error => log.warn('track advancement failed:', error))
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
      endedTransition?: number | null
    }>('player:position', (e) => {
      if (!isCurrentRequest(e.payload.requestGeneration)) return
      if (e.payload.endedTransition != null) {
        acceptNativeTrackEnd(e.payload.requestGeneration, e.payload.endedTransition, e.payload.positionMs, e.payload.durationMs)
        return
      }
      // seek 后时间窗口内忽略旧位置事件
      if (isWithinSeekGuard()) return
      if (!shouldAcceptPositionAfterSeek(e.payload.positionMs, e.payload.durationMs)) return
      const normalizedPositionMs = normalizePositionAfterPause(e.payload.positionMs)
      if (normalizedPositionMs === null) return
      commitBackendPosition(normalizedPositionMs, e.payload.durationMs)
      if (maybeAutoSkipBiliVideoInterval(normalizedPositionMs, e.payload.durationMs)) return

      // 节流保存播放进度（每 15s）
      if (isInterpolating()) {
        maybePersistProgress()
      }
    })

    // 监听音频电平
    listen<{ level: number; beat: number }>('player:audio-level', (e) => {
      audioLevel.value = e.payload.level
      beatImpulse.value = e.payload.beat
    })

    // 监听播放完成（对齐 Android handleTrackEnded）
    listen<{
      requestGeneration: number
      endedTransition: number
      positionMs: number
      durationMs: number
    }>('player:track-ended', ({ payload }) => {
      acceptNativeTrackEnd(payload.requestGeneration, payload.endedTransition, payload.positionMs, payload.durationMs)
    })

    // 系统媒体键事件（SMTC / MPRIS，来自 Rust 后端）
    // 曲中断流: 后端发 stalled 而非 track-ended, 从当前位置重试当前曲, 超限再跳（PB-02）
    listen<{ positionMs: number; requestGeneration: number }>('player:playback-stalled', (e) => {
      if (!isCurrentRequest(e.payload.requestGeneration)) return
      recoverStalledPlayback(e.payload.positionMs)
    })

    listen('media:play', () => {
      void resume()
    })

    listen('media:pause', () => {
      void pause()
    })

    listen('media:toggle', () => {
      if (currentTrack.value) void togglePlayPause()
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

  return { initEvents, nativeTrackEndCount }
}
