export type PlaybackFailureAdvanceAction = 'next' | 'wrap' | 'stop'

export function resolvePlaybackFailureAdvanceAction(options: {
  currentIndex: number
  queueSize: number
  repeatAll: boolean
  shuffle: boolean
  shuffleFutureSize: number
  shuffleBagSize: number
}): PlaybackFailureAdvanceAction {
  const {
    currentIndex,
    queueSize,
    repeatAll,
    shuffle,
    shuffleFutureSize,
    shuffleBagSize,
  } = options

  if (queueSize <= 0 || currentIndex < 0 || currentIndex >= queueSize) {
    return 'stop'
  }

  if (shuffle) {
    if (shuffleFutureSize > 0 || shuffleBagSize > 0) return 'next'
    return repeatAll && queueSize > 1 ? 'wrap' : 'stop'
  }

  if (currentIndex < queueSize - 1) return 'next'
  return repeatAll && queueSize > 1 ? 'wrap' : 'stop'
}

/** 播放超过该位置后，「上一首」先回到本曲开头（对齐 Android previousImpl） */
export const PREVIOUS_RESTART_THRESHOLD_MS = 3000

/** 「上一首」是否只回到本曲开头；关闭该偏好时总是切到上一首 */
export function shouldRestartOnPrevious(positionMs: number, restartEnabled: boolean): boolean {
  return restartEnabled && positionMs > PREVIOUS_RESTART_THRESHOLD_MS
}

export class PlaybackDemandArbiter {
  private readonly activePlaybackKeys = new Set<string>()

  markPlaybackDemand(cacheKey: string): void {
    const normalizedKey = cacheKey.trim()
    if (normalizedKey) this.activePlaybackKeys.add(normalizedKey)
  }

  shouldYieldPrefetch(cacheKey: string): boolean {
    const normalizedKey = cacheKey.trim()
    return normalizedKey.length > 0 && this.activePlaybackKeys.has(normalizedKey)
  }

  clearPlaybackDemand(cacheKey: string): void {
    const normalizedKey = cacheKey.trim()
    if (normalizedKey) this.activePlaybackKeys.delete(normalizedKey)
  }

  replacePlaybackDemand(previousKey: string | null, nextKey: string | null): void {
    if (previousKey) this.clearPlaybackDemand(previousKey)
    if (nextKey) this.markPlaybackDemand(nextKey)
  }
}

export interface PlaybackStartupWatchdogOptions {
  timeoutMs: number
  startPositionMs: number
  getPositionMs: () => number
  isActive: () => boolean
  onStall: () => void
  positionToleranceMs?: number
}

export class PlaybackStartupWatchdog {
  private timer: ReturnType<typeof setTimeout> | null = null
  private generation = 0

  schedule(options: PlaybackStartupWatchdogOptions): void {
    this.cancel()
    const generation = ++this.generation
    const toleranceMs = Math.max(0, options.positionToleranceMs ?? 1000)
    const timeoutMs = Math.max(1, options.timeoutMs)
    this.timer = setTimeout(() => {
      this.timer = null
      if (generation !== this.generation || !options.isActive()) return
      const advancedMs = options.getPositionMs() - options.startPositionMs
      if (advancedMs <= toleranceMs) options.onStall()
    }, timeoutMs)
  }

  cancel(): void {
    this.generation += 1
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
  }
}
