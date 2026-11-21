import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TrackInfo } from '@/types/track'
import { PlaybackPrefetchManager } from '@/modules/playback/playbackPrefetch'
import {
  PlaybackUrlResolver,
  playbackPrefetchCacheId,
  type PlaybackResolution,
  type PlaybackSourceSettings,
  type ResolvedPlaybackSource,
} from '@/modules/playback/playbackSource'

const invokeMock = vi.hoisted(() =>
  vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
)

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const settings: PlaybackSourceSettings = {
  neteaseQuality: 'exhigh',
  qqMusicQuality: 'high',
  biliQuality: 'high',
  youtubeQuality: 'high',
}

function remoteTrack(id: number): TrackInfo {
  return {
    id: `netease:${id}`,
    title: `song-${id}`,
    artist: 'artist',
    album: 'album',
    durationMs: 180_000,
    coverUrl: '',
    audioUrl: '',
    source: 'netease',
  }
}

const localTrack: TrackInfo = {
  id: 'local:/music/a.flac',
  title: 'local',
  artist: 'artist',
  album: 'album',
  durationMs: 180_000,
  coverUrl: '',
  audioUrl: '/music/a.flac',
  source: 'local',
}

function success(track: TrackInfo): ResolvedPlaybackSource {
  return {
    type: 'success',
    url: `https://cdn.example/${track.id}.mp3`,
    candidateUrls: [],
    cacheKey: playbackPrefetchCacheId(track, settings),
    source: 'netease',
    qualityKey: 'exhigh',
  }
}

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve: (value: T) => void = () => {}
  let reject: (reason: unknown) => void = () => {}
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/** 只替换 resolve，其余行为保持真实 PlaybackUrlResolver */
class FakeResolver extends PlaybackUrlResolver {
  readonly calls: TrackInfo[] = []

  constructor(private readonly impl: (track: TrackInfo) => Promise<PlaybackResolution>) {
    super()
  }

  override resolve(track: TrackInfo): Promise<PlaybackResolution> {
    this.calls.push(track)
    return this.impl(track)
  }
}

function immediateResolver(): FakeResolver {
  return new FakeResolver(async track => success(track))
}

async function settle(): Promise<void> {
  await new Promise<void>(resolve => setTimeout(resolve, 0))
}

