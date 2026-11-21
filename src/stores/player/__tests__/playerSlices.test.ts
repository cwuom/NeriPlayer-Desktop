import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { createPauseGuard } from '@/stores/player/pauseGuard'
import { createShuffleOrder } from '@/stores/player/queueEditing'
import { createSleepTimer } from '@/stores/player/sleepTimer'
import type { TrackInfo } from '@/types/track'

vi.mock('@/stores/listenTogether', () => ({ useListenTogetherStore: vi.fn() }))

function track(id: string): TrackInfo {
  return { id, title: id, artist: 'artist', album: '', durationMs: 180_000, coverUrl: '', audioUrl: '' }
}

describe('createPauseGuard', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(10_000)
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  function guard(renderedMs = 42_000) {
    const setRenderedPosition = vi.fn((position: number) => position)
    return { ...createPauseGuard(() => renderedMs, setRenderedPosition), setRenderedPosition }
  }

  it('passes positions through until playback is paused', () => {
    const { normalizePositionAfterPause } = guard()
    expect(normalizePositionAfterPause(12_345)).toBe(12_345)
  })

  it('holds the frozen position and drops stale earlier reports right after pausing', () => {
    const { freezeRenderedPosition, normalizePositionAfterPause, setRenderedPosition } = guard()
    expect(freezeRenderedPosition()).toBe(42_000)
    expect(setRenderedPosition).toHaveBeenCalledWith(42_000)
    expect(normalizePositionAfterPause(41_000)).toBeNull()
    expect(normalizePositionAfterPause(41_900)).toBe(42_000)
    expect(normalizePositionAfterPause(43_000)).toBe(42_000)
  })

  it('accepts backend positions again after the guard window or an explicit clear', () => {
    const { freezeRenderedPosition, normalizePositionAfterPause, clearPauseGuard } = guard()
    freezeRenderedPosition()
    vi.setSystemTime(12_500)
    expect(normalizePositionAfterPause(30_000)).toBe(30_000)
    freezeRenderedPosition()
    clearPauseGuard()
    expect(normalizePositionAfterPause(30_000)).toBe(30_000)
  })
})

describe('createShuffleOrder', () => {
  function order(current = 1) {
    const state = {
      queue: ref(['a', 'b', 'c', 'd'].map(track)),
      queueIndex: ref(current),
      shuffleBag: ref<number[]>([]),
      shuffleHistory: ref<number[]>([]),
      shuffleFuture: ref<number[]>([]),
    }
    return { ...state, ...createShuffleOrder(state) }
  }

  it('rebuilds the unplayed bag from every slot except the current one', () => {
    const state = order(1)
    state.rebuildShuffleBag()
    expect([...state.shuffleBag.value].sort()).toEqual([0, 2, 3])
  })

  it('queues a play-next insertion on the future stack and shifts later slots', () => {
    const state = order()
    state.shuffleBag.value = [2, 3]
    state.shuffleHistory.value = [0]
    state.shuffleFuture.value = [3]
    state.shiftShuffleIndicesForPlayNext(2)
    expect(state.shuffleBag.value).toEqual([3, 4])
    expect(state.shuffleHistory.value).toEqual([0])
    expect(state.shuffleFuture.value).toEqual([4, 2])
  })

  it('drops a removed slot from every stack and closes the gap', () => {
    const state = order()
    state.shuffleBag.value = [0, 2, 3]
    state.shuffleHistory.value = [2]
    state.shuffleFuture.value = [3, 2]
    state.shiftShuffleIndicesForRemove(2)
    expect(state.shuffleBag.value).toEqual([0, 2])
    expect(state.shuffleHistory.value).toEqual([])
    expect(state.shuffleFuture.value).toEqual([2])
  })
})

describe('createSleepTimer', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(0)
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('counts down and pauses through local safety when the time is up', async () => {
    const pause = vi.fn(async () => {})
    const timer = createSleepTimer(pause)
    timer.startSleepTimer(1)
    expect(timer.sleepTimerMode.value).toBe('countdown')
    expect(timer.sleepRemainingSeconds.value).toBe(60)
    await vi.advanceTimersByTimeAsync(30_000)
    expect(timer.sleepRemainingSeconds.value).toBe(30)
    expect(pause).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(30_000)
    expect(pause).toHaveBeenCalledWith('local_safety')
    expect(timer.sleepTimerMode.value).toBeNull()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('marks end-of-track mode with a sentinel and cancels a running countdown', () => {
    const timer = createSleepTimer(vi.fn(async () => {}))
    timer.startSleepTimer(5)
    timer.startSleepTimerEndOfTrack()
    expect(vi.getTimerCount()).toBe(0)
    expect(timer.sleepTimerMode.value).toBe('end_of_track')
    expect(timer.sleepRemainingSeconds.value).toBe(-1)
    timer.startSleepTimerEndOfQueue()
    expect(timer.sleepTimerMode.value).toBe('end_of_queue')
    timer.cancelSleepTimer()
    expect(timer.sleepTimerMode.value).toBeNull()
    expect(timer.sleepRemainingSeconds.value).toBe(0)
  })
})
