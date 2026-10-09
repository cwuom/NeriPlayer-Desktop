<script setup lang="ts">
import { ref, computed, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { SUPPORTED_LOCALES, setLocaleWithTransition } from '@/i18n'
import DesktopLyricsStage from '@/components/desktopLyrics/DesktopLyricsStage.vue'
import { closeDesktopLyricsWindow, desktopLyricsOpen, openDesktopLyricsWindow } from '@/modules/desktopLyrics/bridge'
import type { DesktopLyricsFrameLine } from '@/modules/desktopLyrics/frame'
import {
  DEFAULT_DESKTOP_LYRICS_STYLE,
  DESKTOP_LYRICS_ALIGNS,
  DESKTOP_LYRICS_BACKGROUND_OPACITY,
  DESKTOP_LYRICS_BACKGROUNDS,
  DESKTOP_LYRICS_FONT_PRESETS,
  DESKTOP_LYRICS_FONT_SIZE,
  DESKTOP_LYRICS_KARAOKE,
  DESKTOP_LYRICS_LAYOUTS,
  DESKTOP_LYRICS_LETTER_SPACING,
  DESKTOP_LYRICS_OPACITY,
  DESKTOP_LYRICS_SECONDARY,
  DESKTOP_LYRICS_SECONDARY_SCALE,
  DESKTOP_LYRICS_SHADOW_BLUR,
  DESKTOP_LYRICS_STROKE_WIDTH,
  DESKTOP_LYRICS_THEMES,
  DESKTOP_LYRICS_WEIGHTS,
  applyDesktopLyricsTheme,
  normalizeDesktopLyricsStyle,
  parseCssColor,
  type DesktopLyricsStyle,
} from '@/modules/desktopLyrics/style'
import { openUrl } from '@tauri-apps/plugin-opener'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import {
  COVER_BLUR_PX_PER_UNIT,
  LYRIC_DEFAULT_OFFSET_RANGE_MS,
  LYRIC_DEFAULT_OFFSET_STEP_MS,
  LYRIC_FONT_SCALE_MAX,
  LYRIC_FONT_SCALE_MIN,
  LYRIC_FONT_SCALE_STEP,
  normalizeLyricDefaultOffset,
  MAX_COVER_BLUR_AMOUNT,
  MAX_MEDIA_CACHE_SIZE_MB,
  MIN_MEDIA_CACHE_SIZE_MB,
  MAX_DOWNLOAD_PARALLELISM,
  MIN_DOWNLOAD_PARALLELISM,
  YOUTUBE_PLAYBACK_SOURCES,
  DEFAULT_LYRIC_SOURCES,
  ENHANCED_BLUR_RADIUS_MAX,
  ENHANCED_BLUR_RADIUS_MIN,
  ENHANCED_BLUR_RADIUS_STEP,
  useSettingsStore,
  type ColorMode,
} from '@/stores/settings'
import { useAuthStore } from '@/stores/auth'
import { useSyncStore, type SyncFrequency } from '@/stores/sync'
import { useDownloadStore } from '@/stores/download'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { isValidLtNickname, LT_NICKNAME_MAX_LENGTH } from '@/stores/listenTogether/protocol'
import { usePlayerStore } from '@/stores/player'
import { DEFAULT_LYRIC_OFFSET_MS, formatLyricOffsetMs } from '@/modules/lyrics/lyricOffset'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { DEFAULT_DOWNLOAD_NAME_TEMPLATE } from '@/stores/settings'
import StorageManagementDialog from '@/components/StorageManagementDialog.vue'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import {
  clearBrowserCache,
  mergeBrowserCacheUsage,
  type StorageCacheClearOptions,
  type StorageUsageSummary,
} from '@/utils/storage'
import { applyTheme, switchThemeWithRipple, type ThemeMode } from '@/utils/theme'
import { THEME_COLORS, getSwatchColor, applyThemeColor, getSavedThemeColor, switchThemeColorWithRipple } from '@/utils/themeColor'
import { shortcutDescriptors } from '@/modules/shortcuts/globalShortcuts'
import { useEscapeClose } from '@/composables/useEscapeClose'
import { createLogger } from '@/utils/logger'

const log = createLogger('settings-view')

const { t, locale } = useI18n()
const router = useRouter()
const settings = useSettingsStore()
const auth = useAuthStore()
const syncStore = useSyncStore()
const downloadStore = useDownloadStore()
const player = usePlayerStore()
const lt = useListenTogetherStore()
const toast = useToastStore()
const {
  darkMode, themeColor: selectedColor, coverStyle,
  defaultScreen, closeToTray, showCoverBadge, showNowPlayingTitle, showToolbarDock,
  showQualitySwitch, lyricFontScale,
  normalizeVolume, multichannelDrc, volumeBalance, audioOutputDevice,
  fadeIn, fadeInDuration, fadeOutDuration,
  crossfadeNext, crossfadeInDuration, crossfadeOutDuration,
  keepProgress, rememberLongFormProgress, keepPlaybackMode,
  showTranslation, showRomanization, lyricBlur, lyricBlurAmount,
  advancedLyrics, preferWordTimedLyrics, defaultLyricSource, dynamicBackground, colorMode, audioReactive,
  coverBlurBg, coverBlurAmount, coverBlurDarken,
  neteaseQuality, qqMusicQuality, youtubeQuality, biliQuality,
  youtubePlaybackSource, neteaseAutoSourceSwitch, neteaseLocalSourceFallback,
  bypassProxy, internationalizationEnabled, exploreSearchHistoryEnabled,
  backgroundImageUri, backgroundImageBlur, backgroundImageAlpha,
  enhancedAdvancedBlur, enhancedAdvancedBlurRadius,
  devModeEnabled, logToFile, logLevel,
  maxCacheSize, downloadNameTemplate, downloadDir,
  downloadParallelism, downloadAutoFillMetadata, downloadEmbedLyrics,
  downloadFollowPlaybackQuality, downloadNeteaseQuality, downloadQqMusicQuality,
  downloadYoutubeQuality, downloadBiliQuality,
  ltServerUrl, ltNickname, ltAllowMemberControl, ltAutoPauseOnMemberChange, ltShareAudioLinks,
  locale: settingLocale,
} = storeToRefs(settings)

const audioDisplayOptions = [
  { key: 'showAudioBitrate', label: 'audio_bitrate', icon: 'speed' },
  { key: 'showAudioFormat', label: 'audio_format', icon: 'audio_file' },
  { key: 'showAudioChannels', label: 'audio_channels', icon: 'speaker_group' },
  { key: 'showAudioSampleRate', label: 'audio_sample_rate', icon: 'graphic_eq' },
  { key: 'showAudioBitDepth', label: 'audio_bit_depth', icon: 'equalizer' },
] as const

const syncFrequencyOptions = computed<Array<{ value: SyncFrequency; label: string }>>(() => [
  { value: 'immediate', label: t('settings.sync_immediate') },
  { value: 'every_10_minutes', label: t('settings.sync_every_10_minutes') },
  { value: 'every_15_minutes', label: t('settings.sync_every_15_minutes') },
  { value: 'every_30_minutes', label: t('settings.sync_every_30_minutes') },
])

const logLevelOptions = computed<Array<{ value: string; label: string }>>(() => [
  { value: 'off', label: t('settings.log_level_off') },
  { value: 'error', label: t('settings.log_level_error') },
  { value: 'warn', label: t('settings.log_level_warn') },
  { value: 'info', label: t('settings.log_level_info') },
  { value: 'debug', label: t('settings.log_level_debug') },
  { value: 'trace', label: t('settings.log_level_trace') },
])

const youtubePlaybackSourceOptions = computed(() => YOUTUBE_PLAYBACK_SOURCES.map(value => ({
  value,
  label: t(`settings.youtube_source_${value}`),
})))
const youtubePlaybackSourceDescription = computed(() => t(`settings.youtube_source_${youtubePlaybackSource.value}_desc`))

const defaultLyricSourceOptions = computed(() => DEFAULT_LYRIC_SOURCES.map(value => ({
  value,
  label: t(`settings.lyric_source_${value}`),
})))
const defaultLyricSourceDescription = computed(() => t(`settings.lyric_source_${defaultLyricSource.value}_desc`))

function changeDefaultLyricSource(value: string) {
  const source = DEFAULT_LYRIC_SOURCES.find(source => source === value)
  if (source) defaultLyricSource.value = source
}

function changeYouTubePlaybackSource(value: string) {
  const source = YOUTUBE_PLAYBACK_SOURCES.find(source => source === value)
  if (source) youtubePlaybackSource.value = source
}

const audioOutputDevices = ref<Array<{ name: string; isDefault: boolean }>>([])
const audioOutputSwitching = ref(false)
const audioOutputOptions = computed(() => {
  const devices = audioOutputDevices.value.map(device => ({
    value: device.name,
    label: device.name,
  }))
  if (audioOutputDevice.value && !devices.some(device => device.value === audioOutputDevice.value)) {
    devices.unshift({ value: audioOutputDevice.value, label: t('settings.audio_output_unavailable', { name: audioOutputDevice.value }) })
  }
  return [{ value: '', label: t('settings.audio_output_default') }, ...devices]
})

async function loadAudioOutputDevices() {
  try {
    audioOutputDevices.value = await invoke('list_audio_output_devices')
  } catch (error) {
    log.warn('failed to list audio output devices:', error)
  }
}

async function changeAudioOutputDevice(selected: string) {
  if (audioOutputSwitching.value || selected === audioOutputDevice.value) return
  audioOutputSwitching.value = true
  try {
    await invoke('set_audio_output_device', { name: selected || null })
    audioOutputDevice.value = selected
  } catch (error) {
    log.warn('failed to switch audio output:', error)
    toast.error(t('settings.audio_output_failed'))
  } finally {
    audioOutputSwitching.value = false
    void loadAudioOutputDevices()
  }
}

async function openLogDir() {
  try {
    const dir = await invoke<string>('get_log_dir')
    // 走后端 reveal_in_file_manager，避免依赖 opener:allow-reveal-item-in-dir 权限
    // （capability 只授了 opener:allow-open-url，revealItemInDir 会被 ACL 拒绝）
    await invoke('reveal_in_file_manager', { path: dir })
  } catch (error) {
    log.error('failed to open log dir:', error)
    toast.error(t('settings.open_log_dir_failed'))
  }
}

// 折叠过渡 hooks
function onExpandEnter(el: Element) {
  const e = el as HTMLElement
  e.style.overflow = 'hidden'
  e.style.height = '0'
  // 强制 reflow
  void e.offsetHeight
  e.style.transition = 'height 300ms cubic-bezier(0.2, 0, 0, 1), opacity 250ms ease'
  e.style.height = e.scrollHeight + 'px'
  e.style.opacity = '1'
}
function onExpandAfterEnter(el: Element) {
  const e = el as HTMLElement
  e.style.height = ''
  e.style.overflow = ''
  e.style.transition = ''
}
function onExpandLeave(el: Element) {
  const e = el as HTMLElement
  e.style.overflow = 'hidden'
  e.style.height = e.scrollHeight + 'px'
  void e.offsetHeight
  e.style.transition = 'height 250ms cubic-bezier(0.3, 0, 0.8, 0.15), opacity 200ms ease'
  e.style.height = '0'
  e.style.opacity = '0'
}
function onExpandAfterLeave(el: Element) {
  const e = el as HTMLElement
  e.style.height = ''
  e.style.overflow = ''
  e.style.transition = ''
  e.style.opacity = ''
}

// 分组切换 out-in 过渡：新面板插入布局后立即恢复滚动位置
// 此时元素已占位但尚未淡入完成，不会产生可见的滚动跳动；
// 若在旧面板离场期间恢复，scrollHeight 不足会导致 scrollTop 被截断
function onPanelEnter() {
  void restoreSettingsScrollPosition(activeSettingsSection.value)
}

const presetColors = THEME_COLORS.map(c => ({
  key: c.key,
  color: c.dark['--md-primary'],
}))

const activeColorKey = ref(selectedColor.value || getSavedThemeColor())

watch(selectedColor, value => {
  activeColorKey.value = value
})

// 动态背景与封面模糊互斥（对齐 Android effectiveDynamicBackgroundEnabled = dynamic && !coverBlur）:
// 启用一方即关闭另一方，避免 coverBlurBg 残留导致动态背景开关看似失效
watch(dynamicBackground, on => {
  if (on && coverBlurBg.value) coverBlurBg.value = false
})
watch(coverBlurBg, on => {
  if (on && dynamicBackground.value) dynamicBackground.value = false
})

// 文件日志开关仅在下次启动生效（受插件限制）：用户切换时提示需重启。
// 用 @change 而非 watch，避免启动 hydrate 时误触发提示
function onLogToFileChange() {
  toast.show(t('settings.log_to_file_restart_hint'), 'info')
}

// 头像地址可能短暂失效，记录具体失败地址，换地址后仍允许重新加载
const failedAvatarUrls = ref<Record<string, string | null>>({})

function isAvatarLoadFailed(platform: string, avatarUrl: string | null) {
  return Boolean(avatarUrl && failedAvatarUrls.value[platform] === avatarUrl)
}

function handleAvatarError(platform: string, avatarUrl: string | null) {
  if (!avatarUrl) return
  failedAvatarUrls.value = { ...failedAvatarUrls.value, [platform]: avatarUrl }
}

function handleColorSwitch(key: string, event: MouseEvent) {
  activeColorKey.value = key
  selectedColor.value = key
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const x = rect.left + rect.width / 2
  const y = rect.top + rect.height / 2
  switchThemeColorWithRipple(key, x, y, false)
}

function toggleSection(key: string) {
  const next = new Set(expandedSections.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  expandedSections.value = next
  persistSettingsUiState()
}
function isExpanded(key: string) { return expandedSections.value.has(key) }

const darkModeOptions = computed(() => [
  { value: 'system', label: t('settings.dark_mode_system'), icon: 'brightness_auto' },
  { value: 'dark', label: t('settings.dark_mode_on'), icon: 'dark_mode' },
  { value: 'light', label: t('settings.dark_mode_off'), icon: 'light_mode' },
])

const darkModeThumbIndex = computed(() => {
  const idx = darkModeOptions.value.findIndex(o => o.value === darkMode.value)
  return idx < 0 ? 0 : idx
})

// 拇指位移：36px 宽 + 2px gap
const darkModeThumbStyle = computed(() => ({
  transform: `translateX(${darkModeThumbIndex.value * 38}px)`,
}))

const colorModeOptions = computed<{ value: ColorMode; label: string; desc: string }[]>(() => [
  { value: 'default', label: t('settings.color_mode_default'), desc: t('settings.color_mode_default_desc') },
  { value: 'cover', label: t('settings.color_mode_cover'), desc: t('settings.color_mode_cover_desc') },
  { value: 'system', label: t('settings.color_mode_system'), desc: t('settings.color_mode_system_desc') },
])

function handleDarkModeSwitch(mode: ThemeMode, event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const x = rect.left + rect.width / 2
  const y = rect.top + rect.height / 2
  // 同步改 mode：拇指 650ms 滑动；ripple 不再禁用 .pill-thumb 的 transition
  darkMode.value = mode as any
  document.documentElement.classList.add('theme-ripple-active')
  void switchThemeWithRipple(mode, x, y, false)
}

function handleLocaleSwitch(code: string, event: MouseEvent) {
  settingLocale.value = code
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const x = rect.left + rect.width / 2
  const y = rect.top + rect.height / 2
  setLocaleWithTransition(code, x, y, false)
}

const defaultScreenOptions = computed(() => [
  { value: 'home', label: t('nav.home') },
  { value: 'explore', label: t('nav.explore') },
  { value: 'library', label: t('nav.library') },
])

const neteaseQualityOptions = computed(() => [
  { value: 'standard', label: t('settings.q_standard') },
  { value: 'higher', label: t('settings.q_high') },
  { value: 'exhigh', label: t('settings.q_exhigh') },
  { value: 'lossless', label: t('settings.q_lossless') },
  { value: 'hires', label: t('settings.q_hires') },
  { value: 'jyeffect', label: t('settings.q_surround') },
  { value: 'sky', label: t('settings.q_sky') },
  { value: 'jymaster', label: t('settings.q_master') },
])

const downloadNeteaseQualityOptions = neteaseQualityOptions

// 与播放页音质列表同一套文案：QQ 的 high 档是「高」而不是网易云的「较高」
const qqQualityOptions = computed(() => [
  { value: 'standard', label: t('settings.q_standard') },
  { value: 'high', label: t('settings.q_high_yt') },
  { value: 'lossless', label: t('settings.q_lossless') },
])

const youtubeQualityOptions = computed(() => [
  { value: 'low', label: t('settings.q_low') },
  { value: 'medium', label: t('settings.q_medium') },
  { value: 'high', label: t('settings.q_high_yt') },
  { value: 'very_high', label: t('settings.q_very_high') },
])

const biliQualityOptions = computed(() => [
  { value: 'low', label: t('settings.q_smooth') },
  { value: 'medium', label: t('settings.q_standard') },
  { value: 'high', label: t('settings.q_good') },
  { value: 'lossless', label: t('settings.q_lossless') },
  { value: 'hires', label: t('settings.q_hires') },
  { value: 'dolby', label: t('settings.q_dolby') },
])

type OnlineQualitySource = 'netease' | 'qq' | 'youtube' | 'bilibili'
const qualitySwitching = ref(false)

function qualityForSource(source: OnlineQualitySource): string {
  if (source === 'netease') return neteaseQuality.value
  if (source === 'qq') return qqMusicQuality.value
  if (source === 'youtube') return youtubeQuality.value
  return biliQuality.value
}

function setQualityForSource(source: OnlineQualitySource, value: string) {
  if (source === 'netease') neteaseQuality.value = value
  else if (source === 'qq') qqMusicQuality.value = value
  else if (source === 'youtube') youtubeQuality.value = value
  else biliQuality.value = value
}

async function handleQualityChange(source: OnlineQualitySource, value: string) {
  const previous = qualityForSource(source)
  if (previous === value || qualitySwitching.value) return

  setQualityForSource(source, value)
  const track = player.currentTrack
  const isCurrentSource = !!track && track.id.startsWith(`${source}:`)
  if (!track || player.isLoadingAudio || player.isPlayingFromDownload || !isCurrentSource) return

  qualitySwitching.value = true
  try {
    await player.replayWithQuality()
  } catch (error) {
    setQualityForSource(source, previous)
    const message = error instanceof Error ? error.message : String(error)
    toast.error(message)
  } finally {
    qualitySwitching.value = false
  }
}

type SettingsSectionId =
  | 'accounts'
  | 'personalization'
  | 'playback'
  | 'playback_sources'
  | 'quality'
  | 'motion'
  | 'lyrics'
  | 'network'
  | 'storage'
  | 'backup'
  | 'listen_together'
  | 'language'
  | 'about'

const SETTINGS_UI_STATE_KEY = 'neri:settings-ui-state'
const SETTINGS_SECTION_IDS: SettingsSectionId[] = [
  'accounts', 'playback', 'playback_sources', 'quality', 'storage', 'personalization', 'motion', 'lyrics', 'network',
  'backup', 'listen_together', 'language', 'about',
]

type SettingsUiState = {
  activeSection?: SettingsSectionId
  expandedSections?: string[]
  visitedSections?: string[]
  scrollPositions?: Record<string, number>
}

function readSettingsUiState(): SettingsUiState {
  try {
    const parsed = JSON.parse(localStorage.getItem(SETTINGS_UI_STATE_KEY) || '{}') as SettingsUiState
    return SETTINGS_SECTION_IDS.includes(parsed.activeSection as SettingsSectionId) ? parsed : {}
  } catch {
    return {}
  }
}

const savedSettingsUiState = readSettingsUiState()
const expandedSections = ref<Set<string>>(
  new Set(savedSettingsUiState.expandedSections || ['personal']),
)
const visitedSettingsSections = ref<Set<string>>(
  new Set(savedSettingsUiState.visitedSections || ['personalization']),
)
const activeSettingsSection = ref<SettingsSectionId>(
  savedSettingsUiState.activeSection || 'personalization',
)
const sectionScrollPositions = ref<Record<string, number>>(savedSettingsUiState.scrollPositions || {})
const settingsRootRef = ref<HTMLElement | null>(null)
const settingsContentRef = ref<HTMLElement | null>(null)

function settingsScrollContainer() {
  const settingsContent = settingsContentRef.value
  if (!settingsContent) return null
  if (settingsContent.scrollHeight > settingsContent.clientHeight + 1) return settingsContent
  return settingsContent.closest('.content') as HTMLElement | null
}

function persistSettingsUiState() {
  try {
    localStorage.setItem(SETTINGS_UI_STATE_KEY, JSON.stringify({
      activeSection: activeSettingsSection.value,
      expandedSections: Array.from(expandedSections.value),
      visitedSections: Array.from(visitedSettingsSections.value),
      scrollPositions: sectionScrollPositions.value,
    }))
  } catch {
    // UI 状态保存失败不影响设置项
  }
}

function rememberSettingsScrollPosition() {
  const container = settingsScrollContainer()
  if (!container) return
  sectionScrollPositions.value = {
    ...sectionScrollPositions.value,
    [activeSettingsSection.value]: container.scrollTop,
  }
}

async function restoreSettingsScrollPosition(section: SettingsSectionId) {
  await nextTick()
  const container = settingsScrollContainer()
  if (!container) return
  container.scrollTo({
    top: Math.max(0, sectionScrollPositions.value[section] || 0),
    behavior: 'auto',
  })
}

const settingsNavGroups = computed(() => [
  {
    items: [
      {
        id: 'accounts' as SettingsSectionId,
        label: t('settings.accounts'),
        description: t('settings.nav_accounts_desc'),
        icon: 'account_circle',
      },
    ],
  },
  {
    items: [
      {
        id: 'playback' as SettingsSectionId,
        label: t('settings.playback'),
        description: t('settings.nav_playback_desc'),
        icon: 'play_circle',
      },
      {
        id: 'quality' as SettingsSectionId,
        label: t('settings.audio_quality'),
        description: t('settings.audio_quality_desc'),
        icon: 'high_quality',
      },
      {
        id: 'playback_sources' as SettingsSectionId,
        label: t('settings.playback_sources'),
        description: t('settings.playback_sources_desc'),
        icon: 'alt_route',
      },
      {
        id: 'storage' as SettingsSectionId,
        label: t('settings.storage'),
        description: t('settings.nav_storage_desc'),
        icon: 'storage',
      },
    ],
  },
  {
    items: [
      {
        id: 'personalization' as SettingsSectionId,
        label: t('settings.personalization'),
        description: t('settings.nav_personalization_desc'),
        icon: 'tune',
      },
      {
        id: 'motion' as SettingsSectionId,
        label: t('settings.motion'),
        description: t('settings.motion_desc'),
        icon: 'bolt',
      },
      {
        id: 'lyrics' as SettingsSectionId,
        label: t('settings.lyrics'),
        description: t('settings.nav_lyrics_desc'),
        icon: 'lyrics',
      },
      {
        id: 'network' as SettingsSectionId,
        label: t('settings.network'),
        description: t('settings.nav_network_desc'),
        icon: 'router',
      },
    ],
  },
  {
    items: [
      {
        id: 'backup' as SettingsSectionId,
        label: t('settings.backup'),
        description: t('settings.nav_backup_desc'),
        icon: 'sync',
      },
      {
        id: 'listen_together' as SettingsSectionId,
        label: t('listen_together.title'),
        description: t('settings.nav_listen_together_desc'),
        icon: 'cloud',
      },
      {
        id: 'language' as SettingsSectionId,
        label: t('settings.language'),
        description: t('settings.language_desc'),
        icon: 'translate',
      },
      {
        id: 'about' as SettingsSectionId,
        label: t('settings.about'),
        description: t('settings.about_desc'),
        icon: 'info',
      },
    ],
  },
])

const activeSettingsItem = computed(() => settingsNavGroups.value
  .flatMap(group => group.items)
  .find(item => item.id === activeSettingsSection.value))

function selectSettingsSection(id: SettingsSectionId) {
  if (id === activeSettingsSection.value) return
  rememberSettingsScrollPosition()
  activeSettingsSection.value = id
  if (!visitedSettingsSections.value.has(id)) {
    const defaults: Record<SettingsSectionId, string[]> = {
      accounts: [],
      personalization: ['personal'],
      playback: ['playback'],
      playback_sources: [],
      quality: ['quality'],
      motion: ['effects'],
      lyrics: ['lyrics'],
      network: [],
      storage: ['storage'],
      backup: ['backup'],
      listen_together: ['listen_together'],
      language: [],
      about: [],
    }
    expandedSections.value = new Set([...expandedSections.value, ...defaults[id]])
    visitedSettingsSections.value = new Set([...visitedSettingsSections.value, id])
  }
  persistSettingsUiState()
  // 滚动位置恢复改由面板过渡的 onPanelEnter 钩子触发（等新面板插入布局后再恢复）
}

// 其它地方（桌面歌词工具栏的「设置」）用 ?section=lyrics&focus=desktop-lyrics 直接打开某个分区
const route = useRoute()
function applyRouteSection() {
  const section = String(route.query.section || '')
  if (!SETTINGS_SECTION_IDS.includes(section as SettingsSectionId)) return
  selectSettingsSection(section as SettingsSectionId)
  const focus = String(route.query.focus || '')
  if (focus === 'desktop-lyrics') {
    expandedSections.value = new Set([...expandedSections.value, 'desktop_lyrics'])
    void nextTick(() => {
      setTimeout(() => document.getElementById('settings-desktop-lyrics')?.scrollIntoView({ block: 'start', behavior: 'smooth' }), 260)
    })
  }
}
watch(() => [route.query.section, route.query.focus], applyRouteSection)

// 启动时检查登录状态
onMounted(() => {
  auth.checkStatus()
  syncStore.loadConfigs()
  void downloadStore.initEvents().catch(error => log.error('Download listener failed:', error))
  downloadStore.loadDownloads()
  // 加载构建信息
  loadBuildInfo()
  // 加载默认下载目录
  loadDefaultDownloadDir()
  void loadAudioOutputDevices()
  void restoreSettingsScrollPosition(activeSettingsSection.value)
  refreshDesktopLyricsAccent()
  applyRouteSection()
})

onBeforeUnmount(() => {
  rememberSettingsScrollPosition()
  persistSettingsUiState()
  // 开发者模式 7-tap 计时器兜底清理
  if (tapTimer) clearTimeout(tapTimer)
})

// 网络：绕过代理
async function handleBypassProxyChange(val: boolean) {
  const previous = bypassProxy.value
  bypassProxy.value = val
  try {
    await invoke('set_bypass_proxy', { bypass: val })
  } catch (e) {
    log.error('Failed to set bypass proxy:', e)
    bypassProxy.value = previous
    toast.error(t('settings.bypass_proxy_failed'))
  }
}

// 一起听
const showResetLtIdentityConfirm = ref(false)

function confirmResetLtIdentity() {
  showResetLtIdentityConfirm.value = false
  if (lt.resetIdentity()) toast.success(t('listen_together.identity_reset'))
  else toast.error(t('listen_together.reset_identity_in_room'))
}

// 与协议校验同一规则：去首尾空白，空值回到默认昵称，非法值不保存
function handleLtNicknameChange(event: Event) {
  const input = event.target as HTMLInputElement
  const value = input.value.trim()
  if (value && !isValidLtNickname(value)) {
    toast.error(t('listen_together.invalid_nickname'))
    input.value = ltNickname.value
    return
  }
  ltNickname.value = value
  input.value = value
}

const showConfigExportWarning = ref(false)

async function confirmConfigExport() {
  showConfigExportWarning.value = false
  await syncStore.exportConfig()
}

// 导入只写回了设置值：主题、强调色和播放引擎（均衡器、响度、倍速等）要按新值重新应用
async function importConfig() {
  const result = await syncStore.importConfig()
  if (!result?.success) return
  applyTheme(darkMode.value, false)
  // 三态取色：仅「默认取色」需要按预设主题色重刷，system/cover 由动态取色路径接管
  if (colorMode.value === 'default') applyThemeColor(selectedColor.value, undefined, false)
  lt.reloadIdentity()
  await player.applyPersistedSettings()
}

// 下载管理
const activeDownloadCount = computed(() => downloadStore.runningDownloadCount)
const completedDownloadCount = computed(() => downloadStore.downloads.length)
const activeDownloadTasks = computed(() => downloadStore.activeDownloads)

function formatDownloadSize(bytes?: number): string {
  if (typeof bytes !== 'number') return ''
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function activeDownloadStatusText(status: string) {
  switch (status) {
    case 'queued': return t('download.queued')
    case 'resolving': return t('download.resolving')
    case 'processing': return t('download.processing')
    case 'cancelling': return t('download.cancelling')
    case 'cancelled': return t('download.cancelled')
    case 'error': return t('download.download_failed')
    case 'already_exists': return t('download.already_exists')
    default: return t('download.downloading')
  }
}

function activeDownloadProgressText(task: {
  status: string
  progress?: number
  downloadedBytes?: number
  totalBytes?: number
  message?: string
}) {
  if (['queued', 'resolving', 'processing', 'cancelling', 'cancelled', 'already_exists'].includes(task.status)) {
    return activeDownloadStatusText(task.status)
  }
  if (task.status === 'error') {
    return task.message
      ? `${activeDownloadStatusText(task.status)} · ${task.message}`
      : activeDownloadStatusText(task.status)
  }

  const downloaded = typeof task.downloadedBytes === 'number' ? formatDownloadSize(task.downloadedBytes) : ''
  const total = typeof task.totalBytes === 'number' && task.totalBytes > 0 ? formatDownloadSize(task.totalBytes) : ''
  const percent = typeof task.progress === 'number' ? `${task.progress}%` : ''

  if (downloaded && total && percent) return `${downloaded} / ${total} · ${percent}`
  if (downloaded && total) return `${downloaded} / ${total}`
  if (downloaded && percent) return `${downloaded} · ${percent}`
  return activeDownloadStatusText(task.status)
}

// 下载目录
const defaultDownloadDir = ref('')

async function loadDefaultDownloadDir() {
  try {
    defaultDownloadDir.value = await invoke<string>('get_default_download_dir')
  } catch (e) {
    log.error('Failed to get default download dir:', e)
  }
}

const displayDownloadDir = computed(() => downloadDir.value || defaultDownloadDir.value || '...')

async function selectDownloadDir() {
  try {
    const result = await dialogOpen({ directory: true })
    if (result) {
      const path = typeof result === 'string' ? result : (result as any).path || String(result)
      const validated = await invoke<string>('set_download_dir', { path })
      downloadDir.value = validated
      toast.success(t('settings.download_dir_changed'))
    }
  } catch (e: any) {
    log.error('Failed to set download dir:', e)
    toast.error(t('settings.download_dir_invalid'))
  }
}

function resetDownloadDir() {
  downloadDir.value = ''
}

// 下载文件名格式
const showDownloadTemplateDialog = ref(false)
const pendingTemplate = ref('')

function openDownloadTemplateDialog() {
  pendingTemplate.value = downloadNameTemplate.value || DEFAULT_DOWNLOAD_NAME_TEMPLATE
  showDownloadTemplateDialog.value = true
}

function replaceTemplateToken(template: string, token: string, value: string): string {
  return template.split(token).join(value)
}

const templatePreview = computed(() => {
  let preview = pendingTemplate.value || DEFAULT_DOWNLOAD_NAME_TEMPLATE
  for (const [token, value] of [
    ['{title}', '晴天'], ['%title%', '晴天'],
    ['{artist}', '周杰伦'], ['%artist%', '周杰伦'],
    ['{album}', '叶惠美'], ['%album%', '叶惠美'],
    ['{source}', 'netease'], ['%source%', 'netease'],
    ['{id}', 'netease:123456'], ['%id%', 'netease:123456'],
    ['{audioId}', '123456'], ['%audioId%', '123456'],
    ['{subAudioId}', ''], ['%subAudioId%', ''],
    ['{hash}', '4c853a1f'], ['%hash%', '4c853a1f'],
  ]) {
    preview = replaceTemplateToken(preview, token, value)
  }
  return preview
})

function applyDownloadTemplate() {
  downloadNameTemplate.value = pendingTemplate.value.trim() || DEFAULT_DOWNLOAD_NAME_TEMPLATE
  showDownloadTemplateDialog.value = false
}

function resetDownloadTemplate() {
  pendingTemplate.value = DEFAULT_DOWNLOAD_NAME_TEMPLATE
  downloadNameTemplate.value = DEFAULT_DOWNLOAD_NAME_TEMPLATE
  showDownloadTemplateDialog.value = false
}

function cancelAllDownloads() {
  downloadStore.cancelAllDownloads()
}

function cancelDownload(trackId: string) {
  downloadStore.cancelDownload(trackId)
}

function goToDownloads() {
  router.push('/downloads')
}

const showStorageManagement = ref(false)
const storageLoading = ref(false)
const storageClearing = ref(false)
const storageSummary = ref<StorageUsageSummary | null>(null)

async function loadStorageUsage() {
  storageLoading.value = true
  try {
    const result = await invoke<StorageUsageSummary>('get_storage_usage', {
      downloadDir: downloadDir.value || null,
    })
    storageSummary.value = mergeBrowserCacheUsage(result)
  } catch (error) {
    log.error('Failed to read storage usage:', error)
    toast.error(t('settings.storage_load_failed'))
  } finally {
    storageLoading.value = false
  }
}

async function openStorageManagement() {
  showStorageManagement.value = true
  await loadStorageUsage()
}

async function clearStorageCache(options: StorageCacheClearOptions) {
  storageClearing.value = true
  try {
    const result = await invoke<{ clearedBytes: number; deletedFiles: number; failedCount: number }>(
      'clear_storage_cache',
      { options, downloadDir: downloadDir.value || null },
    )
    clearBrowserCache(options)
    const mb = ((result.clearedBytes || 0) / 1024 / 1024).toFixed(1)
    if (result.failedCount > 0) {
      toast.show(t('settings.storage_cache_partial', { mb, count: result.failedCount }), 'info')
    } else if (result.clearedBytes === 0) {
      toast.show(t('settings.storage_cache_empty'), 'info')
    } else {
      toast.success(t('settings.storage_cache_cleared', { mb }))
    }
    await loadStorageUsage()
  } catch (error) {
    log.error('Failed to clear storage cache:', error)
    toast.error(t('settings.storage_cache_failed'))
  } finally {
    storageClearing.value = false
  }
}

// YouTube 国际化
const intlChecking = ref(false)

// 对齐 Android：开关直接生效。开启时只探测传输层连通性并提示，不回退用户的选择
async function handleIntlToggle(val: boolean) {
  if (intlChecking.value) return
  internationalizationEnabled.value = val
  if (!val) return
  intlChecking.value = true
  try {
    const reachable = await invoke<boolean>('probe_platform_connectivity', { platform: 'youtube' })
    if (!reachable) toast.show(t('settings.intl_check_failed'), 'info')
  } catch (error) {
    log.warn('YouTube connectivity probe failed:', error)
  } finally {
    intlChecking.value = false
  }
}

// 键盘快捷键说明（修饰键按平台自动切换 ⌘ / Ctrl）
const keyboardShortcuts = computed(() =>
  shortcutDescriptors().map((item) => ({
    ...item,
    label: t(`shortcuts.${item.id}`),
  })),
)

// 封面样式选项
const coverStyleOptions = computed(() => [
  { value: 'disc', label: t('settings.cover_style_disc') },
  { value: 'card', label: t('settings.cover_style_card') },
])

// 背景图片选择
async function selectBackgroundImage() {
  try {
    const result = await dialogOpen({
      multiple: false,
      filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp'] }],
    })
    if (result) {
      const picked = typeof result === 'string' ? result : (result as any).path || String(result)
      backgroundImageUri.value = await invoke<string>('import_background_image', { source: picked })
    }
  } catch (e) {
    log.error('Failed to select image:', e)
    toast.error(t('settings.background_image_failed'))
  }
}

function clearBackgroundImage() {
  backgroundImageUri.value = ''
  invoke('clear_background_images').catch((error) => log.warn('Failed to delete background copies:', error))
}

// 开发者模式：7-tap 解锁
const versionTapCount = ref(0)
let tapTimer: ReturnType<typeof setTimeout> | null = null

function handleVersionTap() {
  toggleSection('about')
  if (devModeEnabled.value) return // 已解锁
  versionTapCount.value++
  if (tapTimer) clearTimeout(tapTimer)
  tapTimer = setTimeout(() => { versionTapCount.value = 0 }, 3000)

  const remaining = 7 - versionTapCount.value
  if (remaining <= 0) {
    devModeEnabled.value = true
    versionTapCount.value = 0
    toast.success(t('settings.dev_mode_toast'))
  } else if (remaining <= 3) {
    toast.success(t('settings.dev_mode_tap_hint', { count: remaining }))
  }
}

// 构建信息
interface BuildInfo {
  app_version: string
  build_uuid: string
  build_timestamp: string
  version: string
}

const buildInfo = ref<BuildInfo | null>(null)

async function loadBuildInfo() {
  try {
    buildInfo.value = await invoke('get_build_info')
  } catch (e) {
    log.error('Failed to load build info:', e)
  }
}

async function copyBuildValue(value: string | undefined | null, event?: Event) {
  event?.stopPropagation()
  const text = value?.trim()
  if (!text) return
  try {
    await writeText(text)
    toast.success(t('settings.build_info_copied'))
  } catch (e) {
    log.error('Failed to copy build info:', e)
    toast.error(t('settings.build_info_copy_failed'))
  }
}

// GitHub 同步引导（对齐 Android）
const showGitHubDialog = ref(false)
const githubPhase = ref<1 | 2>(1) // 当前步骤：1=token 验证，2=仓库选择
const githubToken = ref('')
const githubUsername = ref('')
const githubIsValidating = ref(false)
const githubRepoMode = ref<'create' | 'existing'>('create')
const githubNewRepoName = ref('neriplayer-backup')
const githubExistingRepo = ref('') // owner/repo 格式
const githubIsSettingRepo = ref(false)
const GITHUB_TOKEN_URL = 'https://github.com/settings/tokens/new?scopes=repo&description=NeriPlayer%20Backup'
// 各歌词来源的默认偏移，顺序与默认值对齐 Android 设置页
const lyricOffsetSettings = [
  { key: 'cloudMusicOffset', label: 'settings.netease_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.netease, iconSvg: '/icons/ic_netease.svg' },
  { key: 'qqMusicOffset', label: 'settings.qq_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.qq, iconSvg: '/icons/ic_qq_music.svg' },
  { key: 'kugouOffset', label: 'settings.kugou_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.kugou, iconSvg: '/icons/ic_kugou.svg' },
  { key: 'lrclibOffset', label: 'settings.lrclib_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.lrclib, iconSvg: '/icons/ic_lrclib.svg' },
  { key: 'amllTtmlOffset', label: 'settings.amll_ttml_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.amll_ttml, iconSvg: '/icons/ic_amll.svg' },
] as const

const lyricOffsetsChanged = computed(() => lyricOffsetSettings.some(item => settings[item.key] !== item.defaultMs))

function resetLyricOffsets() {
  for (const item of lyricOffsetSettings) settings[item.key] = item.defaultMs
}

// ---- 桌面歌词 ----
const desktopLyricsStyle = computed(() => settings.desktopLyrics)

function updateDesktopLyrics(patch: Partial<DesktopLyricsStyle>) {
  settings.desktopLyrics = normalizeDesktopLyricsStyle({ ...settings.desktopLyrics, ...patch })
}

/** 改任一颜色就变成自定义配色 */
function setDesktopLyricsGradient(key: 'playedColors' | 'unplayedColors', index: 0 | 1, value: string) {
  const colors = [...desktopLyricsStyle.value[key]] as [string, string]
  colors[index] = value
  updateDesktopLyrics({ [key]: colors, theme: 'custom' })
}

function chooseDesktopLyricsTheme(id: string) {
  settings.desktopLyrics = applyDesktopLyricsTheme(desktopLyricsStyle.value, id)
}

function resetDesktopLyrics() {
  const { bounds, locked } = desktopLyricsStyle.value
  settings.desktopLyrics = normalizeDesktopLyricsStyle({ ...DEFAULT_DESKTOP_LYRICS_STYLE, bounds, locked })
}

const desktopLyricsBusy = ref(false)
async function toggleDesktopLyrics(open: boolean) {
  if (desktopLyricsBusy.value) return
  desktopLyricsBusy.value = true
  try {
    if (open) await openDesktopLyricsWindow()
    else await closeDesktopLyricsWindow()
  } catch (error) {
    log.error('Desktop lyrics toggle failed:', error)
    toast.error(t('desktop_lyrics.open_failed'))
  } finally {
    desktopLyricsBusy.value = false
  }
}

const CUSTOM_FONT = '__custom__'
const desktopLyricsCustomFont = ref(false)
const desktopLyricsFontOptions = computed(() => [
  { value: '', label: t('desktop_lyrics.font_default') },
  ...DESKTOP_LYRICS_FONT_PRESETS.map(font => ({ value: font, label: font })),
  { value: CUSTOM_FONT, label: t('desktop_lyrics.font_custom') },
])
const desktopLyricsFontChoice = computed(() => {
  const family = desktopLyricsStyle.value.fontFamily
  if (desktopLyricsCustomFont.value) return CUSTOM_FONT
  return !family || (DESKTOP_LYRICS_FONT_PRESETS as readonly string[]).includes(family) ? family : CUSTOM_FONT
})
function chooseDesktopLyricsFont(value: string) {
  desktopLyricsCustomFont.value = value === CUSTOM_FONT
  if (value !== CUSTOM_FONT) updateDesktopLyrics({ fontFamily: value })
}
const desktopLyricsWeightOptions = computed(() => DESKTOP_LYRICS_WEIGHTS.map(weight => ({ value: String(weight), label: String(weight) })))
const desktopLyricsSecondaryOptions = computed(() => DESKTOP_LYRICS_SECONDARY.map(value => ({
  value, label: t(`desktop_lyrics.secondary_${value}`),
})))

function themeSwatch(id: string): string {
  if (id === 'accent') {
    const accent = desktopLyricsAccent.value || '#d0bcff'
    return `linear-gradient(135deg, #ffffff, ${accent})`
  }
  const theme = DESKTOP_LYRICS_THEMES.find(candidate => candidate.id === id)
  return theme ? `linear-gradient(135deg, ${theme.played[0]}, ${theme.played[1]})` : 'transparent'
}

// 预览：两句示例歌词循环播放，第一句逐字
const PREVIEW_LOOP_MS = 7_600
const desktopLyricsAccent = ref<string | null>(null)
const desktopLyricsPreviewLines = computed<DesktopLyricsFrameLine[]>(() => {
  const characters = Array.from(t('desktop_lyrics.preview_line'))
  const step = Math.max(120, Math.floor(3_000 / Math.max(1, characters.length)))
  const words = characters.map((text, index) => ({ startMs: 400 + index * step, durationMs: step, text }))
  const firstEnd = 400 + characters.length * step
  return [
    { startMs: 400, durationMs: firstEnd - 400, text: characters.join(''), translation: t('desktop_lyrics.preview_translation'), roman: 'desktop lyrics preview', words },
    { startMs: firstEnd + 300, durationMs: 3_000, text: t('desktop_lyrics.preview_next'), translation: '', roman: '', words: [] },
  ]
})
function desktopLyricsPreviewClock(): number {
  return performance.now() % PREVIEW_LOOP_MS
}
function refreshDesktopLyricsAccent() {
  desktopLyricsAccent.value = parseCssColor(getComputedStyle(document.documentElement).getPropertyValue('--md-primary'))
}

const volumeBalanceLabel = computed(() => {
  const percent = Math.round(Math.abs(volumeBalance.value) * 100)
  if (percent === 0) return t('settings.volume_balance_center')
  return t(volumeBalance.value < 0 ? 'settings.volume_balance_left' : 'settings.volume_balance_right', { percent })
})

const PROJECT_REPOSITORY_URL = 'https://github.com/cwuom/NeriPlayer-Desktop'
const FFMPEG_LEGAL_URL = 'https://ffmpeg.org/legal.html'
// 随包的 FFmpeg 是否加载成功、加载的是哪个版本；没加载上时说明哪些格式受影响
const ffmpegComponentText = computed(() => {
  const capabilities = player.decoderCapabilities
  if (capabilities?.ffmpeg) return t('settings.ffmpeg_component_loaded', { version: capabilities.ffmpeg.avcodec })
  return t('settings.ffmpeg_component_unavailable')
})

function openGitHubSetup() {
  githubPhase.value = 1
  githubToken.value = ''
  githubUsername.value = ''
  githubRepoMode.value = 'create'
  githubNewRepoName.value = 'neriplayer-backup'
  githubExistingRepo.value = ''
  syncStore.dialogError = null
  showGitHubDialog.value = true
}

async function githubValidateToken() {
  if (!githubToken.value.trim()) return
  githubIsValidating.value = true
  syncStore.dialogError = null
  const username = await syncStore.validateGitHubToken(githubToken.value)
  githubIsValidating.value = false
  if (username) {
    githubUsername.value = username
    githubPhase.value = 2
  }
}

async function githubFinishSetup() {
  githubIsSettingRepo.value = true
  syncStore.dialogError = null
  let ok = false
  if (githubRepoMode.value === 'create') {
    ok = await syncStore.createGitHubRepo(githubNewRepoName.value || 'neriplayer-backup')
  } else {
    // 解析 owner/repo 格式
    const parts = githubExistingRepo.value.split('/')
    if (parts.length === 2 && parts[0] && parts[1]) {
      ok = await syncStore.useExistingGitHubRepo(parts[0], parts[1])
    } else {
      syncStore.dialogError = t('settings.github_repo_format_hint')
    }
  }
  githubIsSettingRepo.value = false
  if (ok) showGitHubDialog.value = false
}

async function openExternalUrl(url: string) {
  try {
    await openUrl(url)
  } catch (error) {
    log.error('Failed to open external URL', error)
    toast.error(t('settings.open_external_link_failed'))
  }
}

async function openGitHubTokenPage() {
  await openExternalUrl(GITHUB_TOKEN_URL)
}

// WebDAV 同步对话框状态
const showWebDavDialog = ref(false)
const webdavUrl = ref('')
const webdavUsername = ref('')
const webdavPassword = ref('')
const webdavBasePath = ref('')
const webdavConfiguring = ref(false)

async function configureWebDav() {
  if (!webdavUrl.value.trim() || !webdavUsername.value.trim() || !webdavPassword.value) return
  webdavConfiguring.value = true
  const ok = await syncStore.configureWebDav(
    webdavUrl.value, webdavUsername.value, webdavPassword.value, webdavBasePath.value || undefined,
  )
  webdavConfiguring.value = false
  if (ok) {
    showWebDavDialog.value = false
    webdavUrl.value = ''
    webdavUsername.value = ''
    webdavPassword.value = ''
    webdavBasePath.value = ''
  }
}

function formatSyncTime(ms: number): string {
  if (!ms) return ''
  return new Date(ms).toLocaleString(locale.value)
}

// 平台账号配置
const platformAccounts = computed(() => [
  { key: 'netease', label: t('settings.netease_account'), iconSvg: '/icons/ic_netease.svg', auth: auth.netease, login: auth.loginNetease },
  { key: 'bilibili', label: t('settings.bilibili_account'), iconSvg: '/icons/ic_bilibili.svg', auth: auth.bilibili, login: auth.loginBilibili },
  { key: 'youtube', label: t('settings.youtube_account'), iconSvg: '/icons/ic_youtube.svg', auth: auth.youtube, login: auth.loginYoutube },
])

// 退出登录确认对话框
const showLogoutConfirm = ref(false)
const logoutTargetKey = ref('')
const logoutTargetLabel = ref('')

function requestLogout(key: string, label: string) {
  logoutTargetKey.value = key
  logoutTargetLabel.value = label
  showLogoutConfirm.value = true
}

async function confirmLogout() {
  showLogoutConfirm.value = false
  await auth.logout(logoutTargetKey.value)
}

// 清除 GitHub 配置确认
const showClearGitHubConfirm = ref(false)
async function confirmClearGitHub() {
  showClearGitHubConfirm.value = false
  await syncStore.disconnectGitHub()
}

const hideProtocolUpgrade = ref(false)
const upgradeDevicesConfirmed = ref(false)
watch(() => syncStore.pendingProtocolUpgrade, () => {
  hideProtocolUpgrade.value = false
  upgradeDevicesConfirmed.value = false
})

// 页内旧式对话框（.dialog-overlay）同样响应 Escape，并与 M3Dialog 共用弹层栈
for (const dialog of [
  showGitHubDialog,
  showWebDavDialog,
  showLogoutConfirm,
  showClearGitHubConfirm,
  showResetLtIdentityConfirm,
  showConfigExportWarning,
  showDownloadTemplateDialog,
]) {
  useEscapeClose(() => dialog.value, () => { dialog.value = false })
}
useEscapeClose(
  () => !!syncStore.pendingProtocolUpgrade && !hideProtocolUpgrade.value,
  () => { hideProtocolUpgrade.value = true },
)
</script>

<template>
  <div ref="settingsRootRef" class="settings-view">
    <div class="settings-layout">
      <aside class="settings-sidebar">
        <h1 class="page-title">{{ t('settings.title') }}</h1>
        <nav class="settings-nav" :aria-label="t('settings.title')">
          <div v-for="(group, groupIndex) in settingsNavGroups" :key="groupIndex" class="settings-nav-group">
            <button
              v-for="item in group.items"
              :key="item.id"
              class="settings-nav-item"
              :class="{ active: activeSettingsSection === item.id }"
              type="button"
              :aria-current="activeSettingsSection === item.id ? 'page' : undefined"
              @click="selectSettingsSection(item.id)"
            >
              <span class="settings-nav-icon material-symbols-rounded">{{ item.icon }}</span>
              <span class="settings-nav-copy">
                <strong>{{ item.label }}</strong>
                <small>{{ item.description }}</small>
              </span>
              <span class="material-symbols-rounded settings-nav-arrow">chevron_right</span>
            </button>
          </div>
        </nav>
      </aside>

      <main class="settings-content">
        <header class="settings-content-header">
          <div>
            <h2>{{ activeSettingsItem?.label }}</h2>
            <p>{{ activeSettingsItem?.description }}</p>
          </div>
        </header>

        <!-- 头部固定不滚动，仅面板区滚动；切换分组时以分组 id 为 key 做 out-in 交叉淡入 -->
        <div ref="settingsContentRef" class="settings-content-scroll">
        <Transition name="fade" mode="out-in" @enter="onPanelEnter">
        <div :key="activeSettingsSection" class="settings-panels">

        <div v-show="activeSettingsSection === 'accounts'" class="settings-section-panel">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">account_circle</span>
      <span>{{ t('settings.accounts') }}</span>
    </div>
    <p class="settings-section-intro">{{ t('settings.accounts_desc') }}</p>

    <div
      v-for="account in platformAccounts"
      :key="account.key"
      class="setting-card account-card"
    >
      <div class="setting-icon-wrap">
        <span class="platform-icon" :style="{ maskImage: `url(${account.iconSvg})` }"></span>
      </div>
      <div class="setting-info">
        <div class="setting-title">{{ account.label }}</div>
        <div class="setting-desc" v-if="account.auth.loggedIn">
          {{ t('settings.signed_in_as', { name: account.auth.nickname || '—' }) }}
        </div>
        <div class="setting-desc" v-else-if="auth.loggingIn === account.key">
          {{ t('settings.signing_in') }}
        </div>
        <div class="account-status" :class="{ connected: account.auth.loggedIn }">
          <span class="account-status-dot"></span>
          {{ account.auth.loggedIn ? t('settings.account_connected') : t('settings.account_not_connected') }}
        </div>
      </div>
      <template v-if="account.auth.loggedIn">
        <BilibiliCoverImage
          v-if="account.key === 'bilibili' && account.auth.avatarUrl"
          :src="account.auth.avatarUrl"
          class="account-avatar"
        >
          <span class="material-symbols-rounded account-avatar account-avatar-fallback">account_circle</span>
        </BilibiliCoverImage>
        <img
          v-else-if="account.auth.avatarUrl && !isAvatarLoadFailed(account.key, account.auth.avatarUrl)"
          :src="account.auth.avatarUrl"
          class="account-avatar"
          referrerpolicy="no-referrer"
          @error="handleAvatarError(account.key, account.auth.avatarUrl)"
        />
        <span v-else class="material-symbols-rounded account-avatar account-avatar-fallback">account_circle</span>
        <button class="account-logout-btn" @click="requestLogout(account.key, account.label)">
          <span class="material-symbols-rounded" style="font-size: 16px">logout</span>
          {{ t('settings.sign_out') }}
        </button>
      </template>
      <template v-else>
        <button
          class="account-login-btn"
          :disabled="auth.loggingIn === account.key"
          @click="account.login()"
        >
          <span v-if="auth.loggingIn === account.key" class="material-symbols-rounded spin" style="font-size: 18px">progress_activity</span>
          <span v-else>{{ t('settings.sign_in') }}</span>
        </button>
      </template>
    </div>
        </div>

        <div v-show="activeSettingsSection === 'personalization'" class="settings-section-panel">

    <!-- YouTube 国际化 -->
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">language</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.internationalization') }}</div>
        <div class="setting-desc">
          <template v-if="intlChecking">{{ t('settings.intl_checking') }}</template>
          <template v-else>{{ t('settings.internationalization_desc') }}</template>
        </div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" :checked="internationalizationEnabled" :disabled="intlChecking" @change="handleIntlToggle(($event.target as HTMLInputElement).checked)" />
        <span class="track"><span class="thumb">
          <span v-if="intlChecking" class="material-symbols-rounded spinning" style="font-size: 14px">progress_activity</span>
          <span v-else-if="internationalizationEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span>
        </span></span>
      </label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">manage_search</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.explore_search_history') }}</div>
        <div class="setting-desc">{{ t('settings.explore_search_history_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="exploreSearchHistoryEnabled" /><span class="track"><span class="thumb"><span v-if="exploreSearchHistoryEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <!-- 外观 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">palette</span>
      <span>{{ t('settings.appearance') }}</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap">
        <span class="material-symbols-rounded filled">dark_mode</span>
      </div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.dark_mode') }}</div>
        <div class="setting-desc">{{ darkModeOptions.find(o => o.value === darkMode)?.label }}</div>
      </div>
      <div class="dark-mode-pills">
        <span
          class="pill-thumb"
          :style="darkModeThumbStyle"
          aria-hidden="true"
        />
        <button
          v-for="opt in darkModeOptions"
          :key="opt.value"
          class="pill"
          :class="{ active: darkMode === opt.value }"
          @click="handleDarkModeSwitch(opt.value as ThemeMode, $event)"
        >
          <span class="material-symbols-rounded" style="font-size: 16px">{{ opt.icon }}</span>
        </button>
      </div>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap">
        <span class="material-symbols-rounded filled">format_paint</span>
      </div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.theme_color') }}</div>
        <div class="color-row" :style="colorMode !== 'default' ? { opacity: 0.4, pointerEvents: 'none' } : undefined">
          <button
            v-for="c in presetColors" :key="c.key"
            class="color-dot"
            :class="{ selected: activeColorKey === c.key }"
            :style="{ background: c.color }"
            @click="handleColorSwitch(c.key, $event)"
          >
            <span v-if="activeColorKey === c.key" class="material-symbols-rounded" style="font-size: 16px; color: white">check</span>
          </button>
        </div>
      </div>
    </div>

    <!-- 取色方式 -->
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">colorize</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.color_mode') }}</div>
        <div class="setting-desc">{{ colorModeOptions.find(o => o.value === colorMode)?.desc }}</div>
      </div>
      <div class="chip-row" role="radiogroup" :aria-label="t('settings.color_mode')">
        <button
          v-for="o in colorModeOptions"
          :key="o.value"
          class="m3-chip"
          :class="{ active: colorMode === o.value }"
          role="radio"
          :aria-checked="colorMode === o.value"
          @click="colorMode = o.value"
        >{{ o.label }}</button>
      </div>
    </div>

    <!-- 个性化 -->
    <div class="section-label clickable" @click="toggleSection('personal')">
      <span class="material-symbols-rounded" style="font-size: 18px">tune</span>
      <span>{{ t('settings.personalization') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('personal') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('personal')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">home</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.default_screen') }}</div>
          <div class="setting-desc">{{ defaultScreenOptions.find(o => o.value === defaultScreen)?.label }}</div>
        </div>
        <div class="chip-row">
          <button v-for="o in defaultScreenOptions" :key="o.value" class="m3-chip" :class="{ active: defaultScreen === o.value }" @click="defaultScreen = o.value as any">{{ o.label }}</button>
        </div>
      </div>
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tab_inactive</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.close_to_tray') }}</div>
          <div class="setting-desc">{{ t('settings.close_to_tray_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="closeToTray" /><span class="track"><span class="thumb"><span v-if="closeToTray" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">badge</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_badge') }}</div>
          <div class="setting-desc">{{ t('settings.cover_badge_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showCoverBadge" /><span class="track"><span class="thumb"><span v-if="showCoverBadge" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">title</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.np_title') }}</div>
          <div class="setting-desc">{{ t('settings.np_title_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showNowPlayingTitle" /><span class="track"><span class="thumb"><span v-if="showNowPlayingTitle" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card shortcut-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">keyboard</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('shortcuts.title') }}</div>
          <div class="setting-desc">{{ t('shortcuts.desc') }}</div>
          <ul class="shortcut-list">
            <li v-for="item in keyboardShortcuts" :key="item.id" class="shortcut-row">
              <span class="shortcut-label">{{ item.label }}</span>
              <span class="shortcut-keys">
                <kbd v-for="key in item.keys" :key="key">{{ key }}</kbd>
              </span>
            </li>
          </ul>
        </div>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">dock_to_bottom</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.np_toolbar') }}</div>
          <div class="setting-desc">{{ t('settings.np_toolbar_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showToolbarDock" /><span class="track"><span class="thumb"><span v-if="showToolbarDock" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">high_quality</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.quality_switch') }}</div>
          <div class="setting-desc">{{ t('settings.quality_switch_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showQualitySwitch" /><span class="track"><span class="thumb"><span v-if="showQualitySwitch" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div v-for="option in audioDisplayOptions" :key="option.key" class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">{{ option.icon }}</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.' + option.label) }}</div>
          <div class="setting-desc">{{ t('settings.' + option.label + '_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="settings[option.key]" /><span class="track"><span class="thumb"><span v-if="settings[option.key]" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">format_size</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.lyric_font_size') }}</div>
          <EditableRangeValue
            v-model="lyricFontScale"
            class="setting-desc"
            :min="LYRIC_FONT_SCALE_MIN"
            :max="LYRIC_FONT_SCALE_MAX"
            :step="LYRIC_FONT_SCALE_STEP"
            :display-value="`${Math.round(lyricFontScale * 100)}%`"
            :input-scale="100"
            input-suffix="%"
            :aria-label="t('settings.lyric_font_size')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="lyricFontScale" :min="LYRIC_FONT_SCALE_MIN" :max="LYRIC_FONT_SCALE_MAX" :step="LYRIC_FONT_SCALE_STEP" />
      </div>

      <!-- 封面样式 -->
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">album</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_style') }}</div>
        </div>
        <div class="chip-row">
          <button v-for="o in coverStyleOptions" :key="o.value" class="m3-chip" :class="{ active: coverStyle === o.value }" @click="coverStyle = o.value as any">{{ o.label }}</button>
        </div>
      </div>

      <!-- 自定义背景图 -->
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">wallpaper</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.background_image') }}</div>
          <div class="setting-desc">{{ backgroundImageUri ? backgroundImageUri.split(/[\\/]/).pop() : t('settings.background_image_desc') }}</div>
        </div>
        <div class="chip-row">
          <button class="m3-chip sm" @click="selectBackgroundImage">{{ t('settings.select_image') }}</button>
          <button v-if="backgroundImageUri" class="m3-chip sm" @click="clearBackgroundImage">{{ t('settings.clear_image') }}</button>
        </div>
      </div>

      <template v-if="backgroundImageUri">
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bg_blur') }}</div>
          <EditableRangeValue
            v-model="backgroundImageBlur"
            class="setting-desc"
            :min="0"
            :max="100"
            :step="5"
            :display-value="`${backgroundImageBlur}px`"
            input-suffix="px"
            :aria-label="t('settings.bg_blur')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="backgroundImageBlur" min="0" max="100" step="5" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bg_opacity') }}</div>
          <EditableRangeValue
            v-model="backgroundImageAlpha"
            class="setting-desc"
            :min="0"
            :max="1"
            :step="0.05"
            :inputScale="100"
            :display-value="`${(backgroundImageAlpha * 100).toFixed(0)}%`"
            input-suffix="%"
            :aria-label="t('settings.bg_opacity')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="backgroundImageAlpha" min="0" max="1" step="0.05" />
        </div>

        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.enhanced_advanced_blur') }}</div>
            <div class="setting-desc">{{ t('settings.enhanced_advanced_blur_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" v-model="enhancedAdvancedBlur" /><span class="track"><span class="thumb"><span v-if="enhancedAdvancedBlur" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
        </div>

        <div v-if="enhancedAdvancedBlur" class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.enhanced_advanced_blur_radius') }}</div>
            <EditableRangeValue
              v-model="enhancedAdvancedBlurRadius"
              class="setting-desc"
              :min="ENHANCED_BLUR_RADIUS_MIN"
              :max="ENHANCED_BLUR_RADIUS_MAX"
              :step="ENHANCED_BLUR_RADIUS_STEP"
              :display-value="`${enhancedAdvancedBlurRadius}px`"
              input-suffix="px"
              :aria-label="t('settings.enhanced_advanced_blur_radius')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="enhancedAdvancedBlurRadius"
            :min="ENHANCED_BLUR_RADIUS_MIN" :max="ENHANCED_BLUR_RADIUS_MAX" :step="ENHANCED_BLUR_RADIUS_STEP" />
        </div>
      </template>
    </div></Transition>
        </div>

    <!-- 播放 -->
        <div v-show="activeSettingsSection === 'playback'" class="settings-section-panel">
    <div class="setting-card setting-card--select">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">speaker</span></div>
      <div class="setting-info">
        <label class="setting-title" for="audio-output-device">{{ t('settings.audio_output') }}</label>
        <div class="setting-desc">{{ t('settings.audio_output_desc') }}</div>
      </div>
      <CustomSelect id="audio-output-device" class="settings-select settings-select--device"
        :model-value="audioOutputDevice" :options="audioOutputOptions" :label="t('settings.audio_output')"
        :disabled="audioOutputSwitching" @open="loadAudioOutputDevices" @update:model-value="changeAudioOutputDevice" />
    </div>
    <div class="section-label clickable" @click="toggleSection('playback')">
      <span class="material-symbols-rounded" style="font-size: 18px">play_circle</span>
      <span>{{ t('settings.playback') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('playback') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.normalize') }}</div>
        <div class="setting-desc">{{ t('settings.normalize_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="normalizeVolume" /><span class="track"><span class="thumb"><span v-if="normalizeVolume" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('playback')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">balance</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.volume_balance') }}</div>
          <EditableRangeValue
            v-model="volumeBalance"
            class="setting-desc"
            :min="-1"
            :max="1"
            :step="0.01"
            :input-scale="100"
            :display-value="volumeBalanceLabel"
            input-suffix="%"
            :aria-label="t('settings.volume_balance')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="volumeBalance" min="-1" max="1" step="0.05" :aria-label="t('settings.volume_balance')" :aria-valuetext="volumeBalanceLabel" />
      </div>
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">surround_sound</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.multichannel_drc') }}</div>
          <div class="setting-desc">{{ t('settings.multichannel_drc_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="multichannelDrc" /><span class="track"><span class="thumb"><span v-if="multichannelDrc" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">volume_up</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.fade_in') }}</div>
          <div class="setting-desc">{{ t('settings.fade_in_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="fadeIn" /><span class="track"><span class="thumb"><span v-if="fadeIn" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="fadeIn">
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.fade_in_duration') }}</div>
          <EditableRangeValue
            v-model="fadeInDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${fadeInDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.fade_in_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="fadeInDuration" min="0" max="3000" step="100" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.fade_out_duration') }}</div>
          <EditableRangeValue
            v-model="fadeOutDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${fadeOutDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.fade_out_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="fadeOutDuration" min="0" max="3000" step="100" />
        </div>
      </template>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync_alt</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.crossfade_next') }}</div>
          <div class="setting-desc">{{ t('settings.crossfade_next_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="crossfadeNext" /><span class="track"><span class="thumb"><span v-if="crossfadeNext" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="crossfadeNext">
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.crossfade_in_duration') }}</div>
          <EditableRangeValue
            v-model="crossfadeInDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${crossfadeInDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.crossfade_in_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="crossfadeInDuration" min="0" max="3000" step="100" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.crossfade_out_duration') }}</div>
          <EditableRangeValue
            v-model="crossfadeOutDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${crossfadeOutDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.crossfade_out_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="crossfadeOutDuration" min="0" max="3000" step="100" />
        </div>
      </template>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">history</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.keep_progress') }}</div>
          <div class="setting-desc">{{ t('settings.keep_progress_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="keepProgress" /><span class="track"><span class="thumb"><span v-if="keepProgress" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">bookmark</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.remember_long_form_progress') }}</div>
          <div class="setting-desc">{{ t('settings.remember_long_form_progress_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="rememberLongFormProgress" /><span class="track"><span class="thumb"><span v-if="rememberLongFormProgress" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">repeat</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.keep_mode') }}</div>
          <div class="setting-desc">{{ t('settings.keep_mode_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="keepPlaybackMode" /><span class="track"><span class="thumb"><span v-if="keepPlaybackMode" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>
    </div></Transition>
        </div>

    <!-- 网络 -->
        <div v-show="activeSettingsSection === 'network'" class="settings-section-panel">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">wifi</span>
      <span>{{ t('settings.network') }}</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">vpn_lock</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.bypass_proxy') }}</div>
        <div class="setting-desc">{{ t('settings.bypass_proxy_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" :checked="bypassProxy" @change="handleBypassProxyChange(($event.target as HTMLInputElement).checked)" />
        <span class="track"><span class="thumb"><span v-if="bypassProxy" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>
        </div>

    <!-- 一起听 -->
        <div v-show="activeSettingsSection === 'listen_together'" class="settings-section-panel">
    <div class="section-label clickable" @click="toggleSection('listen_together')">
      <span class="material-symbols-rounded" style="font-size: 18px">group</span>
      <span>{{ t('listen_together.title') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('listen_together') }">expand_more</span>
    </div>

    <!-- 摘要卡与展开编辑区互斥，整卡可点击展开 -->
    <div
      v-if="!isExpanded('listen_together')"
      class="setting-card"
      style="cursor: pointer"
      @click="toggleSection('listen_together')"
    >
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">dns</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('listen_together.server_url') }}</div>
        <div class="setting-desc" style="word-break: break-all">{{ ltServerUrl || 'https://neriplayer.hancat.work' }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; color: var(--md-on-surface-variant)">edit</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('listen_together')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">edit</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.server_url') }}</div>
        </div>
        <input
          type="text"
          class="lt-url-input"
          :value="ltServerUrl"
          @change="ltServerUrl = ($event.target as HTMLInputElement).value.trim() || 'https://neriplayer.hancat.work'"
        />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">badge</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.nickname') }}</div>
        </div>
        <input
          type="text"
          class="lt-url-input lt-input-left"
          style="width: 140px"
          :value="ltNickname"
          @change="handleLtNicknameChange"
          :maxlength="LT_NICKNAME_MAX_LENGTH"
          :placeholder="t('listen_together.nickname_placeholder')"
          :aria-label="t('listen_together.nickname')"
        />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tune</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.allow_member_control') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" :checked="ltAllowMemberControl" @change="lt.updateRoomSettings({ allowMemberControl: ($event.target as HTMLInputElement).checked })" />
          <span class="track"><span class="thumb"><span v-if="ltAllowMemberControl" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">pause_circle</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.auto_pause_on_change') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" :checked="ltAutoPauseOnMemberChange" @change="lt.updateRoomSettings({ autoPauseOnMemberChange: ($event.target as HTMLInputElement).checked })" />
          <span class="track"><span class="thumb"><span v-if="ltAutoPauseOnMemberChange" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">link</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.share_audio_links') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" :checked="ltShareAudioLinks" @change="lt.updateRoomSettings({ shareAudioLinks: ($event.target as HTMLInputElement).checked })" />
          <span class="track"><span class="thumb"><span v-if="ltShareAudioLinks" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">restart_alt</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.reset_identity') }}</div>
          <div class="setting-desc">{{ t('listen_together.reset_identity_desc') }}</div>
        </div>
        <button
          class="m3-chip sm danger"
          :disabled="lt.isInSession"
          :title="lt.isInSession ? t('listen_together.reset_identity_in_room') : undefined"
          @click="showResetLtIdentityConfirm = true"
        >{{ t('listen_together.reset_btn') }}</button>
      </div>
    </div></Transition>
        </div>

    <!-- 下载管理 -->
        <div v-show="activeSettingsSection === 'storage'" class="settings-section-panel">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">download</span>
      <span>{{ t('settings.download_manage') }}</span>
    </div>

    <template v-if="activeDownloadCount > 0">
      <div class="setting-card download-summary-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">downloading</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('download.active_tasks', { count: activeDownloadCount }) }}</div>
          <div class="setting-desc">{{ completedDownloadCount > 0 ? t('player.track_count', { count: completedDownloadCount }) : t('settings.no_active_downloads') }}</div>
        </div>
        <button class="m3-chip sm" style="color: var(--md-error); border-color: var(--md-error)" @click="cancelAllDownloads">{{ t('settings.cancel_all_downloads') }}</button>
      </div>
      <div
        v-for="task in activeDownloadTasks"
        :key="'settings-download-' + task.trackId"
        class="setting-card sub-card active-download-card"
      >
        <div class="setting-icon-wrap">
          <span class="material-symbols-rounded">{{ task.status === 'resolving' ? 'network_node' : task.status === 'error' ? 'error' : task.status === 'cancelled' ? 'cancel' : 'downloading' }}</span>
        </div>
        <div class="setting-info">
          <div class="setting-title">{{ task.title }}</div>
          <div class="setting-desc">{{ task.artist }} · {{ activeDownloadProgressText(task) }}</div>
          <div
            class="settings-download-progress"
            :class="{
              indeterminate: task.status === 'processing' || task.status === 'resolving' || (task.status === 'downloading' && !task.totalBytes),
              error: task.status === 'error',
              muted: task.status === 'queued' || task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'already_exists',
            }"
          >
            <div
              class="settings-download-progress-fill"
              :style="{ width: `${Math.max(4, task.status === 'cancelled' || task.status === 'already_exists' ? 100 : task.progress ?? 0)}%` }"
            />
          </div>
        </div>
        <button
          class="download-cancel-btn"
          :disabled="task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'error' || task.status === 'already_exists'"
          @click="cancelDownload(task.trackId)"
        >
          <span class="material-symbols-rounded">close</span>
        </button>
      </div>
    </template>
    <div v-else class="setting-card" style="cursor: pointer" @click="goToDownloads">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder_open</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.go_to_downloads') }}</div>
        <div class="setting-desc">{{ completedDownloadCount > 0 ? t('player.track_count', { count: completedDownloadCount }) : t('settings.no_active_downloads') }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
    </div>
        </div>

    <!-- 歌词 -->
        <div v-show="activeSettingsSection === 'lyrics'" class="settings-section-panel">
    <div class="section-label clickable" @click="toggleSection('lyrics')">
      <span class="material-symbols-rounded" style="font-size: 18px">lyrics</span>
      <span>{{ t('settings.lyrics') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('lyrics') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">translate</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.show_translation') }}</div>
        <div class="setting-desc">{{ t('settings.show_translation_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="showTranslation" /><span class="track"><span class="thumb"><span v-if="showTranslation" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">abc</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.show_romanization') }}</div>
        <div class="setting-desc">{{ t('settings.show_romanization_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="showRomanization" /><span class="track"><span class="thumb"><span v-if="showRomanization" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <!-- 来源偏好（对齐 Android SettingsLyricSourceSection） -->
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">timer</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.prefer_word_timed_lyrics') }}</div>
        <div class="setting-desc">{{ t('settings.prefer_word_timed_lyrics_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="preferWordTimedLyrics" /><span class="track"><span class="thumb"><span v-if="preferWordTimedLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <div class="setting-card setting-card--select">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">source</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.default_lyric_source') }}</div>
        <div class="setting-desc">{{ t('settings.default_lyric_source_desc') }}</div>
        <div class="setting-desc">{{ defaultLyricSourceDescription }}</div>
      </div>
      <CustomSelect class="settings-select" :model-value="defaultLyricSource" :options="defaultLyricSourceOptions"
        :label="t('settings.default_lyric_source')" @update:model-value="changeDefaultLyricSource" />
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">blur_on</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.lyric_blur') }}</div>
        <div class="setting-desc">{{ t('settings.lyric_blur_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="lyricBlur" /><span class="track"><span class="thumb"><span v-if="lyricBlur" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('lyrics')">
      <div v-if="lyricBlur" class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.blur_strength') }}</div>
          <EditableRangeValue
            v-model="lyricBlurAmount"
            class="setting-desc"
            :min="0"
            :max="8"
            :step="0.5"
            :display-value="`${lyricBlurAmount.toFixed(1)}px`"
            input-suffix="px"
            :aria-label="t('settings.blur_strength')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="lyricBlurAmount" min="0" max="8" step="0.5" />
      </div>

      <!-- 各歌词来源的默认偏移（对齐 Android）：显示的就是这类歌词实际生效的偏移 -->
      <div v-for="item in lyricOffsetSettings" :key="item.key" class="setting-card">
        <div class="setting-icon-wrap"><span class="platform-icon" :style="{ maskImage: `url(${item.iconSvg})` }"></span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t(item.label) }}</div>
          <EditableRangeValue
            :model-value="settings[item.key]"
            class="setting-desc"
            :min="-LYRIC_DEFAULT_OFFSET_RANGE_MS"
            :max="LYRIC_DEFAULT_OFFSET_RANGE_MS"
            :step="LYRIC_DEFAULT_OFFSET_STEP_MS"
            :display-value="formatLyricOffsetMs(settings[item.key])"
            input-suffix="ms"
            :aria-label="t(item.label)"
            @update:model-value="settings[item.key] = normalizeLyricDefaultOffset($event, item.defaultMs)"
          />
        </div>
        <button
          v-if="settings[item.key] !== item.defaultMs"
          class="m3-chip sm"
          :title="t('settings.lyric_offset_reset_to', { value: formatLyricOffsetMs(item.defaultMs) })"
          @click="settings[item.key] = item.defaultMs"
        >{{ t('settings.lyric_offset_reset') }}</button>
        <input
          type="range" class="m3-slider"
          :value="settings[item.key]"
          :min="-LYRIC_DEFAULT_OFFSET_RANGE_MS" :max="LYRIC_DEFAULT_OFFSET_RANGE_MS" :step="LYRIC_DEFAULT_OFFSET_STEP_MS"
          :aria-label="t(item.label)"
          @input="settings[item.key] = normalizeLyricDefaultOffset(Number(($event.target as HTMLInputElement).value), item.defaultMs)"
        />
      </div>
      <div v-if="lyricOffsetsChanged" class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-desc">{{ t('settings.lyric_offset_reset_all_desc') }}</div>
        </div>
        <button class="m3-chip sm" @click="resetLyricOffsets">{{ t('settings.lyric_offset_reset_all') }}</button>
      </div>
    </div></Transition>

    <!-- 桌面歌词 -->
    <div id="settings-desktop-lyrics" class="section-label clickable" @click="toggleSection('desktop_lyrics')">
      <span class="material-symbols-rounded" style="font-size: 18px">subtitles</span>
      <span>{{ t('desktop_lyrics.title') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('desktop_lyrics') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">desktop_windows</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.open') }}</div>
        <div class="setting-desc">{{ t('desktop_lyrics.open_desc') }}</div>
      </div>
      <button
        v-if="desktopLyricsOpen"
        class="m3-chip sm"
        :class="{ active: desktopLyricsStyle.locked }"
        @click="updateDesktopLyrics({ locked: !desktopLyricsStyle.locked })"
      >
        {{ t(desktopLyricsStyle.locked ? 'desktop_lyrics.unlock' : 'desktop_lyrics.lock') }}
      </button>
      <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsOpen" :disabled="desktopLyricsBusy" @change="toggleDesktopLyrics(($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"><span v-if="desktopLyricsOpen" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('desktop_lyrics')">
      <div class="setting-card dl-preview-card">
        <div class="dl-preview" :aria-label="t('desktop_lyrics.preview')">
          <DesktopLyricsStage
            :lines="desktopLyricsPreviewLines"
            :first-index="0"
            :clock="desktopLyricsPreviewClock"
            :style="desktopLyricsStyle"
            :accent="desktopLyricsAccent"
            :placeholder="t('desktop_lyrics.preview')"
          />
        </div>
      </div>

      <div class="setting-card dl-stack-card">
        <div class="setting-title">{{ t('desktop_lyrics.theme') }}</div>
        <div class="dl-swatches">
          <button
            v-for="theme in [...DESKTOP_LYRICS_THEMES.map(item => item.id), 'accent']"
            :key="theme"
            class="dl-swatch"
            :class="{ active: desktopLyricsStyle.theme === theme }"
            :title="t(`desktop_lyrics.theme_${theme}`)"
            @click="chooseDesktopLyricsTheme(theme)"
          >
            <span class="dl-swatch-dot" :style="{ background: themeSwatch(theme) }"></span>
            <span>{{ t(`desktop_lyrics.theme_${theme}`) }}</span>
          </button>
          <span v-if="desktopLyricsStyle.theme === 'custom'" class="m3-chip sm active">{{ t('desktop_lyrics.theme_custom') }}</span>
        </div>
      </div>

      <div class="setting-card dl-stack-card">
        <div class="setting-title">{{ t('desktop_lyrics.layout') }}</div>
        <div class="dl-chips">
          <button v-for="layout in DESKTOP_LYRICS_LAYOUTS" :key="layout" class="m3-chip sm" :class="{ active: desktopLyricsStyle.layout === layout }" @click="updateDesktopLyrics({ layout })">
            {{ t(`desktop_lyrics.layout_${layout}`) }}
          </button>
        </div>
        <div v-if="desktopLyricsStyle.layout === 'double'" class="setting-desc">{{ t('desktop_lyrics.layout_double_desc') }}</div>
      </div>

      <div class="setting-card dl-stack-card">
        <div class="setting-title">{{ t('desktop_lyrics.align') }}</div>
        <div class="dl-chips">
          <button v-for="align in DESKTOP_LYRICS_ALIGNS" :key="align" class="m3-chip sm" :class="{ active: desktopLyricsStyle.align === align }" @click="updateDesktopLyrics({ align })">
            {{ t(`desktop_lyrics.align_${align}`) }}
          </button>
        </div>
      </div>

      <div class="setting-card dl-stack-card">
        <div class="setting-title">{{ t('desktop_lyrics.karaoke') }}</div>
        <div class="dl-chips">
          <button v-for="mode in DESKTOP_LYRICS_KARAOKE" :key="mode" class="m3-chip sm" :class="{ active: desktopLyricsStyle.karaoke === mode }" @click="updateDesktopLyrics({ karaoke: mode })">
            {{ t(`desktop_lyrics.karaoke_${mode}`) }}
          </button>
        </div>
      </div>

      <div class="setting-card setting-card--select">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.secondary') }}</div></div>
        <CustomSelect class="settings-select" :model-value="desktopLyricsStyle.secondary" :options="desktopLyricsSecondaryOptions"
          :label="t('desktop_lyrics.secondary')" :disabled="desktopLyricsStyle.layout === 'double'"
          @update:model-value="updateDesktopLyrics({ secondary: $event as DesktopLyricsStyle['secondary'] })" />
      </div>

      <div class="setting-card setting-card--select">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.font') }}</div></div>
        <CustomSelect class="settings-select" :model-value="desktopLyricsFontChoice" :options="desktopLyricsFontOptions"
          :label="t('desktop_lyrics.font')" @update:model-value="chooseDesktopLyricsFont" />
      </div>
      <div v-if="desktopLyricsFontChoice === CUSTOM_FONT" class="setting-card sub-card">
        <input
          class="lt-url-input lt-input-left dl-font-input"
          :value="desktopLyricsStyle.fontFamily"
          :placeholder="t('desktop_lyrics.font_custom_placeholder')"
          maxlength="64"
          @change="updateDesktopLyrics({ fontFamily: ($event.target as HTMLInputElement).value })"
        />
      </div>

      <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.font_size') }}</div>
          <div class="setting-desc">{{ desktopLyricsStyle.fontSize }}px</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.fontSize" :min="DESKTOP_LYRICS_FONT_SIZE.min" :max="DESKTOP_LYRICS_FONT_SIZE.max" :step="DESKTOP_LYRICS_FONT_SIZE.step"
          :aria-label="t('desktop_lyrics.font_size')" @input="updateDesktopLyrics({ fontSize: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card setting-card--select sub-card">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.font_weight') }}</div></div>
        <CustomSelect class="settings-select" :model-value="String(desktopLyricsStyle.fontWeight)" :options="desktopLyricsWeightOptions"
          :label="t('desktop_lyrics.font_weight')" @update:model-value="updateDesktopLyrics({ fontWeight: Number($event) })" />
      </div>

      <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.letter_spacing') }}</div>
          <div class="setting-desc">{{ desktopLyricsStyle.letterSpacing }}px</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.letterSpacing" :min="DESKTOP_LYRICS_LETTER_SPACING.min" :max="DESKTOP_LYRICS_LETTER_SPACING.max" :step="DESKTOP_LYRICS_LETTER_SPACING.step"
          :aria-label="t('desktop_lyrics.letter_spacing')" @input="updateDesktopLyrics({ letterSpacing: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.secondary_size') }}</div>
          <div class="setting-desc">{{ Math.round(desktopLyricsStyle.secondaryScale * 100) }}%</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.secondaryScale" :min="DESKTOP_LYRICS_SECONDARY_SCALE.min" :max="DESKTOP_LYRICS_SECONDARY_SCALE.max" :step="DESKTOP_LYRICS_SECONDARY_SCALE.step"
          :aria-label="t('desktop_lyrics.secondary_size')" @input="updateDesktopLyrics({ secondaryScale: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card dl-stack-card">
        <div class="setting-desc">{{ t('desktop_lyrics.gradient_hint') }}</div>
        <div class="dl-color-rows">
          <label class="dl-color-row">
            <span>{{ t('desktop_lyrics.played') }}</span>
            <input type="color" :value="desktopLyricsStyle.playedColors[0]" @input="setDesktopLyricsGradient('playedColors', 0, ($event.target as HTMLInputElement).value)" />
            <input type="color" :value="desktopLyricsStyle.playedColors[1]" @input="setDesktopLyricsGradient('playedColors', 1, ($event.target as HTMLInputElement).value)" />
          </label>
          <label class="dl-color-row">
            <span>{{ t('desktop_lyrics.unplayed') }}</span>
            <input type="color" :value="desktopLyricsStyle.unplayedColors[0]" @input="setDesktopLyricsGradient('unplayedColors', 0, ($event.target as HTMLInputElement).value)" />
            <input type="color" :value="desktopLyricsStyle.unplayedColors[1]" @input="setDesktopLyricsGradient('unplayedColors', 1, ($event.target as HTMLInputElement).value)" />
          </label>
          <label class="dl-color-row">
            <span>{{ t('desktop_lyrics.secondary_color') }}</span>
            <input type="color" :value="desktopLyricsStyle.secondaryColor" @input="updateDesktopLyrics({ secondaryColor: ($event.target as HTMLInputElement).value, theme: 'custom' })" />
          </label>
        </div>
      </div>

      <div class="setting-card">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.stroke') }}</div></div>
        <input v-if="desktopLyricsStyle.strokeEnabled" type="color" class="dl-color-inline" :value="desktopLyricsStyle.strokeColor" :aria-label="t('desktop_lyrics.stroke')"
          @input="updateDesktopLyrics({ strokeColor: ($event.target as HTMLInputElement).value })" />
        <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.strokeEnabled" @change="updateDesktopLyrics({ strokeEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.strokeEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>
      <div v-if="desktopLyricsStyle.strokeEnabled" class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.stroke_width') }}</div>
          <div class="setting-desc">{{ desktopLyricsStyle.strokeWidth }}px</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.strokeWidth" :min="DESKTOP_LYRICS_STROKE_WIDTH.min" :max="DESKTOP_LYRICS_STROKE_WIDTH.max" :step="DESKTOP_LYRICS_STROKE_WIDTH.step"
          :aria-label="t('desktop_lyrics.stroke_width')" @input="updateDesktopLyrics({ strokeWidth: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.shadow') }}</div></div>
        <input v-if="desktopLyricsStyle.shadowEnabled" type="color" class="dl-color-inline" :value="desktopLyricsStyle.shadowColor" :aria-label="t('desktop_lyrics.shadow')"
          @input="updateDesktopLyrics({ shadowColor: ($event.target as HTMLInputElement).value })" />
        <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.shadowEnabled" @change="updateDesktopLyrics({ shadowEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.shadowEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>
      <div v-if="desktopLyricsStyle.shadowEnabled" class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.shadow_blur') }}</div>
          <div class="setting-desc">{{ desktopLyricsStyle.shadowBlur }}px</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.shadowBlur" :min="DESKTOP_LYRICS_SHADOW_BLUR.min" :max="DESKTOP_LYRICS_SHADOW_BLUR.max" :step="DESKTOP_LYRICS_SHADOW_BLUR.step"
          :aria-label="t('desktop_lyrics.shadow_blur')" @input="updateDesktopLyrics({ shadowBlur: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.glow') }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.glowEnabled" @change="updateDesktopLyrics({ glowEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.glowEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.opacity') }}</div>
          <div class="setting-desc">{{ Math.round(desktopLyricsStyle.opacity * 100) }}%</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.opacity" :min="DESKTOP_LYRICS_OPACITY.min" :max="DESKTOP_LYRICS_OPACITY.max" :step="DESKTOP_LYRICS_OPACITY.step"
          :aria-label="t('desktop_lyrics.opacity')" @input="updateDesktopLyrics({ opacity: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card dl-stack-card">
        <div class="setting-title">{{ t('desktop_lyrics.background') }}</div>
        <div class="dl-chips">
          <button v-for="mode in DESKTOP_LYRICS_BACKGROUNDS" :key="mode" class="m3-chip sm" :class="{ active: desktopLyricsStyle.background === mode }" @click="updateDesktopLyrics({ background: mode })">
            {{ t(`desktop_lyrics.background_${mode}`) }}
          </button>
        </div>
      </div>
      <div v-if="desktopLyricsStyle.background !== 'never'" class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.background_opacity') }}</div>
          <div class="setting-desc">{{ Math.round(desktopLyricsStyle.backgroundOpacity * 100) }}%</div>
        </div>
        <input type="range" class="m3-slider" :value="desktopLyricsStyle.backgroundOpacity" :min="DESKTOP_LYRICS_BACKGROUND_OPACITY.min" :max="DESKTOP_LYRICS_BACKGROUND_OPACITY.max" :step="DESKTOP_LYRICS_BACKGROUND_OPACITY.step"
          :aria-label="t('desktop_lyrics.background_opacity')" @input="updateDesktopLyrics({ backgroundOpacity: Number(($event.target as HTMLInputElement).value) })" />
      </div>

      <div class="setting-card">
        <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.show_track_info') }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.showTrackInfo" @change="updateDesktopLyrics({ showTrackInfo: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.showTrackInfo" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('desktop_lyrics.reset') }}</div>
          <div class="setting-desc">{{ t('desktop_lyrics.reset_desc') }}</div>
        </div>
        <button class="m3-chip sm" @click="resetDesktopLyrics">{{ t('desktop_lyrics.reset') }}</button>
      </div>
    </div></Transition>
        </div>

    <!-- 动效 & 视觉 -->
        <div v-show="activeSettingsSection === 'motion'" class="settings-section-panel">
    <div class="section-label clickable" @click="toggleSection('effects')">
      <span class="material-symbols-rounded" style="font-size: 18px">auto_awesome</span>
      <span>{{ t('settings.effects') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('effects') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">animation</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.advanced_lyrics') }}</div>
        <div class="setting-desc">{{ t('settings.advanced_lyrics_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="advancedLyrics" /><span class="track"><span class="thumb"><span v-if="advancedLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">wallpaper</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.dynamic_bg') }}</div>
        <div class="setting-desc">{{ t('settings.dynamic_bg_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="dynamicBackground" /><span class="track"><span class="thumb"><span v-if="dynamicBackground" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('effects')">
      <div v-if="dynamicBackground" class="setting-card sub-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.audio_reactive') }}</div>
          <div class="setting-desc">{{ t('settings.audio_reactive_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="audioReactive" /><span class="track"><span class="thumb"><span v-if="audioReactive" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">blur_circular</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_blur') }}</div>
          <div class="setting-desc">{{ t('settings.cover_blur_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="coverBlurBg" /><span class="track"><span class="thumb"><span v-if="coverBlurBg" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="coverBlurBg">
        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.blur_amount') }}</div>
            <EditableRangeValue
              v-model="coverBlurAmount"
              class="setting-desc"
              :min="0"
              :max="MAX_COVER_BLUR_AMOUNT"
              :step="0.1"
              :display-value="`${Math.round(coverBlurAmount * COVER_BLUR_PX_PER_UNIT)} px`"
              :input-scale="COVER_BLUR_PX_PER_UNIT"
              input-suffix="px"
              :aria-label="t('settings.blur_amount')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="coverBlurAmount" min="0" :max="MAX_COVER_BLUR_AMOUNT" step="0.1" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.bg_darken') }}</div>
            <EditableRangeValue
              v-model="coverBlurDarken"
              class="setting-desc"
              :min="0"
              :max="0.8"
              :step="0.05"
              :inputScale="100"
              :display-value="`${(coverBlurDarken * 100).toFixed(0)}%`"
              input-suffix="%"
              :aria-label="t('settings.bg_darken')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="coverBlurDarken" min="0" max="0.8" step="0.05" />
        </div>
      </template>
    </div></Transition>
        </div>

    <!-- 播放源 -->
        <div v-show="activeSettingsSection === 'playback_sources'" class="settings-section-panel">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">alt_route</span>
      <span>{{ t('settings.playback_sources') }}</span>
    </div>

    <div class="setting-card setting-card--select">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">smart_display</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.youtube_playback_source') }}</div>
        <div class="setting-desc">{{ t('settings.youtube_playback_source_desc') }}</div>
        <div class="setting-desc">{{ youtubePlaybackSourceDescription }}</div>
      </div>
      <CustomSelect class="settings-select" :model-value="youtubePlaybackSource" :options="youtubePlaybackSourceOptions"
        :label="t('settings.youtube_playback_source')" @update:model-value="changeYouTubePlaybackSource" />
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">library_music</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.netease_local_source_fallback') }}</div>
        <div class="setting-desc">{{ t('settings.netease_local_source_fallback_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" v-model="neteaseLocalSourceFallback" :aria-label="t('settings.netease_local_source_fallback')" />
        <span class="track"><span class="thumb"><span v-if="neteaseLocalSourceFallback" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync_alt</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.netease_auto_source_switch') }}</div>
        <div class="setting-desc">{{ t('settings.netease_auto_source_switch_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" v-model="neteaseAutoSourceSwitch" :aria-label="t('settings.netease_auto_source_switch')" />
        <span class="track"><span class="thumb"><span v-if="neteaseAutoSourceSwitch" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>
        </div>

    <!-- 音质 -->
        <div v-show="activeSettingsSection === 'quality'" class="settings-section-panel">
    <div class="section-label clickable" @click="toggleSection('quality')">
      <span class="material-symbols-rounded" style="font-size: 18px">headphones</span>
      <span>{{ t('settings.audio_quality') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('quality') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('quality')">
      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.netease_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in neteaseQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: neteaseQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('netease', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">library_music</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.qq_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in qqQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: qqMusicQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('qq', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">smart_display</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.youtube_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in youtubeQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: youtubeQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('youtube', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">play_circle</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bili_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in biliQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: biliQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('bilibili', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>
    </div></Transition>
        </div>

    <!-- 存储 & 缓存 -->
        <div v-show="activeSettingsSection === 'storage'" class="settings-section-panel">
    <div class="section-label clickable" @click="toggleSection('storage')">
      <span class="material-symbols-rounded" style="font-size: 18px">folder</span>
      <span>{{ t('settings.storage') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('storage') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('storage')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">sd_storage</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cache_limit') }}</div>
          <EditableRangeValue
            v-model="maxCacheSize"
            class="setting-desc"
            :min="MIN_MEDIA_CACHE_SIZE_MB"
            :max="MAX_MEDIA_CACHE_SIZE_MB"
            :step="256"
            :display-value="maxCacheSize >= 1024 ? `${(maxCacheSize / 1024).toFixed(1)} GB` : `${maxCacheSize} MB`"
            input-suffix="MB"
            :aria-label="t('settings.cache_limit')"
          />
        </div>
        <input
          v-model.number="maxCacheSize"
          type="range"
          class="m3-slider"
          :min="MIN_MEDIA_CACHE_SIZE_MB"
          :max="MAX_MEDIA_CACHE_SIZE_MB"
          step="256"
        />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">downloading</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_parallelism') }}</div>
          <div class="setting-desc">{{ t('settings.download_parallelism_desc') }}</div>
          <EditableRangeValue
            v-model="downloadParallelism"
            class="setting-desc"
            :min="MIN_DOWNLOAD_PARALLELISM"
            :max="MAX_DOWNLOAD_PARALLELISM"
            :step="1"
            :display-value="t('settings.download_parallelism_value', { count: downloadParallelism })"
            :aria-label="t('settings.download_parallelism')"
          />
        </div>
        <input v-model.number="downloadParallelism" type="range" class="m3-slider" :min="MIN_DOWNLOAD_PARALLELISM" :max="MAX_DOWNLOAD_PARALLELISM" step="1" :aria-label="t('settings.download_parallelism')" />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">audio_file</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_metadata') }}</div>
          <div class="setting-desc">{{ t('settings.download_metadata_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="downloadAutoFillMetadata" /><span class="track"><span class="thumb"><span v-if="downloadAutoFillMetadata" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card sub-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">lyrics</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_embed_lyrics') }}</div>
          <div class="setting-desc">{{ t('settings.download_embed_lyrics_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="downloadEmbedLyrics" :disabled="!downloadAutoFillMetadata" /><span class="track"><span class="thumb"><span v-if="downloadEmbedLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">high_quality</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_follow_playback_quality') }}</div>
          <div class="setting-desc">{{ t('settings.download_follow_playback_quality_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="downloadFollowPlaybackQuality" /><span class="track"><span class="thumb"><span v-if="downloadFollowPlaybackQuality" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="!downloadFollowPlaybackQuality">
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.netease_quality') }}</div></div>
          <CustomSelect v-model="downloadNeteaseQuality" :options="downloadNeteaseQualityOptions" :label="t('settings.netease_quality')" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.qq_quality') }}</div></div>
          <CustomSelect v-model="downloadQqMusicQuality" :options="qqQualityOptions" :label="t('settings.qq_quality')" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.youtube_quality') }}</div></div>
          <CustomSelect v-model="downloadYoutubeQuality" :options="youtubeQualityOptions" :label="t('settings.youtube_quality')" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.bili_quality') }}</div></div>
          <CustomSelect v-model="downloadBiliQuality" :options="biliQualityOptions" :label="t('settings.bili_quality')" />
        </div>
      </template>

      <div class="setting-card" style="cursor: pointer" @click="openDownloadTemplateDialog">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">text_fields</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_format') }}</div>
          <div class="setting-desc">{{ downloadNameTemplate }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">edit</span>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_dir') }}</div>
          <div class="setting-desc" style="word-break: break-all">{{ displayDownloadDir }}</div>
          <div class="setting-desc">{{ t('settings.download_dir_desc') }}</div>
        </div>
        <div class="chip-row">
          <button class="m3-chip sm" :disabled="activeDownloadCount > 0" @click="selectDownloadDir">{{ t('settings.download_dir_select') }}</button>
          <button v-if="downloadDir" class="m3-chip sm" :disabled="activeDownloadCount > 0" @click="resetDownloadDir">{{ t('settings.download_dir_reset') }}</button>
        </div>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="openStorageManagement">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">database</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.storage_usage_title') }}</div>
          <div class="setting-desc">{{ t('settings.storage_usage_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="section-label" style="margin-top: 8px">
        <span class="material-symbols-rounded" style="font-size: 18px">description</span>
        <span>{{ t('settings.logs_title') }}</span>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">edit_note</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.log_to_file') }}</div>
          <div class="setting-desc">{{ t('settings.log_to_file_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="logToFile" @change="onLogToFileChange" /><span class="track"><span class="thumb"><span v-if="logToFile" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tune</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.log_level') }}</div>
          <div class="setting-desc">{{ t('settings.log_level_desc') }}</div>
          <div class="chip-row" style="margin-top: 8px">
            <button v-for="opt in logLevelOptions" :key="opt.value" class="m3-chip sm" :class="{ active: logLevel === opt.value }" @click="logLevel = opt.value">{{ opt.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="openLogDir">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder_open</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.open_log_dir') }}</div>
          <div class="setting-desc">{{ t('settings.open_log_dir_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>
    </div></Transition>
        </div>

    <!-- 备份 & 恢复 -->
        <div v-show="activeSettingsSection === 'backup'" class="settings-section-panel">
    <div class="section-label clickable" @click="toggleSection('backup')">
      <span class="material-symbols-rounded" style="font-size: 18px">cloud_sync</span>
      <span>{{ t('settings.backup') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('backup') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('backup')">

      <!-- 播放历史同步频率：与 Android 一样作为全局偏好 -->
      <div class="setting-card sub-card sync-frequency-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">timer</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.sync_frequency') }}</div>
          <div class="setting-desc">{{ t('settings.sync_frequency_desc') }}</div>
          <div class="chip-wrap">
            <button
              v-for="option in syncFrequencyOptions"
              :key="option.value"
              class="m3-chip sm"
              :class="{ active: syncStore.syncFrequency === option.value }"
              type="button"
              @click="syncStore.syncFrequency = option.value"
            >{{ option.label }}</button>
          </div>
        </div>
      </div>

      <!-- GitHub 同步 -->
      <template v-if="!syncStore.github.configured">
        <!-- 未配置：单行入口 -->
        <div class="setting-card" style="cursor: pointer" @click="openGitHubSetup">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud_sync</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.github_sync') }}</div>
            <div class="setting-desc">{{ t('settings.github_sync_desc') }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
        </div>
      </template>
      <template v-else>
        <!-- 已配置：完整管理面板 -->
        <div class="setting-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud_sync</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.github_sync') }}</div>
            <div class="setting-desc">{{ syncStore.github.owner }}/{{ syncStore.github.repo }}</div>
          </div>
          <span class="sync-status-pill configured">{{ t('settings.sync_configured') }}</span>
        </div>

        <!-- 自动同步开关 -->
        <div class="setting-card sub-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.auto_sync') }}</div>
            <div class="setting-desc">{{ t('settings.auto_sync_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" v-model="syncStore.github.autoSync" /><span class="track"><span class="thumb"><span v-if="syncStore.github.autoSync" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
        </div>

        <!-- 立即同步 -->
        <div class="setting-card sub-card" style="cursor: pointer" @click="syncStore.syncGitHub()">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud_upload</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.sync_now') }}</div>
            <div class="setting-desc">
              <template v-if="syncStore.github.lastSyncTime">{{ t('settings.last_sync', { time: formatSyncTime(syncStore.github.lastSyncTime) }) }}</template>
              <template v-else>{{ t('settings.not_synced') }}</template>
            </div>
          </div>
          <span v-if="syncStore.isSyncing" class="material-symbols-rounded spinning" style="font-size: 20px">progress_activity</span>
          <span v-else class="sync-action-label">{{ t('settings.sync_action') }}</span>
        </div>

        <!-- 静默同步失败 -->
        <div class="setting-card sub-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">error</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.silent_failures') }}</div>
            <div class="setting-desc">{{ t('settings.silent_failures_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" v-model="syncStore.github.silentFailures" /><span class="track"><span class="thumb"><span v-if="syncStore.github.silentFailures" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
        </div>

        <!-- 清除配置 -->
        <div class="setting-card sub-card" style="justify-content: center;">
          <button class="clear-config-btn" @click="showClearGitHubConfirm = true">
            <span class="material-symbols-rounded" style="font-size: 16px">delete_outline</span>
            {{ t('settings.clear_config') }}
          </button>
        </div>
      </template>

      <!-- WebDAV 同步 -->
      <div class="setting-card" style="cursor: pointer" @click="syncStore.webdav.configured ? syncStore.syncWebDav() : (showWebDavDialog = true)">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">dns</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.webdav_sync') }}</div>
          <div class="setting-desc">
            <template v-if="syncStore.webdav.configured">
              {{ syncStore.webdav.serverUrl }}
              <span v-if="syncStore.webdav.lastSyncTime"> · {{ formatSyncTime(syncStore.webdav.lastSyncTime) }}</span>
            </template>
            <template v-else>{{ t('settings.webdav_sync_desc') }}</template>
          </div>
        </div>
        <span v-if="syncStore.isSyncing" class="material-symbols-rounded spinning" style="font-size: 20px">progress_activity</span>
        <button v-else-if="syncStore.webdav.configured" class="sync-disconnect-btn" @click.stop="syncStore.disconnectWebDav()">
          <span class="material-symbols-rounded" style="font-size: 18px">link_off</span>
        </button>
        <span v-else class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div v-if="syncStore.webdav.configured" class="setting-card sub-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.auto_sync') }}</div>
          <div class="setting-desc">{{ t('settings.auto_sync_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="syncStore.webdav.autoSync" /><span class="track"><span class="thumb"><span v-if="syncStore.webdav.autoSync" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="syncStore.exportPlaylists()">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">upload_file</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.export_playlist') }}</div>
          <div class="setting-desc">{{ t('settings.export_playlist_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="syncStore.importPlaylists()">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">download</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.import_playlist') }}</div>
          <div class="setting-desc">{{ t('settings.import_playlist_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="showConfigExportWarning = true">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">settings_backup_restore</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.export_config') }}</div>
          <div class="setting-desc">{{ t('settings.export_config_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="importConfig">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">restore</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.import_config') }}</div>
          <div class="setting-desc">{{ t('settings.import_config_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>
    </div></Transition>
        </div>

    <!-- 语言 -->
        <div v-show="activeSettingsSection === 'language'" class="settings-section-panel">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">language</span>
      <span>{{ t('settings.language') }}</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">translate</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.language') }}</div>
        <div class="setting-desc">{{ t('settings.language_desc') }}</div>
      </div>
      <div class="chip-row">
        <button v-for="loc in SUPPORTED_LOCALES" :key="loc.code" class="m3-chip" :class="{ active: locale === loc.code }" @click="handleLocaleSwitch(loc.code, $event)">{{ loc.label }}</button>
      </div>
    </div>
        </div>

    <!-- 关于 -->
        <div v-show="activeSettingsSection === 'about'" class="settings-section-panel">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">info</span>
      <span>{{ t('settings.about') }}</span>
    </div>

    <div class="setting-card about-card" @click="handleVersionTap" style="cursor: pointer">
      <div class="setting-icon-wrap accent">
        <img src="/app-icon.png" alt="NeriPlayer" style="width: 24px; height: 24px; border-radius: 4px;" />
      </div>
      <div class="setting-info">
        <div class="setting-title">NeriPlayer Desktop{{ devModeEnabled ? ' (dev)' : '' }}</div>
        <div class="setting-desc mono-build-value">
          {{ t('settings.version_info', {
            version: buildInfo?.version || 'dev',
          }) }}
        </div>
      </div>
      <button
        type="button"
        class="about-copy-btn"
        :title="t('settings.copy_build_version')"
        :aria-label="t('settings.copy_build_version')"
        @click="copyBuildValue(buildInfo?.version, $event)"
      >
        <span class="material-symbols-rounded">content_copy</span>
      </button>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('about') }" style="font-size: 20px; opacity: 0.3">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('about') && buildInfo">
      <div
        class="setting-card sub-card copyable-setting-card"
        :title="t('settings.copy_build_uuid')"
        @click="copyBuildValue(buildInfo.build_uuid, $event)"
      >
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">fingerprint</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.build_uuid') }}</div>
          <div class="setting-desc mono-build-value">{{ buildInfo.build_uuid }}</div>
        </div>
        <span class="material-symbols-rounded copy-hint-icon">content_copy</span>
      </div>
      <div
        class="setting-card sub-card copyable-setting-card"
        :title="t('settings.copy_build_time')"
        @click="copyBuildValue(buildInfo.build_timestamp, $event)"
      >
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">schedule</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.build_time') }}</div>
          <div class="setting-desc">{{ buildInfo.build_timestamp }}</div>
        </div>
        <span class="material-symbols-rounded copy-hint-icon">content_copy</span>
      </div>
    </div></Transition>

    <div class="setting-card" style="cursor: pointer" @click="openExternalUrl(PROJECT_REPOSITORY_URL)">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">code</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.github') }}</div>
        <div class="setting-desc">{{ t('settings.github_desc') }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">open_in_new</span>
    </div>
    <div
      class="setting-card"
      style="cursor: pointer"
      :title="player.decoderCapabilities?.ffmpegError ?? player.decoderCapabilities?.ffmpeg?.directory ?? ''"
      @click="openExternalUrl(FFMPEG_LEGAL_URL)"
    >
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.ffmpeg_component') }}</div>
        <div class="setting-desc">{{ ffmpegComponentText }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">open_in_new</span>
    </div>
        </div>

        </div>
        </Transition>
        </div>
      </main>
    </div>

    <!-- GitHub 两阶段配置对话框 -->
    <Teleport to="body">
      <div v-if="showGitHubDialog" class="dialog-overlay" @click.self="showGitHubDialog = false">
        <div class="dialog-card" style="width: 420px">
          <h3 class="dialog-title">{{ t('settings.github_sync_config') }}</h3>

          <!-- Token 验证 -->
          <div class="phase-section">
            <div class="phase-header">
              <span class="phase-number" :class="{ done: githubPhase === 2 }">{{ githubPhase === 2 ? '✓' : '1' }}</span>
              <span class="phase-label">{{ t('settings.github_step1') }}</span>
            </div>

            <div v-if="githubPhase === 1" class="phase-body">
              <div class="dialog-field">
                <label>GitHub Personal Access Token</label>
                <input v-model="githubToken" type="password" placeholder="ghp_xxxxxxxxxxxx" @keyup.enter="githubValidateToken" />
              </div>
              <p class="field-hint">{{ t('settings.github_token_hint') }}</p>
              <button type="button" class="text-link-btn" @click="openGitHubTokenPage">
                <span class="material-symbols-rounded" style="font-size: 16px">open_in_new</span>
                {{ t('settings.github_create_token') }}
              </button>
            </div>
            <div v-else class="phase-done-info">
              <span class="material-symbols-rounded" style="font-size: 16px; color: var(--md-primary)">check_circle</span>
              <span>{{ githubUsername }}</span>
            </div>
          </div>

          <!-- 仓库选择 -->
          <div v-if="githubPhase === 2" class="phase-section">
            <div class="phase-header">
              <span class="phase-number">2</span>
              <span class="phase-label">{{ t('settings.github_step2') }}</span>
            </div>
            <div class="phase-body">
              <div class="radio-group">
                <label class="radio-option" :class="{ active: githubRepoMode === 'create' }" @click="githubRepoMode = 'create'">
                  <span class="radio-dot" :class="{ checked: githubRepoMode === 'create' }"></span>
                  {{ t('settings.github_create_repo') }}
                </label>
                <div v-if="githubRepoMode === 'create'" class="dialog-field" style="margin-left: 28px; margin-top: 8px">
                  <input v-model="githubNewRepoName" type="text" placeholder="neriplayer-backup" />
                </div>

                <label class="radio-option" :class="{ active: githubRepoMode === 'existing' }" @click="githubRepoMode = 'existing'">
                  <span class="radio-dot" :class="{ checked: githubRepoMode === 'existing' }"></span>
                  {{ t('settings.github_use_existing') }}
                </label>
                <div v-if="githubRepoMode === 'existing'" class="dialog-field" style="margin-left: 28px; margin-top: 8px">
                  <input v-model="githubExistingRepo" type="text" :placeholder="t('settings.github_repo_format_hint')" />
                </div>
              </div>
            </div>
          </div>

          <p v-if="syncStore.dialogError" class="dialog-error">{{ syncStore.dialogError }}</p>

          <div class="dialog-actions">
            <button class="dialog-btn" @click="showGitHubDialog = false">{{ t('settings.cancel') }}</button>
            <button v-if="githubPhase === 1" class="dialog-btn primary" :disabled="githubIsValidating || !githubToken.trim()" @click="githubValidateToken">
              <span v-if="githubIsValidating" class="material-symbols-rounded spinning" style="font-size: 16px">progress_activity</span>
              <span v-else>{{ t('settings.github_verify_token') }}</span>
            </button>
            <button v-else class="dialog-btn primary" :disabled="githubIsSettingRepo" @click="githubFinishSetup">
              <span v-if="githubIsSettingRepo" class="material-symbols-rounded spinning" style="font-size: 16px">progress_activity</span>
              <span v-else>{{ t('settings.github_done') }}</span>
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- WebDAV 配置对话框 -->
    <Teleport to="body">
      <div v-if="showWebDavDialog" class="dialog-overlay" @click.self="showWebDavDialog = false">
        <div class="dialog-card">
          <h3 class="dialog-title">{{ t('settings.webdav_sync') }}</h3>
          <div class="dialog-field">
            <label>{{ t('settings.webdav_server') }}</label>
            <input v-model="webdavUrl" type="url" placeholder="https://dav.example.com" />
          </div>
          <div class="dialog-field">
            <label>{{ t('settings.webdav_username') }}</label>
            <input v-model="webdavUsername" type="text" />
          </div>
          <div class="dialog-field">
            <label>{{ t('settings.webdav_password') }}</label>
            <input v-model="webdavPassword" type="password" />
          </div>
          <div class="dialog-field">
            <label>{{ t('settings.webdav_path') }}</label>
            <input v-model="webdavBasePath" type="text" placeholder="/neriplayer" />
          </div>
          <p v-if="syncStore.dialogError" class="dialog-error">{{ syncStore.dialogError }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="showWebDavDialog = false">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn primary" :disabled="webdavConfiguring || !webdavUrl.trim() || !webdavUsername.trim() || !webdavPassword" @click="configureWebDav">
              <span v-if="webdavConfiguring" class="material-symbols-rounded spinning" style="font-size: 16px">progress_activity</span>
              <span v-else>{{ t('settings.connect') }}</span>
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 退出登录确认对话框 -->
    <Teleport to="body">
      <div v-if="showLogoutConfirm" class="dialog-overlay" @click.self="showLogoutConfirm = false">
        <div class="dialog-card" style="width: 340px">
          <h3 class="dialog-title">{{ t('settings.logout_confirm_title') }}</h3>
          <p class="dialog-desc">{{ t('settings.logout_confirm_msg', { platform: logoutTargetLabel }) }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="showLogoutConfirm = false">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn danger" @click="confirmLogout">{{ t('settings.sign_out') }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 清除 GitHub 配置确认 -->
    <Teleport to="body">
      <div v-if="showClearGitHubConfirm" class="dialog-overlay" @click.self="showClearGitHubConfirm = false">
        <div class="dialog-card" style="width: 340px">
          <h3 class="dialog-title">{{ t('settings.clear_config_title') }}</h3>
          <p class="dialog-desc">{{ t('settings.clear_config_msg') }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="showClearGitHubConfirm = false">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn danger" @click="confirmClearGitHub">{{ t('settings.clear_config_confirm') }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 旧客户端无法读取升级后的归档，写入前需明确确认 -->
    <Teleport to="body">
      <div v-if="syncStore.pendingProtocolUpgrade && !hideProtocolUpgrade" class="dialog-overlay" @click.self="hideProtocolUpgrade = true">
        <div class="dialog-card" style="width: 380px">
          <div class="dialog-icon warning">
            <span class="material-symbols-rounded">warning</span>
          </div>
          <h3 class="dialog-title">{{ t('settings.sync_upgrade_title') }}</h3>
          <p class="dialog-desc">{{ t('settings.sync_upgrade_message', { provider: syncStore.pendingProtocolUpgrade.backend === 'github' ? 'GitHub' : 'WebDAV' }) }}</p>
          <label class="dialog-check">
            <input v-model="upgradeDevicesConfirmed" type="checkbox" :disabled="syncStore.isSyncing" />
            <span>{{ t('settings.sync_upgrade_all_devices') }}</span>
          </label>
          <p v-if="syncStore.upgradeError" class="dialog-error">{{ syncStore.upgradeError }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" :disabled="syncStore.isSyncing" @click="hideProtocolUpgrade = true">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn primary" :disabled="syncStore.isSyncing || !upgradeDevicesConfirmed" @click="syncStore.approveProtocolUpgrade(upgradeDevicesConfirmed)">{{ t('settings.sync_upgrade_confirm') }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 一起听身份重置确认 -->
    <Teleport to="body">
      <div v-if="showResetLtIdentityConfirm" class="dialog-overlay" @click.self="showResetLtIdentityConfirm = false">
        <div class="dialog-card" style="width: 380px">
          <div class="dialog-icon warning">
            <span class="material-symbols-rounded">restart_alt</span>
          </div>
          <h3 class="dialog-title">{{ t('listen_together.reset_identity_confirm_title') }}</h3>
          <p class="dialog-desc">{{ t('listen_together.reset_identity_confirm_desc') }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="showResetLtIdentityConfirm = false">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn danger" @click="confirmResetLtIdentity">{{ t('listen_together.reset_btn') }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 配置导出敏感信息确认 -->
    <Teleport to="body">
      <div v-if="showConfigExportWarning" class="dialog-overlay" @click.self="showConfigExportWarning = false">
        <div class="dialog-card config-warning-dialog">
          <div class="dialog-icon danger">
            <span class="material-symbols-rounded">warning</span>
          </div>
          <h3 class="dialog-title">{{ t('settings.export_config_warning_title') }}</h3>
          <p class="dialog-desc">{{ t('settings.export_config_warning_desc') }}</p>
          <div class="warning-list">
            <div><span class="material-symbols-rounded">key</span>{{ t('settings.export_config_warning_credentials') }}</div>
            <div><span class="material-symbols-rounded">cloud_sync</span>{{ t('settings.export_config_warning_sync') }}</div>
            <div><span class="material-symbols-rounded">lock</span>{{ t('settings.export_config_warning_private') }}</div>
          </div>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="showConfigExportWarning = false">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn danger" @click="confirmConfigExport">{{ t('settings.export_config_warning_confirm') }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 下载文件名格式编辑对话框 -->
    <Teleport to="body">
      <div v-if="showDownloadTemplateDialog" class="dialog-overlay" @click.self="showDownloadTemplateDialog = false">
        <div class="dialog-card" style="width: 420px">
          <h3 class="dialog-title">{{ t('settings.download_format') }}</h3>
          <p class="dialog-desc">{{ t('settings.download_format_desc') }}</p>
          <div class="dialog-field">
            <label>{{ t('settings.download_format_template') }}</label>
          <input v-model="pendingTemplate" type="text" :placeholder="DEFAULT_DOWNLOAD_NAME_TEMPLATE" />
          </div>
          <p class="field-hint">{{ t('settings.download_format_supported') }}</p>
          <div class="template-preview">
            <span class="preview-label">{{ t('settings.download_format_preview') }}</span>
            <span class="preview-value">{{ templatePreview }}</span>
          </div>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="resetDownloadTemplate">{{ t('settings.download_format_reset') }}</button>
            <button class="dialog-btn" @click="showDownloadTemplateDialog = false">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn primary" @click="applyDownloadTemplate">{{ t('settings.download_format_apply') }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <StorageManagementDialog
      v-model:open="showStorageManagement"
      :loading="storageLoading"
      :clearing="storageClearing"
      :active-download-count="activeDownloadCount"
      :summary="storageSummary"
      @clear="clearStorageCache"
    />
  </div>
</template>

<style scoped lang="scss">
.setting-card--select {
  flex-wrap: wrap;

  .setting-info { flex-basis: 240px; }
}

.settings-select {
  flex: 0 1 200px;
  margin-left: auto;

  &--device { flex-basis: 300px; }

  :deep(.custom-select-trigger) {
    min-height: 44px;
    padding: 10px 14px;
    font-size: 14px;
  }
}

.settings-view {
  width: 100%;
  height: 100%;
  min-height: 0;
  box-sizing: border-box;
  overflow: hidden;
  /* 顶部总留白（标题栏 + 本内边距）恒定 64px：
     原 28px 是按 36px 标题栏调的，macOS 标题栏加高到 52px 后
     继续写死会叠加出双重留白，这里随 --titlebar-height 自动收窄 */
  padding: max(8px, calc(64px - var(--titlebar-height, 36px))) clamp(24px, 4vw, 64px) 0;
}

.settings-layout {
  display: grid;
  grid-template-columns: minmax(230px, 280px) minmax(0, 760px);
  justify-content: center;
  align-items: stretch;
  gap: clamp(28px, 5vw, 72px);
  height: 100%;
  min-height: 0;
  max-width: 1160px;
  margin: 0 auto;
}

/* 左列标题固定不滚动，仅分组列表滚动，与右列头部固定的结构对称，
   两列顶部基线不再随各自滚动位置错位 */
.settings-sidebar {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding-top: 18px;
}

.settings-nav {
  display: flex;
  flex-direction: column;
  gap: 10px;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding-right: 4px;

  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
}

.settings-nav-group {
  padding: 4px;
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--md-surface-container) 78%, transparent);
}

.settings-nav-item {
  width: 100%;
  min-height: 60px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--md-on-surface);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard),
              color var(--duration-short) var(--ease-standard);

  &:hover:not(.active) { background: var(--md-surface-container-high); }

  &.active {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }
}

.settings-nav-icon {
  flex-shrink: 0;
  font-size: 21px;
  color: var(--md-primary);
}

.settings-nav-copy {
  min-width: 0;
  flex: 1;

  strong,
  small {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  strong {
    font-size: 13px;
    font-weight: 600;
    line-height: 1.25;
  }

  small {
    margin-top: 3px;
    color: var(--md-on-surface-variant);
    font-size: 11px;
    line-height: 1.35;
    white-space: nowrap;
  }
}

.settings-nav-item.active .settings-nav-icon,
.settings-nav-item.active .settings-nav-copy small {
  color: var(--md-on-primary-container);
}

.settings-nav-arrow {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--md-on-surface-variant);
  font-size: 18px;
  opacity: 0.7;
}

.settings-nav-item.active .settings-nav-arrow {
  color: var(--md-on-primary-container);
}

/* 右列头部固定，滚动职责移交给 .settings-content-scroll */
.settings-content {
  width: 100%;
  min-width: 0;
  min-height: 0;
  max-width: 760px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding-top: 18px;
}

.settings-content-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding-right: 4px;

  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
}

.settings-panels {
  min-width: 0;
}

:global(.content:has(.settings-view)) {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  box-sizing: border-box;
  overflow: hidden;
  scrollbar-width: none;
}

:global(.content:has(.settings-view)::-webkit-scrollbar) {
  display: none;
}

.settings-content-header {
  margin: 0 4px 24px;

  h2 {
    margin: 0;
    font-size: 28px;
    font-weight: 650;
    line-height: 1.2;
  }

  p {
    margin: 8px 0 0;
    color: var(--md-on-surface-variant);
    font-size: 13px;
    line-height: 1.5;
  }
}

.settings-section-panel {
  min-width: 0;
}

.settings-section-intro {
  margin: -2px 4px 14px;
  color: var(--md-on-surface-variant);
  font-size: 12px;
  line-height: 1.5;
}

/* 每个分组面板的首个标签统一顶到面板顶部，
   任意分组激活时内容顶部基线一致，不再出现有的分组多 24px 空白 */
.settings-section-panel > .section-label:first-child {
  margin-top: 0;
}

/* 桌面歌词设置 */
.dl-preview {
  width: 100%;
  height: 156px;
  padding: 12px 20px;
  box-sizing: border-box;
  border-radius: inherit;
  /* 模拟桌面：歌词实际叠在各种壁纸上，深浅交错更容易看出描边和阴影 */
  background:
    radial-gradient(circle at 20% 30%, rgb(122 160 255 / 55%), transparent 55%),
    radial-gradient(circle at 80% 70%, rgb(255 180 120 / 45%), transparent 55%),
    linear-gradient(135deg, #2a3350, #4b3a4f 60%, #a7b4c8);
  color: #fff;
}

/* 比 .setting-card 多一个类：基础规则写在后面，同等优先级会把这里的对齐盖掉 */
.setting-card.dl-stack-card {
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
  text-align: left;
}

.setting-card.dl-preview-card {
  padding: 0;
  overflow: hidden;
}

.dl-chips,
.dl-swatches {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-start;
  gap: 6px;
}

.dl-swatch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px 3px 3px;
  border-radius: var(--radius-full);
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition: border-color var(--duration-short), color var(--duration-short);

  &:hover { color: var(--md-on-surface); }
  &.active {
    border-color: var(--md-primary);
    color: var(--md-on-surface);
    box-shadow: inset 0 0 0 1px var(--md-primary);
  }
}

.dl-swatch-dot {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px rgb(0 0 0 / 14%);
}

.dl-color-rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dl-color-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--md-on-surface);

  span { flex: 1; }
}

.dl-color-row input[type='color'],
.dl-color-inline {
  width: 38px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--md-outline-variant);
  border-radius: 8px;
  background: none;
  cursor: pointer;
  flex-shrink: 0;
}

.dl-font-input {
  width: 100%;
}

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
  /* 与右侧详情头部等高（h2 28px×1.2 + 8px 间距 + 描述 13px×1.5 ≈ 61px），
     保证左右两列列表顶部基线对齐 */
  min-height: 61px;
  margin: 0 0 24px;
}

.section-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.8px;
  color: var(--md-primary);
  margin: 24px 0 10px;
  padding: 0 4px;

  &:first-of-type { margin-top: 0; }
}

.setting-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
  margin-bottom: 8px;
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
}

.about-card { cursor: pointer; }

.about-copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  margin-left: auto;
  border: none;
  border-radius: 999px;
  background: transparent;
  color: var(--md-on-surface-variant);
  opacity: 0.45;
  cursor: pointer;
  flex-shrink: 0;
  transition: opacity var(--duration-short), background var(--duration-short), color var(--duration-short);

  .material-symbols-rounded {
    font-size: 18px;
  }

  &:hover {
    opacity: 0.9;
    color: var(--md-primary);
    background: color-mix(in srgb, var(--md-primary) 10%, transparent);
  }
}

.copyable-setting-card {
  cursor: pointer;
  user-select: none;

  &:hover .copy-hint-icon {
    opacity: 0.7;
  }
}

.mono-build-value {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 11px;
  word-break: break-all;
}

.copy-hint-icon {
  font-size: 14px;
  opacity: 0.35;
  flex-shrink: 0;
  transition: opacity var(--duration-short);
}

/* 账号卡片 */
.platform-icon {
  display: block;
  width: 24px;
  height: 24px;
  background: var(--md-on-surface-variant);
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
  flex-shrink: 0;
}

.account-avatar {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  object-fit: cover;
  flex-shrink: 0;
}

.account-avatar-fallback {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  font-size: 20px;
}

.account-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-top: 7px;
  color: var(--md-on-surface-variant);
  font-size: 11px;
}

.account-status.connected { color: var(--md-primary); }

.account-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}

.account-login-btn {
  padding: 6px 16px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  font-family: inherit;
  background: var(--md-primary);
  color: var(--md-on-primary);
  cursor: pointer;
  transition: opacity var(--duration-short);
  white-space: nowrap;
  flex-shrink: 0;

  &:hover { opacity: 0.85; }
  &:disabled { opacity: 0.5; cursor: not-allowed; }
}

.account-logout-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 14px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  font-family: inherit;
  color: var(--md-error, #FFB4AB);
  flex-shrink: 0;
  transition: background var(--duration-short);
  white-space: nowrap;

  &:hover { background: color-mix(in srgb, var(--md-error, #FFB4AB) 12%, transparent); }
}

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 1s linear infinite; }

.setting-icon-wrap {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--md-on-surface-variant);

  &.accent {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }
}

.setting-info { flex: 1; min-width: 0; }
.setting-title { font-size: 14px; font-weight: 500; }
.setting-desc { font-size: 12px; color: var(--md-on-surface-variant); margin-top: 2px; }

.download-summary-card {
  border-color: color-mix(in srgb, var(--md-primary) 24%, var(--md-outline-variant));
}

.active-download-card {
  align-items: flex-start;

  .setting-icon-wrap {
    width: 36px;
    height: 36px;
  }
}

.settings-download-progress {
  position: relative;
  height: 6px;
  margin-top: 8px;
  border-radius: 999px;
  overflow: hidden;
  background: var(--md-surface-container-highest);
}

.settings-download-progress-fill {
  height: 100%;
  min-width: 4px;
  border-radius: inherit;
  background: linear-gradient(90deg, var(--md-primary), color-mix(in srgb, var(--md-primary) 70%, white));
  transition: width 180ms ease;
}

.settings-download-progress.indeterminate .settings-download-progress-fill {
  width: 36% !important;
  animation: settings-download-indeterminate 1.2s ease-in-out infinite;
}

.settings-download-progress.error .settings-download-progress-fill {
  background: var(--md-error);
}

.settings-download-progress.muted .settings-download-progress-fill {
  background: var(--md-outline);
}

.download-cancel-btn {
  width: 30px;
  height: 30px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-error);
  flex-shrink: 0;
  transition: background var(--duration-short);

  &:hover {
    background: color-mix(in srgb, var(--md-error) 12%, transparent);
  }

  &:disabled {
    opacity: 0.45;
    pointer-events: none;
  }

  .material-symbols-rounded {
    font-size: 18px;
  }
}

@keyframes settings-download-indeterminate {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(280%); }
}

/* 深色模式切换胶囊：滑动拇指 */
.dark-mode-pills {
  position: relative;
  display: grid;
  grid-template-columns: repeat(3, 36px);
  background: var(--md-surface-container-highest);
  border-radius: var(--radius-full);
  padding: 3px;
  gap: 2px;
}

.pill-thumb {
  position: absolute;
  top: 3px;
  bottom: 3px;
  left: 3px;
  width: 36px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  /* 650ms 非线性 emphasized：先快后慢，滑动更有一贯性 */
  transition: transform 650ms cubic-bezier(0.2, 0, 0, 1);
  pointer-events: none;
  z-index: 0;
  will-change: transform;
  box-shadow: 0 1px 4px color-mix(in srgb, var(--md-primary) 35%, transparent);
}

.pill {
  width: 36px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  position: relative;
  z-index: 1;
  transition: color 650ms cubic-bezier(0.2, 0, 0, 1);

  &.active {
    color: var(--md-on-primary);
  }
  &:hover:not(.active) { color: var(--md-on-surface); }
}

/* 主题色选择 */
.color-row {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.color-dot {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 2px solid transparent;
  transition: transform var(--duration-short), border-color var(--duration-short);

  &:hover { transform: scale(1.15); }
  &.selected { border-color: rgba(255,255,255,0.8); }
}

/* 折叠区段箭头 */
.section-label.clickable {
  cursor: pointer;
  user-select: none;

  &:hover { opacity: 0.8; }
}

.section-arrow {
  margin-left: auto;
  font-size: 18px !important;
  transition: transform var(--duration-medium) var(--ease-standard);
  opacity: 0.5;

  &.expanded { transform: rotate(180deg); }
}

/* 子级设置卡片 */
.sub-card {
  margin-left: 54px;
  background: var(--md-surface-container-low) !important;
}

.setting-card.sync-frequency-card {
  margin-left: 0;
}

/* M3 Chip 选择器 */
.chip-row, .chip-wrap {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chip-row { flex-shrink: 0; }
.chip-wrap { margin-top: 8px; }

.m3-chip {
  padding: 6px 14px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  font-family: inherit;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  border: 1px solid var(--md-outline-variant);
  cursor: pointer;
  transition: all var(--duration-short) var(--ease-standard);
  white-space: nowrap;

  &:hover { background: var(--md-surface-variant); }

  &.active {
    background: var(--md-primary);
    color: var(--md-on-primary);
    border-color: var(--md-primary);
  }

  &.sm {
    padding: 4px 10px;
    font-size: 12px;
  }

  /* 危险语义变体：error 色 outlined，用于重置身份等破坏性操作 */
  &.danger {
    color: var(--md-error);
    border-color: color-mix(in srgb, var(--md-error) 40%, transparent);

    &:hover { background: color-mix(in srgb, var(--md-error) 10%, transparent); }
  }
}

.quality-card {
  flex-wrap: wrap;
}

/* 一起听服务器地址输入 */
.lt-url-input {
  width: 200px;
  padding: 4px 10px;
  border-radius: var(--radius-full);
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface);
  font-size: 12px;
  text-align: right;
  outline: none;
  flex-shrink: 0;
  transition: border-color 150ms;
  &:focus { border-color: var(--md-primary); }

  /* 左对齐变体：昵称等普通文本输入，右对齐只适合 URL */
  &.lt-input-left { text-align: left; }
}

/* M3 Slider */
.m3-slider {
  appearance: none;
  width: 120px;
  height: 4px;
  border-radius: 2px;
  background: var(--md-surface-container-highest);
  outline: none;
  cursor: pointer;
  flex-shrink: 0;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--md-primary);
    cursor: pointer;
    transition: transform var(--duration-short);
  }

  &::-webkit-slider-thumb:hover { transform: scale(1.2); }
}

/* 同步断开按钮 */
.sync-disconnect-btn {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-error);
  transition: background var(--duration-short);
  flex-shrink: 0;

  &:hover { background: color-mix(in srgb, var(--md-error) 10%, transparent); }
}

/* 同步状态标签 */
.sync-status-pill {
  font-size: 11px;
  font-weight: 500;
  padding: 4px 10px;
  border-radius: var(--radius-full);
  flex-shrink: 0;

  &.configured {
    background: color-mix(in srgb, var(--md-primary) 12%, transparent);
    color: var(--md-primary);
  }
}

.sync-action-label {
  font-size: 13px;
  font-weight: 500;
  color: var(--md-primary);
  flex-shrink: 0;
}

.clear-config-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 20px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  color: var(--md-error);
  transition: background var(--duration-short);
  cursor: pointer;

  &:hover { background: color-mix(in srgb, var(--md-error) 10%, transparent); }
}

.spinning {
  animation: spin 1s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

/* 对话框 */
.dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  backdrop-filter: blur(4px);
  animation: overlay-fade-in 200ms ease;
}

@keyframes overlay-fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

.dialog-card {
  background: var(--md-surface-container-high);
  border-radius: var(--radius-xl);
  padding: 24px;
  width: 380px;
  max-width: 90vw;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
  animation: dialog-scale-in 250ms cubic-bezier(0.05, 0.7, 0.1, 1);
  transform-origin: center;
}

@keyframes dialog-scale-in {
  from { opacity: 0; transform: scale(0.92); }
  to { opacity: 1; transform: scale(1); }
}

.dialog-title {
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 20px;
}

.dialog-icon {
  width: 42px;
  height: 42px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 14px;
  border-radius: 50%;
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);

  &.warning {
    background: color-mix(in srgb, var(--md-tertiary, #7d5260) 16%, transparent);
    color: var(--md-tertiary, #7d5260);
  }

  &.danger {
    background: color-mix(in srgb, var(--md-error) 16%, transparent);
    color: var(--md-error);
  }
}

.config-warning-dialog {
  width: 430px;
}

.warning-list {
  display: flex;
  flex-direction: column;
  gap: 9px;
  margin-top: 16px;
  padding: 12px 14px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--md-error) 7%, var(--md-surface-container));
  color: var(--md-on-surface-variant);
  font-size: 12px;

  div {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .material-symbols-rounded {
    color: var(--md-error);
    font-size: 17px;
  }
}

.dialog-field {
  margin-bottom: 14px;

  label {
    display: block;
    font-size: 12px;
    font-weight: 500;
    color: var(--md-on-surface-variant);
    margin-bottom: 6px;
  }

  input {
    width: 100%;
    padding: 10px 14px;
    border-radius: var(--radius-md);
    border: 1px solid var(--md-outline-variant);
    background: var(--md-surface-container);
    color: var(--md-on-surface);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color var(--duration-short);

    &:focus {
      border-color: var(--md-primary);
    }

    &::placeholder {
      color: var(--md-on-surface-variant);
      opacity: 0.5;
    }
  }
}

.dialog-error {
  font-size: 12px;
  color: var(--md-error);
  margin-bottom: 12px;
}

.dialog-check {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 12px 0;
  font-size: 13px;
  line-height: 1.4;
  color: var(--md-on-surface);
  cursor: pointer;

  input {
    margin-top: 2px;
    accent-color: var(--md-primary);
  }
}

.dialog-desc {
  font-size: 13px;
  color: var(--md-on-surface-variant);
  line-height: 1.5;
  margin-bottom: 4px;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 20px;
}

.dialog-btn {
  padding: 8px 20px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background var(--duration-short);
  color: var(--md-on-surface);
  display: flex;
  align-items: center;
  gap: 6px;

  &:hover { background: var(--md-surface-container-highest); }

  &.primary {
    background: var(--md-primary);
    color: var(--md-on-primary);

    &:hover { opacity: 0.9; }
    &:disabled { opacity: 0.5; cursor: not-allowed; }
  }

  &.danger {
    background: var(--md-error);
    color: var(--md-on-error, #fff);

    &:hover { opacity: 0.9; }
  }
}

/* 两阶段引导样式 */
.phase-section {
  margin-bottom: 18px;
  padding: 14px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
}

.phase-header {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}

.phase-number {
  width: 24px;
  height: 24px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  font-size: 12px;
  font-weight: 600;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;

  &.done {
    background: color-mix(in srgb, var(--md-primary) 20%, transparent);
    color: var(--md-primary);
  }
}

.phase-label {
  font-size: 14px;
  font-weight: 600;
}

.phase-body {
  padding-left: 34px;
}

.phase-done-info {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-left: 34px;
  font-size: 13px;
  color: var(--md-on-surface-variant);
}

.field-hint {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
  margin: 4px 0 8px;
}

.text-link-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 500;
  color: var(--md-primary);
  cursor: pointer;
  padding: 4px 0;
  transition: opacity var(--duration-short);

  &:hover { opacity: 0.8; }
}

.radio-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.radio-option {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
  cursor: pointer;
  padding: 6px 0;
  color: var(--md-on-surface);

  &.active { font-weight: 500; }
}

.radio-dot {
  width: 18px;
  height: 18px;
  border-radius: var(--radius-full);
  border: 2px solid var(--md-outline);
  position: relative;
  flex-shrink: 0;
  transition: border-color var(--duration-short);

  &.checked {
    border-color: var(--md-primary);

    &::after {
      content: '';
      position: absolute;
      inset: 3px;
      border-radius: var(--radius-full);
      background: var(--md-primary);
    }
  }
}

.template-preview {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 14px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
  margin-bottom: 12px;

  .preview-label {
    font-size: 11px;
    color: var(--md-on-surface-variant);
    opacity: 0.7;
  }
  .preview-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--md-on-surface);
    word-break: break-all;
  }
}

@media (max-width: 960px) {
  .settings-view { padding-inline: 24px; }

  .settings-layout {
    grid-template-columns: minmax(210px, 240px) minmax(0, 1fr);
    gap: 24px;
  }

  .settings-content { padding-top: 10px; }
}

@media (max-width: 760px) {
  :global(.content:has(.settings-view)) {
    height: auto;
    overflow-y: auto;
  }

  .settings-view {
    height: auto;
    min-height: 100%;
    overflow: visible;
    padding: 20px 18px 32px;
  }

  .settings-layout {
    display: block;
    height: auto;
    min-height: 0;
    max-width: none;
  }

  .settings-sidebar {
    position: static;
    min-height: 0;
    overflow: visible;
    padding: 0;
  }

  .page-title {
    margin-bottom: 14px;
    font-size: 24px;
    /* 窄屏为单列布局，无需与右侧头部等高 */
    min-height: auto;
  }

  .settings-nav {
    flex-direction: row;
    gap: 8px;
    overflow-x: auto;
    padding: 0 2px 8px;
    scrollbar-width: none;

    &::-webkit-scrollbar { display: none; }
  }

  .settings-nav-group {
    display: contents;
  }

  .settings-nav-item {
    flex: 0 0 210px;
    min-height: 56px;
  }

  .settings-nav-copy small { display: none; }

  .settings-content {
    max-width: none;
    min-height: 0;
    display: block;
    overflow: visible;
    padding-top: 12px;
  }

  /* 窄屏下整页滚动，内部滚动容器退化为普通块 */
  .settings-content-scroll {
    overflow: visible;
    padding-right: 0;
  }

  .settings-content-header {
    margin-bottom: 18px;

    h2 { font-size: 24px; }
  }
}

@media (max-width: 480px) {
  .settings-view { padding-inline: 14px; }

  .settings-nav-item {
    flex-basis: 184px;
    padding-inline: 10px;
  }

  .setting-card {
    gap: 10px;
    padding: 12px;
  }

  .setting-card--select {
    .setting-info { flex-basis: calc(100% - 46px); }
    .settings-select { flex-basis: 100%; margin-left: 46px; }
  }

  .setting-icon-wrap {
    width: 36px;
    height: 36px;
  }

  .sub-card { margin-left: 20px; }
}

/* 快捷键说明 */
.shortcut-card { align-items: flex-start; }

.shortcut-list {
  list-style: none;
  margin: 12px 0 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 6px 20px;
}

.shortcut-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 26px;
}

.shortcut-label {
  font-size: 13px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.shortcut-keys {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.shortcut-keys kbd {
  min-width: 24px;
  padding: 2px 7px;
  border-radius: var(--radius-sm, 8px);
  background: var(--md-surface-container-highest);
  border: 1px solid var(--md-outline-variant, rgba(255, 255, 255, 0.12));
  border-bottom-width: 2px;
  font-family: inherit;
  font-size: 11px;
  font-weight: 600;
  line-height: 16px;
  text-align: center;
  color: var(--md-on-surface);
}
</style>
