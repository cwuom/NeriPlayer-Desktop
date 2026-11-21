import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useExploreSearchStore, type ExploreItem, type ExploreSong } from '@/stores/exploreSearch'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))

function song(id: string): ExploreSong {
  return { kind: 'song', id, title: `title ${id}`, artist: 'a', album: '', duration_ms: 1000, source: 'netease', cover_url: null }
}

function page(items: ExploreItem[], hasMore: boolean) {
  return { items, has_more: hasMore }
}

const ids = (items: ExploreItem[]) => items.map(item => (item.kind === 'notice' ? `notice:${item.reason}` : item.id))

describe('exploreSearch store', () => {
  let router: ReturnType<typeof createInvokeRouter>

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    setActivePinia(createPinia())
  })

  describe('search', () => {
    it('trims the query, requests the first page and exposes paging state', async () => {
      router.on('explore_search', () => page([song('1'), song('2')], true))
      const store = useExploreSearchStore()

      const pending = store.search('netease', 'songs', '  hello  ')
      expect(store.isSearching).toBe(true)
      await pending

      expect(router.callsOf('explore_search').map(call => call.args)).toEqual([
        { platform: 'netease', kind: 'songs', query: 'hello', page: 1 },
      ])
      expect(ids(store.items)).toEqual(['1', '2'])
      expect(store.hasMore).toBe(true)
      expect(store.isSearching).toBe(false)
      expect(store.error).toBeNull()
    })

    it('clears results without any request for a blank query', async () => {
      router.on('explore_search', () => page([song('1')], true))
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'first')

      await store.search('netease', 'songs', '   ')

      expect(router.callsOf('explore_search')).toHaveLength(1)
      expect(store.items).toEqual([])
      expect(store.hasMore).toBe(false)
      expect(store.isSearching).toBe(false)
    })

    it('resolves share links through a dedicated command and never pages them', async () => {
      router.on('resolve_share_link', args => ({ kind: 'notice', reason: `resolved:${String(args?.text)}` }))
      const store = useExploreSearchStore()

      await store.search('link', 'songs', ' https://b23.tv/abc ')

      expect(router.callsOf('resolve_share_link').map(call => call.args)).toEqual([{ text: 'https://b23.tv/abc' }])
      expect(ids(store.items)).toEqual(['notice:resolved:https://b23.tv/abc'])
      expect(store.hasMore).toBe(false)

      await store.loadMore('link', 'songs', ' https://b23.tv/abc ')
      expect(router.calls.map(call => call.command)).toEqual(['resolve_share_link'])
    })

    it('replaces stale results with an error message when the request fails', async () => {
      router.on('explore_search', () => page([song('1')], true), () => { throw 'NETWORK_DOWN' })
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'first')

      await store.search('netease', 'songs', 'second')

      expect(store.items).toEqual([])
      expect(store.error).toBe('NETWORK_DOWN')
      expect(store.isSearching).toBe(false)
      expect(store.hasMore).toBe(false)
    })

    it('drops a slower response from an older query', async () => {
      const slow = deferred<unknown>()
      router.on('explore_search', () => slow.promise, () => page([song('new')], false))
      const store = useExploreSearchStore()

      const older = store.search('netease', 'songs', 'old')
      await store.search('youtube', 'videos', 'new')
      slow.resolve(page([song('old')], true))
      await older

      expect(ids(store.items)).toEqual(['new'])
      expect(store.hasMore).toBe(false)
      expect(store.isSearching).toBe(false)
    })

    it('keeps the newer request loading when an older request fails late', async () => {
      const slow = deferred<unknown>()
      const newer = deferred<unknown>()
      router.on('explore_search', () => slow.promise, () => newer.promise)
      const store = useExploreSearchStore()

      const older = store.search('netease', 'songs', 'old')
      const latest = store.search('netease', 'songs', 'new')
      slow.reject('old failed')
      await older

      expect(store.error).toBeNull()
      expect(store.isSearching).toBe(true)

      newer.resolve(page([song('new')], false))
      await latest
      expect(ids(store.items)).toEqual(['new'])
      expect(store.isSearching).toBe(false)
    })

    it('ignores a response that arrives after clear()', async () => {
      const slow = deferred<unknown>()
      router.on('explore_search', () => slow.promise)
      const store = useExploreSearchStore()

      const pending = store.search('netease', 'songs', 'hello')
      store.clear()
      slow.resolve(page([song('1')], true))
      await pending

      expect(store.items).toEqual([])
      expect(store.hasMore).toBe(false)
      expect(store.isSearching).toBe(false)
    })
  })

  describe('loadMore', () => {
    it('appends the next page and skips items that are already listed', async () => {
      router.on('explore_search',
        () => page([song('1'), song('2')], true),
        () => page([song('2'), song('3')], true),
        () => page([song('4')], false))
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'q')

      await store.loadMore('netease', 'songs', 'q')
      expect(ids(store.items)).toEqual(['1', '2', '3'])
      expect(store.hasMore).toBe(true)

      await store.loadMore('netease', 'songs', 'q')
      expect(ids(store.items)).toEqual(['1', '2', '3', '4'])
      expect(store.hasMore).toBe(false)
      expect(router.callsOf('explore_search').map(call => call.args?.page)).toEqual([1, 2, 3])
    })

    it('stops paging when a page comes back empty even if the server claims more', async () => {
      router.on('explore_search', () => page([song('1')], true), () => page([], true))
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'q')

      await store.loadMore('netease', 'songs', 'q')

      expect(store.hasMore).toBe(false)
      expect(ids(store.items)).toEqual(['1'])
    })

    it('does nothing for a different query, platform or kind than the current results', async () => {
      router.on('explore_search', () => page([song('1')], true))
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'q')

      await store.loadMore('netease', 'songs', 'other')
      await store.loadMore('youtube', 'songs', 'q')
      await store.loadMore('netease', 'playlists', 'q')

      expect(router.callsOf('explore_search')).toHaveLength(1)
    })

    it('does not start a second page request while one is in flight', async () => {
      const more = deferred<unknown>()
      router.on('explore_search', () => page([song('1')], true), () => more.promise)
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'q')

      const first = store.loadMore('netease', 'songs', 'q')
      expect(store.isLoadingMore).toBe(true)
      await store.loadMore('netease', 'songs', 'q')
      more.resolve(page([song('2')], false))
      await first

      expect(router.callsOf('explore_search')).toHaveLength(2)
      expect(store.isLoadingMore).toBe(false)
    })

    it('discards a next page that returns after a new search started', async () => {
      const more = deferred<unknown>()
      const fresh = deferred<unknown>()
      router.on('explore_search', () => page([song('1')], true), () => more.promise, () => fresh.promise)
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'q')

      const loading = store.loadMore('netease', 'songs', 'q')
      const searching = store.search('netease', 'songs', 'q2')
      more.resolve(page([song('stale')], false))
      await loading
      expect(store.items.map(item => (item.kind === 'song' ? item.id : ''))).not.toContain('stale')
      expect(store.isLoadingMore).toBe(false)

      fresh.resolve(page([song('a')], true))
      await searching
      expect(ids(store.items)).toEqual(['a'])
      expect(store.hasMore).toBe(true)
    })

    it('stops paging after a failed next page but keeps what is already shown', async () => {
      router.on('explore_search', () => page([song('1')], true), () => { throw new Error('timeout') })
      const store = useExploreSearchStore()
      await store.search('netease', 'songs', 'q')

      await store.loadMore('netease', 'songs', 'q')
      await flushPromises()

      expect(ids(store.items)).toEqual(['1'])
      expect(store.hasMore).toBe(false)
      expect(store.isLoadingMore).toBe(false)
      expect(store.error).toBeNull()
    })
  })
})
