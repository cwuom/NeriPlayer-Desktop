/**
 * 听众端自检与待确认请求（对齐 Android ListenTogetherListenerWatchdogOwner、
 * ListenTogetherListenerStallRecovery 与 ListenTogetherLocalControlOwner）
 */
import type { ListenTogetherEvent, ListenTogetherRoomState } from './protocol'
import { resolveExpectedPosition } from './playbackSync'

export const WATCHDOG_INTERVAL_MS = 8_000
export const SOCKET_SILENCE_TIMEOUT_MS = 45_000
export const REPAIR_MIN_INTERVAL_MS = 30_000
export const STALL_TIMEOUT_MS = 8_000
export const STALL_RECOVERY_COOLDOWN_MS = 12_000

export const MEMBER_REQUEST_RETRY_MS = 3_000
export const MEMBER_REQUEST_TTL_MS = 18_000
export const MEMBER_REQUEST_MAX_ATTEMPTS = 4
export const SEEK_SATISFIED_DRIFT_MS = 1_500

/** 只跟踪走普通事件的请求；换歌和改队列有队列事件自己的确认与补发 */
export const TRACKED_MEMBER_REQUESTS = new Set(['REQUEST_PLAY', 'REQUEST_PAUSE', 'REQUEST_SEEK', 'REQUEST_PLAYBACK_MODE'])

/**
 * 太久没收到任何消息，或者已知漏掉了一条（repairPending，例如收到解析不了的消息）时拉一次房态兜底，
 * 两次之间至少隔 30 秒（对齐 Android shouldRepairListenTogetherListenerState）
 */
export function shouldRefreshListenerState(
  now: number,
  lastSocketMessageAt: number,
  lastRefreshAt: number,
  repairPending = false,
): boolean {
  if (lastRefreshAt > 0 && now - lastRefreshAt < REPAIR_MIN_INTERVAL_MS) return false
  if (repairPending) return true
  if (lastSocketMessageAt <= 0) return true
  return now - lastSocketMessageAt >= SOCKET_SILENCE_TIMEOUT_MS
}

export interface PendingMemberRequest {
  event: ListenTogetherEvent
  createdAt: number
  lastSentAt: number
  attempts: number
}

export function roomCurrentStableKey(state: ListenTogetherRoomState): string | undefined {
  return state.track?.stableKey ?? state.queue[state.currentIndex]?.stableKey
}

/** 房间状态是否已经体现了这次请求 */
export function isMemberRequestSatisfied(
  event: ListenTogetherEvent,
  state: ListenTogetherRoomState | null | undefined,
  serverNowMs: number,
): boolean {
  if (!state) return false
  switch (event.type.replace(/^REQUEST_/, '')) {
    case 'PLAY':
      return state.playback.state === 'playing'
    case 'PAUSE':
      return state.playback.state === 'paused'
    case 'SEEK':
      return event.positionMs != null
        && Math.abs(resolveExpectedPosition(state, serverNowMs) - event.positionMs) <= SEEK_SATISFIED_DRIFT_MS
    case 'PLAYBACK_MODE':
      return (event.repeatMode == null || state.playback.repeatMode === event.repeatMode)
        && (event.shuffleEnabled == null || state.playback.shuffleEnabled === event.shuffleEnabled)
    default:
      return false
  }
}

export type PendingMemberRequestAction = 'satisfied' | 'expired' | 'wait' | 'retry'

/** 每 3 秒重发一次，最多 4 次或 18 秒；期间房间状态满足了就结束 */
export function pendingMemberRequestAction(
  pending: PendingMemberRequest,
  state: ListenTogetherRoomState | null | undefined,
  now: number,
  serverNowMs: number,
): PendingMemberRequestAction {
  if (isMemberRequestSatisfied(pending.event, state, serverNowMs)) return 'satisfied'
  if (now - pending.createdAt > MEMBER_REQUEST_TTL_MS || pending.attempts >= MEMBER_REQUEST_MAX_ATTEMPTS) return 'expired'
  if (now - pending.lastSentAt < MEMBER_REQUEST_RETRY_MS) return 'wait'
  return 'retry'
}

/** 房间在播、本地同一首歌却一直没播起来：持续 8 秒判定卡住，恢复后冷却 12 秒 */
export class StallDetector {
  private startedAt = 0
  private stableKey: string | null = null
  private lastRecoveryAt = 0

  shouldRecover(stalled: boolean, stableKey: string | null | undefined, now: number): boolean {
    if (!stalled || !stableKey) {
      this.startedAt = 0
      this.stableKey = null
      return false
    }
    if (this.stableKey !== stableKey) {
      this.stableKey = stableKey
      this.startedAt = now
      return false
    }
    if (now - this.startedAt < STALL_TIMEOUT_MS) return false
    if (now - this.lastRecoveryAt < STALL_RECOVERY_COOLDOWN_MS) return false
    this.lastRecoveryAt = now
    this.startedAt = now
    return true
  }

  reset() {
    this.startedAt = 0
    this.stableKey = null
    this.lastRecoveryAt = 0
  }
}
