import type { ListenTogetherRoomState } from './protocol'

export const SOFT_SYNC_RECHECK_INTERVAL_MS = 500

const PLAYING_DRIFT_FORCE_MS = 2500
const HEARTBEAT_DRIFT_FORCE_MS = 5000
const PAUSED_DRIFT_FORCE_MS = 800
const TRACK_SWITCH_FORCE_MS = 500
const SOFT_SYNC_MIN_MS = 600
const SOFT_SYNC_FAST_MS = 1500
const ZERO_POSITION_ROLLBACK_GUARD_MS = 2000
const MAX_CLOCK_SAMPLE_RTT_MS = 30_000

const PASSIVE_POSITION_UPDATE_TYPES = new Set([
  'HEARTBEAT',
  'WATCHDOG',
  'WATCHDOG_STALL',
  'LINK_READY',
  'LINK_UNAVAILABLE',
  'MEMBER_JOINED',
  'MEMBER_LEFT',
])

function nonnegativePosition(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0
}

export function resolveExpectedPosition(
  state: ListenTogetherRoomState,
  serverNowMs: number,
  overridePositionMs?: number,
): number {
  const playback = state.playback
  const durationMs = state.track?.durationMs ?? state.queue[state.currentIndex]?.durationMs ?? 0
  let position = overridePositionMs
  if (position === undefined) {
    position = playback.basePositionMs
    if (playback.state === 'playing' && playback.baseTimestampMs > 0) {
      position += (serverNowMs - playback.baseTimestampMs) * playback.playbackRate
    }
  }
  position = nonnegativePosition(position)
  if (playback.repeatMode === 1 && durationMs > 0) {
    position %= durationMs
  }
  return durationMs > 0 ? Math.min(position, durationMs) : position
}

export interface PositionSyncInput {
  expectedPositionMs: number
  localPositionMs: number
  desiredPlaying: boolean
  isController: boolean
  causeType?: string | null
  playbackContextChanged?: boolean
  targetIndexChanged?: boolean
  trackSwitchGracePeriodActive?: boolean
  forcePositionSync?: boolean
}

export interface PositionSyncResult {
  seekTo?: number
  rate: number | null
}

export function resolvePositionSync(input: PositionSyncInput): PositionSyncResult {
  const localPosition = nonnegativePosition(input.localPositionMs)
  let expectedPosition = nonnegativePosition(input.expectedPositionMs)
  const reload = !!input.playbackContextChanged || !!input.targetIndexChanged
  const passive = PASSIVE_POSITION_UPDATE_TYPES.has(input.causeType ?? '')
  const deferTrackSwitch = !!input.trackSwitchGracePeriodActive && passive
  const ignoreZeroRollback = passive && input.desiredPlaying && expectedPosition === 0
    && localPosition >= ZERO_POSITION_ROLLBACK_GUARD_MS && !reload
  if (deferTrackSwitch || ignoreZeroRollback) expectedPosition = localPosition

  const signedDrift = expectedPosition - localPosition
  const drift = Math.abs(signedDrift)
  const forceThreshold = !input.desiredPlaying
    ? PAUSED_DRIFT_FORCE_MS
    : input.causeType === 'HEARTBEAT'
      ? HEARTBEAT_DRIFT_FORCE_MS
      : PLAYING_DRIFT_FORCE_MS
  const shouldSeek = !!input.forcePositionSync
    || input.causeType === 'LISTENER_SAFETY_RESUME'
    || (!deferTrackSwitch && (reload
      ? expectedPosition > 0 || drift > TRACK_SWITCH_FORCE_MS
      : drift > forceThreshold))
  if (shouldSeek) return { seekTo: expectedPosition, rate: null }
  if (!input.desiredPlaying || input.isController
    || drift < SOFT_SYNC_MIN_MS || drift >= PLAYING_DRIFT_FORCE_MS) {
    return { rate: null }
  }
  const rate = signedDrift >= SOFT_SYNC_FAST_MS
    ? 1.05
    : signedDrift > 0
      ? 1.03
      : signedDrift <= -SOFT_SYNC_FAST_MS
        ? 0.95
        : 0.97
  return { rate }
}

