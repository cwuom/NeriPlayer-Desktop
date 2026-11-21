import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

export type SettingsSectionId =
  | 'accounts'
  | 'general'
  | 'shortcuts'
  | 'language'
  | 'playback'
  | 'audio'
  | 'audio_effects'
  | 'quality'
  | 'playback_sources'
  | 'appearance'
  | 'now_playing'
  | 'background'
  | 'motion'
  | 'lyrics'
  | 'desktop_lyrics'
  | 'downloads'
  | 'storage'
  | 'network'
  | 'backup'
  | 'listen_together'
  | 'about'

const SETTINGS_UI_STATE_KEY = 'neri:settings-ui-state'
export const SETTINGS_SECTION_IDS: SettingsSectionId[] = [
  'accounts', 'general', 'shortcuts', 'language', 'playback', 'audio', 'audio_effects', 'quality', 'playback_sources',
  'appearance', 'now_playing', 'background', 'motion', 'lyrics', 'desktop_lyrics',
  'downloads', 'storage', 'network', 'backup', 'listen_together', 'about',
]

export type SettingsUiState = {
  activeSection?: SettingsSectionId
  expandedSections?: string[]
  visitedSections?: string[]
  scrollPositions?: Record<string, number>
}

export interface SettingsNavItem {
  id: SettingsSectionId
  label: string
  description: string
  icon: string
}

export interface SettingsNavGroup {
  items: SettingsNavItem[]
}

// 存储读取失败或内容损坏时一律回到默认分区，读取本身也放在 try 里兜底
export function readSettingsUiState(storage?: Pick<Storage, 'getItem'>): SettingsUiState {
  try {
    const parsed = JSON.parse((storage ?? localStorage).getItem(SETTINGS_UI_STATE_KEY) || '{}') as SettingsUiState
    return SETTINGS_SECTION_IDS.includes(parsed.activeSection as SettingsSectionId) ? parsed : {}
  } catch {
    return {}
  }
}

/** 首次进入某分区时默认展开的折叠分组 */
const SECTION_DEFAULT_EXPANDED: Record<SettingsSectionId, string[]> = {
  accounts: [],
  general: [],
  shortcuts: [],
  language: [],
  playback: ['playback'],
  audio: [],
  audio_effects: [],
  quality: ['quality'],
  playback_sources: [],
  appearance: [],
  now_playing: [],
  background: [],
  motion: ['effects'],
  lyrics: ['lyrics'],
  desktop_lyrics: ['desktop_lyrics'],
  downloads: [],
  storage: ['storage'],
  network: [],
  backup: ['backup'],
  listen_together: ['listen_together'],
  about: [],
}

export function useSettingsSections() {
  const { t } = useI18n()

  const savedSettingsUiState = readSettingsUiState()
  const expandedSections = ref<Set<string>>(
    new Set(savedSettingsUiState.expandedSections || ['personal']),
  )
  const visitedSettingsSections = ref<Set<string>>(
    new Set(savedSettingsUiState.visitedSections || ['appearance']),
  )
  const activeSettingsSection = ref<SettingsSectionId>(
    savedSettingsUiState.activeSection || 'appearance',
  )
  const sectionScrollPositions = ref<Record<string, number>>(savedSettingsUiState.scrollPositions || {})
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

  const settingsNavGroups = computed<SettingsNavGroup[]>(() => [
    {
      items: [
        { id: 'accounts', label: t('settings.accounts'), description: t('settings.nav_accounts_desc'), icon: 'account_circle' },
      ],
    },
    {
      items: [
        { id: 'general', label: t('settings.general'), description: t('settings.nav_general_desc'), icon: 'settings' },
        { id: 'shortcuts', label: t('shortcuts.title'), description: t('settings.nav_shortcuts_desc'), icon: 'keyboard' },
        { id: 'language', label: t('settings.language'), description: t('settings.language_desc'), icon: 'translate' },
      ],
    },
    {
      items: [
        { id: 'playback', label: t('settings.playback'), description: t('settings.nav_playback_desc'), icon: 'play_circle' },
        { id: 'audio', label: t('settings.audio_processing'), description: t('settings.nav_audio_desc'), icon: 'graphic_eq' },
        { id: 'audio_effects', label: t('settings.audio_effects_section'), description: t('settings.audio_effects_desc'), icon: 'tune' },
        { id: 'quality', label: t('settings.audio_quality'), description: t('settings.audio_quality_desc'), icon: 'high_quality' },
        { id: 'playback_sources', label: t('settings.playback_sources'), description: t('settings.playback_sources_desc'), icon: 'alt_route' },
      ],
    },
    {
      items: [
        { id: 'appearance', label: t('settings.appearance'), description: t('settings.nav_appearance_desc'), icon: 'palette' },
        { id: 'now_playing', label: t('settings.now_playing_section'), description: t('settings.nav_now_playing_desc'), icon: 'album' },
        { id: 'background', label: t('settings.background_section'), description: t('settings.nav_background_desc'), icon: 'wallpaper' },
        { id: 'motion', label: t('settings.motion'), description: t('settings.motion_desc'), icon: 'bolt' },
      ],
    },
    {
      items: [
        { id: 'lyrics', label: t('settings.lyrics'), description: t('settings.nav_lyrics_desc'), icon: 'lyrics' },
        { id: 'desktop_lyrics', label: t('desktop_lyrics.title'), description: t('settings.nav_desktop_lyrics_desc'), icon: 'subtitles' },
      ],
    },
    {
      items: [
        { id: 'downloads', label: t('settings.downloads_section'), description: t('settings.nav_downloads_desc'), icon: 'download' },
        { id: 'storage', label: t('settings.storage'), description: t('settings.nav_storage_desc'), icon: 'storage' },
        { id: 'network', label: t('settings.network'), description: t('settings.nav_network_desc'), icon: 'router' },
      ],
    },
    {
      items: [
        { id: 'backup', label: t('settings.backup'), description: t('settings.nav_backup_desc'), icon: 'sync' },
        { id: 'listen_together', label: t('listen_together.title'), description: t('settings.nav_listen_together_desc'), icon: 'cloud' },
      ],
    },
    {
      items: [
        { id: 'about', label: t('settings.about'), description: t('settings.about_desc'), icon: 'info' },
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
      expandedSections.value = new Set([...expandedSections.value, ...SECTION_DEFAULT_EXPANDED[id]])
      visitedSettingsSections.value = new Set([...visitedSettingsSections.value, id])
    }
    persistSettingsUiState()
    // 滚动位置恢复改由面板过渡的 onPanelEnter 钩子触发（等新面板插入布局后再恢复）
  }

  function toggleSection(key: string) {
    const next = new Set(expandedSections.value)
    if (next.has(key)) next.delete(key)
    else next.add(key)
    expandedSections.value = next
    persistSettingsUiState()
  }
  function isExpanded(key: string) { return expandedSections.value.has(key) }

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

  return {
    expandedSections,
    activeSettingsSection,
    settingsContentRef,
    settingsNavGroups,
    activeSettingsItem,
    persistSettingsUiState,
    rememberSettingsScrollPosition,
    restoreSettingsScrollPosition,
    selectSettingsSection,
    toggleSection,
    isExpanded,
    applyRouteSection,
  }
}

export type SettingsSections = ReturnType<typeof useSettingsSections>
