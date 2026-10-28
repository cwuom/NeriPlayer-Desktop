<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter, type RouteLocationRaw } from 'vue-router'

defineOptions({ name: 'ExploreView' })
import { useI18n } from 'vue-i18n'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import {
  EXPLORE_KINDS, useExploreSearchStore,
  type ExploreCollection, type ExploreItem, type ExploreKind, type ExploreSong,
} from '@/stores/exploreSearch'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useAuthStore } from '@/stores/auth'
import { useRecommendStore, type HomeFeedItem, type PlaylistInfo } from '@/stores/recommend'
import { useSettingsStore } from '@/stores/settings'
import { useSearchHistoryStore } from '@/stores/searchHistory'
import { useBiliVideoPartsStore } from '@/stores/biliVideoParts'
import { invoke } from '@tauri-apps/api/core'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import TrackContextMenu from '@/components/TrackContextMenu.vue'
import CollectionContextMenu from '@/components/CollectionContextMenu.vue'
import { collectionWebUrl, type CollectionMenuTarget } from '@/utils/collectionLinks'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'

const router = useRouter()
const route = useRoute()
const { t } = useI18n()
const searchStore = useExploreSearchStore()
const player = usePlayerStore()
const auth = useAuthStore()
const recommend = useRecommendStore()
const settings = useSettingsStore()
const searchHistory = useSearchHistoryStore()
const biliParts = useBiliVideoPartsStore()

// 搜索
const searchQuery = ref('')
const isFocused = ref(false)

// 平台 Tab
// 探索页不含 QQ 音乐（QQ 的搜索/播放/账号能力仍保留在其它模块）
// 「链接」页对齐 Android LINK_RECOGNITION：粘贴分享链接直接识别出歌曲、歌单或歌手
type PlatformTab = 'netease' | 'bilibili' | 'youtube' | 'link'
const PLATFORM_KEYS: PlatformTab[] = ['netease', 'bilibili', 'youtube', 'link']
// 国际化开启时 YouTube Music 为高优先级来源: tab 提前且作为默认平台
const ytmFirst = settings.internationalizationEnabled
const initialPlatform = PLATFORM_KEYS.includes(String(route.query.platform) as PlatformTab)
  ? String(route.query.platform) as PlatformTab
  : (ytmFirst ? 'youtube' : 'netease')
const activeTab = ref<PlatformTab>(initialPlatform)

const platformTabs = computed(() => {
  const tabs: Array<{ key: PlatformTab; label: string; icon?: string; symbol?: string }> = [
    { key: 'netease', label: t('settings.netease_account'), icon: '/icons/ic_netease.svg' },
    { key: 'bilibili', label: t('settings.bilibili_account'), icon: '/icons/ic_bilibili.svg' },
    { key: 'youtube', label: t('settings.youtube_account'), icon: '/icons/ic_youtube.svg' },
  ]
  if (settings.internationalizationEnabled) {
    tabs.unshift(...tabs.splice(2, 1))
  }
  tabs.push({ key: 'link', label: t('explore.tab_link'), symbol: 'link' })
  return tabs
})

// 每个平台记住自己上次选的分类，切回来不用重选
const searchKinds = ref<Record<Exclude<PlatformTab, 'link'>, ExploreKind>>({ netease: 'songs', bilibili: 'videos', youtube: 'songs' })
const activeKind = computed<ExploreKind>(() => activeTab.value === 'link' ? 'songs' : searchKinds.value[activeTab.value])
const kindOptions = computed(() => activeTab.value === 'link' ? [] : EXPLORE_KINDS[activeTab.value])
const KIND_ICONS: Record<ExploreKind, string> = {
  songs: 'music_note', playlists: 'queue_music', artists: 'person', videos: 'smart_display', creators: 'account_circle',
}
function selectKind(kind: ExploreKind) {
  if (activeTab.value === 'link' || searchKinds.value[activeTab.value] === kind) return
  searchKinds.value = { ...searchKinds.value, [activeTab.value]: kind }
}
const searchPlaceholder = computed(() => activeTab.value === 'link' ? t('explore.link_placeholder') : t('explore.search_placeholder'))

// 网易云歌单 Tag
const TAG_KEYS = [
  'tag_all', 'tag_pop', 'tag_soundtrack', 'tag_chinese', 'tag_nostalgia', 'tag_rock',
  'tag_acg', 'tag_western', 'tag_fresh', 'tag_night', 'tag_children', 'tag_folk',
  'tag_japanese', 'tag_romantic', 'tag_study', 'tag_korean', 'tag_work', 'tag_electronic',
  'tag_cantonese', 'tag_dance', 'tag_sad', 'tag_game', 'tag_afternoon_tea', 'tag_healing',
  'tag_rap', 'tag_light_music',
] as const

// Tag key -> 网易云 API 的 cat 参数值
const TAG_TO_CAT: Record<string, string> = {
  tag_all: '全部', tag_pop: '流行', tag_soundtrack: '影视原声', tag_chinese: '华语',
  tag_nostalgia: '怀旧', tag_rock: '摇滚', tag_acg: 'ACG', tag_western: '欧美',
  tag_fresh: '清新', tag_night: '夜晚', tag_children: '儿童', tag_folk: '民谣',
  tag_japanese: '日语', tag_romantic: '浪漫', tag_study: '学习', tag_korean: '韩语',
  tag_work: '工作', tag_electronic: '电子', tag_cantonese: '粤语', tag_dance: '舞曲',
  tag_sad: '伤感', tag_game: '游戏', tag_afternoon_tea: '下午茶', tag_healing: '治愈',
  tag_rap: '说唱', tag_light_music: '轻音乐',
}

