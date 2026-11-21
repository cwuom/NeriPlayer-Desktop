import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { TrackInfo } from '@/types/track'
import type { BiliCollectionIdentity } from '@/stores/biliVideoParts'
import { useLikedSongsStore } from '@/stores/likedSongs'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

type Save = (track: TrackInfo, identity?: BiliCollectionIdentity) => void
type IsLiked = (track: TrackInfo, video: { bvid: string | null; avid: number | null }) => boolean

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  listeners: new Map<string, () => void>(),
  listenCalls: 0,
  listenError: null as Error | null,
  toasts: [] as string[],
  cloudLikes: [] as Array<[number, boolean]>,
  cloudResult: (): Promise<boolean> => Promise.resolve(true),
  collection: null as null | ((track: TrackInfo, save: Save, isLiked?: IsLiked) => void),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (event: string, handler: () => void) => {
    mocks.listenCalls++
    if (mocks.listenError) throw mocks.listenError
    mocks.listeners.set(event, handler)
    return () => { mocks.listeners.delete(event) }
  },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/i18n', () => ({ default: { global: { t: (key: string) => key } } }))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({ show: (text: string, type = 'info') => { mocks.toasts.push(`${type}:${text}`) } }),
}))
vi.mock('@/stores/recommend', () => ({
  useRecommendStore: () => ({
    toggleLikeSong: (id: number, like: boolean) => {
      mocks.cloudLikes.push([id, like])
      return mocks.cloudResult()
    },
  }),
}))
vi.mock('@/stores/biliVideoParts', () => ({
  useBiliVideoPartsStore: () => ({
    requestCollection: async (track: TrackInfo, save: Save, isLiked?: IsLiked) => mocks.collection!(track, save, isLiked),
  }),
}))

const FAVORITES_ID = -1001

function track(id: string, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return { id, title: `title ${id}`, artist: 'artist', album: '', durationMs: 1000, coverUrl: '', audioUrl: '', ...overrides }
}

/** B 站分 P：曲目 id 带 avid 与 cid，album 记录 cid 与 BV 号 */
function biliPart(cid: number, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return track(`bilibili:170001:${cid}`, { album: `Bilibili|${cid}|BV1abc`, source: 'bilibili', ...overrides })
}

function storedRow(id: string, extra: Record<string, unknown> = {}) {
  return { id, album: '', source: id.split(':')[0], ...extra }
}

