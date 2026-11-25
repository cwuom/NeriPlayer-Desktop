import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  normalizeLyricDefaultOffset,
  normalizeLyricEmphasisThresholdMs,
  normalizePlaybackPitch,
  normalizePlaybackSpeed,
  normalizeVolumeBalance,
  useSettingsStore,
  type AppSettings,
} from '@/stores/settings'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => {
  // settings.ts 加载时就读 localStorage 推断语言；先注入空存储，免得碰到 Node 的实验性 localStorage 并告警
  vi.stubGlobal('localStorage', { getItem: () => null })
  return { router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null }
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))

const PERSIST_DEBOUNCE_MS = 180

function legacy(storage: MemoryStorage, key: string, value: unknown) {
  storage.setItem(`neri:${key}`, JSON.stringify(value))
}

describe('settings store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage
  let saved: AppSettings[]

  beforeEach(() => {
    vi.useFakeTimers()
    router = createInvokeRouter()
    mocks.router = router
    saved = []
    router.on('save_settings', (args) => {
      saved.push(args?.settings as AppSettings)
      return args?.settings
    })
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  /** 读入后 watcher 可能排一次保存，先让它跑完，后续断言只看本用例触发的写入 */
  async function settle() {
    await flushPromises()
    vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
    await flushPromises()
    saved = []
    router.calls.length = 0
  }

  describe('startup from legacy localStorage', () => {
    it('uses defaults when nothing is stored', () => {
      const settings = useSettingsStore()

      expect(settings.isHydrated).toBe(false)
      expect(settings.darkMode).toBe('dark')
      expect(settings.downloadParallelism).toBe(6)
      expect(settings.cloudMusicOffset).toBe(1000)
      expect(settings.equalizerBands).toEqual([0, 0, 0, 0, 0])
    })

    it('reads and normalizes legacy keys synchronously, ignoring corrupt ones', () => {
      legacy(storage, 'dark_mode', 'light')
      storage.setItem('theme-mode', 'system')
      legacy(storage, 'cloud_offset', 1234)
      storage.setItem('neri:qq_offset', '{broken')
      legacy(storage, 'download_parallelism', 99)
      storage.setItem('neri:player-state', JSON.stringify({ volume: 0.4 }))
      storage.setItem('locale', 'ja')

      const settings = useSettingsStore()

      expect(settings.darkMode).toBe('system')
      expect(settings.cloudMusicOffset).toBe(1250)
      expect(settings.qqMusicOffset).toBe(500)
      expect(settings.downloadParallelism).toBe(8)
      expect(settings.volume).toBe(0.4)
      expect(settings.locale).toBe('ja')
    })
  })

  describe('normalization', () => {
    function applied(input: Record<string, unknown>) {
      const settings = useSettingsStore()
      settings.applySnapshot(input)
      return settings.snapshot()
    }

    it('drops values of the wrong type or outside the allowed choices', () => {
      const result = applied({
        darkMode: 5, coverStyle: 'round', defaultScreen: 'nowhere', logLevel: 'verbose', locale: 'fr',
        localLibrarySortBy: 'random', qqMusicQuality: 'ultra', biliQuality: 'hires', themeColor: 'teal',
      })

      expect(result).toMatchObject({
        formatVersion: 1, darkMode: 'dark', coverStyle: 'card', defaultScreen: 'home', logLevel: 'info',
        localLibrarySortBy: 'default', qqMusicQuality: 'high', biliQuality: 'hires', themeColor: 'teal',
      })
      expect(['zh-CN', 'zh-TW', 'en', 'ja']).toContain(result.locale)
    })

    it('migrates renamed values from older versions', () => {
      expect(applied({ neteaseQuality: 'high', downloadNeteaseQuality: ' high ', themeColor: '#6750A4' })).toMatchObject({
        neteaseQuality: 'higher', downloadNeteaseQuality: 'higher', themeColor: 'purple',
      })
      expect(applied({ themeColor: '#123456' }).themeColor).toBe('purple')
      expect(applied({ youtubePlaybackSource: ' Vision_OS ' }).youtubePlaybackSource).toBe('visionos')
      expect(applied({ youtubePlaybackSource: 'androidvr' }).youtubePlaybackSource).toBe('android_vr')
      expect(applied({ youtubePlaybackSource: 'bogus' }).youtubePlaybackSource).toBe('automatic')
    })

    it('splits the old audio codec/spec toggles into the new per-field toggles', () => {
      expect(applied({ showAudioCodec: false, showAudioSpec: true })).toMatchObject({
        showAudioFormat: false, showAudioChannels: true, showAudioSampleRate: true, showAudioBitDepth: true,
      })
      expect(applied({ showAudioCodec: false, showAudioFormat: true }).showAudioFormat).toBe(true)
    })

    it('keeps the previous-restarts-track behaviour for old snapshots and accepts an explicit opt-out', () => {
      expect(applied({}).prevRestartsTrack).toBe(true)
      expect(applied({ prevRestartsTrack: 'false' }).prevRestartsTrack).toBe(true)
      expect(applied({ prevRestartsTrack: false }).prevRestartsTrack).toBe(false)
    })

    it('defaults the media cache directory to the system location and trims custom paths', () => {
      expect(applied({}).mediaCacheDir).toBe('')
      expect(applied({ mediaCacheDir: '  D:\\NeriCache  ' }).mediaCacheDir).toBe('D:\\NeriCache')
      expect(applied({ mediaCacheDir: null }).mediaCacheDir).toBe('')
    })

    it('folds the legacy crossfade switch into crossfadeNext with the old fade durations', () => {
      expect(applied({ crossfade: true, crossfadeNext: false, fadeInDuration: 800, fadeOutDuration: 900 })).toMatchObject({
        crossfade: false, crossfadeNext: true, crossfadeInDuration: 800, crossfadeOutDuration: 900,
      })
      expect(applied({ crossfade: true, crossfadeNext: true, fadeInDuration: 800, crossfadeInDuration: 300 })).toMatchObject({
        crossfade: false, crossfadeNext: true, crossfadeInDuration: 300,
      })
    })

    it('clamps and rounds numbers the Rust side stores as integers or bounded values', () => {
      const result = applied({
        downloadParallelism: 0, maxCacheSize: 300.6, volume: 2, lyricFontScale: 3, fadeInDuration: 99_999.4,
        enhancedAdvancedBlurRadius: 37, playbackSpeed: 1.03, playbackPitch: 9, loudnessGainMb: 2000.7,
        cloudMusicOffset: Number.NaN, qqMusicOffset: -7777, lyricEmphasisThresholdMs: 10, volumeBalance: -0.004,
      })

      expect(result).toMatchObject({
        downloadParallelism: 1, maxCacheSize: 301, volume: 1, lyricFontScale: 1.6, fadeInDuration: 10_000,
        enhancedAdvancedBlurRadius: 36, playbackSpeed: 1.05, playbackPitch: 2, loudnessGainMb: 1500,
        cloudMusicOffset: 1000, qqMusicOffset: -5000, lyricEmphasisThresholdMs: 300, volumeBalance: 0,
      })
      expect(Object.is(result.volumeBalance, -0)).toBe(false)
      expect(applied({ maxCacheSize: 10 }).maxCacheSize).toBe(256)
      expect(applied({ maxCacheSize: 1e9 }).maxCacheSize).toBe(512 * 1024)
    })

    it('normalizes text fields and equalizer bands', () => {
      const result = applied({
        downloadNameTemplate: '   ', downloadDir: '  /data/music  ', audioOutputDevice: ' Speakers ',
        biliCcSubtitleLanguage: ' zh-Hans ', equalizerBands: [2000, -2000, 3.4], equalizerEnabled: true,
      })

      expect(result).toMatchObject({
        downloadNameTemplate: '%title% - %artist% - %album% - %source%', downloadDir: '/data/music',
        audioOutputDevice: 'Speakers', biliCcSubtitleLanguage: 'zh-Hans', equalizerBands: [1500, -1500, 3, 0, 0],
        equalizerEnabled: false,
      })
      expect(applied({ biliCcSubtitleLanguage: 'zh hans!' }).biliCcSubtitleLanguage).toBe('')
      expect(applied({ equalizerBands: [1, 'x', 2] }).equalizerBands).toEqual([0, 0, 0, 0, 0])
    })

    it('copies arrays instead of sharing them with the caller', () => {
      const bands = [100, 200, 300, 400, 500]
      const settings = useSettingsStore()

      settings.applySnapshot({ equalizerBands: bands })
      bands[0] = 999

      expect(settings.equalizerBands[0]).toBe(100)
    })
  })

  describe('hydrate', () => {
    it('applies persisted settings and refreshes the legacy shadow without migrating', async () => {
      router.on('get_settings', () => ({ persisted: true, settings: { darkMode: 'light', downloadDir: '/music' } }))
      const settings = useSettingsStore()

      await settings.hydrate()

      expect(settings.isHydrated).toBe(true)
      expect(settings.darkMode).toBe('light')
      expect(settings.downloadDir).toBe('/music')
      expect(saved).toEqual([])
      expect(storage.getItem('theme-mode')).toBe('light')
      expect(storage.getItem('neri:download_dir')).toBe('"/music"')
    })

    it('migrates legacy localStorage values into the database on first launch', async () => {
      storage.setItem('theme-mode', 'light')
      legacy(storage, 'qq_offset', 250)
      router.on('get_settings', () => ({ persisted: false, settings: { darkMode: 'dark' } }))
      const settings = useSettingsStore()

      await settings.hydrate()

      expect(saved).toHaveLength(1)
      expect(saved[0]).toMatchObject({ darkMode: 'light', qqMusicOffset: 250, formatVersion: 1 })
      expect(settings.qqMusicOffset).toBe(250)
    })

    it('writes defaults once when nothing was ever persisted', async () => {
      router.on('get_settings', () => ({ persisted: false, settings: {} }))
      const settings = useSettingsStore()

      await settings.hydrate()

      expect(saved).toHaveLength(1)
      expect(saved[0].downloadParallelism).toBe(6)
    })

    it('falls back to legacy values when the backend is unavailable', async () => {
      legacy(storage, 'download_dir', '/legacy')
      router.on('get_settings', () => { throw new Error('no bridge') })
      const settings = useSettingsStore()

      await settings.hydrate()

      expect(settings.isHydrated).toBe(true)
      expect(settings.downloadDir).toBe('/legacy')
      expect(saved).toEqual([])
      expect(storage.getItem('theme-mode')).toBe('dark')
    })

    // 回归：applySnapshot 换掉对象型设置会触发 deep watch，读入后 180ms 曾再写一次未改动的设置
    it('does not write persisted settings back after the debounce', async () => {
      router.on('get_settings', () => ({
        persisted: true,
        settings: { darkMode: 'light', equalizerBands: [1, 2, 3, 4, 5] },
      }))
      const settings = useSettingsStore()

      await settings.hydrate()
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS * 2)
      await flushPromises()

      expect(router.callsOf('save_settings')).toHaveLength(0)
    })

    it('writes first-launch settings exactly once, including after the debounce', async () => {
      router.on('get_settings', () => ({ persisted: false, settings: {} }))
      const settings = useSettingsStore()

      await settings.hydrate()
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS * 2)
      await flushPromises()

      expect(router.callsOf('save_settings')).toHaveLength(1)
    })

    it('loads only once for concurrent callers', async () => {
      router.on('get_settings', () => ({ persisted: true, settings: {} }))
      const settings = useSettingsStore()

      await Promise.all([settings.hydrate(), settings.hydrate()])

      expect(router.callsOf('get_settings')).toHaveLength(1)
    })
  })

  describe('persistence', () => {
    async function hydrated() {
      router.on('get_settings', () => ({ persisted: true, settings: {} }))
      const settings = useSettingsStore()
      await settings.hydrate()
      await settle()
      return settings
    }

    it('does not save changes made before hydration', async () => {
      const settings = useSettingsStore()

      settings.volume = 0.5
      await flushPromises()
      vi.advanceTimersByTime(1000)
      await flushPromises()

      expect(saved).toEqual([])
    })

    it('debounces bursts of changes into one save with the latest values', async () => {
      const settings = await hydrated()

      settings.volume = 0.5
      await flushPromises()
      vi.advanceTimersByTime(100)
      settings.darkMode = 'light'
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS - 1)
      await flushPromises()
      expect(saved).toEqual([])

      vi.advanceTimersByTime(1)
      await flushPromises()
      expect(saved).toHaveLength(1)
      expect(saved[0]).toMatchObject({ volume: 0.5, darkMode: 'light' })
      expect(storage.getItem('theme-mode')).toBe('light')
    })

    it('saves deep changes inside nested settings', async () => {
      const settings = await hydrated()

      settings.equalizerBands[2] = 600
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
      await flushPromises()

      expect(saved.at(-1)?.equalizerBands).toEqual([0, 0, 600, 0, 0])
    })

    it('adopts the value the backend canonicalized when nothing changed meanwhile', async () => {
      const settings = await hydrated()
      router.set('save_settings', args => ({ ...(args?.settings as AppSettings), downloadDir: '/canonical' }))

      settings.downloadDir = '/raw/'
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
      await flushPromises()

      expect(settings.downloadDir).toBe('/canonical')
    })

    it('keeps a newer local edit instead of the canonicalized save result', async () => {
      const settings = await hydrated()
      const response = deferred<unknown>()
      router.set('save_settings', () => response.promise)

      settings.downloadDir = '/raw/'
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
      await flushPromises()
      settings.volume = 0.25
      response.resolve({ ...settings.snapshot(), volume: 1, downloadDir: '/canonical' })
      await flushPromises()

      expect(settings.downloadDir).toBe('/raw/')
      expect(settings.volume).toBe(0.25)
    })

    // 回归（PR #69 评审）：保存还在进行时改回已同步的值，不能因为与旧快照相同就跳过，否则在途保存会把旧改动写进后端
    it('saves a value restored to the synced one while an earlier save is still pending', async () => {
      const settings = await hydrated()
      const original = settings.volume
      const firstSave = deferred<unknown>()
      router.set('save_settings', (args) => {
        saved.push(args?.settings as AppSettings)
        return saved.length === 1 ? firstSave.promise : args?.settings
      })

      settings.volume = original / 2
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
      await flushPromises()
      expect(saved).toHaveLength(1)

      settings.volume = original
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
      await flushPromises()
      firstSave.resolve({ ...saved[0] })
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS * 2)
      await flushPromises()

      expect(saved).toHaveLength(2)
      expect(saved[1].volume).toBe(original)
      expect(settings.volume).toBe(original)
    })

    it('keeps local changes and the localStorage shadow when saving fails', async () => {
      const settings = await hydrated()
      router.set('save_settings', () => { throw new Error('disk full') })

      settings.darkMode = 'light'
      await flushPromises()
      vi.advanceTimersByTime(PERSIST_DEBOUNCE_MS)
      await flushPromises()

      expect(router.callsOf('save_settings')).toHaveLength(1)
      expect(settings.darkMode).toBe('light')
      expect(storage.getItem('theme-mode')).toBe('light')
    })
  })

  describe('exported normalizers', () => {
    it('aligns lyric default offsets to 50 ms within ±5000 ms', () => {
      expect(normalizeLyricDefaultOffset(1234, 0)).toBe(1250)
      expect(normalizeLyricDefaultOffset(-9999, 0)).toBe(-5000)
      expect(normalizeLyricDefaultOffset(Number.NaN, 7)).toBe(7)
      expect(Object.is(normalizeLyricDefaultOffset(-10, 0), 0)).toBe(true)
    })

    it('keeps the emphasis threshold, balance, speed and pitch in range', () => {
      expect(normalizeLyricEmphasisThresholdMs(725)).toBe(750)
      expect(normalizeLyricEmphasisThresholdMs(99_999)).toBe(2000)
      expect(normalizeLyricEmphasisThresholdMs(Number.NaN)).toBe(700)
      expect(normalizeVolumeBalance(0.126)).toBe(0.13)
      expect(normalizeVolumeBalance(5)).toBe(1)
      expect(normalizeVolumeBalance(Number.NaN)).toBe(0)
      expect(normalizePlaybackSpeed(0.01)).toBe(0.1)
      expect(normalizePlaybackSpeed('fast')).toBe(1)
      expect(normalizePlaybackPitch(0.33)).toBe(0.35)
    })
  })
})