const DEFAULT_TAG_COUNT = 26  // 桌面端屏幕足够大，默认显示全部
const tagsExpanded = ref(true)
const selectedTag = ref('tag_all')
const visibleTags = computed(() =>
  tagsExpanded.value ? TAG_KEYS : TAG_KEYS.slice(0, DEFAULT_TAG_COUNT)
)

// 精品歌单（按 Tag）
const qualityPlaylists = ref<PlaylistInfo[]>([])
const isLoadingPlaylists = ref(false)

interface SearchResult {
  id: string
  title: string
  artist: string
  album: string
  duration_ms: number
  source: string
  cover_url: string | null
}

interface DiscoveryShelf {
  key: string
  /** i18n key 前缀，渲染时取 `${labelKey}_title` / `${labelKey}_subtitle`，跟随语言切换 */
  labelKey: string
  platform: PlatformTab
  items: SearchResult[]
}

const biliDiscoveryShelves = ref<DiscoveryShelf[]>([])
const youtubeDiscoveryShelves = ref<DiscoveryShelf[]>([])
const isLoadingBiliDiscovery = ref(false)
const isLoadingYoutubeDiscovery = ref(false)

// query 是平台侧的搜索词，不随界面语言变化
const biliDiscoveryQueries = [
  { key: 'bili-hot', labelKey: 'explore.bili_shelf_live', query: '音乐现场' },
  { key: 'bili-vocal', labelKey: 'explore.bili_shelf_cover', query: '翻唱 音乐' },
  { key: 'bili-acg', labelKey: 'explore.bili_shelf_acg', query: 'ACG 音乐' },
]

const youtubeDiscoveryQueries = [
  { key: 'yt-new', labelKey: 'explore.yt_shelf_new', query: 'new music mix' },
  { key: 'yt-live', labelKey: 'explore.yt_shelf_live', query: 'live session music' },
  { key: 'yt-focus', labelKey: 'explore.yt_shelf_focus', query: 'focus chill music' },
]

async function loadDiscoveryShelves(platform: 'bilibili' | 'youtube') {
  const queries = platform === 'bilibili' ? biliDiscoveryQueries : youtubeDiscoveryQueries
  const loading = platform === 'bilibili' ? isLoadingBiliDiscovery : isLoadingYoutubeDiscovery
  const target = platform === 'bilibili' ? biliDiscoveryShelves : youtubeDiscoveryShelves
  if (target.value.length > 0 || loading.value) return

  loading.value = true
  try {
    const results = await Promise.all(queries.map(async q => {
      try {
        const items = await invoke<SearchResult[]>('search', { query: q.query, platform })
        return { key: q.key, labelKey: q.labelKey, platform, items: items.slice(0, 10) }
      } catch {
        return { key: q.key, labelKey: q.labelKey, platform, items: [] }
      }
    }))
    target.value = results.filter(shelf => shelf.items.length > 0)
  } finally {
    loading.value = false
  }
}

async function loadYoutubeHomeFeedIfAvailable() {
  if (!auth.youtube.loggedIn || recommend.homeFeedShelves.length > 0) return
  await recommend.fetchHomeFeed()
}

// 登录态是启动后异步拉回来的：只在挂载/切 tab 时判一次，
// 若挂载时正停在 YouTube tab 且登录态尚未到位就会永远空着
watch(
  () => auth.youtube.loggedIn,
  (loggedIn) => {
    if (!loggedIn || activeTab.value !== 'youtube') return
    void loadDiscoveryShelves('youtube')
    void loadYoutubeHomeFeedIfAvailable()
  },
)

// 连续点标签时只采纳最后一次请求，避免慢响应把旧标签的歌单盖回来
let tagRequestSeq = 0
async function loadQualityByTag(tagKey: string) {
  const requestId = ++tagRequestSeq
  selectedTag.value = tagKey
  isLoadingPlaylists.value = true
  try {
    const cat = TAG_TO_CAT[tagKey] || '全部'
    const playlists = await recommend.fetchHighQualityPlaylists(cat, 30)
    if (requestId === tagRequestSeq) qualityPlaylists.value = playlists
  } finally {
    if (requestId === tagRequestSeq) isLoadingPlaylists.value = false
  }
}

// 搜索逻辑
const isSearching = computed(() => !!searchQuery.value.trim())

let searchTimer: ReturnType<typeof setTimeout> | null = null
function runSearch() {
  void searchStore.search(activeTab.value, activeKind.value, searchQuery.value)
}
watch(searchQuery, (q) => {
  if (searchTimer) clearTimeout(searchTimer)
  if (!q.trim()) { searchStore.clear(); return }
  // 粘贴链接不需要等用户停手
  searchTimer = setTimeout(runSearch, activeTab.value === 'link' ? 0 : 300)
})
watch(activeKind, () => {
  if (searchQuery.value.trim()) runSearch()
})

