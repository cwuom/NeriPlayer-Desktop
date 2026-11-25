import i18n from '@/i18n'
import { useToastStore } from '@/stores/toast'
import { getPlaybackSourceKind, isRemotePlaybackTrack } from '@/modules/playback/playbackSource'
import {
  playbackFailureMessageKey,
  shouldThrottlePlaybackRefresh,
  type PlaybackFailure,
} from '@/modules/playback/playbackFailure'
import {
  resolvePlaybackFailureAdvanceAction, shouldRestartOnPrevious, type PlaybackStartupWatchdog,
} from '@/modules/playback/playbackPolicy'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'
import type { TrackInfo } from '@/types/track'
import type { PlaybackCommandSource, PlaybackControl, PlayerCoreState, PlayFunction, SeekControl } from './types'

const log = createLogger('player')

// 取流失败按原因给出与 Android 一致的提示；其他错误（解码、设备）保留原始信息
function playbackFailureText(reason: PlaybackFailure['reason'] | null, message: string): string {
  const t = i18n.global.t
  if (!reason) return t('player.play_failed', { msg: message })
  if (reason === 'url_error') return t('player.playback_url_error', { msg: summarizeLogError(message) })
  return t(playbackFailureMessageKey(reason))
}

// 连续失败熔断
let consecutivePlayFailures = 0
const MAX_CONSECUTIVE_FAILURES = 10
// 需要登录的曲目直接跳过、不计入失败（对齐 Android）；整个队列都需要登录时停下
let consecutiveLoginSkips = 0
let _isAutoSkipping = false
let _stallRecovering = false
let _lastStallRecovery: { key: string; at: number } | null = null

// “下一首播放”选中当前曲目、且未开 shuffle 时，不能移动队列游标；记下待重播的 id，
// 让下一次 next() 消费。只在本次会话有效，不持久化。
let pendingReplayTrackId: string | null = null

// 启动看门狗：起播后迟迟不出声时原地重试，超过次数按失败跳过
let startupRecoveryAttempts = 0
const MAX_STARTUP_RECOVERY_ATTEMPTS = 2
const STARTUP_WATCHDOG_REMOTE_MS = 8_000
const STARTUP_WATCHDOG_YOUTUBE_MS = 12_000

export interface QueueNavigationContext extends PlayerCoreState {
  play: PlayFunction
  pause: PlaybackControl
  seekTo: SeekControl
  markCommandSource: (source: PlaybackCommandSource) => void
  blockedByListenTogether: (commandSource: PlaybackCommandSource) => boolean
  internalFollowUpSource: (commandSource: PlaybackCommandSource) => PlaybackCommandSource
  rebuildShuffleBag: () => void
  shiftShuffleIndicesForPlayNext: (insertIdx: number) => void
  shiftShuffleIndicesForRemove: (removeIdx: number) => void
  savePlayerState: () => void
  currentRenderedPosition: () => number
  /** token 仍是最新的播放请求 */
  isCurrentRequest: (token: number) => boolean
  playbackStartupWatchdog: PlaybackStartupWatchdog
}

/**
 * 队列推进：上一首/下一首、「下一首播放」的待重播、随机播放开关，
 * 以及共享连续失败计数的自动跳过（取流失败、启动卡住、曲中断流）
 */
