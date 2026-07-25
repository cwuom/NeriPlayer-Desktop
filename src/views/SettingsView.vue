<script setup lang="ts">
import { ref, computed, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { SUPPORTED_LOCALES, setLocaleWithTransition } from '@/i18n'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import {
  MAX_MEDIA_CACHE_SIZE_MB,
  MIN_MEDIA_CACHE_SIZE_MB,
  useSettingsStore,
} from '@/stores/settings'
import { useAuthStore } from '@/stores/auth'
import { useSyncStore, type SyncFrequency } from '@/stores/sync'
import { useDownloadStore } from '@/stores/download'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { DEFAULT_DOWNLOAD_NAME_TEMPLATE } from '@/stores/settings'
import StorageManagementDialog from '@/components/StorageManagementDialog.vue'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'
import {
  clearBrowserCache,
  mergeBrowserCacheUsage,
  type StorageCacheClearOptions,
  type StorageUsageSummary,
} from '@/utils/storage'
import { switchThemeWithRipple, type ThemeMode } from '@/utils/theme'
import { THEME_COLORS, getSwatchColor, applyThemeColor, getSavedThemeColor, switchThemeColorWithRipple } from '@/utils/themeColor'
import { createLogger } from '@/utils/logger'

const log = createLogger('settings-view')

const { t, locale } = useI18n()
const router = useRouter()
const settings = useSettingsStore()
const auth = useAuthStore()
const syncStore = useSyncStore()
const downloadStore = useDownloadStore()
const player = usePlayerStore()
const toast = useToastStore()
const {
  darkMode, themeColor: selectedColor, coverStyle,
  defaultScreen, showCoverBadge, showNowPlayingTitle, showToolbarDock,
  showQualitySwitch, showAudioCodec, showAudioSpec, lyricFontScale,
  crossfade, normalizeVolume,
  fadeIn, fadeInDuration, fadeOutDuration,
  crossfadeNext, crossfadeInDuration, crossfadeOutDuration,
  keepProgress, keepPlaybackMode,
  showTranslation, lyricBlur, lyricBlurAmount,
  cloudMusicOffset, qqMusicOffset,
  advancedLyrics, dynamicBackground, dynamicColor, audioReactive,
  coverBlurBg, coverBlurAmount, coverBlurDarken,
  neteaseQuality, youtubeQuality, biliQuality,
  bypassProxy, internationalizationEnabled,
  backgroundImageUri, backgroundImageBlur, backgroundImageAlpha,
  devModeEnabled, logToFile, logLevel,
  maxCacheSize, downloadNameTemplate, downloadDir,
  ltServerUrl, ltNickname, ltAllowMemberControl, ltAutoPauseOnMemberChange, ltShareAudioLinks,
  locale: settingLocale,
} = storeToRefs(settings)

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

async function openLogDir() {
  try {
    const dir = await invoke<string>('get_log_dir')
    await revealItemInDir(dir)
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

const presetColors = THEME_COLORS.map(c => ({
  key: c.key,
  color: c.dark['--md-primary'],
}))

const activeColorKey = ref(selectedColor.value || getSavedThemeColor())

watch(selectedColor, value => {
  activeColorKey.value = value
})

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
  { value: 'high', label: t('settings.q_high') },
  { value: 'exhigh', label: t('settings.q_exhigh') },
  { value: 'lossless', label: t('settings.q_lossless') },
  { value: 'hires', label: t('settings.q_hires') },
  { value: 'jyeffect', label: t('settings.q_surround') },
  { value: 'sky', label: t('settings.q_sky') },
  { value: 'jymaster', label: t('settings.q_master') },
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

type OnlineQualitySource = 'netease' | 'youtube' | 'bilibili'
const qualitySwitching = ref(false)

function qualityForSource(source: OnlineQualitySource): string {
  if (source === 'netease') return neteaseQuality.value
  if (source === 'youtube') return youtubeQuality.value
  return biliQuality.value
}

function setQualityForSource(source: OnlineQualitySource, value: string) {
  if (source === 'netease') neteaseQuality.value = value
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
  'accounts', 'playback', 'quality', 'storage', 'personalization', 'motion', 'lyrics', 'network',
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
      quality: ['quality'],
      motion: ['effects'],
      lyrics: ['lyrics'],
      network: [],
      storage: ['storage', 'downloads'],
      backup: ['backup'],
      listen_together: ['listen_together'],
      language: [],
      about: [],
    }
    expandedSections.value = new Set([...expandedSections.value, ...defaults[id]])
    visitedSettingsSections.value = new Set([...visitedSettingsSections.value, id])
  }
  persistSettingsUiState()
  void restoreSettingsScrollPosition(id)
}

// 启动时检查登录状态
onMounted(() => {
  auth.checkStatus()
  syncStore.loadConfigs()
  downloadStore.initEvents()
  downloadStore.loadDownloads()
  // 加载构建信息
  loadBuildInfo()
  // 加载默认下载目录
  loadDefaultDownloadDir()
  void restoreSettingsScrollPosition(activeSettingsSection.value)
})

onBeforeUnmount(() => {
  rememberSettingsScrollPosition()
  persistSettingsUiState()
})

// 网络：绕过代理
async function handleBypassProxyChange(val: boolean) {
  bypassProxy.value = val
  try {
    await invoke('set_bypass_proxy', { bypass: val })
  } catch (e) {
    log.error('Failed to set bypass proxy:', e)
  }
}

// 一起听
const showResetLtIdentityConfirm = ref(false)

function confirmResetLtIdentity() {
  showResetLtIdentityConfirm.value = false
  localStorage.removeItem('neri:lt-uuid')
  toast.success(t('listen_together.identity_reset'))
}

const showConfigExportWarning = ref(false)

async function confirmConfigExport() {
  showConfigExportWarning.value = false
  await syncStore.exportConfig()
}

async function importConfig() {
  await syncStore.importConfig()
}

// 下载管理
const activeDownloadCount = computed(() => downloadStore.downloading.size)
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
    case 'resolving': return t('download.resolving')
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
  if (task.status === 'resolving' || task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'already_exists') {
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
      { options },
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

async function handleIntlToggle(val: boolean) {
  const prev = internationalizationEnabled.value
  internationalizationEnabled.value = val
  if (val) {
    intlChecking.value = true
    try {
      // 简单连通性检测：尝试调用 YouTube API
      await invoke('get_youtube_audio_url', { videoId: 'dQw4w9WgXcQ' })
    } catch {
      // 失败时回退
      internationalizationEnabled.value = prev
      toast.error(t('settings.intl_check_failed'))
    } finally {
      intlChecking.value = false
    }
  }
}

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
      backgroundImageUri.value = typeof result === 'string' ? result : (result as any).path || String(result)
    }
  } catch (e) {
    log.error('Failed to select image:', e)
  }
}