// 页面被 KeepAlive 缓存：离开探索页时 route.query 属于别的页面，不能据此清空搜索
watch(() => route.query.q, (q) => {
  if (route.name !== 'explore') return
  const value = typeof q === 'string' ? q.trim() : ''
  if (searchQuery.value !== value) searchQuery.value = value
}, { immediate: true })

function retrySearch() {
  if (searchQuery.value.trim()) runSearch()
}

async function pasteLink() {
  try {
    const text = (await readText())?.trim()
    if (text) searchQuery.value = text
  } catch {
    // 剪贴板里不是文本时什么都不做
  }
}

// 列表滚到底自动加载下一页（网易云、B 站；YouTube 后端一次取满）
const loadMoreSentinel = ref<HTMLElement | null>(null)
let loadMoreObserver: IntersectionObserver | null = null
watch(loadMoreSentinel, (element) => {
  loadMoreObserver?.disconnect()
  if (!element) return
  loadMoreObserver = new IntersectionObserver((entries) => {
    if (entries.some(entry => entry.isIntersecting)) {
      void searchStore.loadMore(activeTab.value, activeKind.value, searchQuery.value)
    }
  }, { rootMargin: '400px 0px' })
  loadMoreObserver.observe(element)
})
onBeforeUnmount(() => loadMoreObserver?.disconnect())

const songResults = computed(() => searchStore.items.filter((item): item is ExploreSong => item.kind === 'song'))

function songToTrack(song: ExploreSong): TrackInfo {
  return {
    id: song.id,
    title: song.title,
    artist: song.artist,
    album: song.album || '',
    durationMs: song.duration_ms,
    coverUrl: song.cover_url || '',
    audioUrl: '',
  } as TrackInfo
}

// 搜索只播放选中的版本，避免其余翻唱结果自动进入队列
function openResult(item: ExploreItem) {
  if (item.kind === 'song') {
    rememberSearch()
    const track = songToTrack(item)
    void biliParts.request(track, () => player.playAll([track], track.id))
    return
  }
  if (item.kind === 'notice') return
  rememberSearch()
  openCollection(item)
}

function collectionRoute(item: ExploreCollection): RouteLocationRaw {
  if (item.kind === 'playlist') {
    if (item.platform === 'netease') return { name: 'netease-playlist', params: { id: item.id } }
    if (item.platform === 'bilibili') return { name: 'bili-playlist', params: { mediaId: item.id } }
    return { name: 'youtube-playlist', params: { browseId: item.id } }
  }
  const query = { name: item.name, cover: item.cover_url || '' }
  if (item.platform === 'netease') return { name: 'netease-artist', params: { id: item.id }, query }
  if (item.platform === 'bilibili') return { name: 'bili-artist', params: { mid: item.id }, query }
  return { name: 'youtube-artist', params: { browseId: item.id }, query: { ...query, subtitle: item.subtitle } }
}

function openCollection(item: ExploreCollection) {
  void router.push(collectionRoute(item))
}

function collectionMeta(item: ExploreCollection): string {
  if (item.kind === 'playlist') {
    const count = item.track_count ? t('library.track_count', { count: item.track_count }) : ''
    return [count, item.subtitle].filter(Boolean).join(' · ')
  }
  // 网易云歌手副标题是「单曲数|专辑数」
  const [songs, albums] = item.subtitle.split('|')
  if (item.platform === 'netease' && albums !== undefined) {
    return t('explore.artist_meta', { songs: Number(songs) || 0, albums: Number(albums) || 0 })
  }
  return item.subtitle
}

function resultKey(item: ExploreItem, index: number): string {
  if (item.kind === 'song') return `song:${item.id}`
  if (item.kind === 'notice') return `notice:${index}`
  return `${item.kind}:${item.platform}:${item.id}`
}

// 边输入边搜，只在确认时（回车、播放结果、点历史）记历史，免得记下半截关键词
function rememberSearch() {
  if (activeTab.value === 'link') return
  searchHistory.record(searchQuery.value)
}

function searchFromHistory(keyword: string) {
  searchQuery.value = keyword
  searchHistory.record(keyword)
}


watch(() => route.query.platform, (platform) => {
  if (route.name !== 'explore') return
  if (typeof platform === 'string' && PLATFORM_KEYS.includes(platform as PlatformTab)) {
    activeTab.value = platform as PlatformTab
  }
})

// 播放页「找不到这位创作者」时带 kind=creators/artists 跳来，直接落在对应分类
watch(() => route.query.kind, (kind) => {
  if (route.name !== 'explore' || typeof kind !== 'string' || activeTab.value === 'link') return
  if (EXPLORE_KINDS[activeTab.value].includes(kind as ExploreKind)) selectKind(kind as ExploreKind)
}, { immediate: true })

// 切换平台 Tab 时，如果有搜索关键词则重新搜索
watch(activeTab, (tab) => {
  if (searchQuery.value.trim()) {
    runSearch()
    return
  }
  if (tab === 'bilibili') void loadDiscoveryShelves('bilibili')
  if (tab === 'youtube') {
    void loadDiscoveryShelves('youtube')
    void loadYoutubeHomeFeedIfAvailable()
  }
})

// 工具函数
function discoveryTrack(r: SearchResult): TrackInfo {
  return {
    id: r.id,
    title: r.title,
    artist: r.artist,
    album: r.album || '',
    durationMs: r.duration_ms,
    coverUrl: r.cover_url || '',
    audioUrl: '',
  }
}