export interface SoftSyncRecheckInput {
  currentRate: number
  sessionConnected: boolean
  isController: boolean
  desiredPlaying: boolean
  localPlaying: boolean
  currentTrackMatchesRoom: boolean
  expectedPositionMs: number
  localPositionMs: number
}

export function resolveSoftSyncRecheckAction(
  input: SoftSyncRecheckInput,
): 'none' | 'reset_rate' | 'keep_rate' | 'apply_state' {
  if (Math.abs(input.currentRate - 1) < 0.001) return 'none'
  if (!input.sessionConnected || input.isController || !input.desiredPlaying
    || !input.localPlaying || !input.currentTrackMatchesRoom) return 'reset_rate'
  const drift = Math.abs(nonnegativePosition(input.expectedPositionMs)
    - nonnegativePosition(input.localPositionMs))
  if (drift >= PLAYING_DRIFT_FORCE_MS) return 'apply_state'
  return drift < SOFT_SYNC_MIN_MS ? 'reset_rate' : 'keep_rate'
}

export function acceptRoomState(
  current: ListenTogetherRoomState | null,
  candidate: ListenTogetherRoomState,
  activeRoomId: string | null,
  lastAppliedVersion = -1,
): ListenTogetherRoomState | null {
  if (candidate.roomId !== activeRoomId || !Number.isFinite(candidate.version)) return null
  const latestVersion = Math.max(lastAppliedVersion,
    current?.roomId === activeRoomId ? current.version : -1)
  if (candidate.version < latestVersion) return null
  if (current?.roomId === activeRoomId && candidate.version === latestVersion) return current
  return candidate
}

export interface ServerClockOffsetInput {
  previousOffsetMs: number
  serverNowMs?: number | null
  sentAtWallMs?: number
  sentAtElapsedMs?: number
  nowWallMs: number
  nowElapsedMs: number
}

export function updateServerClockOffset(input: ServerClockOffsetInput): number {
  const serverNow = input.serverNowMs
  if (serverNow == null || !Number.isFinite(serverNow) || serverNow <= 0) {
    return input.previousOffsetMs
  }
  const hasRoundTrip = (input.sentAtWallMs ?? 0) > 0 && (input.sentAtElapsedMs ?? 0) > 0
  const rtt = hasRoundTrip ? input.nowElapsedMs - input.sentAtElapsedMs! : 0
  if (!Number.isFinite(rtt) || rtt < 0 || rtt > MAX_CLOCK_SAMPLE_RTT_MS) {
    return input.previousOffsetMs
  }
  const reference = hasRoundTrip ? input.sentAtWallMs! + Math.trunc(rtt / 2) : input.nowWallMs
  if (!Number.isFinite(reference)) return input.previousOffsetMs
  const sample = serverNow - reference
  return Math.trunc(input.previousOffsetMs === 0
    ? sample
    : (input.previousOffsetMs * 7 + sample * 3) / 10)
}

export interface TrackOccurrence {
  stableKey: string
  occurrence: number
}

export function trackOccurrenceAt(
  queue: readonly { stableKey: string }[],
  index: number,
): TrackOccurrence | null {
  if (!Number.isInteger(index) || index < 0 || index >= queue.length) return null
  const stableKey = queue[index]!.stableKey
  let occurrence = 0
  for (let i = 0; i < index; i++) {
    if (queue[i]!.stableKey === stableKey) occurrence++
  }
  return { stableKey, occurrence }
}

export function findTrackOccurrenceIndex(
  queue: readonly { stableKey: string }[],
  reference: TrackOccurrence | null,
): number {
  if (!reference || !Number.isInteger(reference.occurrence) || reference.occurrence < 0) return -1
  let occurrence = 0
  return queue.findIndex(track => {
    if (track.stableKey !== reference.stableKey) return false
    return occurrence++ === reference.occurrence
  })
}
