import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import type { LyricLine, TrackInfo } from '@/types/track'
import { lyricSourceOf, rememberLyricSource } from '@/modules/lyrics/lyricSource'
import { useCurrentLyricsStore } from '@/stores/currentLyrics'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

interface Fetched { lines: LyricLine[]; source: string }

interface PlayerStub {
  hasPlaybackSession: boolean
  currentTrack: TrackInfo | null
  isLoadingAudio: boolean
  loadedPlaybackTrackKey: string | null
  durationMs: number
  downloadedTrackForPlayback: (track: TrackInfo) => { filePath: string } | null
}

interface SettingsStub {
  defaultLyricSource: string
  preferWordTimedLyrics: boolean
  biliCcSubtitleEnabled: boolean
  biliCcSubtitleLanguage: string
  biliCcSubtitleAiFallback: boolean
}

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  player: null as PlayerStub | null,
  settings: null as SettingsStub | null,
  cached: [] as Array<(track: TrackInfo) => LyricLine[] | null>,
  saved: [] as Array<[string, string[]]>,
  automatic: null as null | ((track: TrackInfo) => Promise<{ lines: LyricLine[]; source: string }>),
  preferred: null as null | ((track: TrackInfo, source: string) => Promise<{ lines: LyricLine[]; source: string } | null>),
  subtitles: null as null | ((track: TrackInfo) => Promise<{ lines: LyricLine[]; source: string } | null>),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/player', () => ({ usePlayerStore: () => mocks.player }))
vi.mock('@/stores/settings', () => ({ useSettingsStore: () => mocks.settings }))
vi.mock('@/modules/lyrics/lyricsCache', () => ({
  // 依次消费排好的缓存应答，用完后回落到最后一个
  getCachedLyrics: async (track: TrackInfo) => {
    const next = mocks.cached.length > 1 ? mocks.cached.shift()! : mocks.cached[0]
    return next ? next(track) : null
  },
  saveCachedLyrics: async (track: TrackInfo, lines: LyricLine[]) => {
    mocks.saved.push([track.id, lines.map(line => line.text)])
  },
}))
vi.mock('@/modules/lyrics/lyricsRequest', () => ({
  hasWordTimedLyrics: (lines: LyricLine[]) => lines.some(line => line.words.length > 0),
  loadLyricsSingleFlight: (_track: TrackInfo, load: () => Promise<unknown>) => load(),
}))
vi.mock('@/modules/lyrics/lyricsFetch', () => ({
  fetchAutomaticLyrics: (track: TrackInfo) => mocks.automatic!(track),
  fetchPreferredSourceLyrics: (track: TrackInfo, source: string) => mocks.preferred!(track, source),
  fetchNeteaseRomanization: async () => null,
  fetchWordTimedLyrics: async () => ({ lines: [], source: 'amll_ttml' }),
  preferredLyricMatchSource: (_kind: string | null, preference: string) => (preference === 'automatic' ? null : preference),
}))
vi.mock('@/modules/lyrics/biliSubtitleLoader', async () => {
  const { ref } = await import('vue')
  return {
    biliSubtitleRevision: ref(0),
    loadBiliSubtitleLyrics: (track: TrackInfo) => mocks.subtitles!(track),
  }
})

function line(text: string, startMs = 0): LyricLine {
  return { startMs, durationMs: 1000, words: [], text }
}

function lines(...texts: string[]): LyricLine[] {
  return texts.map((text, index) => line(text, index * 1000))
}

function track(id: string, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return { id, title: `title ${id}`, artist: 'artist', album: '', durationMs: 180_000, coverUrl: '', audioUrl: '', ...overrides }
}

