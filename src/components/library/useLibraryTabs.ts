import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

interface LibraryTab { label: string; key: string; icon?: string; brandIcon?: string }

export type NeteaseCategory = 'playlists' | 'albums'

// 根据路由 query 参数设置初始标签；旧的 netease_albums / netease_playlists 仍可用
const tabKeyToIndex: Record<string, number> = {
  local: 0, favorites: 1, downloads: 2,
  netease: 3, netease_playlists: 3, netease_albums: 3,
  bilibili_favorites: 4, youtube_playlists: 5,
}

/** 资料库顶部 tab：路由 query 同步，以及按 tab 分别保存的搜索关键词 */
export function useLibraryTabs() {
  const route = useRoute()
  const { t } = useI18n()

  const tabs = computed<LibraryTab[]>(() => [
    { label: t('library.tab_local'), icon: 'folder_open', key: 'local' },
    { label: t('library.tab_favorites'), icon: 'favorite', key: 'favorites' },
    { label: t('library.tab_downloads'), icon: 'download', key: 'downloads' },
    // 网易云歌单与专辑合成一个板块，内部再分类（对齐 Android
    // LibraryTab.NETEASEALBUM.asVisibleLibraryTab() == NETEASE）
    { label: t('library.tab_netease'), brandIcon: '/icons/ic_netease.svg', key: 'netease' },
    { label: t('library.bilibili_favorites'), brandIcon: '/icons/ic_bilibili.svg', key: 'bilibili_favorites' },
    { label: 'YouTube Music', brandIcon: '/icons/ic_youtube.svg', key: 'youtube_playlists' },
  ])
  const neteaseCategory = ref<NeteaseCategory>('playlists')
  const initialTab = typeof route.query.tab === 'string' ? (tabKeyToIndex[route.query.tab] ?? 0) : 0
  if (route.query.tab === 'netease_albums') neteaseCategory.value = 'albums'
  const activeTab = ref(initialTab)

  // 监听路由 query 变化（同页面内导航）
  watch(() => route.query.tab, (tab) => {
    if (typeof tab === 'string' && tab in tabKeyToIndex) {
      activeTab.value = tabKeyToIndex[tab]
      if (tab === 'netease_albums') neteaseCategory.value = 'albums'
      else if (tab === 'netease_playlists') neteaseCategory.value = 'playlists'
    }
  })

  // 每个 tab 的搜索（对齐 Android 各库页顶部的搜索栏）
  //
  // 按 tab 分别保存关键词，切回来时不会丢；本地页的歌手分类复用同一个
  // 搜索框实例（位置、尺寸不变，避免布局跳动），但绑定到歌手专用的
  // 关键词（要同时匹配歌手与其名下曲目）
  const tabQueries = ref<Record<number, string>>({})
  const tabQuery = computed({
    get: () => tabQueries.value[activeTab.value] ?? '',
    set: (value: string) => { tabQueries.value = { ...tabQueries.value, [activeTab.value]: value } },
  })

  return { tabs, neteaseCategory, activeTab, tabQuery }
}
