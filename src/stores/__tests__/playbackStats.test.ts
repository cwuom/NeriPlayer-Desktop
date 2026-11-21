import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import type { TrackInfo } from '@/types/track'
import { usePlaybackStatsStore, type StatsSummary } from '@/stores/playbackStats'
import { createInvokeRouter, deferred, flushPromises, type Deferred } from './storeTestUtils'

interface PlayerStub {
  currentTrack: TrackInfo | null
  isPlaying: boolean
  positionMs: number
  lastSeekCommand: { seq: number }
  nativeTrackEndCount: number
  localPlaylistIdFor: (track: TrackInfo) => string | null
}

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  player: null as PlayerStub | null,
  statsListener: null as (() => void) | null,
  listenReady: null as Promise<void> | null,
  unlistened: 0,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (_event: string, handler: () => void) => {
    await mocks.listenReady
    mocks.statsListener = handler
    return () => {
      mocks.unlistened++
      if (mocks.statsListener === handler) mocks.statsListener = null
    }
  },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/player', () => ({ usePlayerStore: () => mocks.player }))

interface RecordedSession {
  track: { id: string; duration_ms: number; source: string; cover_url: string | null; sync_payload: unknown }
  listenedMs: number
  playCountIncrement: number
  localPlaylistId: string | null
}

function track(id: string, durationMs = 200_000): TrackInfo {
  return { id, title: `title ${id}`, artist: 'artist', album: '', durationMs, coverUrl: '', audioUrl: '', source: 'netease' }
}

function summary(period: StatsSummary['period'], totalPlayCount: number): StatsSummary {
  return { period, totalPlayCount, totalListenMs: totalPlayCount * 1000, trackCount: 1, items: [] }
}

