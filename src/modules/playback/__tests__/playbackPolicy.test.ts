import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import {
  PlaybackDemandArbiter,
  PlaybackStartupWatchdog,
  PREVIOUS_RESTART_THRESHOLD_MS,
  resolvePlaybackFailureAdvanceAction,
  shouldRestartOnPrevious,
  type PlaybackStartupWatchdogOptions,
} from '@/modules/playback/playbackPolicy'

type AdvanceOptions = Parameters<typeof resolvePlaybackFailureAdvanceAction>[0]

function advance(overrides: Partial<AdvanceOptions>): ReturnType<typeof resolvePlaybackFailureAdvanceAction> {
  return resolvePlaybackFailureAdvanceAction({
    currentIndex: 0,
    queueSize: 3,
    repeatAll: false,
    shuffle: false,
    shuffleFutureSize: 0,
    shuffleBagSize: 0,
    ...overrides,
  })
}

describe('resolvePlaybackFailureAdvanceAction', () => {
  it('stops when the queue is empty or the current index is out of range', () => {
    expect(advance({ queueSize: 0, currentIndex: 0, repeatAll: true })).toBe('stop')
    expect(advance({ currentIndex: -1, repeatAll: true })).toBe('stop')
    expect(advance({ currentIndex: 3, queueSize: 3, repeatAll: true })).toBe('stop')
    expect(advance({ currentIndex: 3, queueSize: 3, shuffle: true, shuffleFutureSize: 2 })).toBe('stop')
  })

  it('moves to the next track while sequential playback has tracks left', () => {
    expect(advance({ currentIndex: 0 })).toBe('next')
    expect(advance({ currentIndex: 1 })).toBe('next')
  })

  it('wraps from the last track only when repeat-all is on and the queue has more than one track', () => {
    expect(advance({ currentIndex: 2, repeatAll: true })).toBe('wrap')
    expect(advance({ currentIndex: 2, repeatAll: false })).toBe('stop')
    expect(advance({ currentIndex: 0, queueSize: 1, repeatAll: true })).toBe('stop')
  })

  it('advances in shuffle mode while either the future list or the bag still has tracks', () => {
    expect(advance({ shuffle: true, currentIndex: 2, shuffleFutureSize: 1 })).toBe('next')
    expect(advance({ shuffle: true, currentIndex: 2, shuffleBagSize: 1 })).toBe('next')
  })

  it('ignores the queue position in shuffle mode once the shuffle bag is exhausted', () => {
    expect(advance({ shuffle: true, currentIndex: 0 })).toBe('stop')
    expect(advance({ shuffle: true, currentIndex: 0, repeatAll: true })).toBe('wrap')
    expect(advance({ shuffle: true, currentIndex: 0, queueSize: 1, repeatAll: true })).toBe('stop')
  })
})

describe('shouldRestartOnPrevious', () => {
  it('restarts only after the threshold while the preference is on', () => {
    expect(shouldRestartOnPrevious(PREVIOUS_RESTART_THRESHOLD_MS + 1, true)).toBe(true)
    expect(shouldRestartOnPrevious(PREVIOUS_RESTART_THRESHOLD_MS, true)).toBe(false)
    expect(shouldRestartOnPrevious(0, true)).toBe(false)
  })

  it('never restarts when the preference is off (#75)', () => {
    expect(shouldRestartOnPrevious(PREVIOUS_RESTART_THRESHOLD_MS + 1, false)).toBe(false)
    expect(shouldRestartOnPrevious(10 * 60_000, false)).toBe(false)
  })
})

