import { describe, expect, it } from 'vitest'
import {
  PlaybackStatsTracker,
  type PlaybackStatsSnapshot,
} from '@/modules/playback/playbackStatsTracker'

interface StatsTrack {
  identityKey: string
  durationMs: number
  title: string
}

const songA: StatsTrack = { identityKey: 'netease:1', durationMs: 200_000, title: 'A' }
const songB: StatsTrack = { identityKey: 'netease:2', durationMs: 180_000, title: 'B' }

class FakeClock {
  value = 0
  readonly now = () => this.value
  advance(ms: number): void {
    this.value += ms
  }
}

function createTracker(periodicFlushMs = 15_000) {
  const clock = new FakeClock()
  const tracker = new PlaybackStatsTracker<StatsTrack>(periodicFlushMs, clock.now)
  return { clock, tracker }
}

function requireSnapshot(
  snapshot: PlaybackStatsSnapshot<StatsTrack> | null,
): PlaybackStatsSnapshot<StatsTrack> {
  if (!snapshot) throw new Error('expected a stats snapshot')
  return snapshot
}

describe('PlaybackStatsTracker listening time', () => {
  it('records nothing before a track is being tracked', () => {
    const { clock, tracker } = createTracker()
    expect(tracker.onPlayingChanged(true)).toBeNull()
    clock.advance(60_000)
    expect(tracker.onPlayingChanged(false)).toBeNull()
    expect(tracker.flushFinal()).toBeNull()
    expect(tracker.onProgress(1_000)).toBeNull()
  })

  it('accumulates wall-clock time only while playing and flushes it on pause', () => {
    const { clock, tracker } = createTracker()
    expect(tracker.onTrackChanged(songA)).toBeNull()
    tracker.onPlayingChanged(true)
    clock.advance(10_000)

    expect(tracker.onPlayingChanged(false)).toEqual({
      track: songA,
      listenedMs: 10_000,
      playCountIncrement: 0,
      scheduleSync: true,
    })

    clock.advance(50_000)
    expect(tracker.flushFinal()).toBeNull()

    tracker.onPlayingChanged(true)
    clock.advance(4_000)
    expect(requireSnapshot(tracker.onPlayingChanged(false)).listenedMs).toBe(4_000)
  })

  it('treats repeated playing notifications as a single segment', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(3_000)
    expect(tracker.onPlayingChanged(true)).toBeNull()
    clock.advance(2_000)

    expect(requireSnapshot(tracker.flushFinal()).listenedMs).toBe(5_000)
  })

  it('rounds fractional clock values and ignores a clock that moves backwards', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(1_000.4)
    expect(requireSnapshot(tracker.flushPeriodic()).listenedMs).toBe(1_000)

    clock.advance(-5_000)
    expect(tracker.flushPeriodic()).toBeNull()
  })

  it('distinguishes periodic flushes, which do not schedule sync, from final flushes', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(6_000)

    const periodic = requireSnapshot(tracker.flushPeriodic())
    expect(periodic.scheduleSync).toBe(false)
    expect(periodic.listenedMs).toBe(6_000)

    clock.advance(2_000)
    const final = requireSnapshot(tracker.flushFinal())
    expect(final.scheduleSync).toBe(true)
    expect(final.listenedMs).toBe(2_000)
  })
})

describe('PlaybackStatsTracker play counting', () => {
  it('counts a play once the current play reaches 30 seconds of listening', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(29_999)
    expect(requireSnapshot(tracker.flushPeriodic()).playCountIncrement).toBe(0)

    clock.advance(1)
    expect(requireSnapshot(tracker.flushPeriodic()).playCountIncrement).toBe(1)
  })

  it('adds up listening across pauses within the same play before counting it', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(20_000)
    expect(requireSnapshot(tracker.onPlayingChanged(false)).playCountIncrement).toBe(0)

    tracker.onPlayingChanged(true)
    clock.advance(15_000)
    const snapshot = requireSnapshot(tracker.onPlayingChanged(false))
    expect(snapshot.listenedMs).toBe(15_000)
    expect(snapshot.playCountIncrement).toBe(1)
  })

  it('counts each play at most once no matter how often it is flushed', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(40_000)
    expect(requireSnapshot(tracker.onPlayingChanged(false)).playCountIncrement).toBe(1)

    tracker.onPlayingChanged(true)
    clock.advance(40_000)
    expect(requireSnapshot(tracker.flushPeriodic()).playCountIncrement).toBe(0)
    clock.advance(1_000)
    const ended = requireSnapshot(tracker.onTrackEnded())
    expect(ended.listenedMs).toBe(1_000)
    expect(ended.playCountIncrement).toBe(0)
  })

  it('always counts a play when the track ends, even after a short listen', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(5_000)

    expect(tracker.onTrackEnded()).toEqual({
      track: songA,
      listenedMs: 5_000,
      playCountIncrement: 1,
      scheduleSync: true,
    })
  })

  it('reports a finished play even when no time was accumulated since the last flush', () => {
    const { tracker } = createTracker()
    tracker.onTrackChanged(songA)

    expect(tracker.onTrackEnded()).toEqual({
      track: songA,
      listenedMs: 0,
      playCountIncrement: 1,
      scheduleSync: true,
    })
  })

  it('starts a fresh play after the track ends and keeps timing while still playing', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(200_000)
    expect(requireSnapshot(tracker.onTrackEnded()).playCountIncrement).toBe(1)

    clock.advance(35_000)
    const loop = requireSnapshot(tracker.onPlayingChanged(false))
    expect(loop.listenedMs).toBe(35_000)
    expect(loop.playCountIncrement).toBe(1)
  })
})

