import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TrackInfo } from '@/types/track'
import type { BiliVideoInfo } from '@/modules/playback/biliVideoParts'
import { useBiliVideoPartsStore, type BiliCollectionIdentity } from '@/stores/biliVideoParts'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  player: { currentTrack: null as unknown, playAllCalls: [] as Array<[string[], string]> },
  restriction: null as string | null,
  toasts: [] as string[],
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/i18n', () => ({ default: { global: { t: (key: string) => key } } }))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({ error: (text: string) => { mocks.toasts.push(text) } }),
}))
vi.mock('@/stores/listenTogether', () => ({
  useListenTogetherStore: () => ({ localControlRestriction: mocks.restriction }),
}))
vi.mock('@/stores/player', () => ({
  usePlayerStore: () => ({
    get currentTrack() { return mocks.player.currentTrack },
    playAll: (tracks: TrackInfo[], startId: string) => { mocks.player.playAllCalls.push([tracks.map(item => item.id), startId]) },
  }),
}))

const NAME_ORDER_KEY = 'neri:bili-part-name-order:v1:BV1abc'

function track(id: string, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return { id, title: `title ${id}`, artist: 'artist', album: '', durationMs: 1000, coverUrl: '', audioUrl: '', ...overrides }
}

const wholeVideo = () => track('bilibili:BV1abc', { source: 'bilibili', title: 'Concert', coverUrl: 'https://i0.hdslb.com/parent.jpg' })
const part = (cid: number, overrides: Partial<TrackInfo> = {}) =>
  track(`bilibili:170001:${cid}`, { album: `Bilibili|${cid}|BV1abc`, source: 'bilibili', ...overrides })

function detail(pages: BiliVideoInfo['pages']): BiliVideoInfo {
  return { bvid: 'BV1abc', aid: 170001, title: 'Concert', owner: 'Uploader', cover: 'https://i0.hdslb.com/video.jpg', cid: pages[0].cid, duration: 300, pages }
}

const twoParts = () => detail([
  { cid: 200, page: 1, part: '1. Singer - Opening', duration: 100 },
  { cid: 300, page: 2, part: 'Encore', duration: 200 },
])

