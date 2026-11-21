import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { NETEASE_HOME_SECTIONS, type NeteaseHomeSource } from '@/modules/library/neteaseHome'
import { useHomeFeedStore } from '@/stores/homeFeed'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises, type Deferred } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))

const CACHE_KEY = 'neri:home-feed:v1'
const NOW = 1_800_000_000_000
const TTL_MS = 30 * 60 * 1000
const PUBLIC = NETEASE_HOME_SECTIONS.filter(section => !section.requiresLogin).map(section => section.key)
const PRIVATE = NETEASE_HOME_SECTIONS.filter(section => section.requiresLogin).map(section => section.key)
const ALL = NETEASE_HOME_SECTIONS.map(section => section.key)

function kindOf(source: NeteaseHomeSource) {
  return NETEASE_HOME_SECTIONS.find(section => section.key === source)!.kind
}

/** 每个分区返回一条带分区名的内容，便于断言是哪个响应写进了哪个分区 */
function sectionResponse(source: NeteaseHomeSource, label: string = source) {
  return kindOf(source) === 'songs'
    ? { code: 200, result: [{ id: 11, name: `${label} song`, ar: [{ name: 'Artist' }], al: { name: 'Album' }, dt: 1000 }] }
    : { code: 200, result: [{ id: 22, name: `${label} list`, trackCount: 3, playCount: 4 }] }
}

function firstTitle(store: ReturnType<typeof useHomeFeedStore>, source: NeteaseHomeSource): string | undefined {
  const section = store.sections[source]
  return section.songs[0]?.title ?? section.playlists[0]?.name
}

interface PendingSection { source: NeteaseHomeSource; response: Deferred<unknown> }