describe('currentLyrics store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let player: PlayerStub
  let settings: SettingsStub
  const touched: TrackInfo[] = []

  const texts = (store: ReturnType<typeof useCurrentLyricsStore>) => store.lines.map(item => item.text)

  function playing(target: TrackInfo) {
    touched.push(target)
    player.currentTrack = target
  }

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    player = reactive<PlayerStub>({
      hasPlaybackSession: true,
      currentTrack: null,
      isLoadingAudio: false,
      loadedPlaybackTrackKey: null,
      durationMs: 0,
      downloadedTrackForPlayback: () => null,
    })
    mocks.player = player
    settings = reactive<SettingsStub>({
      defaultLyricSource: 'automatic', preferWordTimedLyrics: false, biliCcSubtitleEnabled: true, biliCcSubtitleLanguage: '', biliCcSubtitleAiFallback: true,
    })
    mocks.settings = settings
    mocks.cached = []
    mocks.saved = []
    mocks.automatic = async target => ({ lines: lines(`auto ${target.id}`), source: 'netease' })
    mocks.preferred = async () => null
    mocks.subtitles = async () => null
    setActivePinia(createPinia())
  })

  afterEach(() => {
    for (const target of touched.splice(0)) rememberLyricSource(target, null)
  })

  it('does not fetch anything until a view acquires the lyrics', async () => {
    const fetchSpy = vi.fn(mocks.automatic!)
    mocks.automatic = fetchSpy
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()
    await flushPromises()
    expect(fetchSpy).not.toHaveBeenCalled()

    store.acquire()
    await flushPromises()

    expect(texts(store)).toEqual(['auto netease:1'])
    expect(store.loading).toBe(false)
    expect(mocks.saved).toEqual([['netease:1', ['auto netease:1']]])
    expect(lyricSourceOf(track('netease:1'))).toBe('netease')
  })

  it('stays empty without a playback session', async () => {
    player.hasPlaybackSession = false
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()

    expect(texts(store)).toEqual([])
    expect(store.loading).toBe(false)
    expect(mocks.saved).toEqual([])
  })

  it('discards lyrics that arrive for a track that is no longer playing', async () => {
    const slow = deferred<Fetched>()
    mocks.automatic = async target => (target.id === 'netease:1' ? slow.promise : { lines: lines('second'), source: 'qq' })
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()
    store.acquire()
    await flushPromises()

    playing(track('netease:2'))
    await flushPromises()
    slow.resolve({ lines: lines('first'), source: 'netease' })
    await flushPromises()

    expect(texts(store)).toEqual(['second'])
    expect(mocks.saved.map(([id]) => id)).toEqual(['netease:2'])
    expect(lyricSourceOf(track('netease:1'))).toBeNull()
  })

  it('shows cached lyrics first and lets lyrics from the sync payload replace them', async () => {
    const parsed = deferred<unknown>()
    router.on('parse_lrc_content', () => parsed.promise)
    mocks.cached = [() => lines('cached')]
    playing(track('netease:1', { syncPayload: { matchedLyric: '[00:01.00]synced', matchedLyricSource: 'qq' } }))
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()
    expect(texts(store)).toEqual(['cached'])
    expect(store.loading).toBe(true)

    parsed.resolve([{ start_ms: 1000, duration_ms: 500, text: 'synced', words: [] }])
    await flushPromises()

    expect(router.callsOf('parse_lrc_content')[0].args).toEqual({ content: '[00:01.00]synced', title: 'title netease:1', artist: 'artist' })
    expect(texts(store)).toEqual(['synced'])
    expect(store.lines[0].startMs).toBe(1000)
    expect(store.loading).toBe(false)
    expect(lyricSourceOf(player.currentTrack)).toBe('qq')
  })

  it('keeps lyrics that the user deliberately cleared', async () => {
    const fetchSpy = vi.fn(mocks.automatic!)
    mocks.automatic = fetchSpy
    mocks.cached = [() => lines('cached')]
    playing(track('netease:1', { syncPayload: { matchedLyric: '   ' } }))
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()

    expect(texts(store)).toEqual([])
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  it('treats a cached local edit as final', async () => {
    const fetchSpy = vi.fn(mocks.automatic!)
    mocks.automatic = fetchSpy
    mocks.cached = [() => lines('my edit')]
    const target = track('netease:1')
    playing(target)
    rememberLyricSource(target, 'local_edit')
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()

    expect(texts(store)).toEqual(['my edit'])
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  it('prefers the configured lyric source and reloads when that setting changes', async () => {
    mocks.preferred = async (_target, source) => ({ lines: lines(`from ${source}`), source })
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()
    store.acquire()
    await flushPromises()
    expect(texts(store)).toEqual(['auto netease:1'])

    settings.defaultLyricSource = 'qq'
    await flushPromises()

    expect(texts(store)).toEqual(['from qq'])
    expect(lyricSourceOf(player.currentTrack)).toBe('qq')
  })

  it('uses Bilibili CC subtitles without caching them as music lyrics', async () => {
    mocks.subtitles = async () => ({ lines: lines('作词 : someone', 'subtitle line'), source: 'bilibili_cc' })
    playing(track('bilibili:BV1abc', { source: 'bilibili' }))
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()

    expect(store.subtitle).not.toBeNull()
    // 字幕原样显示，不做作词作曲行过滤
    expect(texts(store)).toEqual(['作词 : someone', 'subtitle line'])
    expect(mocks.saved).toEqual([])
  })

  it('hides credit lines from music lyrics', async () => {
    mocks.automatic = async () => ({ lines: lines('作词 : someone', 'real line'), source: 'netease' })
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()

    expect(texts(store)).toEqual(['real line'])
  })

  it('falls back to whatever the cache holds when fetching fails', async () => {
    mocks.cached = [() => null, () => lines('saved by another window')]
    mocks.automatic = async () => { throw new Error('network down') }
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()

    store.acquire()
    await flushPromises()

    expect(texts(store)).toEqual(['saved by another window'])
    expect(store.loading).toBe(false)
  })

  it('lets an edit replace lyrics that are still loading', async () => {
    const slow = deferred<Fetched>()
    mocks.automatic = () => slow.promise
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()
    store.acquire()
    await flushPromises()

    store.replace(lines('edited'))
    expect(store.loading).toBe(false)
    slow.resolve({ lines: lines('fetched'), source: 'netease' })
    await flushPromises()

    expect(texts(store)).toEqual(['edited'])
    expect(mocks.saved).toEqual([])
  })

  it('stops fetching after the last view releases, and catches up on the next acquire', async () => {
    const fetchSpy = vi.fn(mocks.automatic!)
    mocks.automatic = fetchSpy
    playing(track('netease:1'))
    const store = useCurrentLyricsStore()
    const releaseA = store.acquire()
    const releaseB = store.acquire()
    await flushPromises()
    expect(fetchSpy).toHaveBeenCalledTimes(1)

    releaseA()
    releaseA()
    playing(track('netease:2'))
    await flushPromises()
    expect(fetchSpy).toHaveBeenCalledTimes(2)

    releaseB()
    playing(track('netease:3'))
    await flushPromises()
    expect(fetchSpy).toHaveBeenCalledTimes(2)
    expect(texts(store)).toEqual([])

    store.acquire()
    await flushPromises()
    expect(fetchSpy).toHaveBeenCalledTimes(3)
    expect(texts(store)).toEqual(['auto netease:3'])
  })
})