function playResult(r: SearchResult) {
  rememberSearch()
  const track = discoveryTrack(r)
  void biliParts.request(track, () => player.play(track))
}

const trackMenuRef = ref<InstanceType<typeof TrackContextMenu> | null>(null)

const collectionMenuRef = ref<InstanceType<typeof CollectionContextMenu> | null>(null)

function openResultMenu(event: MouseEvent, item: ExploreItem) {
  if (item.kind === 'song') trackMenuRef.value?.open(event, songToTrack(item))
  else if (item.kind !== 'notice') {
    collectionMenuRef.value?.open(event, {
      route: collectionRoute(item),
      webUrl: collectionWebUrl(item.platform, item.kind, item.id),
    })
  }
}

// 菜单播放沿用对应入口的队列和搜索历史行为
function playFromMenu(track: TrackInfo) {
  const song = songResults.value.find(item => item.id === track.id)
  if (song) openResult(song)
  else void biliParts.request(track, () => player.play(track))
}

function goToPlaylist(pl: PlaylistInfo) {
  router.push({ name: 'netease-playlist', params: { id: pl.id } })
}

function playDiscoveryItem(item: SearchResult) {
  playResult(item)
}

function platformLabel(source?: string) {
  switch ((source || '').toLowerCase()) {
    case 'netease': return t('player.source_netease')
    case 'qq': return t('player.source_qq')
    case 'bilibili': return t('player.source_bilibili')
    case 'youtube': return t('player.source_youtube')
    case 'local': return t('player.source_local')
    default: return source || ''
  }
}

function youtubeShelfTrack(item: HomeFeedItem): TrackInfo {
  return {
    id: `youtube:${item.videoId}`,
    title: item.title,
    artist: item.subtitle || 'YouTube Music',
    album: '',
    durationMs: item.durationMs || 0,
    coverUrl: item.coverUrl || '',
    audioUrl: '',
  }
}

function youtubeShelfTarget(item: HomeFeedItem): CollectionMenuTarget | null {
  if (!item.browseId) return null
  const isArtist = /ARTIST|USER_CHANNEL/.test(item.pageType?.toUpperCase() || '') || item.browseId.startsWith('UC')
  return {
    route: isArtist
      ? { name: 'youtube-artist', params: { browseId: item.browseId }, query: { name: item.title, cover: item.coverUrl, subtitle: item.subtitle } }
      : { name: 'youtube-playlist', params: { browseId: item.browseId } },
    webUrl: collectionWebUrl('youtube', isArtist ? 'artist' : item.browseId.startsWith('MPRE') ? 'album' : 'playlist', item.browseId),
  }
}

function openYoutubeShelfMenu(event: MouseEvent, item: HomeFeedItem) {
  const target = youtubeShelfTarget(item)
  if (target) collectionMenuRef.value?.open(event, target)
  else if (item.videoId) trackMenuRef.value?.open(event, youtubeShelfTrack(item))
}

function goToYoutubeShelfItem(item: HomeFeedItem) {
  const target = youtubeShelfTarget(item)
  if (target) {
    void router.push(target.route)
    return
  }
  if (item.videoId) {
    player.play(youtubeShelfTrack(item))
  }
}

// 初始化
onMounted(() => {
  // 首次加载网易云精品歌单
  if (qualityPlaylists.value.length === 0) {
    loadQualityByTag('tag_all')
  }
  if (activeTab.value === 'bilibili') void loadDiscoveryShelves('bilibili')
  if (activeTab.value === 'youtube') {
    void loadDiscoveryShelves('youtube')
    void loadYoutubeHomeFeedIfAvailable()
  }
})
</script>

