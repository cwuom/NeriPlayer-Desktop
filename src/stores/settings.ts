import { defineStore } from 'pinia'
import { ref, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export type ThemeMode = 'system' | 'dark' | 'light'
export type CoverStyle = 'disc' | 'card'
export type ColorMode = 'system' | 'default' | 'cover'

export interface AppSettings {
  formatVersion: number
  darkMode: ThemeMode
  themeColor: string
  locale: string
  defaultScreen: string
  showCoverBadge: boolean
  showNowPlayingTitle: boolean
  showToolbarDock: boolean
  showQualitySwitch: boolean
  showAudioCodec: boolean
  showAudioSpec: boolean
  lyricFontScale: number
  crossfade: boolean
  normalizeVolume: boolean
  fadeIn: boolean
  fadeInDuration: number
  fadeOutDuration: number
  crossfadeNext: boolean
  crossfadeInDuration: number
  crossfadeOutDuration: number
  keepProgress: boolean
  keepPlaybackMode: boolean
  showTranslation: boolean
  lyricBlur: boolean
  lyricBlurAmount: number
  cloudMusicOffset: number
  qqMusicOffset: number
  coverStyle: CoverStyle
  advancedLyrics: boolean
  dynamicBackground: boolean
  colorMode: ColorMode
  audioReactive: boolean
  coverBlurBg: boolean
  coverBlurAmount: number
  coverBlurDarken: number
  neteaseQuality: string
  qqMusicQuality: string
  youtubeQuality: string
  biliQuality: string
  bypassProxy: boolean
  internationalizationEnabled: boolean
  backgroundImageUri: string
  backgroundImageBlur: number
  backgroundImageAlpha: number
  devModeEnabled: boolean
  logToFile: boolean
  logLevel: string
  maxCacheSize: number
  downloadNameTemplate: string
  downloadDir: string
  ltServerUrl: string
  ltNickname: string
  ltAllowMemberControl: boolean
  ltAutoPauseOnMemberChange: boolean
  ltShareAudioLinks: boolean
  volume: number
  playbackSpeed: number
  loudnessGainMb: number
  equalizerEnabled: boolean
  equalizerPresetId: string
  equalizerBands: number[]
}

interface SettingsLoadResult {
  settings: AppSettings
  persisted: boolean
}

type SettingKey = Exclude<keyof AppSettings, 'formatVersion'>
type SettingRefs = { [K in SettingKey]: Ref<AppSettings[K]> }

const SETTINGS_FORMAT_VERSION = 1
const LEGACY_PREFIX = 'neri:'
export const DEFAULT_DOWNLOAD_NAME_TEMPLATE = '{source} - {artist} - {title}'
export const MIN_MEDIA_CACHE_SIZE_MB = 256
export const MAX_MEDIA_CACHE_SIZE_MB = 512 * 1024

const DEFAULT_SETTINGS: AppSettings = {
  formatVersion: SETTINGS_FORMAT_VERSION,
  darkMode: 'dark',
  themeColor: 'purple',
  locale: detectLocale(),
  defaultScreen: 'home',
  showCoverBadge: true,
  showNowPlayingTitle: true,
  showToolbarDock: true,
  showQualitySwitch: true,
  showAudioCodec: true,
  showAudioSpec: true,
  lyricFontScale: 1,
  crossfade: false,
  normalizeVolume: false,
  fadeIn: false,
  fadeInDuration: 500,
  fadeOutDuration: 500,
  crossfadeNext: false,
  crossfadeInDuration: 500,
  crossfadeOutDuration: 500,
  keepProgress: true,
  keepPlaybackMode: true,
  showTranslation: true,
  lyricBlur: true,
  lyricBlurAmount: 1.5,
  cloudMusicOffset: 500,
  qqMusicOffset: 500,
  coverStyle: 'card',
  advancedLyrics: true,
  dynamicBackground: true,
  colorMode: 'default',
  audioReactive: true,
  coverBlurBg: false,
  coverBlurAmount: 1.5,
  coverBlurDarken: 0.2,
  neteaseQuality: 'exhigh',
  qqMusicQuality: 'high',
  youtubeQuality: 'very_high',
  biliQuality: 'high',
  bypassProxy: true,
  internationalizationEnabled: typeof navigator !== 'undefined' && !navigator.language.startsWith('zh'),
  backgroundImageUri: '',
  backgroundImageBlur: 20,
  backgroundImageAlpha: 0.3,
  devModeEnabled: false,
  logToFile: false,
  logLevel: 'info',
  maxCacheSize: 1024,
  downloadNameTemplate: DEFAULT_DOWNLOAD_NAME_TEMPLATE,
  downloadDir: '',
  ltServerUrl: 'https://neriplayer.hancat.work',
  ltNickname: '',
  ltAllowMemberControl: true,
  ltAutoPauseOnMemberChange: true,
  ltShareAudioLinks: true,
  volume: 1,
  playbackSpeed: 1,
  loudnessGainMb: 0,
  equalizerEnabled: false,
  equalizerPresetId: 'flat',
  equalizerBands: [0, 0, 0, 0, 0],
}

const LEGACY_KEYS: Partial<Record<SettingKey, string>> = {
  darkMode: 'dark_mode',
  themeColor: 'theme_color',
  defaultScreen: 'default_screen',
  showCoverBadge: 'cover_badge',
  showNowPlayingTitle: 'np_title',
  showToolbarDock: 'np_toolbar',
  showQualitySwitch: 'quality_switch',
  showAudioCodec: 'audio_codec',
  showAudioSpec: 'audio_spec',
  lyricFontScale: 'lyric_font_scale',
  crossfade: 'crossfade',
  normalizeVolume: 'normalize',
  fadeIn: 'fade_in',
  fadeInDuration: 'fade_in_duration',
  fadeOutDuration: 'fade_out_duration',
  crossfadeNext: 'crossfade_next',
  crossfadeInDuration: 'crossfade_in_duration',
  crossfadeOutDuration: 'crossfade_out_duration',
  keepProgress: 'keep_progress',
  keepPlaybackMode: 'keep_mode',
  showTranslation: 'show_translation',
  lyricBlur: 'lyric_blur',
  lyricBlurAmount: 'lyric_blur_amount',
  cloudMusicOffset: 'cloud_offset',
  qqMusicOffset: 'qq_offset',
  coverStyle: 'cover_style',
  advancedLyrics: 'advanced_lyrics',
  dynamicBackground: 'dynamic_bg',
  audioReactive: 'audio_reactive',
  coverBlurBg: 'cover_blur_bg',
  coverBlurAmount: 'cover_blur_amount',
  coverBlurDarken: 'cover_blur_darken',
  neteaseQuality: 'netease_quality',
  qqMusicQuality: 'qq_quality',
  youtubeQuality: 'youtube_quality',
  biliQuality: 'bili_quality',
  bypassProxy: 'bypass_proxy',
  internationalizationEnabled: 'intl_enabled',
  backgroundImageUri: 'bg_image_uri',
  backgroundImageBlur: 'bg_image_blur',
  backgroundImageAlpha: 'bg_image_alpha',
  devModeEnabled: 'dev_mode',
  logToFile: 'log_to_file',
  logLevel: 'log_level',
  maxCacheSize: 'cache_size',
  downloadNameTemplate: 'download_template',
  downloadDir: 'download_dir',
  ltServerUrl: 'lt_server_url',
  ltNickname: 'lt_nickname',
  ltAllowMemberControl: 'lt_allow_member_control',
  ltAutoPauseOnMemberChange: 'lt_auto_pause_on_change',
  ltShareAudioLinks: 'lt_share_audio_links',
}

function detectLocale(): string {
  if (typeof localStorage !== 'undefined') {
    const stored = localStorage.getItem('locale')
    if (stored && ['zh-CN', 'zh-TW', 'en', 'ja'].includes(stored)) return stored
  }
  const language = typeof navigator !== 'undefined' ? navigator.language : 'zh-CN'
  if (language === 'zh-TW' || language === 'zh-Hant') return 'zh-TW'
  if (language.startsWith('zh')) return 'zh-CN'
  if (language.startsWith('ja')) return 'ja'
  return 'en'
}

function parseLegacyValue(key: string): unknown {
  if (typeof localStorage === 'undefined') return undefined
  try {
    const raw = localStorage.getItem(`${LEGACY_PREFIX}${key}`)
    return raw === null ? undefined : JSON.parse(raw)
  } catch {
    return undefined
  }
}

function readLegacySnapshot(): Partial<AppSettings> {
  const result: Partial<AppSettings> = {}
  for (const [settingKey, legacyKey] of Object.entries(LEGACY_KEYS)) {
    const value = parseLegacyValue(legacyKey)
    if (value !== undefined) {
      ;(result as Record<string, unknown>)[settingKey] = value
    }
  }

  if (typeof localStorage !== 'undefined') {
    const themeMode = localStorage.getItem('theme-mode')
    const themeColor = localStorage.getItem('theme-color')
    const locale = localStorage.getItem('locale')
    if (themeMode) result.darkMode = themeMode as ThemeMode
    if (themeColor) result.themeColor = themeColor
    if (locale) result.locale = locale

    try {
      const playerState = JSON.parse(localStorage.getItem('neri:player-state') || '{}')
      if (typeof playerState.volume === 'number') result.volume = playerState.volume
    } catch {
      // 旧播放器快照损坏时继续使用默认设置
    }
  }
  return result
}

function hasLegacySnapshot(): boolean {
  if (typeof localStorage === 'undefined') return false
  return Object.values(LEGACY_KEYS).some(key => localStorage.getItem(`${LEGACY_PREFIX}${key}`) !== null)
    || localStorage.getItem('theme-mode') !== null
    || localStorage.getItem('theme-color') !== null
    || localStorage.getItem('locale') !== null
    || localStorage.getItem('neri:player-state') !== null
}

function normalizeSnapshot(input: unknown): AppSettings {
  const source = input && typeof input === 'object' ? input as Partial<AppSettings> : {}
  const result = { ...DEFAULT_SETTINGS, equalizerBands: [...DEFAULT_SETTINGS.equalizerBands] }

  for (const key of Object.keys(DEFAULT_SETTINGS) as Array<keyof AppSettings>) {
    if (key === 'formatVersion' || !(key in source)) continue
    const value = source[key]
    const defaultValue = DEFAULT_SETTINGS[key]
    if (Array.isArray(defaultValue)) {
      if (Array.isArray(value) && value.every(item => typeof item === 'number')) {
        ;(result as Record<string, unknown>)[key] = [...value]
      }
    } else if (typeof value === typeof defaultValue) {
      ;(result as Record<string, unknown>)[key] = value
    }
  }

  if (!['system', 'dark', 'light'].includes(result.darkMode)) result.darkMode = DEFAULT_SETTINGS.darkMode
  if (!['disc', 'card'].includes(result.coverStyle)) result.coverStyle = DEFAULT_SETTINGS.coverStyle
  if (!['home', 'explore', 'library'].includes(result.defaultScreen)) result.defaultScreen = DEFAULT_SETTINGS.defaultScreen
  if (!['zh-CN', 'zh-TW', 'en', 'ja'].includes(result.locale)) result.locale = DEFAULT_SETTINGS.locale
  if (!['off', 'error', 'warn', 'info', 'debug', 'trace'].includes(result.logLevel)) result.logLevel = DEFAULT_SETTINGS.logLevel
  if (result.themeColor.startsWith('#')) result.themeColor = result.themeColor === '#6750A4' ? 'purple' : DEFAULT_SETTINGS.themeColor

  result.lyricFontScale = clamp(result.lyricFontScale, 0.5, 1.5)
  result.fadeInDuration = clamp(result.fadeInDuration, 0, 10000)
  result.fadeOutDuration = clamp(result.fadeOutDuration, 0, 10000)
  result.crossfadeInDuration = clamp(result.crossfadeInDuration, 0, 10000)
  result.crossfadeOutDuration = clamp(result.crossfadeOutDuration, 0, 10000)
  result.lyricBlurAmount = clamp(result.lyricBlurAmount, 0, 8)
  result.cloudMusicOffset = clamp(result.cloudMusicOffset, -30000, 30000)
  result.qqMusicOffset = clamp(result.qqMusicOffset, -30000, 30000)
  result.coverBlurAmount = clamp(result.coverBlurAmount, 0, 500)
  result.coverBlurDarken = clamp(result.coverBlurDarken, 0, 1)
  result.backgroundImageBlur = clamp(result.backgroundImageBlur, 0, 100)
  result.backgroundImageAlpha = clamp(result.backgroundImageAlpha, 0, 1)
  result.maxCacheSize = clamp(
    result.maxCacheSize,
    MIN_MEDIA_CACHE_SIZE_MB,
    MAX_MEDIA_CACHE_SIZE_MB,
  )
  result.volume = clamp(result.volume, 0, 1)
  result.playbackSpeed = clamp(result.playbackSpeed, 0.25, 3)
  result.loudnessGainMb = clamp(Math.round(result.loudnessGainMb), 0, 1500)
  result.equalizerBands = result.equalizerBands.slice(0, 5).map(value => clamp(Math.round(value), -1500, 1500))
  while (result.equalizerBands.length < 5) result.equalizerBands.push(0)
  return result
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(value) ? value : min))
}