describe('likedSongs store', () => {
  let router: ReturnType<typeof createInvokeRouter>

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    mocks.listeners = new Map()
    mocks.listenCalls = 0
    mocks.listenError = null
    mocks.toasts = []
    mocks.cloudLikes = []
    mocks.cloudResult = () => Promise.resolve(true)
    mocks.collection = (target, save) => save(target)
    setActivePinia(createPinia())
  })

  /** 收藏歌单的后端内容随 remove_from_playlist 变化，重新读取时与真实后端一致 */
  async function withFavorites(initial: Array<Record<string, unknown>>) {
    let rows = [...initial]
    router.on('list_playlists', () => [{ id: 3, name: 'Road trip' }, { id: FAVORITES_ID, name: 'whatever' }])
    router.on('get_playlist_tracks', () => rows.map(row => ({ ...row })))
    router.on('remove_from_playlist', (args) => {
      rows = rows.filter(row => row.playlist_key !== args?.trackId && row.id !== args?.trackId)
    })
    const liked = useLikedSongsStore()
    await liked.loadLikedPlaylist()
    return liked
  }

  async function toggle(liked: ReturnType<typeof useLikedSongsStore>, target: TrackInfo) {
    liked.requestToggleTrack(target)
    await flushPromises(60)
  }

  describe('loadLikedPlaylist', () => {
    it('finds the system favorites playlist and mirrors its tracks', async () => {
      const liked = await withFavorites([storedRow('netease:1', { playlist_key: 'k1', sync_payload: { a: 1 } })])

      expect(liked.likedPlaylistId).toBe(FAVORITES_ID)
      expect(router.callsOf('get_playlist_tracks')[0].args).toEqual({ id: FAVORITES_ID })
      expect(liked.isTrackLiked(track('netease:1'))).toBe(true)
      expect(liked.isReady).toBe(true)
      expect(liked.isLoading).toBe(false)
    })

    it('recognises a localized favorites name on a legacy negative id', async () => {
      router.on('list_playlists', () => [{ id: 5, name: 'Liked Songs' }, { id: -7, name: 'Liked Songs' }])
      router.on('get_playlist_tracks', () => [])
      const liked = useLikedSongsStore()

      expect(await liked.loadLikedPlaylist()).toBe(true)
      expect(liked.likedPlaylistId).toBe(-7)
    })

    it('clears state when there is no favorites playlist', async () => {
      const liked = await withFavorites([storedRow('netease:1')])
      router.set('list_playlists', () => [{ id: 3, name: 'Road trip' }])

      expect(await liked.loadLikedPlaylist()).toBe(true)
      expect(liked.likedPlaylistId).toBeNull()
      expect(liked.isTrackLiked(track('netease:1'))).toBe(false)
    })

    it('reports failure but still marks the store ready', async () => {
      router.on('list_playlists', () => { throw 'db locked' })
      const liked = useLikedSongsStore()

      expect(await liked.loadLikedPlaylist()).toBe(false)
      expect(liked.isReady).toBe(true)
      expect(liked.isLoading).toBe(false)
    })

    it('shares one read between concurrent callers', async () => {
      const playlists = deferred<unknown>()
      router.on('list_playlists', () => playlists.promise)
      router.on('get_playlist_tracks', () => [])
      const liked = useLikedSongsStore()

      const first = liked.loadLikedPlaylist()
      const second = liked.loadLikedPlaylist()
      expect(liked.isLoading).toBe(true)
      playlists.resolve([{ id: FAVORITES_ID, name: '' }])

      expect(await Promise.all([first, second])).toEqual([true, true])
      expect(router.callsOf('list_playlists')).toHaveLength(1)
    })
  })

  describe('isTrackLiked with Bilibili parts', () => {
    it('matches parts by video and CID, and the whole video only when asked', async () => {
      const liked = await withFavorites([
        storedRow('bilibili:BV1abc'),
        storedRow('bilibili:170001:200', { album: 'Bilibili|200|BV1abc' }),
      ])

      expect(liked.isTrackLiked(biliPart(200))).toBe(true)
      expect(liked.isTrackLiked(biliPart(300))).toBe(false)
      expect(liked.isTrackLiked(biliPart(300), true)).toBe(true)
      expect(liked.isTrackLiked(track('bilibili:BV1abc', { source: 'bilibili' }))).toBe(true)
      expect(liked.isTrackLiked(biliPart(200, { album: 'Bilibili|200|BV9zzz', id: 'bilibili:999:200' }))).toBe(false)
      expect(liked.isTrackLiked(null)).toBe(false)
    })
  })

  describe('toggling', () => {
    it('creates the favorites playlist on first like and syncs the NetEase heart', async () => {
      router.on('list_playlists', () => [], () => [{ id: FAVORITES_ID, name: '' }])
      router.on('ensure_favorites_playlist', () => ({ id: FAVORITES_ID, name: '我喜欢的音乐', track_count: 0 }))
      router.on('add_to_playlist', () => undefined)
      router.on('get_playlist_tracks', () => [storedRow('netease:7')])
      const liked = useLikedSongsStore()

      await toggle(liked, track('netease:7', { coverUrl: 'https://c', audioUrl: 'https://a', addedAt: 12.6, playlistKey: 'pk' }))

      expect(router.callsOf('ensure_favorites_playlist')[0].args).toEqual({ name: '我喜欢的音乐' })
      expect(router.callsOf('add_to_playlist')[0].args).toEqual({
        playlistId: FAVORITES_ID,
        track: {
          id: 'netease:7', title: 'title netease:7', artist: 'artist', album: '', duration_ms: 1000,
          cover_url: 'https://c', url: 'https://a', source: 'netease', added_at: 13, sync_payload: null, playlist_key: 'pk',
        },
      })
      expect(liked.isTrackLiked(track('netease:7'))).toBe(true)
      expect(mocks.cloudLikes).toEqual([[7, true]])
      expect(mocks.toasts).toEqual([])
    })

    it('re-reads favorites that a sync created while the playlist was being ensured', async () => {
      router.on('list_playlists', () => [], () => [{ id: FAVORITES_ID, name: '' }])
      router.on('ensure_favorites_playlist', () => ({ id: FAVORITES_ID, name: '', track_count: 2 }))
      router.on('add_to_playlist', () => undefined)
      router.on('get_playlist_tracks', () => [storedRow('local:/a.mp3')], () => [storedRow('local:/a.mp3'), storedRow('local:/b.mp3')])
      const liked = useLikedSongsStore()

      await toggle(liked, track('local:/b.mp3'))

      expect(router.callsOf('get_playlist_tracks')).toHaveLength(2)
      expect((router.callsOf('add_to_playlist')[0].args?.track as { source: string }).source).toBe('local')
      expect(mocks.cloudLikes).toEqual([])
    })

    it('removes by playlist key when present and falls back to the track id', async () => {
      const liked = await withFavorites([storedRow('netease:7', { playlist_key: 'pk7' }), storedRow('qq:8')])

      await toggle(liked, track('netease:7'))
      await toggle(liked, track('qq:8'))

      expect(router.callsOf('remove_from_playlist').map(call => call.args)).toEqual([
        { playlistId: FAVORITES_ID, trackId: 'pk7' },
        { playlistId: FAVORITES_ID, trackId: 'qq:8' },
      ])
      expect(liked.isTrackLiked(track('netease:7'))).toBe(false)
      expect(liked.isTrackLiked(track('qq:8'))).toBe(false)
      expect(mocks.cloudLikes).toEqual([[7, false]])
    })

    it('refuses to remove a Bilibili entry it cannot address precisely', async () => {
      const liked = await withFavorites([storedRow('bilibili:BV1abc')])

      await toggle(liked, track('bilibili:BV1abc', { source: 'bilibili' }))

      expect(router.callsOf('remove_from_playlist')).toHaveLength(0)
      expect(liked.isTrackLiked(track('bilibili:BV1abc', { source: 'bilibili' }))).toBe(true)
    })

    it('unlikes the whole video from a part when the collection dialog allows it', async () => {
      const liked = await withFavorites([
        storedRow('bilibili:BV1abc', { playlist_key: 'whole' }),
        storedRow('bilibili:170001:300', { album: 'Bilibili|300|BV1abc', playlist_key: 'p300' }),
      ])
      const video = { bvid: 'BV1abc', avid: 170001 }
      mocks.collection = (target, save) => save(target, { video, allowWholeFavorite: true })

      await toggle(liked, biliPart(200))

      expect(router.callsOf('remove_from_playlist')[0].args).toEqual({ playlistId: FAVORITES_ID, trackId: 'whole' })
      expect(liked.isTrackLiked(biliPart(300))).toBe(true)
    })

    it('lets the collection dialog ask whether a part is already liked', async () => {
      const liked = await withFavorites([storedRow('bilibili:170001:200', { album: 'Bilibili|200|BV1abc' })])
      let answers: boolean[] = []
      mocks.collection = (target, _save, isLiked) => {
        answers = [isLiked!(target, { bvid: 'BV1abc', avid: null }), isLiked!(target, { bvid: 'BVother', avid: null })]
      }

      await toggle(liked, biliPart(200))

      expect(answers).toEqual([true, false])
    })

    it('reloads instead of guessing when the write fails', async () => {
      const liked = await withFavorites([])
      router.on('add_to_playlist', () => { throw 'constraint failed' })

      await toggle(liked, track('netease:9'))

      // 初次加载、切换前加载、失败后重新加载
      expect(router.callsOf('list_playlists')).toHaveLength(3)
      expect(liked.isTrackLiked(track('netease:9'))).toBe(false)
      expect(mocks.cloudLikes).toEqual([])
    })

    it('warns when the NetEase heart could not be synced', async () => {
      const liked = await withFavorites([storedRow('netease:1'), storedRow('netease:2')])
      mocks.cloudResult = () => Promise.resolve(false)
      await toggle(liked, track('netease:1'))
      mocks.cloudResult = () => Promise.reject(new Error('offline'))
      await toggle(liked, track('netease:2'))

      expect(mocks.toasts).toEqual(['info:player.like_cloud_sync_failed', 'info:player.like_cloud_sync_failed'])
    })

    it('ignores empty requests', async () => {
      const liked = useLikedSongsStore()
      let asked = false
      mocks.collection = () => { asked = true }

      liked.requestToggleTrack(null)
      liked.requestToggleTrack(track(''))
      await flushPromises()

      expect(asked).toBe(false)
      expect(router.calls).toEqual([])
    })
  })

  describe('start / stop', () => {
    it('subscribes once, reloads on playlist changes and unsubscribes on stop', async () => {
      const liked = await withFavorites([])

      await liked.start()
      await liked.start()
      expect(mocks.listenCalls).toBe(1)
      const before = router.callsOf('list_playlists').length

      mocks.listeners.get('playlists-changed')!()
      await flushPromises()
      expect(router.callsOf('list_playlists').length).toBe(before + 1)

      liked.stop()
      expect(mocks.listeners.has('playlists-changed')).toBe(false)
    })

    it('still loads when the event bridge is unavailable', async () => {
      mocks.listenError = new Error('no bridge')
      router.on('list_playlists', () => [])
      const liked = useLikedSongsStore()

      await liked.start()

      expect(liked.isReady).toBe(true)
    })
  })
})