<template>
  <div class="explore-view">
    <h1 class="page-title">{{ t('explore.title') }}</h1>

    <!-- 搜索栏 -->
    <div class="search-bar" :class="{ focused: isFocused }">
      <span class="material-symbols-rounded search-icon">search</span>
      <input
        v-model="searchQuery"
        type="text"
        :placeholder="searchPlaceholder"
        data-shortcut-search
        @focus="isFocused = true"
        @blur="isFocused = false"
        @keydown.enter="rememberSearch"
      />
      <button v-if="searchQuery" class="clear-btn" @click="searchQuery = ''; searchStore.clear()">
        <span class="material-symbols-rounded" style="font-size: 20px">close</span>
      </button>
    </div>

    <!-- 三平台 Tab -->
    <div class="platform-tabs">
      <button
        v-for="tab in platformTabs"
        :key="tab.key"
        class="platform-tab"
        :class="{ active: activeTab === tab.key }"
        @click="activeTab = tab.key"
      >
        <span v-if="tab.icon" class="tab-icon" :style="{ maskImage: `url(${tab.icon})` }"></span>
        <span v-else class="material-symbols-rounded tab-symbol">{{ tab.symbol }}</span>
        <span class="tab-label">{{ tab.label }}</span>
      </button>
    </div>

    <!-- 分类：网易云 歌曲/歌单/歌手，YouTube Music 歌曲/视频/创作者 -->
    <div v-if="kindOptions.length > 1" class="kind-chips" role="tablist">
      <button
        v-for="kind in kindOptions"
        :key="kind"
        class="kind-chip"
        role="tab"
        :aria-selected="activeKind === kind"
        :class="{ active: activeKind === kind }"
        @click="selectKind(kind)"
      >
        <span class="material-symbols-rounded">{{ KIND_ICONS[kind] }}</span>
        {{ t(`explore.kind_${kind}`) }}
      </button>
    </div>

    <div v-if="!isSearching && activeTab !== 'link' && searchHistory.visible.length > 0" class="search-history">
      <div class="search-history-header">
        <span class="search-history-title">{{ t('explore.search_history') }}</span>
        <button class="search-history-clear" @click="searchHistory.clear()">{{ t('explore.clear_search_history') }}</button>
      </div>
      <div class="tag-flow">
        <button v-for="keyword in searchHistory.visible" :key="keyword" class="tag-chip" @click="searchFromHistory(keyword)">
          {{ keyword }}
        </button>
      </div>
    </div>

    <!-- 加载状态 -->
    <div v-if="searchStore.isSearching" class="loading-state">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <!-- 搜索结果 -->
    <div v-else-if="isSearching && searchStore.items.length > 0" class="search-results">
      <template v-for="(item, index) in searchStore.items" :key="resultKey(item, index)">
        <div v-if="item.kind === 'notice'" class="link-notice" role="status">
          <span class="material-symbols-rounded">{{ item.reason === 'no_link' ? 'link_off' : 'info' }}</span>
          <div>
            <strong>{{ t(`explore.link_${item.reason}_title`) }}</strong>
            <p>{{ t(`explore.link_${item.reason}_desc`) }}</p>
          </div>
        </div>
        <div
          v-else
          class="result-item"
          :class="{ playing: item.kind === 'song' && player.currentTrack?.id === item.id }"
          role="button"
          tabindex="0"
          @click="openResult(item)"
          @keydown.enter="openResult(item)"
          @contextmenu="openResultMenu($event, item)"
        >
          <div class="result-cover" :class="{ round: item.kind === 'artist' }">
            <BilibiliCoverImage v-if="item.cover_url" :src="item.cover_url" loading="lazy">
              <span class="material-symbols-rounded filled">{{ item.kind === 'song' ? 'music_note' : item.kind === 'artist' ? 'person' : 'queue_music' }}</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled">{{ item.kind === 'song' ? 'music_note' : item.kind === 'artist' ? 'person' : 'queue_music' }}</span>
          </div>
          <div class="result-info">
            <template v-if="item.kind === 'song'">
              <div class="result-title">{{ item.title }}</div>
              <div class="result-meta">{{ item.artist }}<span v-if="item.album && !item.album.startsWith('Bilibili|')"> · {{ item.album }}</span></div>
            </template>
            <template v-else>
              <div class="result-title">{{ item.name || t(item.kind === 'artist' ? 'explore.kind_artists' : 'explore.kind_playlists') }}</div>
              <div class="result-meta">{{ collectionMeta(item) }}</div>
            </template>
          </div>
          <template v-if="item.kind === 'song'">
            <div v-if="activeTab === 'link'" class="result-source">{{ platformLabel(item.source) }}</div>
            <div class="result-duration">{{ item.duration_ms ? formatDuration(item.duration_ms) : '' }}</div>
          </template>
          <span v-else class="material-symbols-rounded result-chevron">chevron_right</span>
        </div>
      </template>
      <div v-if="searchStore.hasMore" ref="loadMoreSentinel" class="load-more-state">
        <span v-if="searchStore.isLoadingMore" class="material-symbols-rounded spinning small">progress_activity</span>
      </div>
    </div>

    <!-- 搜索失败：与无结果区分开，并允许重试 -->
    <div v-else-if="isSearching && searchStore.error" class="empty-state" style="padding: 40px 0">
      <span class="material-symbols-rounded" style="font-size: 32px; opacity: 0.4">wifi_off</span>
      <p class="empty-desc" style="margin-top: 8px">{{ t('explore.search_failed') }}</p>
      <button class="retry-btn" @click="retrySearch">{{ t('player.retry') }}</button>
    </div>

    <!-- 搜索无结果 -->
    <div v-else-if="isSearching && searchStore.items.length === 0" class="empty-state" style="padding: 40px 0">
      <span class="material-symbols-rounded" style="font-size: 32px; opacity: 0.4">search_off</span>
      <p class="empty-desc" style="margin-top: 8px">{{ t('player.no_results') }}</p>
    </div>

    <!-- 默认内容（按平台） -->
    <template v-else>

      <!-- 网易云 Tab：Tag 选择 + 精品歌单 -->
      <template v-if="activeTab === 'netease'">
        <div class="tag-section">
          <div class="tag-flow">
            <button
              v-for="tagKey in visibleTags"
              :key="tagKey"
              class="tag-chip"
              :class="{ active: selectedTag === tagKey }"
              @click="loadQualityByTag(tagKey)"
            >
              {{ t(`explore.${tagKey}`) }}
            </button>
          </div>
        </div>

        <!-- 精品歌单网格 -->
        <div v-if="isLoadingPlaylists" class="loading-state">
          <span class="material-symbols-rounded spinning">progress_activity</span>
        </div>
        <div v-else-if="qualityPlaylists.length > 0" class="playlist-grid">
          <div
            v-for="pl in qualityPlaylists"
            :key="pl.id"
            class="playlist-card"
            @click="goToPlaylist(pl)"
            @contextmenu="collectionMenuRef?.open($event, { route: { name: 'netease-playlist', params: { id: pl.id } }, webUrl: collectionWebUrl('netease', 'playlist', pl.id) })"
          >
            <div class="playlist-cover">
              <BilibiliCoverImage v-if="pl.coverUrl" :src="pl.coverUrl" loading="lazy">
                <span class="material-symbols-rounded filled">queue_music</span>
              </BilibiliCoverImage>
              <span v-else class="material-symbols-rounded filled">queue_music</span>
            </div>
            <div class="playlist-name">{{ pl.name }}</div>
            <div v-if="pl.trackCount" class="playlist-count">{{ t('library.track_count', { count: pl.trackCount }) }}</div>
          </div>
        </div>
      </template>

      <!-- 链接 Tab：粘贴分享链接识别 -->
      <template v-else-if="activeTab === 'link'">
        <div class="platform-hero link">
          <span class="material-symbols-rounded hero-symbol">link</span>
          <div>
            <h2>{{ t('explore.link_title') }}</h2>
            <p>{{ t('explore.link_hint') }}</p>
          </div>
          <button class="hero-action" @click="pasteLink">
            <span class="material-symbols-rounded">content_paste</span>{{ t('explore.link_paste') }}
          </button>
        </div>
        <div class="link-examples">
          <div v-for="example in ['netease', 'bilibili', 'youtube']" :key="example" class="link-example">
            <span class="tab-icon" :style="{ maskImage: `url(/icons/ic_${example}.svg)` }"></span>
            <div>
              <strong>{{ t(`settings.${example}_account`) }}</strong>
              <p>{{ t(`explore.link_example_${example}`) }}</p>
            </div>
          </div>
        </div>
      </template>

      <!-- B站 Tab：默认发现内容 -->
      <template v-else-if="activeTab === 'bilibili'">
        <div class="platform-hero bilibili">
          <span class="tab-icon large" :style="{ maskImage: 'url(/icons/ic_bilibili.svg)' }"></span>
          <div>
            <h2>{{ t('settings.bilibili_account') }}</h2>
            <p>{{ t('explore.bili_hint') }} · {{ t('explore.bili_discovery_hint') }}</p>
          </div>
        </div>
        <div v-if="isLoadingBiliDiscovery" class="loading-state">
          <span class="material-symbols-rounded spinning">progress_activity</span>
        </div>
        <div v-else-if="biliDiscoveryShelves.length > 0" class="discovery-stack">
          <section v-for="shelf in biliDiscoveryShelves" :key="shelf.key" class="discovery-shelf">
            <div class="discovery-header">
              <div>
                <h2 class="section-title">{{ t(`${shelf.labelKey}_title`) }}</h2>
                <p>{{ t(`${shelf.labelKey}_subtitle`) }}</p>
              </div>
            </div>
            <div class="discovery-row">
              <div v-for="item in shelf.items" :key="item.id" class="discovery-card video" @click="playDiscoveryItem(item)" @contextmenu="trackMenuRef?.open($event, discoveryTrack(item))">
                <div class="discovery-cover wide">
                  <BilibiliCoverImage v-if="item.cover_url" :src="item.cover_url" loading="lazy">
                    <span class="material-symbols-rounded filled">movie</span>
                  </BilibiliCoverImage>
                  <span v-else class="material-symbols-rounded filled">movie</span>
                </div>
                <div class="discovery-title">{{ item.title }}</div>
                <div class="discovery-meta">{{ item.artist }}</div>
              </div>
            </div>
          </section>
        </div>
      </template>

      <!-- YouTube Tab：默认发现内容 -->
      <template v-else-if="activeTab === 'youtube'">
        <div class="platform-hero youtube">
          <span class="tab-icon large" :style="{ maskImage: 'url(/icons/ic_youtube.svg)' }"></span>
          <div>
            <h2>{{ t('settings.youtube_account') }}</h2>
            <p>{{ auth.youtube.loggedIn ? t('explore.yt_feed_hint') : t('explore.yt_hint') }}</p>
          </div>
        </div>

        <div v-if="auth.youtube.loggedIn && recommend.homeFeedShelves.length > 0" class="discovery-stack">
          <section v-for="shelf in recommend.homeFeedShelves.slice(0, 4)" :key="shelf.title" class="discovery-shelf">
            <div class="discovery-header">
              <h2 class="section-title">{{ shelf.title }}</h2>
            </div>
            <div class="discovery-row">
              <div v-for="item in shelf.items.slice(0, 10)" :key="item.browseId || item.videoId || item.title" class="discovery-card" @click="goToYoutubeShelfItem(item)" @contextmenu="openYoutubeShelfMenu($event, item)">
                <div class="discovery-cover">
                  <BilibiliCoverImage v-if="item.coverUrl" :src="item.coverUrl" loading="lazy">
                    <span class="material-symbols-rounded filled">music_note</span>
                  </BilibiliCoverImage>
                  <span v-else class="material-symbols-rounded filled">music_note</span>
                </div>
                <div class="discovery-title">{{ item.title }}</div>
                <div class="discovery-meta">{{ item.subtitle }}</div>
              </div>
            </div>
          </section>
        </div>

        <div v-if="isLoadingYoutubeDiscovery" class="loading-state">
          <span class="material-symbols-rounded spinning">progress_activity</span>
        </div>
        <div v-else-if="youtubeDiscoveryShelves.length > 0" class="discovery-stack">
          <section v-for="shelf in youtubeDiscoveryShelves" :key="shelf.key" class="discovery-shelf">
            <div class="discovery-header">
              <div>
                <h2 class="section-title">{{ t(`${shelf.labelKey}_title`) }}</h2>
                <p>{{ t(`${shelf.labelKey}_subtitle`) }}</p>
              </div>
            </div>
            <div class="discovery-row">
              <div v-for="item in shelf.items" :key="item.id" class="discovery-card" @click="playDiscoveryItem(item)" @contextmenu="trackMenuRef?.open($event, discoveryTrack(item))">
                <div class="discovery-cover">
                  <BilibiliCoverImage v-if="item.cover_url" :src="item.cover_url" loading="lazy">
                    <span class="material-symbols-rounded filled">music_note</span>
                  </BilibiliCoverImage>
                  <span v-else class="material-symbols-rounded filled">music_note</span>
                </div>
                <div class="discovery-title">{{ item.title }}</div>
                <div class="discovery-meta">{{ item.artist }}</div>
              </div>
            </div>
          </section>
        </div>
      </template>
    </template>
    <TrackContextMenu ref="trackMenuRef" @play="playFromMenu" />
    <CollectionContextMenu ref="collectionMenuRef" />
  </div>
