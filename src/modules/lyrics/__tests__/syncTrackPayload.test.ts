import { describe, expect, it, vi } from 'vitest'
import type { TrackInfo } from '@/types/track'
import {
  persistTrackSyncPayload,
  toBackendTrack,
  withUpdatedCustomInfoPayload,
} from '@/modules/lyrics/syncTrackPayload'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
  warn: vi.fn<(...args: unknown[]) => void>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: mocks.warn,
    error: vi.fn(),
  }),
}))

function track(overrides: Partial<TrackInfo> = {}): TrackInfo {
  return {
    id: 'netease:1',
    title: 'Song',
    artist: 'Artist',
    album: 'Album',
    durationMs: 200_000,
    coverUrl: 'https://img.example/1.jpg',
    audioUrl: '',
    ...overrides,
  }
}

const original = { title: 'Original', artist: 'Original Artist', coverUrl: 'https://img.example/o.jpg' }

describe('toBackendTrack', () => {
  it('converts a frontend track to the snake_case IPC shape', () => {
    expect(toBackendTrack(track({
      source: 'netease',
      addedAt: 1_700_000_000_000.6,
      audioUrl: 'https://cdn.example/1.mp3',
      syncPayload: { lyricOffsetMs: 300 },
      playlistKey: 'pl:1',
    }))).toEqual({
      id: 'netease:1',
      title: 'Song',
      artist: 'Artist',
      album: 'Album',
      duration_ms: 200_000,
      cover_url: 'https://img.example/1.jpg',
      url: 'https://cdn.example/1.mp3',
      source: 'netease',
      added_at: 1_700_000_000_001,
      sync_payload: { lyricOffsetMs: 300 },
      playlist_key: 'pl:1',
    })
  })

  it('uses null and zero defaults for missing optional fields', () => {
    const backend = toBackendTrack(track({
      album: '',
      coverUrl: '',
      durationMs: Number.NaN,
      addedAt: -5,
    }))

    expect(backend.album).toBe('')
    expect(backend.cover_url).toBeNull()
    expect(backend.duration_ms).toBe(0)
    expect(backend.added_at).toBe(0)
    expect(backend.sync_payload).toBeNull()
    expect(backend.playlist_key).toBeNull()
    expect(backend.url).toBe('')
  })

  it('infers the source from the id prefix when the track has none', () => {
    expect(toBackendTrack(track({ id: 'netease:1' })).source).toBe('netease')
    expect(toBackendTrack(track({ id: 'qq:002' })).source).toBe('qq')
    expect(toBackendTrack(track({ id: 'bilibili:BV1' })).source).toBe('bilibili')
    expect(toBackendTrack(track({ id: 'youtube:abc' })).source).toBe('youtube')
    expect(toBackendTrack(track({ id: '/music/a.flac' })).source).toBe('local')
  })

  it('keeps an explicit source even if the id prefix disagrees', () => {
    expect(toBackendTrack(track({ id: 'netease:1', source: 'youtube' })).source).toBe('youtube')
  })
})

