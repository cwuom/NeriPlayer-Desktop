import { describe, expect, it } from 'vitest'
import {
  buildPersistedPlaybackQueue,
  restorePersistedPlaybackQueue,
} from '@/modules/playback/playerState'

interface TestTrack {
  id: string
  playlistKey?: string
  audioUrl: string
}

const first: TestTrack = { id: 'netease:1', playlistKey: 'playlist:first', audioUrl: '' }
const current: TestTrack = { id: 'netease:1', playlistKey: 'playlist:current', audioUrl: '' }
const last: TestTrack = { id: 'netease:2', playlistKey: 'playlist:last', audioUrl: '' }

function normalize(raw: unknown): TestTrack {
  if (!raw || typeof raw !== 'object') throw new Error('invalid track')
  return raw as TestTrack
}

function canRestore(track: TestTrack): boolean {
  return !!track.id && (!!track.audioUrl || !track.id.startsWith('local:'))
}

describe('buildPersistedPlaybackQueue', () => {
  it('keeps the full queue and the current index', () => {
    expect(buildPersistedPlaybackQueue([first, current, last], 1, current, false)).toEqual({
      queue: [first, current, last],
      queueIndex: 1,
      hasPlaybackSession: true,
      currentTrackId: 'netease:1',
      currentTrackPlaylistKey: 'playlist:current',
    })
  })

  it('compacts the queue to the current track only', () => {
    expect(buildPersistedPlaybackQueue([first, current, last], 1, current, true)).toEqual({
      queue: [current],
      queueIndex: 0,
      hasPlaybackSession: true,
      currentTrackId: 'netease:1',
      currentTrackPlaylistKey: 'playlist:current',
    })
  })

  it('records a queue without an active playback session', () => {
    const queued = buildPersistedPlaybackQueue([first, last], -1, null, false)
    expect(queued.queueIndex).toBe(-1)
    expect(queued.hasPlaybackSession).toBe(false)
  })
})

describe('restorePersistedPlaybackQueue', () => {
  it('drops unrestorable tracks and relocates the current track by playlist key', () => {
    const restored = restorePersistedPlaybackQueue(
      [null, { id: 'local:missing', audioUrl: '' }, first, current, last],
      3,
      true,
      'netease:1',
      'playlist:current',
      normalize,
      canRestore,
    )
    expect(restored.queue).toEqual([first, current, last])
    expect(restored.queueIndex).toBe(1)
    expect(restored.currentTrack).toBe(current)
    expect(restored.hasPlaybackSession).toBe(true)
  })

  it('clamps legacy fractional indexes without session metadata', () => {
    const restored = restorePersistedPlaybackQueue(
      [first, last],
      99.8,
      undefined,
      undefined,
      undefined,
      normalize,
      canRestore,
    )
    expect(restored.queueIndex).toBe(1)
    expect(restored.currentTrack).toBe(last)
    expect(restored.hasPlaybackSession).toBe(true)
  })

  it('returns an empty session when nothing can be restored', () => {
    const restored = restorePersistedPlaybackQueue(
      [{ id: 'local:missing', audioUrl: '' }],
      0,
      true,
      'local:missing',
      undefined,
      normalize,
      canRestore,
    )
    expect(restored).toEqual({ queue: [], queueIndex: -1, currentTrack: null, hasPlaybackSession: false })
  })

  it('round-trips a queue that had no playback session', () => {
    const queued = buildPersistedPlaybackQueue([first, last], -1, null, false)
    const restored = restorePersistedPlaybackQueue(
      queued.queue,
      queued.queueIndex,
      queued.hasPlaybackSession,
      queued.currentTrackId,
      queued.currentTrackPlaylistKey,
      normalize,
      canRestore,
    )
    expect(restored.queue).toEqual([first, last])
    expect(restored.queueIndex).toBe(-1)
    expect(restored.currentTrack).toBeNull()
    expect(restored.hasPlaybackSession).toBe(false)
  })
})