export function createQueueNavigation(ctx: QueueNavigationContext) {
  const {
    settings, queue, queueIndex, currentTrack, positionMs, isPlaying, isLoadingAudio, queueSelectionNeedsPlayback,
    repeatMode, shuffleEnabled, shuffleBag, shuffleHistory, shuffleFuture,
    play, pause, seekTo, markCommandSource, blockedByListenTogether, internalFollowUpSource,
    rebuildShuffleBag, shiftShuffleIndicesForPlayNext, shiftShuffleIndicesForRemove, savePlayerState,
    currentRenderedPosition, isCurrentRequest, playbackStartupWatchdog,
  } = ctx

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
      isActive: () => isCurrentRequest(token)
        && currentTrack.value?.id === track.id
        && isPlaying.value
        && !isLoadingAudio.value,
      onStall: () => {
        if (!isCurrentRequest(token)) return
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

  /** 起播成功后清零连续失败与启动恢复计数 */
  function resetPlaybackFailures() {
    consecutivePlayFailures = 0
    consecutiveLoginSkips = 0
    startupRecoveryAttempts = 0
  }

  /** 取流或解码失败：提示原因，再按熔断规则决定是否自动跳到下一首 */
  async function handlePlaybackFailure(
    failureReason: PlaybackFailure['reason'] | null,
    message: string,
    commandSource: PlaybackCommandSource,
  ) {
    const toast = useToastStore()
    toast.error(playbackFailureText(failureReason, message))

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
      shuffleFutureSize: shuffleFuture.value.length,
      shuffleBagSize: shuffleBag.value.length,
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
      toast.error(i18n.global.t('player.too_many_failures', '连续播放失败过多，已停止'))
    }
  }

  /** 曲中断流：后端发 stalled 而非 track-ended，从断流位置重试当前曲，超限再跳（PB-02） */
  function recoverStalledPlayback(stalledPositionMs: number) {
    const track = currentTrack.value
    if (!track || _stallRecovering) return
    // 同一首在冷却期内再次断流：不再原地重试（否则每次重启都会清零失败计数而无限循环），
    // 按失败处理并跳到下一首（对齐 Android URL_REFRESH_COOLDOWN_MS）
    const now = Date.now()
    if (shouldThrottlePlaybackRefresh(_lastStallRecovery, track.id, now)) {
      _lastStallRecovery = null
      consecutivePlayFailures++
      useToastStore().error(i18n.global.t('player.playback_network_error'))
      if (consecutivePlayFailures >= MAX_CONSECUTIVE_FAILURES) {
        void pause('local_safety')
        useToastStore().error(i18n.global.t('player.too_many_failures'))
      } else if (!_isAutoSkipping) {
        _isAutoSkipping = true
        void next(true, 'local_safety').finally(() => { _isAutoSkipping = false })
      }
      return
    }
    _lastStallRecovery = { key: track.id, at: now }
    _stallRecovering = true
    const resumeAt = Math.max(0, Math.round(stalledPositionMs))
    void play(track, 'local_safety', resumeAt, true)
      .catch(() => {
        if (!_isAutoSkipping) {
          _isAutoSkipping = true
          void next(true, 'local_safety').finally(() => { _isAutoSkipping = false })
        }
      })
      .finally(() => { _stallRecovering = false })
  }

  /** 播放的不是待重播那首时，作废「下一首播放」记下的当前曲目重播 */
  function discardSupersededReplay(trackId: string) {
    if (pendingReplayTrackId && trackId !== pendingReplayTrackId) pendingReplayTrackId = null
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

    if (!shuffleEnabled.value && pendingReplayTrackId) {
      const replayIndex = pendingReplayIndex()
      pendingReplayTrackId = null
      if (replayIndex >= 0) {
        await play(queue.value[replayIndex], commandSource, 0, false, false)
        return
      }
    }

    let nextIdx: number
    if (shuffleEnabled.value) {
      // Shuffle 三栈模型
      if (shuffleFuture.value.length > 0) {
        // 优先从 future 栈弹出（previous 回退过的）
        if (!queueSelectionNeedsPlayback.value) shuffleHistory.value.push(queueIndex.value)
        nextIdx = shuffleFuture.value.pop()!
      } else if (shuffleBag.value.length > 0) {
        // 未播放池在重建时已洗牌，按顺序消费才能与队列和预取一致
        if (!queueSelectionNeedsPlayback.value) shuffleHistory.value.push(queueIndex.value)
        nextIdx = shuffleBag.value.shift()!
      } else {
        // bag 已空
        if (force || repeatMode.value === 'all') {
          rebuildShuffleBag()
          if (shuffleBag.value.length > 0) {
            if (!queueSelectionNeedsPlayback.value) shuffleHistory.value.push(queueIndex.value)
            nextIdx = shuffleBag.value.shift()!
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
    await play(queue.value[nextIdx], commandSource, 0, false, false, nextIdx, true)
  }

  /**
   * 用户手动上一首（对齐 Android previousImpl）
   * - 播放超过 3 秒则回到开头（可在设置里关闭，关闭后总是切到上一首）
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

    // 媒体键、托盘、全局快捷键、桌面歌词都汇到这里，偏好只需在此判断一次
    if (shouldRestartOnPrevious(positionMs.value, settings.prevRestartsTrack)) {
      seekTo(0, commandSource)
      return
    }

    if (shuffleEnabled.value) {
      // Shuffle：从 history 栈回溯
      if (shuffleHistory.value.length > 0) {
        if (!queueSelectionNeedsPlayback.value) shuffleFuture.value.push(queueIndex.value)
        const prevIdx = shuffleHistory.value.pop()!
        await play(queue.value[prevIdx], commandSource, 0, false, false, prevIdx, true)
      } else {
        seekTo(0, commandSource) // 无历史，重新开始当前曲目
      }
      return
    }

    // 非 shuffle 模式
    if (queueIndex.value > 0) {
      await play(queue.value[queueIndex.value - 1], commandSource, 0, false, false, queueIndex.value - 1)
    } else if (repeatMode.value === 'all') {
      await play(queue.value[queue.value.length - 1], commandSource, 0, false, false, queue.value.length - 1)
    }
    // else: 已在开头且非列表循环，不动
  }

  async function toggleShuffle() {
    pendingReplayTrackId = null
    // 同 PB-03: 本地翻转, 不采信后端 toggle_shuffle 返回值
    shuffleEnabled.value = !shuffleEnabled.value
    // Shuffle 三栈管理
    if (shuffleEnabled.value) {
      rebuildShuffleBag()
      shuffleHistory.value = []
      shuffleFuture.value = []
    } else {
      shuffleBag.value = []
      shuffleHistory.value = []
      shuffleFuture.value = []
    }
    log.info('shuffle ->', shuffleEnabled.value)
    savePlayerState()
  }

  /** Index of the current-track replay queued by Play Next, or -1 if none/stale. */
  function pendingReplayIndex(): number {
    if (!pendingReplayTrackId) return -1
    return queue.value.findIndex(item => item.id === pendingReplayTrackId)
  }

  // Insert directly after the current track ("play next")
  function addToQueueNext(track: TrackInfo) {
    const existing = queue.value.findIndex(t => t.id === track.id)
    // The target is the current track: neither its slot nor the playback index may move,
    // it only has to become the next track. The remove-then-insert path below would drag
    // queueIndex onto the previous track and desynchronize the queue highlight.
    if (existing !== -1 && existing === queueIndex.value) {
      if (shuffleEnabled.value) shuffleFuture.value.push(existing)
      else pendingReplayTrackId = queue.value[existing]?.id ?? null
      savePlayerState()
      return
    }
    // A different Play Next request supersedes a queued current-track replay.
    pendingReplayTrackId = null
    if (existing !== -1) {
      if (shuffleEnabled.value) shiftShuffleIndicesForRemove(existing)
      queue.value.splice(existing, 1)
      if (existing < queueIndex.value) queueIndex.value--
    }
    const idx = queueIndex.value + 1
    queue.value.splice(idx, 0, track)
    if (shuffleEnabled.value) shiftShuffleIndicesForPlayNext(idx)
    savePlayerState()
  }

  return {
    next, previous, toggleShuffle, pendingReplayIndex, addToQueueNext, discardSupersededReplay,
    schedulePlaybackStartupWatchdog, resetPlaybackFailures, handlePlaybackFailure, recoverStalledPlayback,
  }
}