</template>

<style scoped lang="scss">
.explore-view { padding: 20px 28px 32px; }

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
  margin-bottom: 20px;
}

/* 搜索栏 */
.search-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 16px;
  height: 48px;
  background: var(--md-surface-container-high);
  border-radius: var(--radius-xl);
  border: 2px solid transparent;
  transition: background var(--duration-short), border-color var(--duration-short), box-shadow var(--duration-medium);
  margin-bottom: 16px;

  &.focused {
    background: var(--md-surface-container-highest);
    border-color: var(--md-primary);
    box-shadow: 0 0 0 4px rgba(208, 188, 255, 0.08);
  }

  .search-icon {
    color: var(--md-on-surface-variant);
    font-size: 22px;
    flex-shrink: 0;
  }

  input {
    flex: 1;
    border: none;
    background: none;
    color: var(--md-on-surface);
    font-size: 14px;
    outline: none;
    font-family: inherit;

    &::placeholder { color: var(--md-on-surface-variant); }
  }
}

.clear-btn {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-variant); }
}

/* 三平台 Tab */
.platform-tabs {
  display: flex;
  gap: 0;
  margin-bottom: 20px;
  border-bottom: 1px solid var(--md-outline-variant);
}

.platform-tab {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 0;
  font-size: 13px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  border-bottom: 2px solid transparent;
  border-radius: var(--radius-md, 12px) var(--radius-md, 12px) 0 0;
  transition: color var(--duration-short), border-color var(--duration-short);
  cursor: pointer;
  user-select: none;
  position: relative;

  &:hover {
    color: var(--md-on-surface);
    background: var(--md-surface-container);
  }

  &.active {
    color: var(--md-primary);
    border-bottom-color: var(--md-primary);

    .tab-icon {
      background: var(--md-primary);
    }
  }
}

