import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useRecommendStore, type PlaylistInfo } from '@/stores/recommend'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  toasts: [] as string[],
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({
    show: (text: string, type = 'info') => { mocks.toasts.push(`${type}:${text}`) },
    success: (text: string) => { mocks.toasts.push(`success:${text}`) },
    error: (text: string) => { mocks.toasts.push(`error:${text}`) },
  }),
}))

const CACHE_KEY = 'neri:recommend:cache'

function neteasePlaylistResponse(...names: string[]) {
  return {
    playlist: names.map((name, index) => ({
      id: index + 1, name, coverImgUrl: `https://p1.music.126.net/${name}.jpg`, trackCount: 10 + index,
      creator: { nickname: 'me' },
    })),
  }
}

describe('recommend store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    mocks.toasts = []
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('maps recommended playlists from the raw NetEase response and caches them', async () => {
    router.on('get_recommended_playlists', () => ({
      result: [
        { id: 7, name: 'Daily', picUrl: 'http://p1.music.126.net/a.jpg', trackCount: 30, copywriter: 'for you' },
        { id: '8', name: 'No count' },
      ],
    }))
    const recommend = useRecommendStore()

    await recommend.fetchRecommendedPlaylists()

    expect(recommend.recommendedPlaylists).toEqual([
      { id: 7, name: 'Daily', coverUrl: 'https://p1.music.126.net/a.jpg?param=300y300', trackCount: 30, description: 'for you' },
      { id: '8', name: 'No count', coverUrl: '', trackCount: 0, description: '' },
    ])
    expect(JSON.parse(storage.getItem(CACHE_KEY) ?? '{}').recommendedPlaylists).toHaveLength(2)
    expect(recommend.isLoading).toBe(false)
  })

  it('keeps the error text when recommendations fail', async () => {
    router.on('get_recommended_playlists', () => { throw 'NETEASE_RISK_CONTROL' })
    const recommend = useRecommendStore()

    await recommend.fetchRecommendedPlaylists()

    expect(recommend.error).toBe('NETEASE_RISK_CONTROL')
    expect(recommend.recommendedPlaylists).toEqual([])
  })

  describe('cloud playlists', () => {
    it('maps Bilibili folders and backfills covers only for non-empty folders', async () => {
      router.on('get_user_playlists', () => ({
        data: {
          list: [
            { id: 1, title: 'Has cover', cover: 'https://i0.hdslb.com/a.jpg', media_count: 3 },
            { id: 2, title: 'Needs cover', media_count: 5 },
            { id: 3, title: 'Empty', media_count: 0 },
          ],
        },
      }))
      router.on('get_bili_fav_folder_info', args => ({ data: { cover: `https://i0.hdslb.com/folder-${String(args?.mediaId)}.jpg` } }))
      const recommend = useRecommendStore()

      await recommend.fetchUserPlaylists('bilibili')

      expect(recommend.userPlaylists.bilibili).toEqual<PlaylistInfo[]>([
        { id: 1, name: 'Has cover', coverUrl: 'https://i0.hdslb.com/a.jpg', trackCount: 3 },
        { id: 2, name: 'Needs cover', coverUrl: 'https://i0.hdslb.com/folder-2.jpg', trackCount: 5 },
        { id: 3, name: 'Empty', coverUrl: '', trackCount: 0 },
      ])
      expect(router.callsOf('get_bili_fav_folder_info').map(call => call.args)).toEqual([{ mediaId: 2 }])
      expect(recommend.userPlaylistsStatus.bilibili).toEqual({ loading: false, error: null })
    })

    it('shares one request between concurrent callers', async () => {
      const response = deferred<unknown>()
      router.on('get_user_playlists', () => response.promise)
      const recommend = useRecommendStore()

      const first = recommend.fetchUserPlaylists('netease')
      const second = recommend.ensureUserPlaylists('netease')
      expect(recommend.userPlaylistsStatus.netease).toEqual({ loading: true, error: null })
      response.resolve(neteasePlaylistResponse('Mix'))
      await Promise.all([first, second])

      expect(router.callsOf('get_user_playlists')).toHaveLength(1)
      expect(recommend.userPlaylists.netease?.map(playlist => playlist.name)).toEqual(['Mix'])
      expect(recommend.userPlaylists.netease?.[0]).toMatchObject({ creator: 'me', trackCount: 10 })
    })

    it('discards a response that belongs to the account before a login change', async () => {
      const oldAccount = deferred<unknown>()
      router.on('get_user_playlists', () => oldAccount.promise)
      const recommend = useRecommendStore()

      const pending = recommend.fetchUserPlaylists('netease')
      recommend.invalidatePlatform('netease')
      oldAccount.resolve(neteasePlaylistResponse('Previous account'))
      await pending

      expect(recommend.userPlaylists.netease).toBeUndefined()
      expect(recommend.userPlaylistsStatus.netease).toBeUndefined()
    })

    it('keeps cached playlists on failure and only toasts when the page has nothing to show', async () => {
      storage.setItem(CACHE_KEY, JSON.stringify({ userPlaylists: { netease: [{ id: 1, name: 'Cached', coverUrl: '', trackCount: 1 }] } }))
      router.on('get_user_playlists', () => { throw 'timeout' })
      const recommend = useRecommendStore()

      await recommend.fetchUserPlaylists('netease')
      expect(recommend.userPlaylists.netease?.map(playlist => playlist.name)).toEqual(['Cached'])
      expect(recommend.userPlaylistsStatus.netease).toEqual({ loading: false, error: 'timeout' })
      expect(mocks.toasts).toEqual([])

      await recommend.fetchUserPlaylists('youtube', { quiet: true })
      expect(mocks.toasts).toEqual([])

      await recommend.fetchUserPlaylists('youtube')
      expect(mocks.toasts).toEqual(['error:timeout'])
    })

    it('revalidates a cached list once per launch', async () => {
      storage.setItem(CACHE_KEY, JSON.stringify({ userPlaylists: { netease: [{ id: 1, name: 'Cached', coverUrl: '', trackCount: 1 }] } }))
      router.on('get_user_playlists', () => neteasePlaylistResponse('Fresh'))
      const recommend = useRecommendStore()
      expect(recommend.userPlaylists.netease?.[0].name).toBe('Cached')

      await recommend.ensureUserPlaylists('netease')
      await recommend.ensureUserPlaylists('netease')

      expect(router.callsOf('get_user_playlists')).toHaveLength(1)
      expect(recommend.userPlaylists.netease?.[0].name).toBe('Fresh')
    })

    it('maps saved NetEase albums with joined artist names', async () => {
      router.on('get_user_stared_albums', () => ({
        data: [{ id: 5, name: 'Album', picUrl: 'https://p1.music.126.net/x.jpg', artists: [{ name: 'A' }, { name: 'B' }], size: 12 }],
      }))
      const recommend = useRecommendStore()

      await recommend.fetchUserAlbums()

      expect(recommend.userAlbums).toEqual([
        { id: 5, name: 'Album', coverUrl: 'https://p1.music.126.net/x.jpg?param=300y300', artist: 'A, B', trackCount: 12 },
      ])
    })
  })

  describe('liked songs', () => {
    it('keeps only numeric ids from the like list', async () => {
      router.on('get_liked_song_ids', () => ({ ids: [1, '2', 3, null] }))
      const recommend = useRecommendStore()

      await recommend.fetchLikedSongIds()

      expect([...recommend.likedSongIds]).toEqual([1, 3])
    })

    it('updates the liked set only when NetEase confirms with code 200', async () => {
      router.on('like_song',
        () => ({ code: 200 }),
        () => ({ code: 301 }),
        () => { throw 'offline' })
      const recommend = useRecommendStore()

      await expect(recommend.toggleLikeSong(9, true)).resolves.toBe(true)
      await expect(recommend.toggleLikeSong(9, false)).resolves.toBe(false)
      await expect(recommend.toggleLikeSong(9, false)).resolves.toBe(false)

      expect(recommend.likedSongIds.has(9)).toBe(true)
      expect(router.callsOf('like_song').map(call => call.args)).toEqual([
        { songId: 9, like: true }, { songId: 9, like: false }, { songId: 9, like: false },
      ])
    })
  })

  describe('YouTube home feed', () => {
    it('drops a feed that finishes after the YouTube account changed', async () => {
      const feed = deferred<unknown>()
      router.on('get_home_feed', () => feed.promise)
      const recommend = useRecommendStore()

      const pending = recommend.fetchHomeFeed()
      expect(recommend.homeFeedLoading).toBe(true)
      recommend.invalidatePlatform('youtube')
      expect(recommend.homeFeedLoading).toBe(false)
      feed.resolve({
        contents: { sectionListRenderer: { contents: [{ musicCarouselShelfRenderer: {
          header: { musicCarouselShelfBasicHeaderRenderer: { title: { runs: [{ text: 'Old account' }] } } },
          contents: [{ musicTwoRowItemRenderer: { title: { runs: [{ text: 'Song' }] } } }],
        } }] } },
      })
      await pending
      await flushPromises()

      expect(recommend.homeFeedShelves).toEqual([])
      expect(recommend.isLoading).toBe(false)
    })
  })
})
