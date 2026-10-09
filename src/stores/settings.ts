import { defineStore } from 'pinia'
import { ref, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { createLogger } from '@/utils/logger'
import { normalizeDesktopLyricsStyle, type DesktopLyricsStyle } from '@/modules/desktopLyrics/style'

const log = createLogger('settings')

export type ThemeMode = 'system' | 'dark' | 'light'
export type CoverStyle = 'disc' | 'card'
export type ColorMode = 'system' | 'default' | 'cover'

export const YOUTUBE_PLAYBACK_SOURCES = [
  'automatic', 'visionos', 'android_vr', 'web_remix', 'tv_html5', 'web_creator',
] as const
export type YouTubePlaybackSource = typeof YOUTUBE_PLAYBACK_SOURCES[number]
/** 与 Android LyricSourcePreference.storageValue 一致 */
export const DEFAULT_LYRIC_SOURCES = [
  'automatic', 'cloud_music', 'kugou', 'qq_music', 'lrclib', 'amll_ttml',
] as const
export type DefaultLyricSource = typeof DEFAULT_LYRIC_SOURCES[number]

export interface AppSettings {
  formatVersion: number
  darkMode: ThemeMode
  themeColor: string
  locale: string
  defaultScreen: string
  /** 关闭主窗口时收进托盘继续播放；关掉后关闭即退出 */
  closeToTray: boolean
  showCoverBadge: boolean
  showNowPlayingTitle: boolean
  showToolbarDock: boolean
  showQualitySwitch: boolean
  showAudioCodec: boolean
  showAudioSpec: boolean
  showAudioBitrate: boolean
  showAudioFormat: boolean
  showAudioChannels: boolean
  showAudioSampleRate: boolean
  showAudioBitDepth: boolean
  lyricFontScale: number
  crossfade: boolean
  normalizeVolume: boolean
  /** 多声道（AC-3/E-AC-3）音轨保留码流自带的动态范围压缩 */
  multichannelDrc: boolean
  volumeBalance: number
  fadeIn: boolean
  fadeInDuration: number
  fadeOutDuration: number
  crossfadeNext: boolean
  crossfadeInDuration: number
  crossfadeOutDuration: number
  keepProgress: boolean
  rememberLongFormProgress: boolean
  keepPlaybackMode: boolean
  showTranslation: boolean
  showRomanization: boolean
  lyricBlur: boolean
  lyricBlurAmount: number
  cloudMusicOffset: number
  qqMusicOffset: number
  kugouOffset: number
  lrclibOffset: number
  amllTtmlOffset: number
  coverStyle: CoverStyle
  advancedLyrics: boolean
  /** 有逐字结果时优先用；关闭后不再用 AMLL/酷狗补逐字（Android prefer_word_timed_lyrics） */
  preferWordTimedLyrics: boolean
  /** 播放时优先尝试的歌词源，找不到时回退自动（Android default_lyric_source） */
  defaultLyricSource: DefaultLyricSource
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
  youtubePlaybackSource: YouTubePlaybackSource
  neteaseAutoSourceSwitch: boolean
  neteaseLocalSourceFallback: boolean
  bypassProxy: boolean
  internationalizationEnabled: boolean
  exploreSearchHistoryEnabled: boolean
  backgroundImageUri: string
  backgroundImageBlur: number
  backgroundImageAlpha: number
  /** 背景图模式下卡片、搜索框等控件的实时玻璃模糊（Android enhanced_advanced_blur_enabled） */
  enhancedAdvancedBlur: boolean
  /** 玻璃模糊半径（px），12–64 按 4 对齐（Android enhanced_advanced_blur_radius_dp） */
  enhancedAdvancedBlurRadius: number
  devModeEnabled: boolean
  logToFile: boolean
  logLevel: string
  maxCacheSize: number
  downloadNameTemplate: string
  downloadDir: string
  downloadParallelism: number
  downloadAutoFillMetadata: boolean
  downloadEmbedLyrics: boolean
  downloadFollowPlaybackQuality: boolean
  downloadNeteaseQuality: string
  downloadQqMusicQuality: string
  downloadYoutubeQuality: string
  downloadBiliQuality: string
  ltServerUrl: string
  ltNickname: string
  ltAllowMemberControl: boolean
  ltAutoPauseOnMemberChange: boolean
  ltShareAudioLinks: boolean
  volume: number
  audioOutputDevice: string
  playbackSpeed: number
  loudnessGainMb: number
  equalizerEnabled: boolean
  equalizerPresetId: string
  equalizerBands: number[]
  desktopLyrics: DesktopLyricsStyle
}

interface SettingsLoadResult {
  settings: AppSettings
  persisted: boolean
}

type SettingKey = Exclude<keyof AppSettings, 'formatVersion'>
type SettingRefs = { [K in SettingKey]: Ref<AppSettings[K]> }

const SETTINGS_FORMAT_VERSION = 1
const LEGACY_PREFIX = 'neri:'
export const DEFAULT_DOWNLOAD_NAME_TEMPLATE = '%title% - %artist% - %album% - %source%'
export const MIN_DOWNLOAD_PARALLELISM = 1
export const MAX_DOWNLOAD_PARALLELISM = 8
export const DEFAULT_DOWNLOAD_PARALLELISM = 6
export const MIN_MEDIA_CACHE_SIZE_MB = 256
export const MAX_MEDIA_CACHE_SIZE_MB = 512 * 1024
// 对齐 Android LyricFontScale 0.5–1.6
export const LYRIC_FONT_SCALE_MIN = 0.5
export const LYRIC_FONT_SCALE_MAX = 1.6
export const LYRIC_FONT_SCALE_STEP = 0.05
// 封面模糊强度 × 30 = CSS 模糊半径（px），8 档上限即 240px
export const COVER_BLUR_PX_PER_UNIT = 30
export const MAX_COVER_BLUR_AMOUNT = 8
// 对齐 Android EnhancedAdvancedBlurPreference 12–64dp，按 4 对齐
export const ENHANCED_BLUR_RADIUS_MIN = 12
export const ENHANCED_BLUR_RADIUS_MAX = 64
export const ENHANCED_BLUR_RADIUS_STEP = 4
// 对齐 Android LyricDefaultOffset ±5000ms，按 50ms 对齐
export const LYRIC_DEFAULT_OFFSET_RANGE_MS = 5000
export const LYRIC_DEFAULT_OFFSET_STEP_MS = 50
const NETEASE_QUALITIES = ['standard', 'higher', 'exhigh', 'lossless', 'hires', 'jyeffect', 'sky', 'jymaster']
const QQ_QUALITIES = ['standard', 'high', 'lossless']
const YOUTUBE_QUALITIES = ['low', 'medium', 'high', 'very_high']
const BILI_QUALITIES = ['low', 'medium', 'high', 'dolby', 'lossless', 'hires']

const DEFAULT_SETTINGS: AppSettings = {
  formatVersion: SETTINGS_FORMAT_VERSION,
  darkMode: 'dark',
  themeColor: 'purple',
  locale: detectLocale(),
  defaultScreen: 'home',
  closeToTray: true,
  showCoverBadge: true,
  showNowPlayingTitle: true,
  showToolbarDock: true,
  showQualitySwitch: false,
  showAudioCodec: true,
  showAudioSpec: true,
  showAudioBitrate: true,
  showAudioFormat: true,
  showAudioChannels: false,
  showAudioSampleRate: false,
  showAudioBitDepth: false,
  lyricFontScale: 1,
  crossfade: false,
  normalizeVolume: false,
  multichannelDrc: false,
  volumeBalance: 0,
  fadeIn: false,
  fadeInDuration: 500,
  fadeOutDuration: 500,
  crossfadeNext: false,
  crossfadeInDuration: 500,
  crossfadeOutDuration: 500,
  keepProgress: true,
  rememberLongFormProgress: true,
  keepPlaybackMode: true,
  showTranslation: true,
  showRomanization: false,
  lyricBlur: true,
  lyricBlurAmount: 1.5,
  cloudMusicOffset: 1000,
  qqMusicOffset: 500,
  kugouOffset: 0,
  lrclibOffset: 0,
  amllTtmlOffset: 0,
  coverStyle: 'card',
  advancedLyrics: true,
  preferWordTimedLyrics: true,
  defaultLyricSource: 'automatic',
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
  youtubePlaybackSource: 'automatic',
  neteaseAutoSourceSwitch: false,
  neteaseLocalSourceFallback: false,
  bypassProxy: true,
  internationalizationEnabled: false,
  exploreSearchHistoryEnabled: true,
  backgroundImageUri: '',
  backgroundImageBlur: 20,
  backgroundImageAlpha: 0.3,
  enhancedAdvancedBlur: true,
  enhancedAdvancedBlurRadius: 36,
  devModeEnabled: false,
  logToFile: false,
  logLevel: 'info',
  maxCacheSize: 1024,
  downloadNameTemplate: DEFAULT_DOWNLOAD_NAME_TEMPLATE,
  downloadDir: '',
  downloadParallelism: DEFAULT_DOWNLOAD_PARALLELISM,
  downloadAutoFillMetadata: true,
  downloadEmbedLyrics: false,
  downloadFollowPlaybackQuality: true,
  downloadNeteaseQuality: 'exhigh',
  downloadQqMusicQuality: 'high',
  downloadYoutubeQuality: 'high',
  downloadBiliQuality: 'high',
  ltServerUrl: 'https://neriplayer.hancat.work',
  ltNickname: '',
  ltAllowMemberControl: true,
  ltAutoPauseOnMemberChange: true,
  ltShareAudioLinks: true,
  volume: 1,
  audioOutputDevice: '',
  playbackSpeed: 1,
  loudnessGainMb: 0,
  equalizerEnabled: false,
  equalizerPresetId: 'flat',
  equalizerBands: [0, 0, 0, 0, 0],
  desktopLyrics: normalizeDesktopLyricsStyle(null),
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
  showAudioBitrate: 'audio_bitrate',
  showAudioFormat: 'audio_format',
  showAudioChannels: 'audio_channels',
  showAudioSampleRate: 'audio_sample_rate',
  showAudioBitDepth: 'audio_bit_depth',
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
  youtubePlaybackSource: 'youtube_playback_source',
  neteaseAutoSourceSwitch: 'netease_auto_source_switch',
  neteaseLocalSourceFallback: 'netease_local_source_fallback',
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
  downloadParallelism: 'download_parallelism',
  downloadAutoFillMetadata: 'download_auto_fill_metadata',
  downloadEmbedLyrics: 'download_embed_lyrics',
  downloadFollowPlaybackQuality: 'download_follow_playback_quality',
  downloadNeteaseQuality: 'download_netease_quality',
  downloadQqMusicQuality: 'download_qq_quality',
  downloadYoutubeQuality: 'download_youtube_quality',
  downloadBiliQuality: 'download_bili_quality',
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

  for (const [key, legacyKey] of [
    ['showAudioFormat', 'showAudioCodec'],
    ['showAudioChannels', 'showAudioSpec'],
    ['showAudioSampleRate', 'showAudioSpec'],
    ['showAudioBitDepth', 'showAudioSpec'],
  ] as const) {
    if (!(key in source) && typeof source[legacyKey] === 'boolean') result[key] = source[legacyKey]
  }

  if (!['system', 'dark', 'light'].includes(result.darkMode)) result.darkMode = DEFAULT_SETTINGS.darkMode
  if (!['disc', 'card'].includes(result.coverStyle)) result.coverStyle = DEFAULT_SETTINGS.coverStyle
  // 旧版只有「封面动态取色」开关
  const legacyDynamicColor = (source as { dynamicColor?: unknown }).dynamicColor
  if (!('colorMode' in source) && legacyDynamicColor === true) result.colorMode = 'cover'
  if (!['system', 'default', 'cover'].includes(result.colorMode)) result.colorMode = DEFAULT_SETTINGS.colorMode
  if (!['home', 'explore', 'library'].includes(result.defaultScreen)) result.defaultScreen = DEFAULT_SETTINGS.defaultScreen
  if (!['zh-CN', 'zh-TW', 'en', 'ja'].includes(result.locale)) result.locale = DEFAULT_SETTINGS.locale
  if (!['off', 'error', 'warn', 'info', 'debug', 'trace'].includes(result.logLevel)) result.logLevel = DEFAULT_SETTINGS.logLevel
  if (result.themeColor.startsWith('#')) result.themeColor = result.themeColor === '#6750A4' ? 'purple' : DEFAULT_SETTINGS.themeColor
  result.youtubePlaybackSource = normalizeYouTubePlaybackSource(result.youtubePlaybackSource)
  result.downloadParallelism = Number.isFinite(result.downloadParallelism)
    ? clamp(Math.round(result.downloadParallelism), MIN_DOWNLOAD_PARALLELISM, MAX_DOWNLOAD_PARALLELISM)
    : DEFAULT_DOWNLOAD_PARALLELISM
  result.downloadNameTemplate = result.downloadNameTemplate.trim() || DEFAULT_DOWNLOAD_NAME_TEMPLATE
  result.downloadDir = result.downloadDir.trim()
  // 网易云「较高」的取值是 higher（与播放页、Android 一致）；旧版设置页写的是 high
  result.neteaseQuality = normalizeChoice(canonicalNeteaseQuality(result.neteaseQuality), NETEASE_QUALITIES, 'exhigh')
  result.qqMusicQuality = normalizeChoice(result.qqMusicQuality, QQ_QUALITIES, 'high')
  result.youtubeQuality = normalizeChoice(result.youtubeQuality, YOUTUBE_QUALITIES, 'very_high')
  result.biliQuality = normalizeChoice(result.biliQuality, BILI_QUALITIES, 'high')
  result.downloadNeteaseQuality = normalizeChoice(canonicalNeteaseQuality(result.downloadNeteaseQuality), NETEASE_QUALITIES, 'exhigh')
  result.downloadQqMusicQuality = normalizeChoice(result.downloadQqMusicQuality, QQ_QUALITIES, 'high')
  result.downloadYoutubeQuality = normalizeChoice(result.downloadYoutubeQuality, YOUTUBE_QUALITIES, 'high')
  result.downloadBiliQuality = normalizeChoice(result.downloadBiliQuality, BILI_QUALITIES, 'high')
  result.defaultLyricSource = normalizeChoice(result.defaultLyricSource, [...DEFAULT_LYRIC_SOURCES], 'automatic') as DefaultLyricSource

  // 旧版「无缝切换」与「切歌交叉淡入淡出」是同一效果的两个开关，合并到后者并沿用当时的淡入淡出时长
  if (result.crossfade) {
    if (!result.crossfadeNext) {
      result.crossfadeNext = true
      result.crossfadeInDuration = result.fadeInDuration
      result.crossfadeOutDuration = result.fadeOutDuration
    }
    result.crossfade = false
  }

  // 毫秒、MB 等字段在 Rust 端是整数，带小数会让整份设置保存失败
  result.lyricFontScale = clamp(result.lyricFontScale, LYRIC_FONT_SCALE_MIN, LYRIC_FONT_SCALE_MAX)
  result.fadeInDuration = clampInteger(result.fadeInDuration, 0, 10000)
  result.fadeOutDuration = clampInteger(result.fadeOutDuration, 0, 10000)
  result.crossfadeInDuration = clampInteger(result.crossfadeInDuration, 0, 10000)
  result.crossfadeOutDuration = clampInteger(result.crossfadeOutDuration, 0, 10000)
  result.lyricBlurAmount = clamp(result.lyricBlurAmount, 0, 8)
  result.cloudMusicOffset = normalizeLyricDefaultOffset(result.cloudMusicOffset, DEFAULT_SETTINGS.cloudMusicOffset)
  result.qqMusicOffset = normalizeLyricDefaultOffset(result.qqMusicOffset, DEFAULT_SETTINGS.qqMusicOffset)
  result.kugouOffset = normalizeLyricDefaultOffset(result.kugouOffset, DEFAULT_SETTINGS.kugouOffset)
  result.lrclibOffset = normalizeLyricDefaultOffset(result.lrclibOffset, DEFAULT_SETTINGS.lrclibOffset)
  result.amllTtmlOffset = normalizeLyricDefaultOffset(result.amllTtmlOffset, DEFAULT_SETTINGS.amllTtmlOffset)
  result.coverBlurAmount = clamp(result.coverBlurAmount, 0, MAX_COVER_BLUR_AMOUNT)
  result.coverBlurDarken = clamp(result.coverBlurDarken, 0, 1)
  result.backgroundImageBlur = clamp(result.backgroundImageBlur, 0, 100)
  result.backgroundImageAlpha = clamp(result.backgroundImageAlpha, 0, 1)
  result.enhancedAdvancedBlurRadius = Math.round(
    clamp(result.enhancedAdvancedBlurRadius, ENHANCED_BLUR_RADIUS_MIN, ENHANCED_BLUR_RADIUS_MAX) / ENHANCED_BLUR_RADIUS_STEP,
  ) * ENHANCED_BLUR_RADIUS_STEP
  result.maxCacheSize = clampInteger(
    result.maxCacheSize,
    MIN_MEDIA_CACHE_SIZE_MB,
    MAX_MEDIA_CACHE_SIZE_MB,
  )
  result.volume = clamp(result.volume, 0, 1)
  result.audioOutputDevice = result.audioOutputDevice.trim()
  result.playbackSpeed = clamp(result.playbackSpeed, 0.25, 3)
  result.loudnessGainMb = clamp(Math.round(result.loudnessGainMb), 0, 1500)
  result.volumeBalance = normalizeVolumeBalance(result.volumeBalance)
  result.equalizerBands = result.equalizerBands.slice(0, 5).map(value => clamp(Math.round(value), -1500, 1500))
  while (result.equalizerBands.length < 5) result.equalizerBands.push(0)
  result.desktopLyrics = normalizeDesktopLyricsStyle(result.desktopLyrics)
  return result
}

function normalizeYouTubePlaybackSource(value: string): YouTubePlaybackSource {
  let source = value.trim().toLowerCase()
  if (source === 'vision_os') source = 'visionos'
  else if (source === 'androidvr') source = 'android_vr'
  else if (source === 'creator') source = 'web_creator'
  return YOUTUBE_PLAYBACK_SOURCES.find(option => option === source) ?? 'automatic'
}

function normalizeChoice(value: string, allowed: string[], fallback: string): string {
  const normalized = value.trim()
  return allowed.includes(normalized) ? normalized : fallback
}

function canonicalNeteaseQuality(value: string): string {
  return value.trim() === 'high' ? 'higher' : value
}

/** 歌词来源的默认偏移：按 50ms 对齐并夹到 ±5000ms（Android normalizeLyricDefaultOffsetMs）；非数值回到默认 */
export function normalizeLyricDefaultOffset(value: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback
  const aligned = Math.round(value / LYRIC_DEFAULT_OFFSET_STEP_MS) * LYRIC_DEFAULT_OFFSET_STEP_MS
  return Math.min(LYRIC_DEFAULT_OFFSET_RANGE_MS, Math.max(-LYRIC_DEFAULT_OFFSET_RANGE_MS, aligned)) || 0
}

/** 声道平衡 -1（只剩左声道）～1（只剩右声道），按 0.01 取整；非数值时居中（对齐 Android） */
export function normalizeVolumeBalance(value: number): number {
  if (!Number.isFinite(value)) return 0
  return Math.round(Math.min(1, Math.max(-1, value)) * 100) / 100 || 0
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(value) ? value : min))
}

