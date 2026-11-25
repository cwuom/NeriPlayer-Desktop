import { computed, nextTick, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { SETTINGS_SEARCH_INDEX } from '@/modules/settings/searchIndex'
import { isMacPlatform, isWindowsPlatform } from '@/modules/shortcuts/platform'
import { filterAndRank, searchValue } from '@/modules/search/textMatcher'
import { SETTINGS_SECTION_IDS, type SettingsSectionId, type SettingsSections } from './useSettingsSections'

// ---------- 设置搜索（对齐 Android 设置页搜索框）----------

export interface SettingsSearchResult {
  key: string
  section: SettingsSectionId
  title: string
  desc: string
  keywords: string[]
  isSection: boolean
}

/** 每个分区里需要展开的折叠分组，跳转到具体设置项前先全部展开 */
const SECTION_COLLAPSE_KEYS: Partial<Record<SettingsSectionId, string[]>> = {
  playback: ['playback'],
  quality: ['quality'],
  motion: ['effects'],
  lyrics: ['lyrics'],
  desktop_lyrics: ['desktop_lyrics'],
  storage: ['storage'],
  backup: ['backup'],
  listen_together: ['listen_together'],
  about: ['about'],
}

export function useSettingsSearch(sections: SettingsSections) {
  const { t } = useI18n()
  const {
    expandedSections, activeSettingsSection, settingsContentRef, settingsNavGroups,
    persistSettingsUiState, restoreSettingsScrollPosition, selectSettingsSection,
  } = sections

  const settingsQuery = ref('')
  let pendingSearchFocus: { section: SettingsSectionId; title: string } | null = null

  const settingsSearchResults = computed<SettingsSearchResult[]>(() => {
    const query = settingsQuery.value.trim()
    if (!query) return []
    const navItems = settingsNavGroups.value.flatMap(group => group.items)
    const sectionLabel = (id: string) => navItems.find(item => item.id === id)?.label ?? ''
    const entries: SettingsSearchResult[] = [
      ...navItems.map(item => ({
        key: `section:${item.id}`, section: item.id, title: item.label, desc: item.description, keywords: [], isSection: true,
      })),
      ...SETTINGS_SEARCH_INDEX
        .filter(entry => SETTINGS_SECTION_IDS.includes(entry.section as SettingsSectionId) && (isMacPlatform || !entry.macOnly) && (isWindowsPlatform || !entry.windowsOnly))
        .map(entry => ({
          key: `${entry.section}:${entry.title}`,
          section: entry.section as SettingsSectionId,
          title: t(entry.title),
          desc: entry.desc ? t(entry.desc) : '',
          keywords: entry.keywords ?? [],
          isSection: false,
        })),
    ]
    return filterAndRank(query, entries, entry => [
      searchValue(entry.title, 0),
      searchValue(entry.keywords, 4),
      searchValue(sectionLabel(entry.section), 10),
      searchValue(entry.desc, 14),
    ]).slice(0, 50)
  })

  function openSettingsSearchResult(result: SettingsSearchResult | undefined) {
    if (!result) return
    settingsQuery.value = ''
    const collapseKeys = SECTION_COLLAPSE_KEYS[result.section] ?? []
    if (collapseKeys.length) {
      expandedSections.value = new Set([...expandedSections.value, ...collapseKeys])
      persistSettingsUiState()
    }
    if (result.isSection) {
      selectSettingsSection(result.section)
      return
    }
    if (result.section === activeSettingsSection.value) {
      void nextTick(() => focusSettingsItem(result.section, result.title))
      return
    }
    pendingSearchFocus = { section: result.section, title: result.title }
    selectSettingsSection(result.section)
  }

  /// 找到标题文字一致的设置卡片，滚到可视区中间并闪一下
  const audioEffectsSettingsRef = ref<{ revealSetting: (title: string) => Promise<void> } | null>(null)
  async function focusSettingsItem(section: SettingsSectionId, title: string, attempt = 0) {
    if (section === 'audio_effects' && title === t('player.eq_preset')) title = t('dsp.preset_section')
    if (section === 'audio_effects') await audioEffectsSettingsRef.value?.revealSetting(title)
    const panel = settingsContentRef.value?.querySelector(`.settings-section-panel[data-section="${section}"]`)
    const titleEl = [...(panel?.querySelectorAll<HTMLElement>('.setting-title') ?? [])]
      .find(element => element.textContent?.trim() === title)
    const card = titleEl?.closest<HTMLElement>(section === 'audio_effects' ? '.setting-card, .effects-import, .effects-section' : '.setting-card')
    if (!card) {
      // 折叠分组的展开动画还没把内容插进来时稍后再找
      if (attempt < 6) setTimeout(() => focusSettingsItem(section, title, attempt + 1), 80)
      return
    }
    card.scrollIntoView({ block: 'center', behavior: 'smooth' })
    card.classList.remove('search-highlight')
    void card.offsetWidth
    card.classList.add('search-highlight')
    setTimeout(() => card.classList.remove('search-highlight'), 1800)
  }

  // 分组切换 out-in 过渡：新面板插入布局后立即恢复滚动位置
  // 此时元素已占位但尚未淡入完成，不会产生可见的滚动跳动；
  // 若在旧面板离场期间恢复，scrollHeight 不足会导致 scrollTop 被截断
  function onPanelEnter() {
    if (pendingSearchFocus) {
      const target = pendingSearchFocus
      pendingSearchFocus = null
      void nextTick(() => focusSettingsItem(target.section, target.title))
      return
    }
    void restoreSettingsScrollPosition(activeSettingsSection.value)
  }

  return {
    settingsQuery,
    settingsSearchResults,
    openSettingsSearchResult,
    audioEffectsSettingsRef,
    onPanelEnter,
  }
}