describe('withUpdatedCustomInfoPayload', () => {
  it('records custom values and remembers the originals on the first override', () => {
    const payload = withUpdatedCustomInfoPayload(
      null,
      { title: 'Custom', artist: 'Custom Artist', coverUrl: 'https://img.example/c.jpg' },
      original,
    )

    expect(payload).toEqual({
      originalName: 'Original',
      customName: 'Custom',
      originalArtist: 'Original Artist',
      customArtist: 'Custom Artist',
      originalCoverUrl: 'https://img.example/o.jpg',
      customCoverUrl: 'https://img.example/c.jpg',
      syncMetadataVersion: 1,
    })
  })

  it('keeps the first original value across repeated overrides', () => {
    const first = withUpdatedCustomInfoPayload(undefined, { title: 'Edit 1' }, original)
    const second = withUpdatedCustomInfoPayload(
      first,
      { title: 'Edit 2' },
      { ...original, title: 'Edit 1' },
    )

    expect(second.originalName).toBe('Original')
    expect(second.customName).toBe('Edit 2')
  })

  it('treats snake_case originals from other clients as already recorded', () => {
    const payload = withUpdatedCustomInfoPayload(
      { original_name: 'Synced Original', original_artist: 'Synced Artist', original_cover_url: 'x' },
      { title: 'Custom', artist: 'Custom Artist', coverUrl: 'https://img.example/c.jpg' },
      original,
    )

    expect(payload).not.toHaveProperty('originalName')
    expect(payload).not.toHaveProperty('originalArtist')
    expect(payload).not.toHaveProperty('originalCoverUrl')
    expect(payload.original_name).toBe('Synced Original')
  })

  it('replaces snake_case custom fields with their camelCase equivalents', () => {
    const payload = withUpdatedCustomInfoPayload(
      {
        custom_name: 'old',
        custom_artist: 'old',
        custom_cover_url: 'old',
        matched_song_id: 'old',
      },
      { title: 'T', artist: 'A', coverUrl: 'C', matchedSongId: 'netease:42' },
      original,
    )

    expect(payload).toMatchObject({
      customName: 'T',
      customArtist: 'A',
      customCoverUrl: 'C',
      matchedSongId: 'netease:42',
    })
    expect(payload).not.toHaveProperty('custom_name')
    expect(payload).not.toHaveProperty('custom_artist')
    expect(payload).not.toHaveProperty('custom_cover_url')
    expect(payload).not.toHaveProperty('matched_song_id')
  })

  it('ignores blank updates and leaves existing custom values alone', () => {
    const payload = withUpdatedCustomInfoPayload(
      { customName: 'Keep', syncMetadataVersion: 1 },
      { title: '   ', artist: '', coverUrl: '\t', matchedSongId: ' ' },
      original,
    )

    expect(payload).toEqual({ customName: 'Keep', syncMetadataVersion: 1 })
  })

  it('does not record an original value that is blank', () => {
    const payload = withUpdatedCustomInfoPayload(
      {},
      { title: 'Custom' },
      { title: '  ', artist: '', coverUrl: '' },
    )

    expect(payload).not.toHaveProperty('originalName')
    expect(payload.customName).toBe('Custom')
  })

  it('marks the payload as current metadata unless a newer version is already present', () => {
    expect(withUpdatedCustomInfoPayload({}, {}, original).syncMetadataVersion).toBe(1)
    expect(withUpdatedCustomInfoPayload({ syncMetadataVersion: 0 }, {}, original).syncMetadataVersion).toBe(1)
    expect(withUpdatedCustomInfoPayload({ syncMetadataVersion: 3 }, {}, original).syncMetadataVersion).toBe(3)

    const snake = withUpdatedCustomInfoPayload({ sync_metadata_version: 2 }, {}, original)
    expect(snake).toEqual({ sync_metadata_version: 2 })
  })

  it('replaces an unparseable snake_case version with the current camelCase marker', () => {
    const payload = withUpdatedCustomInfoPayload({ sync_metadata_version: 'abc' }, {}, original)

    expect(payload).toEqual({ syncMetadataVersion: 1 })
  })

  it('returns a new object without mutating the input payload', () => {
    const input = { customName: 'Before', custom_artist: 'Old' }
    const payload = withUpdatedCustomInfoPayload(input, { title: 'After', artist: 'New' }, original)

    expect(payload).not.toBe(input)
    expect(input).toEqual({ customName: 'Before', custom_artist: 'Old' })
  })
})

describe('persistTrackSyncPayload', () => {
  it('skips the IPC call for missing tracks or tracks without an id', async () => {
    expect(await persistTrackSyncPayload(null)).toBe(0)
    expect(await persistTrackSyncPayload(undefined)).toBe(0)
    expect(await persistTrackSyncPayload(track({ id: '' }))).toBe(0)
    expect(mocks.invoke).not.toHaveBeenCalled()
  })

  it('writes the backend track to every playlist when no playlist is specified', async () => {
    mocks.invoke.mockResolvedValue(2)
    const current = track({ syncPayload: { customName: 'X' } })

    expect(await persistTrackSyncPayload(current)).toBe(2)
    expect(mocks.invoke).toHaveBeenCalledWith('update_playlist_track', {
      playlistId: null,
      track: toBackendTrack(current),
    })
  })

  it('limits the update to the given playlist', async () => {
    mocks.invoke.mockResolvedValue(1)

    await persistTrackSyncPayload(track(), 7)
    expect(mocks.invoke).toHaveBeenCalledWith('update_playlist_track', expect.objectContaining({ playlistId: 7 }))
  })

  it('coerces the reported count to a number and treats garbage as zero', async () => {
    mocks.invoke.mockResolvedValueOnce('3')
    expect(await persistTrackSyncPayload(track())).toBe(3)

    mocks.invoke.mockResolvedValueOnce('not-a-number')
    expect(await persistTrackSyncPayload(track())).toBe(0)

    mocks.invoke.mockResolvedValueOnce(null)
    expect(await persistTrackSyncPayload(track())).toBe(0)
  })

  it('returns zero and logs a warning when the IPC call fails', async () => {
    const failure = new Error('db locked')
    mocks.invoke.mockRejectedValue(failure)

    expect(await persistTrackSyncPayload(track())).toBe(0)
    expect(mocks.warn).toHaveBeenCalledWith('persistTrackSyncPayload failed:', failure)
  })
})