function clampInteger(value: number, min: number, max: number): number {
  return clamp(Math.round(value), min, max)
}

function sameSnapshot(left: AppSettings, right: AppSettings): boolean {
  return JSON.stringify(left) === JSON.stringify(right)
}

function hasTauriBridge(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
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
  const initial = normalizeSnapshot(readLegacySnapshot())
  const darkMode = ref<ThemeMode>(initial.darkMode)
  const themeColor = ref(initial.themeColor)
  const locale = ref(initial.locale)
  const defaultScreen = ref(initial.defaultScreen)
  const closeToTray = ref(initial.closeToTray)
  const showCoverBadge = ref(initial.showCoverBadge)
  const showNowPlayingTitle = ref(initial.showNowPlayingTitle)
  const showToolbarDock = ref(initial.showToolbarDock)
  const showQualitySwitch = ref(initial.showQualitySwitch)
  const showAudioCodec = ref(initial.showAudioCodec)
  const showAudioSpec = ref(initial.showAudioSpec)
  const showAudioBitrate = ref(initial.showAudioBitrate)
  const showAudioFormat = ref(initial.showAudioFormat)
  const showAudioChannels = ref(initial.showAudioChannels)
  const showAudioSampleRate = ref(initial.showAudioSampleRate)
  const showAudioBitDepth = ref(initial.showAudioBitDepth)
  const lyricFontScale = ref(initial.lyricFontScale)
  const crossfade = ref(initial.crossfade)
  const normalizeVolume = ref(initial.normalizeVolume)
  const multichannelDrc = ref(initial.multichannelDrc)
  const volumeBalance = ref(initial.volumeBalance)
  const fadeIn = ref(initial.fadeIn)
  const fadeInDuration = ref(initial.fadeInDuration)
  const fadeOutDuration = ref(initial.fadeOutDuration)
  const crossfadeNext = ref(initial.crossfadeNext)
  const crossfadeInDuration = ref(initial.crossfadeInDuration)
  const crossfadeOutDuration = ref(initial.crossfadeOutDuration)
  const keepProgress = ref(initial.keepProgress)
  const rememberLongFormProgress = ref(initial.rememberLongFormProgress)
  const keepPlaybackMode = ref(initial.keepPlaybackMode)
  const showTranslation = ref(initial.showTranslation)
  const showRomanization = ref(initial.showRomanization)
  const lyricBlur = ref(initial.lyricBlur)
  const lyricBlurAmount = ref(initial.lyricBlurAmount)
  const cloudMusicOffset = ref(initial.cloudMusicOffset)
  const qqMusicOffset = ref(initial.qqMusicOffset)
  const kugouOffset = ref(initial.kugouOffset)
  const lrclibOffset = ref(initial.lrclibOffset)
  const amllTtmlOffset = ref(initial.amllTtmlOffset)
  const coverStyle = ref<CoverStyle>(initial.coverStyle)
  const advancedLyrics = ref(initial.advancedLyrics)
  const preferWordTimedLyrics = ref(initial.preferWordTimedLyrics)
  const defaultLyricSource = ref<DefaultLyricSource>(initial.defaultLyricSource)
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
  const youtubePlaybackSource = ref(initial.youtubePlaybackSource)
  const neteaseAutoSourceSwitch = ref(initial.neteaseAutoSourceSwitch)
  const neteaseLocalSourceFallback = ref(initial.neteaseLocalSourceFallback)
  const bypassProxy = ref(initial.bypassProxy)
  const internationalizationEnabled = ref(initial.internationalizationEnabled)
  const exploreSearchHistoryEnabled = ref(initial.exploreSearchHistoryEnabled)
  const backgroundImageUri = ref(initial.backgroundImageUri)
  const backgroundImageBlur = ref(initial.backgroundImageBlur)
  const backgroundImageAlpha = ref(initial.backgroundImageAlpha)
  const enhancedAdvancedBlur = ref(initial.enhancedAdvancedBlur)
  const enhancedAdvancedBlurRadius = ref(initial.enhancedAdvancedBlurRadius)
  const devModeEnabled = ref(initial.devModeEnabled)
  const logToFile = ref(initial.logToFile)
  const logLevel = ref(initial.logLevel)
  const maxCacheSize = ref(initial.maxCacheSize)
  const downloadNameTemplate = ref(initial.downloadNameTemplate)
  const downloadDir = ref(initial.downloadDir)
  const downloadParallelism = ref(initial.downloadParallelism)
  const downloadAutoFillMetadata = ref(initial.downloadAutoFillMetadata)
  const downloadEmbedLyrics = ref(initial.downloadEmbedLyrics)
  const downloadFollowPlaybackQuality = ref(initial.downloadFollowPlaybackQuality)
  const downloadNeteaseQuality = ref(initial.downloadNeteaseQuality)
  const downloadQqMusicQuality = ref(initial.downloadQqMusicQuality)
  const downloadYoutubeQuality = ref(initial.downloadYoutubeQuality)
  const downloadBiliQuality = ref(initial.downloadBiliQuality)
  const ltServerUrl = ref(initial.ltServerUrl)
  const ltNickname = ref(initial.ltNickname)
  const ltAllowMemberControl = ref(initial.ltAllowMemberControl)
  const ltAutoPauseOnMemberChange = ref(initial.ltAutoPauseOnMemberChange)
  const ltShareAudioLinks = ref(initial.ltShareAudioLinks)
  const volume = ref(initial.volume)
  const audioOutputDevice = ref(initial.audioOutputDevice)
  const playbackSpeed = ref(initial.playbackSpeed)
  const loudnessGainMb = ref(initial.loudnessGainMb)
  const equalizerEnabled = ref(initial.equalizerEnabled)
  const equalizerPresetId = ref(initial.equalizerPresetId)
  const equalizerBands = ref([...initial.equalizerBands])
  const desktopLyrics = ref<DesktopLyricsStyle>(initial.desktopLyrics)

  const settingRefs: SettingRefs = {
    darkMode, themeColor, locale, defaultScreen, closeToTray, showCoverBadge,
    showNowPlayingTitle, showToolbarDock, showQualitySwitch, showAudioCodec,
    showAudioSpec, lyricFontScale, crossfade, normalizeVolume, multichannelDrc, volumeBalance, fadeIn,
    showAudioBitrate, showAudioFormat, showAudioChannels, showAudioSampleRate, showAudioBitDepth,
    fadeInDuration, fadeOutDuration, crossfadeNext, crossfadeInDuration,
    crossfadeOutDuration, keepProgress, rememberLongFormProgress, keepPlaybackMode, showTranslation,
    showRomanization, lyricBlur, lyricBlurAmount, cloudMusicOffset, qqMusicOffset, kugouOffset, lrclibOffset,
    amllTtmlOffset, coverStyle,
    advancedLyrics, preferWordTimedLyrics, defaultLyricSource, dynamicBackground, colorMode, audioReactive, coverBlurBg,
    coverBlurAmount, coverBlurDarken, neteaseQuality, qqMusicQuality,
    youtubeQuality, biliQuality, bypassProxy, internationalizationEnabled, exploreSearchHistoryEnabled,
    youtubePlaybackSource, neteaseAutoSourceSwitch, neteaseLocalSourceFallback,
    backgroundImageUri, backgroundImageBlur, backgroundImageAlpha, enhancedAdvancedBlur,
    enhancedAdvancedBlurRadius, devModeEnabled,
    logToFile, logLevel,
    maxCacheSize, downloadNameTemplate, downloadDir, ltServerUrl, ltNickname,
    downloadParallelism, downloadAutoFillMetadata, downloadEmbedLyrics,
    downloadFollowPlaybackQuality, downloadNeteaseQuality, downloadQqMusicQuality,
    downloadYoutubeQuality, downloadBiliQuality,
    ltAllowMemberControl, ltAutoPauseOnMemberChange, ltShareAudioLinks, volume, audioOutputDevice,
    playbackSpeed, loudnessGainMb, equalizerEnabled, equalizerPresetId,
    equalizerBands, desktopLyrics,
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
      const saved = await invoke<AppSettings | undefined>('save_settings', { settings: next })
      // Rust 会再规整一遍（去空白、夹范围）；保存期间没有新改动时采用它，两边不必等到重启才一致
      if (saved && sameSnapshot(snapshot(), next) && !sameSnapshot(normalizeSnapshot(saved), next)) {
        applySnapshot(saved)
      }
    } catch (error) {
      // 浏览器开发模式没有 Rust bridge 时仍保留 localStorage 兼容缓存
      if (hasTauriBridge()) log.error('save_settings failed:', error)
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
        applySnapshot(loaded.persisted ? loaded.settings : legacy)
        isHydrated.value = true
        if (shouldMigrate || !loaded.persisted) await persistNow()
        else writeLegacyShadow(snapshot())
      } catch {
        applySnapshot(readLegacySnapshot())
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
    defaultScreen, closeToTray, showCoverBadge, showNowPlayingTitle, showToolbarDock,
    showQualitySwitch, showAudioCodec, showAudioSpec, lyricFontScale,
    showAudioBitrate, showAudioFormat, showAudioChannels, showAudioSampleRate, showAudioBitDepth,
    crossfade, normalizeVolume, multichannelDrc, volumeBalance, fadeIn, fadeInDuration, fadeOutDuration,
    crossfadeNext, crossfadeInDuration, crossfadeOutDuration,
    keepProgress, rememberLongFormProgress, keepPlaybackMode, showTranslation, showRomanization,
    lyricBlur, lyricBlurAmount,
    cloudMusicOffset, qqMusicOffset, kugouOffset, lrclibOffset, amllTtmlOffset,
    advancedLyrics, preferWordTimedLyrics, defaultLyricSource, dynamicBackground,
    colorMode, audioReactive, coverBlurBg, coverBlurAmount, coverBlurDarken,
    neteaseQuality, qqMusicQuality, youtubeQuality, biliQuality, bypassProxy,
    youtubePlaybackSource, neteaseAutoSourceSwitch, neteaseLocalSourceFallback,
    internationalizationEnabled, exploreSearchHistoryEnabled, backgroundImageUri, backgroundImageBlur,
    backgroundImageAlpha, enhancedAdvancedBlur, enhancedAdvancedBlurRadius,
    devModeEnabled, logToFile, logLevel, maxCacheSize, downloadNameTemplate,
    downloadDir, ltServerUrl, ltNickname, ltAllowMemberControl,
    downloadParallelism, downloadAutoFillMetadata, downloadEmbedLyrics,
    downloadFollowPlaybackQuality, downloadNeteaseQuality, downloadQqMusicQuality,
    downloadYoutubeQuality, downloadBiliQuality,
    ltAutoPauseOnMemberChange, ltShareAudioLinks, volume, audioOutputDevice, playbackSpeed,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands, desktopLyrics,
  }
})