describe('PlaybackDemandArbiter', () => {
  it('yields prefetch only for keys with active playback demand', () => {
    const arbiter = new PlaybackDemandArbiter()
    arbiter.markPlaybackDemand('netease-1-exhigh')

    expect(arbiter.shouldYieldPrefetch('netease-1-exhigh')).toBe(true)
    expect(arbiter.shouldYieldPrefetch('netease-2-exhigh')).toBe(false)
  })

  it('treats keys that differ only by surrounding whitespace as the same demand', () => {
    const arbiter = new PlaybackDemandArbiter()
    arbiter.markPlaybackDemand('  netease-1-exhigh ')

    expect(arbiter.shouldYieldPrefetch('netease-1-exhigh')).toBe(true)
    arbiter.clearPlaybackDemand('\tnetease-1-exhigh')
    expect(arbiter.shouldYieldPrefetch('netease-1-exhigh')).toBe(false)
  })

  it('never yields for blank keys', () => {
    const arbiter = new PlaybackDemandArbiter()
    arbiter.markPlaybackDemand('   ')
    arbiter.markPlaybackDemand('')

    expect(arbiter.shouldYieldPrefetch('')).toBe(false)
    expect(arbiter.shouldYieldPrefetch('   ')).toBe(false)
  })

  it('replaces the previous demand with the next one and tolerates null keys', () => {
    const arbiter = new PlaybackDemandArbiter()
    arbiter.replacePlaybackDemand(null, 'a')
    expect(arbiter.shouldYieldPrefetch('a')).toBe(true)

    arbiter.replacePlaybackDemand('a', 'b')
    expect(arbiter.shouldYieldPrefetch('a')).toBe(false)
    expect(arbiter.shouldYieldPrefetch('b')).toBe(true)

    arbiter.replacePlaybackDemand('b', null)
    expect(arbiter.shouldYieldPrefetch('b')).toBe(false)
  })
})

describe('PlaybackStartupWatchdog', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  function watchdogOptions(
    overrides: Partial<PlaybackStartupWatchdogOptions> & { positionMs?: number } = {},
  ): PlaybackStartupWatchdogOptions & { onStall: Mock<() => void> } {
    const { positionMs = 0, ...rest } = overrides
    const onStall = vi.fn<() => void>()
    return {
      timeoutMs: 5_000,
      startPositionMs: 0,
      getPositionMs: () => positionMs,
      isActive: () => true,
      ...rest,
      onStall,
    }
  }

  it('reports a stall when the position has not advanced after the timeout', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const options = watchdogOptions({ startPositionMs: 2_000, positionMs: 2_000 })
    watchdog.schedule(options)

    vi.advanceTimersByTime(4_999)
    expect(options.onStall).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(options.onStall).toHaveBeenCalledTimes(1)
  })

  it('does not report a stall once playback advanced beyond the default one second tolerance', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const withinTolerance = watchdogOptions({ positionMs: 1_000 })
    watchdog.schedule(withinTolerance)
    vi.advanceTimersByTime(5_000)
    expect(withinTolerance.onStall).toHaveBeenCalledTimes(1)

    const advanced = watchdogOptions({ positionMs: 1_001 })
    watchdog.schedule(advanced)
    vi.advanceTimersByTime(5_000)
    expect(advanced.onStall).not.toHaveBeenCalled()
  })

  it('honours a custom tolerance and clamps a negative one to zero', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const strict = watchdogOptions({ positionMs: 300, positionToleranceMs: 250 })
    watchdog.schedule(strict)
    vi.advanceTimersByTime(5_000)
    expect(strict.onStall).not.toHaveBeenCalled()

    const negative = watchdogOptions({ positionMs: 0, positionToleranceMs: -50 })
    watchdog.schedule(negative)
    vi.advanceTimersByTime(5_000)
    expect(negative.onStall).toHaveBeenCalledTimes(1)
  })

  it('stays silent when the playback attempt is no longer active', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const options = watchdogOptions({ isActive: () => false })
    watchdog.schedule(options)

    vi.advanceTimersByTime(10_000)
    expect(options.onStall).not.toHaveBeenCalled()
  })

  it('replaces a pending check when rescheduled so only the latest attempt can stall', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const first = watchdogOptions({ timeoutMs: 1_000 })
    const second = watchdogOptions({ timeoutMs: 3_000 })
    watchdog.schedule(first)
    vi.advanceTimersByTime(500)
    watchdog.schedule(second)

    vi.advanceTimersByTime(1_000)
    expect(first.onStall).not.toHaveBeenCalled()
    vi.advanceTimersByTime(2_000)
    expect(second.onStall).toHaveBeenCalledTimes(1)
    expect(first.onStall).not.toHaveBeenCalled()
  })

  it('never fires after cancel', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const options = watchdogOptions()
    watchdog.schedule(options)
    watchdog.cancel()

    vi.advanceTimersByTime(60_000)
    expect(options.onStall).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('clamps a non-positive timeout so the check still runs asynchronously', () => {
    const watchdog = new PlaybackStartupWatchdog()
    const options = watchdogOptions({ timeoutMs: 0 })
    watchdog.schedule(options)

    expect(options.onStall).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(options.onStall).toHaveBeenCalledTimes(1)
  })
})