function clearBackgroundImage() {
  backgroundImageUri.value = ''
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
const PROJECT_REPOSITORY_URL = 'https://github.com/nicepkg/NeriPlayer'

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
  if (!webdavUrl.value.trim() || !webdavUsername.value.trim()) return
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
  return new Date(ms).toLocaleString()
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

// 切换省流模式前确认，避免用户误解已有云端文件格式
const showDataSaverConfirm = ref(false)
const pendingDataSaverValue = ref<boolean | null>(null)

function requestDataSaverChange(event: Event) {
  const target = event.target as HTMLInputElement | null
  if (!target || target.checked === syncStore.github.dataSaver) return
  pendingDataSaverValue.value = target.checked
  showDataSaverConfirm.value = true
}

function cancelDataSaverChange() {
  pendingDataSaverValue.value = null
  showDataSaverConfirm.value = false
}

function confirmDataSaverChange() {
  if (pendingDataSaverValue.value !== null) {
    syncStore.github.dataSaver = pendingDataSaverValue.value
  }
  cancelDataSaverChange()
}
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

      <main ref="settingsContentRef" class="settings-content">
        <header class="settings-content-header">
          <div>
            <h2>{{ activeSettingsItem?.label }}</h2>
            <p>{{ activeSettingsItem?.description }}</p>
          </div>
        </header>

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

        <div v-show="activeSettingsSection === 'personalization'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('personal') }">

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
        <input type="checkbox" :checked="internationalizationEnabled" @change="handleIntlToggle(($event.target as HTMLInputElement).checked)" />
        <span class="track"><span class="thumb">
          <span v-if="intlChecking" class="material-symbols-rounded spinning" style="font-size: 14px">progress_activity</span>
          <span v-else-if="internationalizationEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span>
        </span></span>
      </label>
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
        <div class="color-row" :style="dynamicColor ? { opacity: 0.4, pointerEvents: 'none' } : undefined">
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

    <!-- 动态取色 -->
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">colorize</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.dynamic_color') }}</div>
        <div class="setting-desc">{{ t('settings.dynamic_color_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="dynamicColor" /><span class="track"><span class="thumb"><span v-if="dynamicColor" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
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

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">audio_file</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.audio_codec') }}</div>
          <div class="setting-desc">{{ t('settings.audio_codec_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showAudioCodec" /><span class="track"><span class="thumb"><span v-if="showAudioCodec" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">equalizer</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.audio_spec') }}</div>
          <div class="setting-desc">{{ t('settings.audio_spec_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showAudioSpec" /><span class="track"><span class="thumb"><span v-if="showAudioSpec" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">format_size</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.lyric_font_size') }}</div>
          <EditableRangeValue
            v-model="lyricFontScale"
            class="setting-desc"
            :min="0.5"
            :max="1.5"
            :step="0.1"
            :display-value="`${lyricFontScale.toFixed(1)}x`"
            input-suffix="x"
            :aria-label="t('settings.lyric_font_size')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="lyricFontScale" min="0.5" max="1.5" step="0.1" />
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
      </template>
    </div></Transition>
        </div>

    <!-- 播放 -->
        <div v-show="activeSettingsSection === 'playback'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('playback') }">
    <div class="section-label clickable" @click="toggleSection('playback')">
      <span class="material-symbols-rounded" style="font-size: 18px">play_circle</span>
      <span>{{ t('settings.playback') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('playback') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">swap_horiz</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.crossfade') }}</div>
        <div class="setting-desc">{{ t('settings.crossfade_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="crossfade" /><span class="track"><span class="thumb"><span v-if="crossfade" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
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
        <div v-show="activeSettingsSection === 'listen_together'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('listen_together') }">
    <div class="section-label clickable" @click="toggleSection('listen_together')">
      <span class="material-symbols-rounded" style="font-size: 18px">group</span>
      <span>{{ t('listen_together.title') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('listen_together') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">dns</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('listen_together.server_url') }}</div>
        <div class="setting-desc" style="word-break: break-all">{{ ltServerUrl || 'https://neriplayer.hancat.work' }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">edit</span>
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
          class="lt-url-input"
          style="width: 140px"
          :value="ltNickname"
          @change="ltNickname = ($event.target as HTMLInputElement).value"
          maxlength="20"
          :placeholder="t('listen_together.nickname')"
        />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tune</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.allow_member_control') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" v-model="ltAllowMemberControl" />
          <span class="track"><span class="thumb"><span v-if="ltAllowMemberControl" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">pause_circle</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.auto_pause_on_change') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" v-model="ltAutoPauseOnMemberChange" />
          <span class="track"><span class="thumb"><span v-if="ltAutoPauseOnMemberChange" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">link</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.share_audio_links') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" v-model="ltShareAudioLinks" />
          <span class="track"><span class="thumb"><span v-if="ltShareAudioLinks" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">restart_alt</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.reset_identity') }}</div>
          <div class="setting-desc">{{ t('listen_together.reset_identity_desc') }}</div>
        </div>
        <button class="m3-chip sm" @click="showResetLtIdentityConfirm = true">{{ t('listen_together.reset_btn') }}</button>
      </div>
    </div></Transition>
        </div>

    <!-- 下载管理 -->
        <div v-show="activeSettingsSection === 'storage'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('downloads') }">
    <div class="section-label clickable" @click="toggleSection('downloads')">
      <span class="material-symbols-rounded" style="font-size: 18px">download</span>
      <span>{{ t('settings.download_manage') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('downloads') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('downloads')">
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
              indeterminate: task.status === 'downloading' && !task.totalBytes,
              error: task.status === 'error',
              muted: task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'already_exists',
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
    </div></Transition>
        </div>

    <!-- 歌词 -->
        <div v-show="activeSettingsSection === 'lyrics'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('lyrics') }">
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

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">music_note</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.netease_offset') }}</div>
          <EditableRangeValue
            v-model="cloudMusicOffset"
            class="setting-desc"
            :min="-2000"
            :max="2000"
            :step="50"
            :display-value="`${cloudMusicOffset >= 0 ? '+' : ''}${cloudMusicOffset}ms`"
            input-suffix="ms"
            :aria-label="t('settings.netease_offset')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="cloudMusicOffset" min="-2000" max="2000" step="50" />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">music_note</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.qq_offset') }}</div>
          <EditableRangeValue
            v-model="qqMusicOffset"
            class="setting-desc"
            :min="-2000"
            :max="2000"
            :step="50"
            :display-value="`${qqMusicOffset >= 0 ? '+' : ''}${qqMusicOffset}ms`"
            input-suffix="ms"
            :aria-label="t('settings.qq_offset')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="qqMusicOffset" min="-2000" max="2000" step="50" />
      </div>
    </div></Transition>
        </div>

    <!-- 动效 & 视觉 -->
        <div v-show="activeSettingsSection === 'motion'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('effects') }">
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

      <div v-if="!dynamicBackground" class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">blur_circular</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_blur') }}</div>
          <div class="setting-desc">{{ t('settings.cover_blur_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="coverBlurBg" /><span class="track"><span class="thumb"><span v-if="coverBlurBg" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="!dynamicBackground && coverBlurBg">
        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.blur_amount') }}</div>
            <EditableRangeValue
              v-model="coverBlurAmount"
              class="setting-desc"
              :min="0"
              :max="500"
              :step="10"
              :display-value="coverBlurAmount.toFixed(1)"
              :aria-label="t('settings.blur_amount')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="coverBlurAmount" min="0" max="500" step="10" />
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

    <!-- 音质 -->
        <div v-show="activeSettingsSection === 'quality'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('quality') }">
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
        <div v-show="activeSettingsSection === 'storage'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('storage') }">
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
        <label class="m3-switch"><input type="checkbox" v-model="logToFile" /><span class="track"><span class="thumb"><span v-if="logToFile" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
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
        <div v-show="activeSettingsSection === 'backup'" class="settings-section-panel" :class="{ 'is-collapsed': !isExpanded('backup') }">
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

        <!-- 数据节省模式 -->
        <div class="setting-card sub-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">download</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.data_saver') }}</div>
            <div class="setting-desc">{{ t('settings.data_saver_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" :checked="syncStore.github.dataSaver" @change="requestDataSaverChange" /><span class="track"><span class="thumb"><span v-if="syncStore.github.dataSaver" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
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
            <button class="dialog-btn primary" :disabled="webdavConfiguring || !webdavUrl.trim() || !webdavUsername.trim()" @click="configureWebDav">
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

    <!-- 切换省流模式确认 -->
    <Teleport to="body">
      <div v-if="showDataSaverConfirm" class="dialog-overlay" @click.self="cancelDataSaverChange">
        <div class="dialog-card" style="width: 380px">
          <div class="dialog-icon warning">
            <span class="material-symbols-rounded">warning</span>
          </div>
          <h3 class="dialog-title">{{ t('settings.data_saver_warning_title') }}</h3>
          <p class="dialog-desc">{{ t('settings.data_saver_warning_message') }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="cancelDataSaverChange">{{ t('settings.cancel') }}</button>
            <button class="dialog-btn primary" @click="confirmDataSaverChange">{{ t('settings.data_saver_warning_confirm') }}</button>
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
.settings-view {
  width: 100%;
  height: 100%;
  min-height: 0;
  box-sizing: border-box;
  overflow: hidden;
  padding: 28px clamp(24px, 4vw, 64px) 0;
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

.settings-sidebar {
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding-top: 18px;
  padding-right: 4px;

  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
}

.settings-nav {
  display: flex;
  flex-direction: column;
  gap: 10px;
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

.settings-content {
  width: 100%;
  min-width: 0;
  min-height: 0;
  max-width: 760px;
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding-right: 4px;
  padding-top: 18px;

  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
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

.settings-section-panel.is-collapsed > .section-label.clickable ~ * {
  display: none !important;
}

.settings-section-intro {
  margin: -2px 4px 14px;
  color: var(--md-on-surface-variant);
  font-size: 12px;
  line-height: 1.5;
}

.settings-section-panel > .section-label:first-child {
  margin-top: 24px;
}

.settings-content > .settings-section-panel:first-of-type > .section-label:first-child {
  margin-top: 0;
}

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
  margin: 0 0 20px;
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

/* M3 Switch：严格对齐 M3 规范 */
.m3-switch {
  position: relative;
  flex-shrink: 0;
  cursor: pointer;

  input { display: none; }

  .track {
    display: flex;
    align-items: center;
    width: 52px;
    height: 32px;
    border-radius: 16px;
    background: var(--md-surface-container-highest);
    border: 2px solid var(--md-outline);
    position: relative;
    transition: background var(--duration-medium) var(--ease-standard),
                border-color var(--duration-medium) var(--ease-standard);
  }

  .thumb {
    position: absolute;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--md-outline);
    top: 50%;
    left: 6px;
    transform: translateY(-50%);
    transition: left var(--duration-medium) var(--ease-standard),
                width var(--duration-medium) var(--ease-standard),
                height var(--duration-medium) var(--ease-standard),
                background var(--duration-medium) var(--ease-standard);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--md-on-primary);
    font-size: 0;
  }

  input:checked + .track {
    background: var(--md-primary);
    border-color: var(--md-primary);

    .thumb {
      left: 22px;
      width: 24px;
      height: 24px;
      background: var(--md-on-primary);
      font-size: 14px;
    }
  }
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
    overflow: visible;
    padding-right: 0;
    padding-top: 12px;
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

  .setting-icon-wrap {
    width: 36px;
    height: 36px;
  }

  .sub-card { margin-left: 20px; }
}
</style>