describe('playbackStats store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let player: PlayerStub
  let sessions: RecordedSession[]

  const sum = (key: 'listenedMs' | 'playCountIncrement') => sessions.reduce((total, session) => total + session[key], 0)

  async function play(target: TrackInfo) {
    player.currentTrack = target
    player.isPlaying = true
    await flushPromises()
  }

  async function update(change: (state: PlayerStub) => void) {
    change(player)
    await flushPromises()
  }

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'performance'] })
    router = createInvokeRouter()
    mocks.router = router
    sessions = []
    router.on('record_playback_session', (args) => { sessions.push(args?.session as RecordedSession) })
    player = reactive<PlayerStub>({
      currentTrack: null,
      isPlaying: false,
      positionMs: 0,
      lastSeekCommand: { seq: 0 },
      nativeTrackEndCount: 0,
      localPlaylistIdFor: (target: TrackInfo) => (target.id === 'netease:1' ? 'playlist-9' : null),
    })
    mocks.player = player
    mocks.statsListener = null
    mocks.listenReady = Promise.resolve()
    mocks.unlistened = 0
    setActivePinia(createPinia())
  })

  afterEach(() => {
    usePlaybackStatsStore().detach()
    vi.useRealTimers()
  })

  describe('recording sessions', () => {
    it('reports wall-clock listening in backend track shape and counts one play past 30 seconds', async () => {
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:1'))

      vi.advanceTimersByTime(31_000)
      await update(state => { state.currentTrack = track('netease:2') })

      expect(sum('listenedMs')).toBe(31_000)
      expect(sum('playCountIncrement')).toBe(1)
      expect(sessions[0]).toMatchObject({
        track: { id: 'netease:1', duration_ms: 200_000, source: 'netease', cover_url: null, sync_payload: null },
        localPlaylistId: 'playlist-9',
      })
      expect(sessions.every(session => session.track.id === 'netease:1')).toBe(true)
    })

    it('records a short listen without a play when playback pauses', async () => {
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:3'))

      vi.advanceTimersByTime(10_000)
      await update(state => { state.isPlaying = false })
      vi.advanceTimersByTime(60_000)
      await update(state => { state.currentTrack = null })

      expect(sessions).toEqual([expect.objectContaining({ listenedMs: 10_000, playCountIncrement: 0, localPlaylistId: null })])
    })

    it('flushes every 15 seconds while playing so a crash loses at most one window', async () => {
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:3'))

      vi.advanceTimersByTime(16_000)

      expect(sessions).toEqual([expect.objectContaining({ listenedMs: 15_000, playCountIncrement: 0 })])
    })

    it('counts a natural track end as a play even when it was short', async () => {
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:3'))

      vi.advanceTimersByTime(5_000)
      player.nativeTrackEndCount++
      await flushPromises()

      expect(sessions).toEqual([expect.objectContaining({ listenedMs: 5_000, playCountIncrement: 1 })])
    })

    it('counts a repeat-one loop but not a manual seek back to the start', async () => {
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:3', 20_000))

      await update(state => { state.positionMs = 19_000 })
      await update(state => { state.positionMs = 1_000 })
      expect(sum('playCountIncrement')).toBe(1)

      await update(state => { state.positionMs = 19_500 })
      await update(state => { state.lastSeekCommand = { seq: state.lastSeekCommand.seq + 1 } })
      await update(state => { state.positionMs = 500 })
      expect(sum('playCountIncrement')).toBe(1)
    })

    it('stops recording after detach and does not double count after re-attach', async () => {
      const stats = usePlaybackStatsStore()
      stats.attach()
      stats.attach()
      stats.detach()
      await play(track('netease:3'))
      vi.advanceTimersByTime(20_000)
      expect(sessions).toEqual([])

      stats.attach()
      await update(state => { state.isPlaying = false })
      await update(state => { state.isPlaying = true })
      vi.advanceTimersByTime(4_000)
      await update(state => { state.isPlaying = false })

      expect(sessions).toEqual([expect.objectContaining({ listenedMs: 4_000 })])
    })

    it('never lets a failed write disturb playback', async () => {
      router.set('record_playback_session', () => { throw new Error('db busy') })
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:3'))
      vi.advanceTimersByTime(3_000)

      await update(state => { state.isPlaying = false })
      await stats.flushFinal()

      expect(router.callsOf('record_playback_session')).toHaveLength(1)
    })

    it('settles the current listen on demand', async () => {
      router.on('get_playback_stats_overview', () => [summary('all', 3)])
      const stats = usePlaybackStatsStore()
      stats.attach()
      await play(track('netease:3'))
      vi.advanceTimersByTime(5_000)

      await stats.flushAndRefresh()
      expect(sessions).toEqual([expect.objectContaining({ listenedMs: 5_000, playCountIncrement: 0 })])
      expect(stats.current.totalPlayCount).toBe(3)

      vi.advanceTimersByTime(2_000)
      await stats.flushFinal()
      expect(sessions.map(session => session.listenedMs)).toEqual([5_000, 2_000])
    })
  })

  describe('summaries', () => {
    it('merges the overview by period and keeps the last good data on failure', async () => {
      router.on('get_playback_stats_overview', () => [summary('day', 1), summary('week', 2)], () => { throw 'db locked' })
      const stats = usePlaybackStatsStore()

      await stats.refresh()
      expect(stats.summaries.day.totalPlayCount).toBe(1)
      expect(stats.summaries.week.totalPlayCount).toBe(2)
      expect(stats.summaries.year).toEqual({ period: 'year', totalPlayCount: 0, totalListenMs: 0, trackCount: 0, items: [] })
      stats.activePeriod = 'week'
      expect(stats.current.totalPlayCount).toBe(2)

      await stats.refresh()
      expect(stats.summaries.day.totalPlayCount).toBe(1)
      expect(stats.loading).toBe(false)
    })

    it('reloads after clearing or removing tracks, and skips an empty removal', async () => {
      router.on('get_playback_stats_overview', () => [summary('all', 0)])
      router.on('clear_playback_stats', () => undefined)
      router.on('remove_playback_stats', () => undefined)
      const stats = usePlaybackStatsStore()

      await stats.removeTracks([])
      expect(router.calls).toEqual([])

      await stats.removeTracks(['netease:1|a'])
      await stats.clearAll()
      expect(router.calls.map(call => call.command)).toEqual([
        'remove_playback_stats', 'get_playback_stats_overview', 'clear_playback_stats', 'get_playback_stats_overview',
      ])
      expect(router.callsOf('remove_playback_stats')[0].args).toEqual({ identityKeys: ['netease:1|a'] })
    })

    it('refreshes when the backend announces new stats', async () => {
      router.on('get_playback_stats_overview', () => [summary('all', 7)])
      const stats = usePlaybackStatsStore()
      stats.attach()
      await flushPromises()

      mocks.statsListener!()
      await flushPromises()

      expect(stats.summaries.all.totalPlayCount).toBe(7)
    })

    it('drops a listener that finished registering after detach', async () => {
      const ready: Deferred<void> = deferred<void>()
      mocks.listenReady = ready.promise
      const stats = usePlaybackStatsStore()

      stats.attach()
      stats.detach()
      ready.resolve()
      await flushPromises()

      expect(mocks.unlistened).toBe(1)
      expect(mocks.statsListener).toBeNull()
    })
  })
})