.tab-icon {
  display: block;
  width: 20px;
  height: 20px;
  background: var(--md-on-surface-variant);
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
  -webkit-mask-size: contain;
  -webkit-mask-repeat: no-repeat;
  -webkit-mask-position: center;
  flex-shrink: 0;
  transition: background var(--duration-short);

  &.large {
    width: 36px;
    height: 36px;
  }
}

.tab-label {
  white-space: nowrap;
}

.tab-symbol { font-size: 20px; }

/* 分类 chips */
.kind-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: -6px 0 18px;
}

.kind-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 34px;
  padding: 0 14px 0 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--md-outline-variant);
  color: var(--md-on-surface-variant);
  font-size: 13px;
  font-weight: 500;
  transition: background var(--duration-short) var(--ease-standard), color var(--duration-short) var(--ease-standard), border-color var(--duration-short) var(--ease-standard);

  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: var(--md-surface-container-high); }
  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    border-color: transparent;
  }
}

/* 链接识别 */
.hero-symbol {
  font-size: 34px;
  color: var(--platform-color);
}

.hero-action {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  height: 40px;
  padding: 0 18px 0 14px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  font-size: 13px;
  font-weight: 600;

  .material-symbols-rounded { font-size: 18px; }
  &:hover { opacity: 0.92; }
}

.link-examples {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 12px;
}

.link-example {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);

  .tab-icon { margin-top: 2px; background: var(--md-primary); }
  strong { font-size: 13px; }
  p { margin-top: 4px; font-size: 12px; color: var(--md-on-surface-variant); line-height: 1.5; overflow-wrap: anywhere; }
}

.link-notice {
  display: flex;
  gap: 14px;
  align-items: flex-start;
  padding: 16px 18px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);

  > .material-symbols-rounded { color: var(--md-primary); }
  strong { font-size: 14px; }
  p { margin-top: 4px; font-size: 13px; color: var(--md-on-surface-variant); }
}