describe('biliVideoParts store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage
  let played: number

  const play = () => { played++ }

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    mocks.player = { currentTrack: null, playAllCalls: [] }
    mocks.restriction = null
    mocks.toasts = []
    played = 0
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  describe('request (play)', () => {
    it('plays tracks that are not whole Bilibili videos right away', async () => {
      const parts = useBiliVideoPartsStore()

      await parts.request(track('netease:1'), play)
      await parts.request(part(200), play)

      expect(played).toBe(2)
      expect(router.calls).toEqual([])
      expect(parts.open).toBe(false)
    })

    it('offers the parts of a multi-part video and plays the chosen one with the full queue', async () => {
      router.on('get_bili_video_info', () => twoParts())
      const parts = useBiliVideoPartsStore()

      await parts.request(wholeVideo(), play)

      expect(router.callsOf('get_bili_video_info')[0].args).toEqual({ bvid: 'BV1abc', avid: null })
      expect(parts.open).toBe(true)
      expect(parts.tracks.map(item => [item.id, item.title, item.artist, item.coverUrl])).toEqual([
        ['bilibili:170001:200', 'Singer', 'Opening', 'https://i0.hdslb.com/parent.jpg'],
        ['bilibili:170001:300', 'Encore', 'Uploader', 'https://i0.hdslb.com/parent.jpg'],
      ])
      expect(played).toBe(0)

      parts.nameOrder = 'artist-first'
      expect(parts.tracks[0].title).toBe('Opening')
      parts.choose(300)

      expect(mocks.player.playAllCalls).toEqual([[['bilibili:170001:200', 'bilibili:170001:300'], 'bilibili:170001:300']])
      expect(storage.getItem(NAME_ORDER_KEY)).toBe('artist-first')
      expect(parts.open).toBe(false)
      expect(parts.nameOrder).toBe('title-first')
    })

    it('restores the remembered name order and forgets it when switched back', async () => {
      storage.setItem(NAME_ORDER_KEY, 'artist-first')
      router.on('get_bili_video_info', () => twoParts())
      const parts = useBiliVideoPartsStore()
      const chosen: string[] = []

      await parts.request(wholeVideo(), play, (_tracks, startId) => { chosen.push(startId) })
      expect(parts.nameOrder).toBe('artist-first')

      parts.nameOrder = 'title-first'
      parts.choose(200)
      expect(chosen).toEqual(['bilibili:170001:200'])
      expect(storage.getItem(NAME_ORDER_KEY)).toBeNull()
    })

    it('ignores a part that is not in the list', async () => {
      router.on('get_bili_video_info', () => twoParts())
      const parts = useBiliVideoPartsStore()
      await parts.request(wholeVideo(), play)

      parts.choose(999)

      expect(parts.open).toBe(true)
      expect(mocks.player.playAllCalls).toEqual([])
    })

    it('plays single-part videos directly', async () => {
      router.on('get_bili_video_info', () => detail([{ cid: 200, page: 1, part: 'Only', duration: 100 }]))
      const parts = useBiliVideoPartsStore()

      await parts.request(wholeVideo(), play)

      expect(played).toBe(1)
      expect(parts.open).toBe(false)
    })

    it('falls back to normal playback when the details cannot be loaded', async () => {
      router.on('get_bili_video_info', () => ({ ...twoParts(), pages: [] }))
      const parts = useBiliVideoPartsStore()

      await parts.request(wholeVideo(), play)

      expect(played).toBe(1)
    })

    it('does not play a failed lookup after the user moved to another track', async () => {
      const response = deferred<unknown>()
      router.on('get_bili_video_info', () => response.promise)
      const parts = useBiliVideoPartsStore()

      const pending = parts.request(wholeVideo(), play)
      mocks.player.currentTrack = track('netease:2')
      response.reject(new Error('timeout'))
      await pending

      expect(played).toBe(0)
    })

    it('drops details that arrive after the picker was closed', async () => {
      const response = deferred<unknown>()
      router.on('get_bili_video_info', () => response.promise)
      const parts = useBiliVideoPartsStore()

      const pending = parts.request(wholeVideo(), play)
      parts.close()
      response.resolve(twoParts())
      await pending

      expect(parts.open).toBe(false)
      expect(played).toBe(0)
    })

    it('drops details from an older request when a newer one started', async () => {
      const older = deferred<unknown>()
      router.on('get_bili_video_info', () => older.promise, () => detail([{ cid: 900, page: 1, part: 'Other', duration: 1 }]))
      const parts = useBiliVideoPartsStore()

      const first = parts.request(wholeVideo(), play)
      await parts.request(track('bilibili:BV9zzz', { source: 'bilibili' }), play)
      older.resolve(twoParts())
      await first

      expect(parts.open).toBe(false)
      expect(played).toBe(1)
    })

    it('refuses to change playback while a listen-together room forbids it', async () => {
      mocks.restriction = 'member_control'
      const parts = useBiliVideoPartsStore()

      await parts.request(wholeVideo(), play)
      mocks.restriction = 'controller_offline'
      await parts.request(part(200), play)
      await parts.request(track('netease:1'), play)

      expect(mocks.toasts).toEqual([
        'listen_together.control_blocked_member_control',
        'listen_together.control_blocked_controller_offline',
      ])
      expect(played).toBe(1)
      expect(router.calls).toEqual([])
    })

    it('closes the picker instead of playing if the room starts forbidding it', async () => {
      router.on('get_bili_video_info', () => twoParts())
      const parts = useBiliVideoPartsStore()
      await parts.request(wholeVideo(), play)

      mocks.restriction = 'member_control'
      parts.choose(200)

      expect(parts.open).toBe(false)
      expect(mocks.player.playAllCalls).toEqual([])
    })
  })

  describe('requestCollection (save)', () => {
    let saves: Array<{ track: TrackInfo; identity?: BiliCollectionIdentity }>
    const save = (target: TrackInfo, identity?: BiliCollectionIdentity) => { saves.push({ track: target, identity }) }

    beforeEach(() => {
      saves = []
    })

    it('saves non-Bilibili tracks as they are', async () => {
      const parts = useBiliVideoPartsStore()

      await parts.requestCollection(track('qq:1'), save)

      expect(saves).toEqual([{ track: track('qq:1'), identity: undefined }])
    })

    it('lets the user save either the whole video or the selected part', async () => {
      router.on('get_bili_video_info', () => twoParts())
      const parts = useBiliVideoPartsStore()
      const selected = part(300, { title: 'My encore', artist: '', coverUrl: '', durationMs: 0, syncPayload: { note: 'kept' } })
      const likedChecks: Array<[string, string | null]> = []

      await parts.requestCollection(selected, save, (target, video) => {
        likedChecks.push([target.id, video.bvid])
        return false
      })

      expect(parts.open).toBe(true)
      expect(parts.collection?.partNumber).toBe(2)
      expect(parts.collection?.whole).toMatchObject({ id: 'bilibili:BV1abc', title: 'Concert', artist: 'Uploader', durationMs: 300_000 })
      expect(parts.collection?.part).toMatchObject({
        id: 'bilibili:170001:300', title: 'My encore', artist: 'Uploader', durationMs: 200_000, album: 'Bilibili|300|BV1abc',
        coverUrl: 'https://i0.hdslb.com/video.jpg',
        syncPayload: { channelId: 'bilibili', subAudioId: '300', bvid: 'BV1abc', note: 'kept', album: 'Bilibili|300|BV1abc' },
      })
      parts.collection?.isLiked?.(parts.collection.part)
      expect(likedChecks).toEqual([['bilibili:170001:300', 'BV1abc']])

      parts.chooseCollection('whole')
      expect(saves).toHaveLength(1)
      expect(saves[0].track.id).toBe('bilibili:BV1abc')
      expect(saves[0].identity).toEqual({ video: { bvid: 'BV1abc', avid: 170001 } })
      expect(parts.open).toBe(false)
    })

    it('saves a single-part video with its CID and allows matching a whole-video favorite', async () => {
      router.on('get_bili_video_info', () => detail([{ cid: 200, page: 1, part: 'Only', duration: 100 }]))
      const parts = useBiliVideoPartsStore()

      await parts.requestCollection(wholeVideo(), save)

      expect(saves).toEqual([{
        track: wholeVideo(),
        identity: { video: { bvid: 'BV1abc', avid: 170001 }, allowWholeFavorite: true, singlePageCid: 200 },
      }])
    })

    it('still saves with the local identity when details are unavailable', async () => {
      router.on('get_bili_video_info', () => { throw new Error('offline') })
      const parts = useBiliVideoPartsStore()

      await parts.requestCollection(part(300), save)

      expect(saves).toEqual([{
        track: part(300),
        identity: { video: { bvid: 'BV1abc', avid: 170001 }, allowWholeFavorite: true, singlePageCid: undefined },
      }])
    })

    it('reuses the video details already loaded by the part picker', async () => {
      router.on('get_bili_video_info', () => twoParts())
      const parts = useBiliVideoPartsStore()
      await parts.request(wholeVideo(), play)
      parts.close()

      await parts.requestCollection(part(200), save)

      expect(router.callsOf('get_bili_video_info')).toHaveLength(1)
      expect(parts.collection?.partNumber).toBe(1)
    })

    it('closes the choice when the caller aborts and ignores an already aborted request', async () => {
      const response = deferred<unknown>()
      router.on('get_bili_video_info', () => response.promise)
      const parts = useBiliVideoPartsStore()
      const aborted = new AbortController()
      aborted.abort()

      await parts.requestCollection(wholeVideo(), save, undefined, aborted.signal)
      expect(router.calls).toEqual([])

      const controller = new AbortController()
      const pending = parts.requestCollection(wholeVideo(), save, undefined, controller.signal)
      controller.abort()
      response.resolve(twoParts())
      await pending
      await flushPromises()

      expect(parts.open).toBe(false)
      expect(saves).toEqual([])
    })
  })
})
