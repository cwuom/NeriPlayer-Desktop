import { describe, expect, it } from 'vitest'
import {
  resolvePlaybackQueueStartIndex,
  type PlaybackQueueTrackIdentity,
} from '@/modules/playback/playbackQueue'

const tracks: PlaybackQueueTrackIdentity[] = [
  { id: 'first', playlistKey: 'membership:first' },
  { id: 'second', playlistKey: 'membership:second-a' },
  { id: 'second', playlistKey: 'membership:second-b' },
  { id: 'third', playlistKey: 'membership:third' },
]

describe('resolvePlaybackQueueStartIndex', () => {
  it('returns -1 for an empty queue', () => {
    expect(resolvePlaybackQueueStartIndex([], 'second')).toBe(-1)
  })

  it('resolves by track id and falls back to the first track', () => {
    expect(resolvePlaybackQueueStartIndex(tracks)).toBe(0)
    expect(resolvePlaybackQueueStartIndex(tracks, 'third')).toBe(3)
    expect(resolvePlaybackQueueStartIndex(tracks, 'missing')).toBe(0)
    expect(resolvePlaybackQueueStartIndex(tracks, '  second  ')).toBe(1)
  })

  it('prefers the playlist membership key over the track id', () => {
    expect(resolvePlaybackQueueStartIndex(tracks, 'second', 'membership:second-b')).toBe(2)
    expect(resolvePlaybackQueueStartIndex(tracks, 'second', 'missing-membership')).toBe(1)
  })
})