/* Tag 选择区 */
.tag-section {
  margin-bottom: 20px;
}

.search-history {
  margin-bottom: 20px;
}

.search-history-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}

.search-history-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
}

.search-history-clear {
  border: none;
  background: none;
  padding: 4px 8px;
  border-radius: 8px;
  font-size: 12px;
  color: var(--md-primary);
  cursor: pointer;

  &:hover {
    background: var(--md-surface-container-high);
  }
}

.tag-flow {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 8px;
}

.tag-chip {
  height: 32px;
  padding: 0 14px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface);
  color: var(--md-on-surface);
  cursor: pointer;
  transition: background var(--duration-short), border-color var(--duration-short), color var(--duration-short);
  user-select: none;

  &:hover {
    background: var(--md-surface-container-high);
    border-color: var(--md-outline);
  }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    border-color: var(--md-secondary);
  }
}

.expand-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--md-primary);
  padding: 4px 8px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--duration-short);

  &:hover {
    background: color-mix(in srgb, var(--md-primary) 8%, transparent);
  }
}

/* 歌单网格 */
.playlist-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 14px;
}

.playlist-card {
  cursor: pointer;
  border-radius: var(--radius-md);
  overflow: hidden;
  transition: transform var(--duration-short) var(--ease-standard);

  &:hover { transform: translateY(-2px); }
}

.playlist-cover {
  aspect-ratio: 1;
  border-radius: var(--radius-md);
  background: var(--md-surface-variant);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .material-symbols-rounded {
    font-size: 32px;
    opacity: 0.4;
  }
}

.playlist-name {
  font-size: 12px;
  font-weight: 500;
  margin-top: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.playlist-count {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  margin-top: 2px;
}


/* 多平台默认发现 */
.platform-hero {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 18px 20px;
  margin-bottom: 18px;
  border-radius: 24px;
  background:
    radial-gradient(circle at 14% 18%, color-mix(in srgb, var(--platform-color) 24%, transparent), transparent 36%),
    var(--md-surface-container);
  border: 1px solid color-mix(in srgb, var(--platform-color) 26%, var(--md-outline-variant));

  &.bilibili { --platform-color: #00a1d6; }
  &.youtube { --platform-color: #ff0033; }
  &.link { --platform-color: var(--md-primary); }

  .tab-icon { background: var(--platform-color); }

  h2 {
    font-size: 18px;
    font-weight: 800;
    margin-bottom: 3px;
  }

  p {
    font-size: 13px;
    color: var(--md-on-surface-variant);
  }
}

.discovery-stack {
  display: flex;
  flex-direction: column;
  gap: 24px;
}

.discovery-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  margin-bottom: 10px;

  p {
    margin-top: 2px;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }
}

.discovery-row {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
  gap: 14px 12px;
}

.discovery-card {
  min-width: 0;
  cursor: pointer;
  border-radius: 14px;
  transition: transform var(--duration-short) var(--ease-standard);

  &:hover { transform: translateY(-2px); }
}

.discovery-cover {
  aspect-ratio: 1;
  border-radius: 14px;
  overflow: hidden;
  background: var(--md-surface-variant);
  display: flex;
  align-items: center;
  justify-content: center;

  &.wide { aspect-ratio: 16 / 9; }

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .material-symbols-rounded { opacity: 0.45; }
}

.discovery-title {
  margin-top: 7px;
  font-size: 12px;
  font-weight: 700;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.discovery-meta {
  margin-top: 2px;
  font-size: 11px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 空状态 */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 60px 0;
}

.empty-circle {
  width: 80px;
  height: 80px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  margin-bottom: 20px;
  opacity: 0.6;
}

.empty-title {
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 6px;
  color: var(--md-on-surface-variant);
}

.empty-desc {
  font-size: 13px;
  color: var(--md-on-surface-variant);
  opacity: 0.6;
}

.retry-btn {
  margin-top: 12px;
  padding: 6px 18px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--md-primary);
  font-size: 13px;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard);

  &:hover { background: color-mix(in srgb, var(--md-primary) 8%, transparent); }
}

/* 加载 & 搜索结果 */
.loading-state {
  display: flex;
  justify-content: center;
  padding: 40px 0;
  color: var(--md-on-surface-variant);
}

.spinning {
  font-size: 32px;
  animation: spin 1s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

.search-results {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.result-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container); }
  &:active { transform: scale(0.99); }
}

.result-cover {
  width: 44px;
  height: 44px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-container-high);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  overflow: hidden;
  color: var(--md-on-surface-variant);

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
}

.result-info {
  flex: 1;
  min-width: 0;
}

.result-title {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-meta {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-source {
  font-size: 11px;
  font-weight: 500;
  color: var(--md-primary);
  padding: 2px 8px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-primary) 10%, transparent);
  text-transform: capitalize;
  flex-shrink: 0;
}

.result-duration {
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.result-item.playing .result-title { color: var(--md-primary); }
.result-cover.round { border-radius: 50%; }
.result-chevron { font-size: 20px; opacity: 0.35; flex-shrink: 0; }

.load-more-state {
  display: flex;
  justify-content: center;
  min-height: 48px;
  padding: 12px 0;
  color: var(--md-on-surface-variant);
}

.spinning.small { font-size: 22px; }
</style>