function writeLegacyShadow(snapshot: AppSettings) {
  if (typeof localStorage === 'undefined') return
  for (const [settingKey, legacyKey] of Object.entries(LEGACY_KEYS)) {
    const value = snapshot[settingKey as SettingKey]
    localStorage.setItem(`${LEGACY_PREFIX}${legacyKey}`, JSON.stringify(value))
  }
  localStorage.setItem('theme-mode', snapshot.darkMode)
  localStorage.setItem('theme-color', snapshot.themeColor)
  localStorage.setItem('locale', snapshot.locale)
}

export const useSettingsStore = defineStore('settings', () => {
  const initial = normalizeSnapshot({ ...DEFAULT_SETTINGS, ...readLegacySnapshot() })
  const darkMode = ref<ThemeMode>(initial.darkMode)
  const themeColor = ref(initial.themeColor)
  const locale = ref(initial.locale)
  const defaultScreen = ref(initial.defaultScreen)
  const showCoverBadge = ref(initial.showCoverBadge)
  const showNowPlayingTitle = ref(initial.showNowPlayingTitle)
  const showToolbarDock = ref(initial.showToolbarDock)
  const showQualitySwitch = ref(initial.showQualitySwitch)
  const showAudioCodec = ref(initial.showAudioCodec)
  const showAudioSpec = ref(initial.showAudioSpec)
  const lyricFontScale = ref(initial.lyricFontScale)
  const crossfade = ref(initial.crossfade)
  const normalizeVolume = ref(initial.normalizeVolume)
  const fadeIn = ref(initial.fadeIn)
  const fadeInDuration = ref(initial.fadeInDuration)
  const fadeOutDuration = ref(initial.fadeOutDuration)
  const crossfadeNext = ref(initial.crossfadeNext)
  const crossfadeInDuration = ref(initial.crossfadeInDuration)
  const crossfadeOutDuration = ref(initial.crossfadeOutDuration)
  const keepProgress = ref(initial.keepProgress)
  const keepPlaybackMode = ref(initial.keepPlaybackMode)
  const showTranslation = ref(initial.showTranslation)
  const lyricBlur = ref(initial.lyricBlur)
  const lyricBlurAmount = ref(initial.lyricBlurAmount)
  const cloudMusicOffset = ref(initial.cloudMusicOffset)
  const qqMusicOffset = ref(initial.qqMusicOffset)
  const coverStyle = ref<CoverStyle>(initial.coverStyle)
  const advancedLyrics = ref(initial.advancedLyrics)
  const dynamicBackground = ref(initial.dynamicBackground)
  const colorMode = ref<ColorMode>(initial.colorMode)
  const audioReactive = ref(initial.audioReactive)
  const coverBlurBg = ref(initial.coverBlurBg)
  const coverBlurAmount = ref(initial.coverBlurAmount)
  const coverBlurDarken = ref(initial.coverBlurDarken)
  const neteaseQuality = ref(initial.neteaseQuality)
  const qqMusicQuality = ref(initial.qqMusicQuality)
  const youtubeQuality = ref(initial.youtubeQuality)
  const biliQuality = ref(initial.biliQuality)
  const bypassProxy = ref(initial.bypassProxy)
  const internationalizationEnabled = ref(initial.internationalizationEnabled)
  const backgroundImageUri = ref(initial.backgroundImageUri)
  const backgroundImageBlur = ref(initial.backgroundImageBlur)
  const backgroundImageAlpha = ref(initial.backgroundImageAlpha)
  const devModeEnabled = ref(initial.devModeEnabled)
  const logToFile = ref(initial.logToFile)
  const logLevel = ref(initial.logLevel)
  const maxCacheSize = ref(initial.maxCacheSize)
  const downloadNameTemplate = ref(initial.downloadNameTemplate)
  const downloadDir = ref(initial.downloadDir)
  const ltServerUrl = ref(initial.ltServerUrl)
  const ltNickname = ref(initial.ltNickname)
  const ltAllowMemberControl = ref(initial.ltAllowMemberControl)
  const ltAutoPauseOnMemberChange = ref(initial.ltAutoPauseOnMemberChange)
  const ltShareAudioLinks = ref(initial.ltShareAudioLinks)
  const volume = ref(initial.volume)
  const playbackSpeed = ref(initial.playbackSpeed)
  const loudnessGainMb = ref(initial.loudnessGainMb)
  const equalizerEnabled = ref(initial.equalizerEnabled)
  const equalizerPresetId = ref(initial.equalizerPresetId)
  const equalizerBands = ref([...initial.equalizerBands])

  const settingRefs: SettingRefs = {
    darkMode, themeColor, locale, defaultScreen, showCoverBadge,
    showNowPlayingTitle, showToolbarDock, showQualitySwitch, showAudioCodec,
    showAudioSpec, lyricFontScale, crossfade, normalizeVolume, fadeIn,
    fadeInDuration, fadeOutDuration, crossfadeNext, crossfadeInDuration,
    crossfadeOutDuration, keepProgress, keepPlaybackMode, showTranslation,
    lyricBlur, lyricBlurAmount, cloudMusicOffset, qqMusicOffset, coverStyle,
    advancedLyrics, dynamicBackground, colorMode, audioReactive, coverBlurBg,
    coverBlurAmount, coverBlurDarken, neteaseQuality, qqMusicQuality,
    youtubeQuality, biliQuality, bypassProxy, internationalizationEnabled,
    backgroundImageUri, backgroundImageBlur, backgroundImageAlpha, devModeEnabled,
    logToFile, logLevel,
    maxCacheSize, downloadNameTemplate, downloadDir, ltServerUrl, ltNickname,
    ltAllowMemberControl, ltAutoPauseOnMemberChange, ltShareAudioLinks, volume,
    playbackSpeed, loudnessGainMb, equalizerEnabled, equalizerPresetId,
    equalizerBands,
  }

  const isHydrated = ref(false)
  let hydrationPromise: Promise<void> | null = null
  let persistTimer: ReturnType<typeof setTimeout> | null = null

  function snapshot(): AppSettings {
    const result = { formatVersion: SETTINGS_FORMAT_VERSION } as AppSettings
    for (const key of Object.keys(settingRefs) as SettingKey[]) {
      ;(result as unknown as Record<string, unknown>)[key] = settingRefs[key].value
    }
    result.equalizerBands = [...equalizerBands.value]
    return normalizeSnapshot(result)
  }

  function applySnapshot(value: unknown) {
    const next = normalizeSnapshot(value)
    for (const key of Object.keys(settingRefs) as SettingKey[]) {
      const nextValue = next[key]
      if (key === 'equalizerBands') {
        equalizerBands.value = [...nextValue as number[]]
      } else {
        ;(settingRefs[key] as Ref<unknown>).value = nextValue
      }
    }
  }

  async function persistNow() {
    const next = snapshot()
    writeLegacyShadow(next)
    try {
      await invoke('save_settings', { settings: next })
    } catch {
      // 浏览器开发模式没有 Rust bridge 时仍保留 localStorage 兼容缓存
    }
  }

  function schedulePersist() {
    if (!isHydrated.value) return
    if (persistTimer) clearTimeout(persistTimer)
    persistTimer = setTimeout(() => {
      persistTimer = null
      void persistNow()
    }, 180)
  }

  async function hydrate() {
    if (hydrationPromise) return hydrationPromise
    hydrationPromise = (async () => {
      try {
        const loaded = await invoke<SettingsLoadResult>('get_settings')
        const legacy = readLegacySnapshot()
        const shouldMigrate = !loaded.persisted && hasLegacySnapshot()
        applySnapshot(loaded.persisted ? loaded.settings : { ...DEFAULT_SETTINGS, ...legacy })
        isHydrated.value = true
        if (shouldMigrate || !loaded.persisted) await persistNow()
        else writeLegacyShadow(snapshot())
      } catch {
        applySnapshot({ ...DEFAULT_SETTINGS, ...readLegacySnapshot() })
        isHydrated.value = true
        writeLegacyShadow(snapshot())
      }
    })()
    return hydrationPromise
  }

  watch(Object.values(settingRefs), schedulePersist, { deep: true })

  return {
    isHydrated, hydrate, snapshot, applySnapshot,
    darkMode, themeColor, locale, coverStyle,
    defaultScreen, showCoverBadge, showNowPlayingTitle, showToolbarDock,
    showQualitySwitch, showAudioCodec, showAudioSpec, lyricFontScale,
    crossfade, normalizeVolume, fadeIn, fadeInDuration, fadeOutDuration,
    crossfadeNext, crossfadeInDuration, crossfadeOutDuration,
    keepProgress, keepPlaybackMode, showTranslation, lyricBlur, lyricBlurAmount,
    cloudMusicOffset, qqMusicOffset, advancedLyrics, dynamicBackground,
    colorMode, audioReactive, coverBlurBg, coverBlurAmount, coverBlurDarken,
    neteaseQuality, qqMusicQuality, youtubeQuality, biliQuality, bypassProxy,
    internationalizationEnabled, backgroundImageUri, backgroundImageBlur,
    backgroundImageAlpha, devModeEnabled, logToFile, logLevel, maxCacheSize, downloadNameTemplate,
    downloadDir, ltServerUrl, ltNickname, ltAllowMemberControl,
    ltAutoPauseOnMemberChange, ltShareAudioLinks, volume, playbackSpeed,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands,
  }
})