describe('PlaybackPrefetchManager', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('caches a successful resolution that can be taken exactly once', async () => {
    const manager = new PlaybackPrefetchManager()
    const resolver = immediateResolver()
    const track = remoteTrack(1)

    manager.prefetch(track, settings, resolver)
    await settle()

    expect(manager.take(track, settings)).toEqual(success(track))
    expect(manager.take(track, settings)).toBeNull()
  })

  it('never resolves local tracks', async () => {
    const manager = new PlaybackPrefetchManager()
    const resolver = immediateResolver()

    manager.prefetch(localTrack, settings, resolver)
    await settle()

    expect(resolver.calls).toHaveLength(0)
    expect(manager.take(localTrack, settings)).toBeNull()
  })

  it('shares one in-flight resolution between repeated prefetch calls', async () => {
    const manager = new PlaybackPrefetchManager()
    const pending = deferred<PlaybackResolution>()
    const resolver = new FakeResolver(() => pending.promise)
    const track = remoteTrack(2)

    manager.prefetch(track, settings, resolver)
    manager.prefetch(track, settings, resolver)
    expect(resolver.calls).toHaveLength(1)

    pending.resolve(success(track))
    await settle()
    expect(manager.take(track, settings)?.url).toBe(success(track).url)
  })

  it('skips resolving again while a fresh entry is cached and refreshes after it expires', async () => {
    const manager = new PlaybackPrefetchManager(1_000)
    const resolver = immediateResolver()
    const track = remoteTrack(3)

    manager.prefetch(track, settings, resolver)
    await settle()
    manager.prefetch(track, settings, resolver)
    expect(resolver.calls).toHaveLength(1)

    vi.advanceTimersByTime(1_000)
    manager.prefetch(track, settings, resolver)
    expect(resolver.calls).toHaveLength(2)
  })

  it('returns nothing for an entry whose TTL has elapsed', async () => {
    const manager = new PlaybackPrefetchManager(5_000)
    const track = remoteTrack(4)
    manager.prefetch(track, settings, immediateResolver())
    await settle()

    vi.advanceTimersByTime(4_999)
    expect(manager.take(track, settings)).not.toBeNull()

    manager.prefetch(track, settings, immediateResolver())
    await settle()
    vi.advanceTimersByTime(5_000)
    expect(manager.take(track, settings)).toBeNull()
  })

  it('yields to a track that is already being played', async () => {
    const manager = new PlaybackPrefetchManager()
    const resolver = immediateResolver()
    const track = remoteTrack(5)

    manager.replacePlaybackDemand(playbackPrefetchCacheId(track, settings))
    manager.prefetch(track, settings, resolver)
    await settle()
    expect(resolver.calls).toHaveLength(0)

    manager.replacePlaybackDemand(null)
    manager.prefetch(track, settings, resolver)
    await settle()
    expect(resolver.calls).toHaveLength(1)
    expect(manager.take(track, settings)).not.toBeNull()
  })

  it('discards an in-flight result when playback starts demanding the same track', async () => {
    const manager = new PlaybackPrefetchManager()
    const pending = deferred<PlaybackResolution>()
    const track = remoteTrack(6)
    manager.prefetch(track, settings, new FakeResolver(() => pending.promise))

    manager.replacePlaybackDemand(playbackPrefetchCacheId(track, settings))
    pending.resolve(success(track))
    await settle()

    expect(manager.take(track, settings)).toBeNull()
  })

  it('releases the previous demand when playback moves to another track', async () => {
    const manager = new PlaybackPrefetchManager()
    const first = remoteTrack(7)
    const second = remoteTrack(8)
    const resolver = immediateResolver()

    manager.replacePlaybackDemand(playbackPrefetchCacheId(first, settings))
    manager.replacePlaybackDemand(playbackPrefetchCacheId(second, settings))
    manager.prefetch(first, settings, resolver)
    manager.prefetch(second, settings, resolver)
    await settle()

    expect(resolver.calls.map(track => track.id)).toEqual([first.id])
  })

  it('does not cache failures and allows retrying after a rejected resolution', async () => {
    const manager = new PlaybackPrefetchManager()
    const track = remoteTrack(9)
    const failing = new FakeResolver(async () => ({
      type: 'failure',
      message: 'network',
      retryable: true,
    }))
    manager.prefetch(track, settings, failing)
    await settle()
    expect(manager.take(track, settings)).toBeNull()

    const rejecting = new FakeResolver(() => Promise.reject(new Error('boom')))
    manager.prefetch(track, settings, rejecting)
    await settle()
    expect(manager.take(track, settings)).toBeNull()

    const working = immediateResolver()
    manager.prefetch(track, settings, working)
    await settle()
    expect(working.calls).toHaveLength(1)
    expect(manager.take(track, settings)).not.toBeNull()
  })

  it('drops a stale in-flight result after clearForTrack without disturbing the newer job', async () => {
    const manager = new PlaybackPrefetchManager()
    const track = remoteTrack(10)
    const stale = deferred<PlaybackResolution>()
    const fresh = deferred<PlaybackResolution>()
    const queue = [stale, fresh]
    const resolver = new FakeResolver(() => {
      const next = queue.shift()
      if (!next) throw new Error('unexpected resolve call')
      return next.promise
    })

    manager.prefetch(track, settings, resolver)
    manager.clearForTrack(track, settings)
    manager.prefetch(track, settings, resolver)
    expect(resolver.calls).toHaveLength(2)

    stale.resolve({ ...success(track), url: 'https://cdn.example/stale.mp3' })
    await settle()
    expect(manager.take(track, settings)).toBeNull()

    // 旧任务结束不能清掉新任务的在途标记
    manager.prefetch(track, settings, resolver)
    expect(resolver.calls).toHaveLength(2)

    fresh.resolve({ ...success(track), url: 'https://cdn.example/fresh.mp3' })
    await settle()
    expect(manager.take(track, settings)?.url).toBe('https://cdn.example/fresh.mp3')
  })

  it('clears cached entries and in-flight jobs for every track', async () => {
    const manager = new PlaybackPrefetchManager()
    const cached = remoteTrack(11)
    const inFlight = remoteTrack(12)
    const pending = deferred<PlaybackResolution>()
    manager.prefetch(cached, settings, immediateResolver())
    await settle()
    manager.prefetch(inFlight, settings, new FakeResolver(() => pending.promise))

    manager.clear()
    pending.resolve(success(inFlight))
    await settle()

    expect(manager.take(cached, settings)).toBeNull()
    expect(manager.take(inFlight, settings)).toBeNull()
  })

  it('evicts the oldest entry once the capacity is reached', async () => {
    const manager = new PlaybackPrefetchManager(60_000, 2)
    const tracks = [remoteTrack(21), remoteTrack(22), remoteTrack(23)]
    const resolver = immediateResolver()
    for (const track of tracks) {
      manager.prefetch(track, settings, resolver)
      await settle()
    }

    expect(manager.take(tracks[0], settings)).toBeNull()
    expect(manager.take(tracks[1], settings)).not.toBeNull()
    expect(manager.take(tracks[2], settings)).not.toBeNull()
  })

  it('keeps at least one entry even when constructed with a non-positive capacity', async () => {
    const manager = new PlaybackPrefetchManager(60_000, 0)
    const track = remoteTrack(41)
    manager.prefetch(track, settings, immediateResolver())
    await settle()

    expect(manager.take(track, settings)).not.toBeNull()
  })

  it('prefetches every remote track in a window and skips local ones', async () => {
    const manager = new PlaybackPrefetchManager()
    const resolver = immediateResolver()
    const window = [remoteTrack(51), localTrack, remoteTrack(52)]

    manager.prefetchWindow(window, settings, resolver)
    await settle()

    expect(resolver.calls.map(track => track.id)).toEqual(['netease:51', 'netease:52'])
    expect(manager.take(window[0], settings)).not.toBeNull()
    expect(manager.take(window[2], settings)).not.toBeNull()
  })
})