describe('PlaybackStatsTracker track changes', () => {
  it('flushes the previous track, including its pending play count, when the track changes', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(31_000)

    expect(tracker.onTrackChanged(songB)).toEqual({
      track: songA,
      listenedMs: 31_000,
      playCountIncrement: 1,
      scheduleSync: true,
    })
  })

  it('does not attribute the previous track\'s time to the next track', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(10_000)
    tracker.onTrackChanged(songB)

    tracker.onPlayingChanged(true)
    clock.advance(7_000)
    const snapshot = requireSnapshot(tracker.flushFinal())
    expect(snapshot.track).toBe(songB)
    expect(snapshot.listenedMs).toBe(7_000)
  })

  it('keeps accumulating when the same track identity is re-announced and adopts the new object', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(5_000)

    const renamed: StatsTrack = { ...songA, title: 'A (edited)' }
    expect(tracker.onTrackChanged(renamed)).toBeNull()
    clock.advance(5_000)

    const snapshot = requireSnapshot(tracker.flushFinal())
    expect(snapshot.track).toBe(renamed)
    expect(snapshot.listenedMs).toBe(10_000)
  })

  it('flushes and stops tracking when the track is cleared', () => {
    const { clock, tracker } = createTracker()
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(8_000)

    expect(requireSnapshot(tracker.onTrackChanged(null)).listenedMs).toBe(8_000)
    clock.advance(8_000)
    expect(tracker.flushFinal()).toBeNull()
    expect(tracker.shouldFlushPeriodically()).toBe(false)
  })
})

describe('PlaybackStatsTracker loop detection', () => {
  function playingTracker() {
    const created = createTracker()
    created.tracker.onTrackChanged(songA)
    created.tracker.onPlayingChanged(true)
    return created
  }

  it('treats a jump from near the end back to near the start as a completed loop', () => {
    const { clock, tracker } = playingTracker()
    expect(tracker.onProgress(198_000)).toBeNull()
    clock.advance(4_000)

    const snapshot = requireSnapshot(tracker.onProgress(1_000))
    expect(snapshot.playCountIncrement).toBe(1)
    expect(snapshot.listenedMs).toBe(4_000)
  })

  it('ignores forward progress and backward jumps that are not a wrap-around', () => {
    const { tracker } = playingTracker()
    expect(tracker.onProgress(10_000)).toBeNull()
    expect(tracker.onProgress(20_000)).toBeNull()
    expect(tracker.onProgress(5_000)).toBeNull()
    expect(tracker.onProgress(198_500)).toBeNull()
    expect(tracker.onProgress(3_001)).toBeNull()
  })

  it('does not count a manual seek back to the start as a loop', () => {
    const { tracker } = playingTracker()
    tracker.onProgress(199_000)
    tracker.onManualSeek(0)
    expect(tracker.onProgress(500)).toBeNull()

    tracker.onProgress(199_500)
    expect(requireSnapshot(tracker.onProgress(200)).playCountIncrement).toBe(1)
  })

  it('suppresses only the first progress update after a manual seek', () => {
    const { tracker } = playingTracker()
    tracker.onManualSeek(199_000)
    expect(tracker.onProgress(199_200)).toBeNull()
    expect(requireSnapshot(tracker.onProgress(100)).playCountIncrement).toBe(1)
  })

  it('never detects loops for tracks without a known duration', () => {
    const { tracker } = createTracker()
    tracker.onTrackChanged({ ...songA, durationMs: 0 })
    tracker.onPlayingChanged(true)

    tracker.onProgress(500_000)
    expect(tracker.onProgress(0)).toBeNull()
  })

  it('clamps negative positions to the start of the track', () => {
    const { tracker } = playingTracker()
    tracker.onProgress(199_000)
    expect(requireSnapshot(tracker.onProgress(-250)).playCountIncrement).toBe(1)
  })
})

describe('PlaybackStatsTracker periodic flush scheduling', () => {
  it('requests a periodic flush once listened plus in-progress time reaches the interval', () => {
    const { clock, tracker } = createTracker(15_000)
    tracker.onTrackChanged(songA)
    expect(tracker.shouldFlushPeriodically()).toBe(false)

    tracker.onPlayingChanged(true)
    clock.advance(14_999)
    expect(tracker.shouldFlushPeriodically()).toBe(false)
    clock.advance(1)
    expect(tracker.shouldFlushPeriodically()).toBe(true)

    tracker.flushPeriodic()
    expect(tracker.shouldFlushPeriodically()).toBe(false)
  })

  it('never requests a periodic flush while paused', () => {
    const { clock, tracker } = createTracker(1_000)
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(5_000)
    tracker.onPlayingChanged(false)

    clock.advance(5_000)
    expect(tracker.shouldFlushPeriodically()).toBe(false)
  })

  it('disables periodic flushing for a non-positive interval', () => {
    const { clock, tracker } = createTracker(0)
    tracker.onTrackChanged(songA)
    tracker.onPlayingChanged(true)
    clock.advance(60_000)

    expect(tracker.shouldFlushPeriodically()).toBe(false)
  })
})