describe('homeFeed store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage
  let pending: PendingSection[]
  let respond: (source: NeteaseHomeSource) => unknown

  const requested = () => router.callsOf('get_netease_home_section').map(call => call.args?.source as NeteaseHomeSource)
  const manual = () => {
    respond = (source) => {
      const response = deferred<unknown>()
      pending.push({ source, response })
      return response.promise
    }
  }

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(NOW)
    router = createInvokeRouter()
    mocks.router = router
    pending = []
    respond = source => sectionResponse(source)
    router.on('get_netease_home_section', args => respond(args?.source as NeteaseHomeSource))
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('loads only public sections for guests with at most three requests in flight', async () => {
    manual()
    const store = useHomeFeedStore()

    const refreshing = store.refresh(false, '')
    await flushPromises()
    expect(requested()).toHaveLength(3)
    expect(store.sections.personal_radar.loading).toBe(true)

    let maxInFlight = 0
    while (pending.length) {
      maxInFlight = Math.max(maxInFlight, pending.length)
      const { source, response } = pending.shift()!
      response.resolve(sectionResponse(source))
      await flushPromises()
    }
    await refreshing

    expect(maxInFlight).toBe(3)
    expect([...requested()].sort()).toEqual([...PUBLIC].sort())
    for (const source of PRIVATE) expect(store.sections[source]).toMatchObject({ songs: [], playlists: [], loading: false })
    expect(firstTitle(store, 'personal_radar')).toBe('personal_radar song')
    expect(firstTitle(store, 'personalized')).toBe('personalized list')
    expect(store.sections.personalized.loading).toBe(false)
  })

  it('caches public sections for the account but never the private ones', async () => {
    const store = useHomeFeedStore()

    await store.refresh(true, ' 42 ')

    expect([...requested()].sort()).toEqual([...ALL].sort())
    const cache = JSON.parse(storage.getItem(CACHE_KEY) ?? '{}') as { version: number; context: string; sections: Record<string, { fetchedAt: number }> }
    expect(cache.version).toBe(1)
    expect(cache.context).toBe('account:42')
    expect(Object.keys(cache.sections).sort()).toEqual([...PUBLIC].sort())
    expect(cache.sections.top_hot.fetchedAt).toBe(NOW)
  })

  it('shows fresh cached sections on first refresh and only requests the rest', async () => {
    storage.setItem(CACHE_KEY, JSON.stringify({
      version: 1,
      context: 'account:1',
      sections: {
        personal_radar: { fetchedAt: NOW - 1000, songs: [{ id: 'netease:5', title: 'Cached song', artist: 'A', album: 'B', coverUrl: '', durationMs: 1000 }], playlists: [] },
        personalized: { fetchedAt: NOW - 1000, songs: [], playlists: [{ id: '9', name: 'Cached list', coverUrl: '', trackCount: 1, playCount: 2 }] },
        top_hot: { fetchedAt: NOW - TTL_MS, songs: [{ id: 'netease:6', title: 'Expired' }], playlists: [] },
        top_new: { fetchedAt: NOW + 60_000, songs: [{ id: 'netease:7', title: 'From the future' }], playlists: [] },
        daily_recommend: { fetchedAt: NOW - 1000, songs: [{ id: 'netease:8', title: 'Private' }], playlists: [] },
      },
    }))
    manual()
    const store = useHomeFeedStore()

    void store.refresh(true, '1')
    await flushPromises()

    expect(firstTitle(store, 'personal_radar')).toBe('Cached song')
    expect(store.sections.personal_radar.songs[0].id).toBe('netease:5')
    expect(firstTitle(store, 'personalized')).toBe('Cached list')
    expect(store.sections.top_hot.songs).toEqual([])
    expect(store.sections.daily_recommend.songs).toEqual([])
    while (pending.length) {
      const { source, response } = pending.shift()!
      response.resolve(sectionResponse(source))
      await flushPromises()
    }
    const fetched = requested()
    expect(fetched).not.toContain('personal_radar')
    expect(fetched).not.toContain('personalized')
    expect(fetched).toEqual(expect.arrayContaining(['top_hot', 'top_new', 'daily_recommend']))
    expect(fetched).toHaveLength(ALL.length - 2)
  })

  it('ignores a cache written for another account or format version', async () => {
    const sections = { personal_radar: { fetchedAt: NOW, songs: [{ id: 'netease:5', title: 'Other account' }], playlists: [] } }
    storage.setItem(CACHE_KEY, JSON.stringify({ version: 1, context: 'account:other', sections }))
    const store = useHomeFeedStore()

    await store.refresh(true, 'me')
    expect(requested()).toContain('personal_radar')
    expect(firstTitle(store, 'personal_radar')).toBe('personal_radar song')

    setActivePinia(createPinia())
    storage.setItem(CACHE_KEY, JSON.stringify({ version: 2, context: 'guest', sections }))
    const fresh = useHomeFeedStore()
    const loading = fresh.refresh(false, '')
    expect(firstTitle(fresh, 'personal_radar')).toBeUndefined()
    await loading
  })

  it('refetches only failed or expired sections when the same account returns', async () => {
    respond = (source) => {
      if (source === 'top_hot') throw new Error('api_code=-460')
      return sectionResponse(source)
    }
    const store = useHomeFeedStore()
    await store.refresh(false, '')
    expect(store.sections.top_hot).toMatchObject({ error: 'api_code=-460', loading: false })

    respond = source => sectionResponse(source, `${source} again`)
    router.calls.length = 0
    vi.setSystemTime(NOW + 1000)
    await store.refresh(false, '')
    expect(requested()).toEqual(['top_hot'])
    expect(store.sections.top_hot.error).toBeNull()
    expect(firstTitle(store, 'personal_radar')).toBe('personal_radar song')

    router.calls.length = 0
    vi.setSystemTime(NOW + TTL_MS)
    await store.refresh(false, '')
    expect([...requested()].sort()).toEqual([...PUBLIC].filter(source => source !== 'top_hot').sort())

    router.calls.length = 0
    await store.refresh(false, '', true)
    expect([...requested()].sort()).toEqual([...PUBLIC].sort())
  })

  it('drops responses that belong to the previous account after a login change', async () => {
    manual()
    const store = useHomeFeedStore()
    const guestRefresh = store.refresh(false, '')
    await flushPromises()
    const oldRequests = pending.splice(0)
    expect(oldRequests).toHaveLength(3)

    void store.refresh(true, 'u1')
    await guestRefresh
    // 旧请求无法取消，仍占着并发名额，新账号的请求要等它们返回
    await flushPromises()
    expect(pending).toHaveLength(0)

    oldRequests[0].response.resolve(sectionResponse(oldRequests[0].source, 'guest'))
    oldRequests[1].response.reject(new Error('guest failure'))
    oldRequests[2].response.resolve(sectionResponse(oldRequests[2].source, 'guest'))
    await flushPromises()

    for (const { source } of oldRequests) {
      expect(firstTitle(store, source)).toBeUndefined()
      expect(store.sections[source].error).toBeNull()
    }
    expect(pending).toHaveLength(3)
    expect(store.sections.daily_recommend.loading).toBe(true)
  })

  describe('retry', () => {
    it('re-requests one section and shares an in-flight retry', async () => {
      respond = (source) => {
        if (source === 'top_new') throw new Error('timeout')
        return sectionResponse(source)
      }
      const store = useHomeFeedStore()
      await store.refresh(false, '')
      router.calls.length = 0
      manual()

      const first = store.retry('top_new')
      const second = store.retry('top_new')
      await flushPromises()
      expect(requested()).toEqual(['top_new'])
      expect(store.sections.top_new).toMatchObject({ loading: true, error: null })

      pending[0].response.resolve(sectionResponse('top_new', 'retried'))
      await Promise.all([first, second])
      expect(firstTitle(store, 'top_new')).toBe('retried song')
    })

    it('does nothing before the page is active or for private sections while logged out', async () => {
      const store = useHomeFeedStore()
      await store.retry('top_new')
      expect(requested()).toEqual([])

      await store.refresh(false, '')
      router.calls.length = 0
      await store.retry('daily_recommend')
      expect(requested()).toEqual([])
    })
  })

  describe('deactivate', () => {
    it('pauses queued requests while away and resumes them without duplicates', async () => {
      manual()
      const store = useHomeFeedStore()
      void store.refresh(false, '')
      await flushPromises()

      store.deactivate()
      for (const { source, response } of pending.splice(0)) response.resolve(sectionResponse(source))
      await flushPromises()

      expect(requested()).toEqual(['personal_radar', 'radar_playlists', 'top_soaring'])
      // 离开首页前发出的请求仍然生效
      expect(firstTitle(store, 'personal_radar')).toBe('personal_radar song')
      expect(store.sections.top_hot.loading).toBe(true)

      const resumed = store.refresh(false, '')
      await flushPromises()
      while (pending.length) {
        const { source, response } = pending.shift()!
        response.resolve(sectionResponse(source))
        await flushPromises()
      }
      await resumed
      expect([...requested()].sort()).toEqual([...PUBLIC].sort())
    })

    it('clears everything when the account changes while away', async () => {
      const store = useHomeFeedStore()
      await store.refresh(false, '')
      expect(firstTitle(store, 'top_hot')).toBe('top_hot song')

      store.deactivate(true, 'u2')
      expect(firstTitle(store, 'top_hot')).toBeUndefined()

      router.calls.length = 0
      await store.refresh(true, 'u2')
      expect([...requested()].sort()).toEqual([...ALL].sort())
    })

    it('keeps content when leaving with the same account', async () => {
      const store = useHomeFeedStore()
      await store.refresh(false, '')

      store.deactivate(false)

      expect(firstTitle(store, 'top_hot')).toBe('top_hot song')
    })
  })
})
