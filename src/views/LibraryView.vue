<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, onDeactivated, nextTick, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'

defineOptions({ name: 'LibraryView' })
import { useI18n } from 'vue-i18n'
import LocalFilesView from '@/views/LocalFilesView.vue'
import { normalizeTrack, usePlayerStore, type TrackInfo } from '@/stores/player'
import { useRecommendStore, type CloudListStatus } from '@/stores/recommend'
import { AUTH_CHANGED_EVENT, useAuthStore } from '@/stores/auth'
import CloudListState from '@/components/CloudListState.vue'
import DownloadsView from '@/views/DownloadsView.vue'
import { useToastStore } from '@/stores/toast'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import {
  filterLocalArtists,
  groupLocalArtists,
  loadArtistSourceTracks,
  sortLocalArtists,
  type LocalArtistSortMode,
  type LocalArtistSummary,
} from '@/modules/library/localArtists'
import { createLogger } from '@/utils/logger'
import {
  isEmptyLocalFilesPlaylist,
  isFavoritesPlaylist,
  isLocalFilesPlaylist,
  isSystemPlaylist,
  LEGACY_PLAYLIST_ORDER_KEY,
  localPlaylistDisplayName,
  playlistOrderIds,
  readLegacyPlaylistOrder,
  visibleSelection,
} from '@/modules/library/localPlaylists'
import { usePointerListReorder } from '@/composables/usePointerListReorder'
import {
  ARTIST_FAVORITE_SOURCES,
  favoriteArtistRoute,
  favoriteKey,
  filterFavoriteArtists,
  isArtistFavoriteSource,
  parseFavoritePlaylists,
  type ArtistFavoriteSource,
  type FavoritePlaylist,
} from '@/modules/library/favoriteArtists'
import { biliPlaylistRoute, parseBiliPlaylistReference } from '@/modules/library/biliPlaylistReference'

const log = createLogger('library-view')

const router = useRouter()
const route = useRoute()
const { t } = useI18n()
const player = usePlayerStore()
const recommend = useRecommendStore()
const auth = useAuthStore()
const toast = useToastStore()

// 喜欢的歌曲计数
const likedCount = computed(() => recommend.likedSongIds.size)

const tabs = computed(() => [
  { label: t('library.tab_local'), icon: 'folder_open', key: 'local' },
  { label: t('library.tab_favorites'), icon: 'favorite', key: 'favorites' },
  { label: t('library.tab_downloads'), icon: 'download', key: 'downloads' },
  // 网易云歌单与专辑合成一个板块，内部再分类（对齐 Android
  // LibraryTab.NETEASEALBUM.asVisibleLibraryTab() == NETEASE）
  { label: t('library.tab_netease'), icon: 'queue_music', key: 'netease' },
  { label: t('library.bilibili_favorites'), icon: 'video_library', key: 'bilibili_favorites' },
  { label: 'YouTube Music', icon: 'subscriptions', key: 'youtube_playlists' },
])
// 根据路由 query 参数设置初始标签；旧的 netease_albums / netease_playlists 仍可用
const tabKeyToIndex: Record<string, number> = {
  local: 0, favorites: 1, downloads: 2,
  netease: 3, netease_playlists: 3, netease_albums: 3,
  bilibili_favorites: 4, youtube_playlists: 5,
}
type NeteaseCategory = 'playlists' | 'albums'
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

// 真实播放列表
interface PlaylistInfo { id: number; name: string; track_count: number; modified_at: number; cover_url: string | null }
const playlists = ref<PlaylistInfo[]>([])

// 多选模式
const isMultiSelectMode = ref(false)
const selectedPlaylists = ref<Set<number>>(new Set())

function enterMultiSelect(pl?: PlaylistInfo) {
  isMultiSelectMode.value = true
  selectedPlaylists.value.clear()
  if (pl && !isProtectedPlaylist(pl)) selectedPlaylists.value.add(pl.id)
}
function exitMultiSelect() {
  cancelPlaylistDrag()
  isMultiSelectMode.value = false
  selectedPlaylists.value.clear()
}
// 多选只属于本地歌单列表：切 tab 或离开资料库（KeepAlive 停用）都要退出
watch(activeTab, () => {
  if (isMultiSelectMode.value) exitMultiSelect()
})
onDeactivated(() => {
  if (isMultiSelectMode.value) exitMultiSelect()
})
function togglePlaylistSelection(id: number) {
  // 受保护歌单（我喜欢的音乐/本地文件）不允许进入选择集，防止被批量删除
  const pl = playlists.value.find(p => p.id === id)
  if (pl && isProtectedPlaylist(pl)) return
  const set = selectedPlaylists.value
  if (set.has(id)) set.delete(id)
  else set.add(id)
}
// 全选 / 反选只作用于当前可见（搜索过滤后）的歌单，被筛掉的歌单不会被批量删除
function selectAll() {
  for (const pl of filteredPlaylists.value) {
    if (!isProtectedPlaylist(pl)) selectedPlaylists.value.add(pl.id)
  }
}
function invertSelection() {
  for (const pl of filteredPlaylists.value) {
    if (isProtectedPlaylist(pl)) continue
    if (selectedPlaylists.value.has(pl.id)) selectedPlaylists.value.delete(pl.id)
    else selectedPlaylists.value.add(pl.id)
  }
  log.info('invert selection ->', selectedPlaylists.value.size, 'selected')
}

// 拖拽排序：与歌单详情页歌曲拖拽同一套指针交互（composable 抽取自 LocalPlaylistView）
const playlistListRef = ref<HTMLElement | null>(null)

function playlistDragKey(pl: PlaylistInfo): string {
  return String(pl.id)
}

const {
  dragKey: plDragKey,
  dragOverKey: plDragOverKey,
  dragInsertPosition: plDragInsertPosition,
  dragLandingKey: plDragLandingKey,
  dragUnderGlassKeys: plDragUnderGlassKeys,
  dragGlassActive: plDragGlassActive,
  dropIndicatorVisible: plDropIndicatorVisible,
  dropIndicatorY: plDropIndicatorY,
  dropIndicatorSnap: plDropIndicatorSnap,
  dragItemStyle: plDragItemStyle,
  startDrag: startPlaylistDrag,
  cancelDrag: cancelPlaylistDrag,
} = usePointerListReorder({
  listRef: playlistListRef,
  itemSelector: '.playlist-item',
  canDrag: () => isMultiSelectMode.value,
  // 「我喜欢的音乐」「本地文件」等系统歌单固定首尾，不参与落点
  isValidTargetKey: (key) => {
    const pl = playlists.value.find(p => playlistDragKey(p) === key)
    return !!pl && !isProtectedPlaylist(pl)
  },
  onReorder: (fromKey, toKey, position) => {
    const arr = [...playlists.value]
    const fromIndex = arr.findIndex(p => playlistDragKey(p) === fromKey)
    let toIndex = arr.findIndex(p => playlistDragKey(p) === toKey)
    if (fromIndex < 0 || toIndex < 0 || fromIndex === toIndex) return

    const [moved] = arr.splice(fromIndex, 1)
    if (toIndex > fromIndex) toIndex -= 1
    const insertIndex = position === 'after' ? toIndex + 1 : toIndex
    arr.splice(insertIndex, 0, moved)

    if (arr.map(playlistDragKey).join('\n') === playlists.value.map(playlistDragKey).join('\n')) return
    playlists.value = arr
    void savePlaylistOrder(arr)
  },
})

// 自定义顺序落库并随同步跨端保留（对齐 Android reorderPlaylists）
async function savePlaylistOrder(ordered: PlaylistInfo[]) {
  const orderedIds = playlistOrderIds(ordered, isProtectedPlaylist)
  try {
    await invoke('reorder_playlists', { orderedIds })
    log.info('Playlist order saved:', orderedIds)
  } catch (e) {
    log.error('Save playlist order failed:', e)
    toast.error(t('library.playlist_order_save_failed'))
    void loadPlaylists()
  }
}

let legacyPlaylistOrderMigrated = false
async function migrateLegacyPlaylistOrder() {
  if (legacyPlaylistOrderMigrated) return
  legacyPlaylistOrderMigrated = true
  const legacyOrder = readLegacyPlaylistOrder(localStorage)
  if (legacyOrder === null) return
  try {
    if (legacyOrder.length > 0) await invoke('reorder_playlists', { orderedIds: legacyOrder })
    localStorage.removeItem(LEGACY_PLAYLIST_ORDER_KEY)
  } catch (e) {
    legacyPlaylistOrderMigrated = false
    log.warn('Migrate legacy playlist order failed:', e)
  }
}

const failedLibraryCoverKeys = ref<Set<string>>(new Set())

function isBilibiliCover(url?: string | null): boolean {
  if (!url) return false
  return /\.(hdslb|biliimg)\.com/i.test(url)
}

function toDisplayableLibraryCoverUrl(value?: string | null): string {
  if (!value) return ''
  if (/^(https?:|asset:|data:|blob:)/i.test(value)) return value
  return convertFileSrc(value)
}

function libraryCoverKey(scope: string, id: string | number, url?: string | null): string {
  return `${scope}:${id}:${url || ''}`
}

function isLibraryCoverFailed(scope: string, id: string | number, url?: string | null): boolean {
  if (!url) return false
  return failedLibraryCoverKeys.value.has(libraryCoverKey(scope, id, url))
}

function markLibraryCoverFailed(scope: string, id: string | number, url?: string | null) {
  if (!url) return
  failedLibraryCoverKeys.value = new Set(failedLibraryCoverKeys.value).add(libraryCoverKey(scope, id, url))
}

async function loadPlaylists() {
  try {
    await migrateLegacyPlaylistOrder()
    const raw = await invoke<PlaylistInfo[]>('list_playlists')
    // 排序：「我喜欢的音乐」置顶，「本地音乐」置底，其余沿用数据库中的自定义顺序
    const liked: PlaylistInfo[] = []
    const localFiles: PlaylistInfo[] = []
    const normal: PlaylistInfo[] = []
    for (const pl of raw) {
      if (isEmptyLocalFilesPlaylist(pl)) continue
      if (isFavoritesPlaylist(pl)) liked.push(pl)
      else if (isLocalFilesPlaylist(pl)) localFiles.push(pl)
      else normal.push(pl)
    }
    playlists.value = [...liked, ...normal, ...localFiles]
  } catch (e) {
    log.error('Load playlists failed:', e)
  }
}

function platformLabel(source?: string) {
  switch ((source || '').toLowerCase()) {
    case 'netease': return t('player.source_netease')
    case 'qq': return t('player.source_qq')
    case 'bilibili': return t('player.source_bilibili')
    case 'youtube': return t('player.source_youtube')
    case 'local': return t('player.source_local')
    default: return source || '—'
  }
}

/** 收藏来源明文标签 (对齐 Android favoriteSourceLabel, 不露出内部 source 标识) */
function favoriteSourceLabel(source: string): string {
  switch (source) {
    case 'netease': return t('player.source_netease')
    case 'neteaseAlbum': return `${t('player.source_netease')} · ${t('library.albums')}`
    case 'neteaseArtist': return `${t('player.source_netease')} · ${t('library.local_category_artists')}`
    case 'youtubeMusic': return 'YouTube Music'
    case 'bili': return t('player.source_bilibili')
    case 'qq': return t('player.source_qq')
    default: return platformLabel(source)
  }
}

// M3 Dialog 创建播放列表
const showCreateDialog = ref(false)
const newPlaylistName = ref('')
const inputRef = ref<InstanceType<typeof M3Input>>()

function openCreateDialog() {
  newPlaylistName.value = ''
  showCreateDialog.value = true
  nextTick(() => inputRef.value?.focus())
}

async function confirmCreate() {
  if (!newPlaylistName.value.trim()) return
  try {
    await invoke('create_playlist', { name: newPlaylistName.value.trim() })
    showCreateDialog.value = false
    await loadPlaylists()
  } catch (e) {
    log.error('Create playlist failed:', e)
    toast.error(t('library.create_playlist_failed'))
  }
}

// 上下文菜单
const contextMenu = ref<{ show: boolean; x: number; y: number; playlist: PlaylistInfo | null }>({
  show: false, x: 0, y: 0, playlist: null,
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

function matchesQuery(query: string, ...fields: (string | number | undefined | null)[]): boolean {
  const trimmed = query.trim().toLowerCase()
  if (!trimmed) return true
  return fields.some((field) => String(field ?? '').toLowerCase().includes(trimmed))
}

// 所有 tab（含下载页与本地的两个分类）共用同一搜索栏，位置固定不动
const isLocalArtistSearch = computed(() => activeTab.value === 0 && localCategory.value === 'artists')
const tabSearchModel = computed({
  get: () => (isLocalArtistSearch.value ? localArtistQuery.value : tabQuery.value),
  set: (value: string) => {
    if (isLocalArtistSearch.value) localArtistQuery.value = value
    else tabQuery.value = value
  },
})
const tabSearchHint = computed(() =>
  isLocalArtistSearch.value ? t('library.local_artist_search_hint')
  : activeTab.value === 0 && localCategory.value === 'files' ? t('library.local_scan_search')
  : activeTab.value === 2 ? t('library.download_search_hint')
  : t('library.tab_search_hint'),
)

const filteredPlaylists = computed(() =>
  playlists.value.filter((pl) => matchesQuery(tabQuery.value, displayName(pl))),
)
watch(filteredPlaylists, (visible) => {
  if (selectedPlaylists.value.size > 0) selectedPlaylists.value = visibleSelection(selectedPlaylists.value, visible)
})
// 收藏分类: 歌单 / 歌手 (对齐 Android FavoritePlaylistList 的二级分类)
const favoriteCategory = ref<'playlists' | 'artists'>('playlists')
const favoriteArtistSource = ref<ArtistFavoriteSource>('neteaseArtist')
const importingArtists = ref(false)
const favoriteRenderCount = ref(100)
const playlistFavorites = computed(() =>
  favoritePlaylists.value.filter((fpl) => !isArtistFavoriteSource(fpl.source)),
)
const artistFavorites = computed(() =>
  filterFavoriteArtists(favoritePlaylists.value, favoriteArtistSource.value),
)
const filteredFavoritePlaylists = computed(() => {
  if (favoriteCategory.value === 'artists') {
    return filterFavoriteArtists(favoritePlaylists.value, favoriteArtistSource.value, tabQuery.value)
  }
  return playlistFavorites.value.filter((fpl) => matchesQuery(tabQuery.value, fpl.name, fpl.source))
})
const visibleFavoritePlaylists = computed(() => filteredFavoritePlaylists.value.slice(0, favoriteRenderCount.value))
watch([favoriteCategory, favoriteArtistSource, tabQuery], () => { favoriteRenderCount.value = 100 })

function favoriteArtistPlatformLabel(source: string): string {
  return source === 'neteaseArtist' ? t('player.source_netease')
    : source === 'biliArtist' ? t('player.source_bilibili') : 'YouTube'
}

async function importFollowedArtists() {
  if (importingArtists.value) return
  const source = favoriteArtistSource.value
  const loggedIn = source === 'neteaseArtist' ? auth.netease.loggedIn : auth.youtube.loggedIn
  if (!loggedIn) {
    toast.show(t('library.artist_import_login'), 'info')
    return
  }
  importingArtists.value = true
  try {
    const count = await invoke<number>('import_followed_artists', { source })
    await loadFavorites()
    toast.show(t('library.artist_import_success', { count }), 'success')
  } catch (error) {
    toast.show(String(error), 'error')
  } finally {
    importingArtists.value = false
  }
}
const filteredNeteasePlaylists = computed(() =>
  neteasePlaylists.value.filter((npl: any) => matchesQuery(tabQuery.value, npl.name)),
)
const filteredNeteaseAlbums = computed(() =>
  recommend.userAlbums.filter((album: any) => matchesQuery(tabQuery.value, album.name, album.artist)),
)
const filteredBiliPlaylists = computed(() =>
  biliPlaylists.value.filter((bpl: any) => matchesQuery(tabQuery.value, bpl.name)),
)
const filteredYoutubePlaylists = computed(() =>
  youtubePlaylists.value.filter((ypl: any) => matchesQuery(tabQuery.value, ypl.name)),
)
// 本地库分类：歌单 / 歌手（对齐 Android LOCAL_CATEGORY_ARTIST）
type LocalCategory = 'playlists' | 'artists' | 'files'
const localCategory = ref<LocalCategory>('playlists')
const localArtistSort = ref<LocalArtistSortMode>('song_count')
const localArtistQuery = ref('')
const localArtistTracks = ref<TrackInfo[]>([])
const localArtistsLoading = ref(false)
const showLocalArtistSort = ref(false)

const localArtistSortModes: LocalArtistSortMode[] = ['song_count', 'name', 'recent']

const localArtists = computed(() =>
  sortLocalArtists(
    filterLocalArtists(
      groupLocalArtists(localArtistTracks.value, t('library.local_artist_unknown')),
      localArtistQuery.value,
    ),
    localArtistSort.value,
  ),
)

async function loadLocalArtistTracks(force = false) {
  if (localArtistsLoading.value) return
  if (localArtistTracks.value.length > 0 && !force) return
  localArtistsLoading.value = true
  try {
    // 与歌手详情页共用同一个来源，避免两边口径漂移
    localArtistTracks.value = await loadArtistSourceTracks()
  } catch (e) {
    log.error('Load local artist tracks failed:', e)
    localArtistTracks.value = []
  } finally {
    localArtistsLoading.value = false
  }
}

function switchLocalCategory(category: LocalCategory) {
  if (localCategory.value === category) return
  if (isMultiSelectMode.value) exitMultiSelect()
  localCategory.value = category
  if (category === 'artists') void loadLocalArtistTracks()
}

function openLocalArtist(artist: LocalArtistSummary) {
  router.push({ name: 'local-artist', params: { name: artist.name } })
}

function playLocalArtist(artist: LocalArtistSummary) {
  if (!artist.tracks.length) return
  player.playAll(artist.tracks)
}

function isProtectedPlaylist(pl: PlaylistInfo) {
  return isSystemPlaylist(pl)
}

function displayName(pl: PlaylistInfo): string {
  return localPlaylistDisplayName(pl, { favorites: t('library.liked_songs'), localFiles: t('library.local_files') })
}

function openContextMenu(e: MouseEvent, pl: PlaylistInfo) {
  if (isProtectedPlaylist(pl)) return
  const btn = e.currentTarget as HTMLElement
  const rect = btn.getBoundingClientRect()
  let x = rect.left - 204
  if (x < 8) x = rect.right + 4
  contextMenu.value = { show: true, x, y: rect.top, playlist: pl }
}

// 行上右键：菜单在光标处弹出（复用 ContextMenu 的边缘翻转与重定位）
function openPlaylistContextMenu(e: MouseEvent, pl: PlaylistInfo) {
  if (isMultiSelectMode.value) {
    togglePlaylistSelection(pl.id)
    return
  }
  if (isProtectedPlaylist(pl)) return
  contextMenu.value = { show: true, x: e.clientX, y: e.clientY, playlist: pl }
}

function closeContextMenu() {
  contextMenu.value.show = false
}

// 删除确认
const showDeleteDialog = ref(false)
const deleteTarget = ref<PlaylistInfo | null>(null)

function requestDelete(pl: PlaylistInfo) {
  closeContextMenu()
  deleteTarget.value = pl
  showDeleteDialog.value = true
}

async function confirmDelete() {
  if (!deleteTarget.value) return
  try {
    await invoke('delete_playlist', { id: deleteTarget.value.id })
    showDeleteDialog.value = false
    deleteTarget.value = null
    await loadPlaylists()
  } catch (e) {
    log.error('Delete playlist failed:', e)
    toast.error(t('library.delete_playlist_failed'))
  }
}

// 重命名
const showRenameDialog = ref(false)
const renameTarget = ref<PlaylistInfo | null>(null)
const renameValue = ref('')
const renameInputRef = ref<InstanceType<typeof M3Input>>()

function requestRename(pl: PlaylistInfo) {
  closeContextMenu()
  renameTarget.value = pl
  renameValue.value = pl.name
  showRenameDialog.value = true
  nextTick(() => renameInputRef.value?.focus())
}

const playlistMenuItems = computed<ContextMenuItem[]>(() => {
  const playlist = contextMenu.value.playlist
  return [
    createContextMenuItem(t('common.multi_select'), { id: 'select', icon: 'checklist' }),
    createContextMenuItem(t('library.rename_playlist'), { id: 'rename', icon: 'edit' }),
    createContextMenuItem(t('library.delete_playlist'), {
      id: 'delete',
      icon: 'delete',
      danger: true,
      disabled: !playlist || isProtectedPlaylist(playlist),
    }),
  ]
})

function handlePlaylistMenuClick(item: ContextMenuActionItem) {
  const playlist = contextMenu.value.playlist
  if (!playlist) return

  switch (item.id) {
    case 'select':
      closeContextMenu()
      enterMultiSelect(playlist)
      break
    case 'rename':
      requestRename(playlist)
      break
    case 'delete':
      requestDelete(playlist)
      break
  }
}

async function confirmRename() {
  if (!renameTarget.value || !renameValue.value.trim()) return
  try {
    await invoke('rename_playlist', { id: renameTarget.value.id, name: renameValue.value.trim() })
    showRenameDialog.value = false
    renameTarget.value = null
    await loadPlaylists()
  } catch (e) {
    log.error('Rename playlist failed:', e)
    toast.error(t('library.rename_playlist_failed'))
  }
}

// 批量删除确认
const showBatchDeleteDialog = ref(false)

function requestDeleteSelected() {
  if (selectedPlaylists.value.size === 0) return
  showBatchDeleteDialog.value = true
}

async function confirmDeleteSelected() {
  // 兜底再过滤一次：系统歌单与当前不可见的歌单都绝不删除
  const ids = [...visibleSelection(selectedPlaylists.value, filteredPlaylists.value)].filter((id) => {
    const pl = playlists.value.find(p => p.id === id)
    return !pl || !isProtectedPlaylist(pl)
  })
  try {
    for (const id of ids) {
      await invoke('delete_playlist', { id })
    }
    showBatchDeleteDialog.value = false
    exitMultiSelect()
    await loadPlaylists()
  } catch (e) {
    log.error('Batch delete playlists failed:', e)
    toast.error(t('library.delete_playlist_failed'))
    await loadPlaylists()
  }
}

// 网易云用户歌单
const neteasePlaylists = computed(() => recommend.userPlaylists['netease'] || [])
// 哔哩哔哩收藏夹
const biliPlaylists = computed(() => recommend.userPlaylists['bilibili'] || [])
// YouTube Music 资料库歌单
const youtubePlaylists = computed(() => recommend.userPlaylists['youtube'] || [])

// 收藏歌单（从同步数据中获取）
const favoritePlaylists = ref<FavoritePlaylist[]>([])
let favoritesLoadVersion = 0

/// 对齐 Android LibraryScreen: 按 source 跳平台详情页懒加载曲目,
/// 无法定位平台页时才退回同步曲目快照的本地详情
function openFavorite(fpl: FavoritePlaylist) {
  if (isArtistFavoriteSource(fpl.source)) {
    const target = favoriteArtistRoute(fpl)
    if (target) router.push(target)
    else toast.show(t('player.load_failed'), 'error')
    return
  }
  switch (fpl.source) {
    case 'netease':
      router.push({ name: 'netease-playlist', params: { id: fpl.id } })
      return
    case 'neteaseAlbum':
      router.push({ name: 'netease-album', params: { id: fpl.id } })
      return
    case 'youtubeMusic': {
      const browseId = fpl.browseId || (fpl.playlistId ? `VL${fpl.playlistId}` : '')
      if (browseId) {
        router.push({ name: 'youtube-playlist', params: { browseId } })
        return
      }
      break
    }
    case 'bili': {
      // 合集、视频列表不是收藏夹，按收藏夹 id 去查只会拿到空的默认收藏夹
      const reference = parseBiliPlaylistReference(fpl.browseId)
      const target = biliPlaylistRoute({
        id: fpl.id, kind: reference?.kind, mid: reference?.mid,
        name: fpl.name, coverUrl: fpl.coverUrl, trackCount: fpl.trackCount, uploader: fpl.subtitle,
      })
      if (target) {
        router.push(target)
        return
      }
      break
    }
  }
  router.push({ name: 'favorite-playlist', params: { id: fpl.id } })
}

async function loadFavorites() {
  const request = ++favoritesLoadVersion
  try {
    const raw = await invoke<unknown>('list_favorite_playlists')
    if (request !== favoritesLoadVersion) return
    favoritePlaylists.value = parseFavoritePlaylists(raw)
  } catch (e) {
    log.error('Load favorites failed:', e)
  }
}

onMounted(loadPlaylists)
onMounted(loadFavorites)
// 拉取云端歌单
/// 平台数据按需拉取
///
/// 绝不能只在 onMounted 判一次登录态：登录状态是启动后异步拉回来的，
/// 挂载那一刻通常还是 false，判完就再没有东西重新触发，
/// 表现就是「明明登录了，云端歌单一直空着」。
/// 这里改成对（登录态 × 当前 tab）响应式求值，任一变化都会补拉。
/// 有缓存时先显示缓存，本次启动第一次打开时在后台刷新；同一平台的请求由 store 合并。
type CloudPlatform = 'netease' | 'bilibili' | 'youtube'

function isCloudPlatform(platform: string): platform is CloudPlatform {
  return platform === 'netease' || platform === 'bilibili' || platform === 'youtube'
}

function ensurePlatformData(platform: string, force = false) {
  if (!isCloudPlatform(platform) || !auth[platform].loggedIn) return
  // 专辑和歌单分开判断：歌单从缓存恢复了，专辑也要照样补拉
  if (platform === 'netease') void (force ? recommend.fetchUserAlbums() : recommend.ensureUserAlbums())
  void (force ? recommend.fetchUserPlaylists(platform) : recommend.ensureUserPlaylists(platform))
}

function syncActiveTabData(force = false) {
  switch (activeTab.value) {
    case 3:
      ensurePlatformData('netease', force)
      break
    case 4:
      ensurePlatformData('bilibili', force)
      break
    case 5:
      ensurePlatformData('youtube', force)
      break
    default:
      break
  }
}

/// 登录态还没查回来时按「已登录」对待：有缓存就先显示，免得启动瞬间闪一下登录提示
function cloudSignedIn(platform: CloudPlatform): boolean {
  return !auth.statusChecked || auth[platform].loggedIn
}

/// 登录态检查返回之前还不知道要不要拉，按加载中显示，不先闪一下「暂无」
const PENDING_AUTH: CloudListStatus = { loading: true, error: null }

function cloudPlaylistStatus(platform: CloudPlatform): CloudListStatus | undefined {
  return auth.statusChecked ? recommend.userPlaylistsStatus[platform] : PENDING_AUTH
}

const neteaseAlbumsStatus = computed(() => auth.statusChecked ? recommend.userAlbumsStatus : PENDING_AUTH)

/// 网易云分类栏右侧的刷新状态：只在已有列表时显示，没有列表时由占位区显示
const neteaseRefreshStatus = computed(() => {
  if (neteaseCategory.value === 'albums') {
    return recommend.userAlbums.length > 0 ? recommend.userAlbumsStatus : undefined
  }
  return neteasePlaylists.value.length > 0 ? recommend.userPlaylistsStatus.netease : undefined
})

function retryNeteaseRefresh() {
  if (neteaseCategory.value === 'albums') void recommend.fetchUserAlbums()
  else void recommend.fetchUserPlaylists('netease')
}

watch(
  () => [
    activeTab.value,
    auth.netease.loggedIn,
    auth.bilibili.loggedIn,
    auth.youtube.loggedIn,
  ],
  () => syncActiveTabData(),
  { immediate: true },
)

/// 登录/登出后强制重新拉取该平台数据；重新登录看到旧的空列表是最常见的抱怨
function handleAuthChanged(event: Event) {
  const platform = (event as CustomEvent<{ platform?: string }>).detail?.platform
  if (!platform) return
  ensurePlatformData(platform, true)
  void loadFavorites()
}

onMounted(() => window.addEventListener(AUTH_CHANGED_EVENT, handleAuthChanged))
onUnmounted(() => window.removeEventListener(AUTH_CHANGED_EVENT, handleAuthChanged))

// 监听同步完成后的歌单变更事件
let unlistenPlaylistsChanged: UnlistenFn | null = null
let libraryUnmounted = false
onMounted(async () => {
  const stop = await listen('playlists-changed', () => {
    void loadPlaylists()
    void loadFavorites()
    // 本地歌手由歌单曲目聚合而来；不在歌手视图时清空，下次进入重新加载
    if (localCategory.value === 'artists') void loadLocalArtistTracks(true)
    else localArtistTracks.value = []
  })
  if (libraryUnmounted) stop()
  else unlistenPlaylistsChanged = stop
})
onUnmounted(() => {
  libraryUnmounted = true
  favoritesLoadVersion++
  unlistenPlaylistsChanged?.()
})
</script>

<template>
  <div class="library-view">
    <header class="lib-header">
      <h1 class="page-title">{{ t('library.title') }}</h1>
      <div class="header-actions">
        <button
          v-if="!isMultiSelectMode"
          class="header-action"
          :title="t('stats.title')"
          :aria-label="t('stats.title')"
          @click="router.push({ name: 'playback-stats' })"
        >
          <span class="material-symbols-rounded">bar_chart</span>
        </button>
        <button v-if="activeTab === 0 && localCategory === 'playlists' && !isMultiSelectMode" class="header-action" @click="enterMultiSelect()" :title="t('common.multi_select')">
          <span class="material-symbols-rounded">checklist</span>
        </button>
        <button v-if="isMultiSelectMode" class="header-action" @click="selectAll" :title="t('common.select_all')">
          <span class="material-symbols-rounded">select_all</span>
        </button>
        <button v-if="isMultiSelectMode" class="header-action" @click="invertSelection" :title="t('common.invert_selection')">
          <span class="material-symbols-rounded">flip</span>
        </button>
        <button v-if="isMultiSelectMode" class="header-action" @click="exitMultiSelect" :title="t('common.exit_selection')">
          <span class="material-symbols-rounded">close</span>
        </button>
      </div>
    </header>

    <div class="tab-bar">
      <button
        v-for="(tab, i) in tabs"
        :key="tab.label"
        class="tab-chip"
        :class="{ active: activeTab === i }"
        @click="activeTab = i"
      >
        <span class="material-symbols-rounded" :class="{ filled: activeTab === i }" style="font-size: 18px">{{ tab.icon }}</span>
        <span>{{ tab.label }}</span>
      </button>
    </div>

    <!-- 各 tab（含下载）共用的搜索栏：单一实例固定在分类 chips 上方，
         切换视图只换 placeholder 与绑定的关键词，位置尺寸不变 -->
      <div class="collapse-row">
        <div class="collapse-clip">
          <div class="tab-search">
            <span class="material-symbols-rounded" style="font-size: 18px">search</span>
            <input
              v-model="tabSearchModel"
              type="search"
              :placeholder="tabSearchHint"
              :aria-label="tabSearchHint"
            />
            <button
              v-if="tabSearchModel"
              class="tab-search-clear"
              :aria-label="t('common.clear')"
              @click="tabSearchModel = ''"
            >
              <span class="material-symbols-rounded" style="font-size: 18px">close</span>
            </button>
          </div>
        </div>
      </div>

    <!-- tab 内容整体用统一的 fade 过渡（复用 global.scss 的 fade 类），
         避免 v-if 直切导致整页闪变 -->
    <Transition name="fade" mode="out-in">

    <!-- Tab: 本地 -->
    <div v-if="activeTab === 0" key="tab-local" class="playlist-list">
      <!-- 歌单 / 歌手分类（对齐 Android 本地页分类切换）；
           排序按钮常驻在同一行右侧，只在歌手视图内以缩放过渡出现 -->
      <div class="local-category-bar">
        <button
          v-for="category in (['playlists', 'artists', 'files'] as const)"
          :key="category"
          class="local-category-chip"
          :class="{ active: localCategory === category }"
          @click="switchLocalCategory(category)"
        >
          <span class="material-symbols-rounded" style="font-size: 17px">
            {{ category === 'playlists' ? 'queue_music' : category === 'artists' ? 'account_circle' : 'folder_open' }}
          </span>
          <span>{{ category === 'playlists' ? t('library.local_category_playlists') : category === 'artists' ? t('library.local_category_artists') : t('library.local_files') }}</span>
        </button>
        <Transition name="lib-zoom">
          <button
            v-if="localCategory === 'artists'"
            class="artist-sort-btn"
            :class="{ active: showLocalArtistSort }"
            :title="t('library.local_artist_sort')"
            :aria-label="t('library.local_artist_sort')"
            @click="showLocalArtistSort = !showLocalArtistSort"
          >
            <span class="material-symbols-rounded" style="font-size: 20px">sort</span>
          </button>
        </Transition>
      </div>

      <!-- 歌单 / 歌手两个子视图之间交叉淡入，消除整块重建的闪变 -->
      <Transition name="fade" mode="out-in">
      <!-- 歌手视图 -->
      <div v-if="localCategory === 'artists'" key="local-artists" class="local-subview">
        <!-- 排序 chips 行：展开 / 收起走折叠过渡，按钮本身保持常驻 -->
        <Transition name="lib-collapse">
          <div v-if="showLocalArtistSort" class="collapse-row">
            <div class="collapse-clip">
              <div class="artist-sort-options">
                <button
                  v-for="mode in localArtistSortModes"
                  :key="mode"
                  class="artist-sort-option"
                  :class="{ active: localArtistSort === mode }"
                  @click="localArtistSort = mode; showLocalArtistSort = false"
                >
                  {{ t(`library.local_artist_sort_${mode}`) }}
                </button>
              </div>
            </div>
          </div>
        </Transition>

        <!-- 加载 / 空态 / 网格三态与排序重排统一走容器级交叉淡入；
             歌手数可达数百，TransitionGroup 的 FLIP move 会卡，故用整容器淡入 -->
        <Transition name="lib-fade" mode="out-in">
        <div v-if="localArtistsLoading" key="loading" class="empty-tab">
          <span class="material-symbols-rounded spinning" style="font-size: 32px">progress_activity</span>
        </div>
        <div v-else-if="localArtists.length === 0" key="empty" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">account_circle</span></div>
          <p class="empty-title">{{ t(localArtistQuery.trim() ? 'library.artist_search_empty' : 'library.local_artist_empty') }}</p>
          <p v-if="!localArtistQuery.trim()" class="empty-desc">{{ t('library.local_artist_hint') }}</p>
        </div>
        <div v-else :key="`grid-${localArtistSort}`" class="artist-grid">
          <div
            v-for="artist in localArtists"
            :key="artist.key"
            class="artist-card"
            role="button"
            tabindex="0"
            @click="openLocalArtist(artist)"
            @keydown.enter="openLocalArtist(artist)"
            @keydown.space.prevent="openLocalArtist(artist)"
          >
            <div class="artist-cover">
              <BilibiliCoverImage v-if="artist.coverUrl" :src="artist.coverUrl" loading="lazy">
                <span class="material-symbols-rounded filled" style="font-size: 34px">account_circle</span>
              </BilibiliCoverImage>
              <span v-else class="material-symbols-rounded filled" style="font-size: 34px">account_circle</span>
              <button
                class="artist-play"
                :title="t('player.play_all')"
                :aria-label="t('player.play_all')"
                @click.stop="playLocalArtist(artist)"
              >
                <span class="material-symbols-rounded filled" style="font-size: 20px">play_arrow</span>
              </button>
            </div>
            <div class="artist-name">{{ artist.name }}</div>
            <div class="artist-count">{{ t('player.track_count', { count: artist.tracks.length }) }}</div>
          </div>
        </div>
        </Transition>
      </div>

      <LocalFilesView v-else-if="localCategory === 'files'" key="local-files" embedded :search-query="tabQuery" />

      <!-- 歌单视图 -->
      <div v-else key="local-playlists" class="local-subview">
      <!-- 新建歌单（对齐 Android：+ 新建歌单 行） -->
      <div class="new-playlist-row" @click="openCreateDialog">
        <span class="material-symbols-rounded" style="font-size: 20px">add</span>
        <span>{{ t('library.create_playlist') }}</span>
      </div>
      <div class="list-divider" />

      <!-- 歌单列表：进入 / 移除走 TransitionGroup；拖拽重排与歌单详情页
           歌曲拖拽同一套指针交互（让路位移 + 单实例落点指示线 + 拖影） -->
      <div ref="playlistListRef" class="lib-list">
      <div
        class="drop-indicator"
        :class="{ visible: plDropIndicatorVisible, snap: plDropIndicatorSnap }"
        :style="{ transform: `translate3d(0, ${plDropIndicatorY}px, 0)` }"
        aria-hidden="true"
      />
      <!-- 拖拽中切到无动画 name，释放后行归位走 FLIP move，避免与指针跟随打架 -->
      <TransitionGroup :name="plDragKey ? 'lib-static' : 'lib-list'">
      <div
        v-for="pl in filteredPlaylists"
        :key="pl.id"
        class="playlist-item"
        :class="{
          selected: selectedPlaylists.has(pl.id),
          dragging: plDragKey === playlistDragKey(pl),
          'glass-active': plDragGlassActive && plDragKey === playlistDragKey(pl),
          landing: plDragLandingKey === playlistDragKey(pl),
          'drag-under-glass': plDragUnderGlassKeys.has(playlistDragKey(pl)),
          'drag-under-glass-active': plDragGlassActive && plDragUnderGlassKeys.has(playlistDragKey(pl)),
          'drag-over-before': plDragOverKey === playlistDragKey(pl) && plDragInsertPosition === 'before',
          'drag-over-after': plDragOverKey === playlistDragKey(pl) && plDragInsertPosition === 'after',
        }"
        :data-drag-key="playlistDragKey(pl)"
        :style="plDragItemStyle(playlistDragKey(pl))"
        @click="isMultiSelectMode ? togglePlaylistSelection(pl.id) : router.push({ name: 'local-playlist', params: { id: pl.id } })"
        @contextmenu.prevent.stop="openPlaylistContextMenu($event, pl)"
      >
        <!-- 多选模式下显示复选框 -->
        <div v-if="isMultiSelectMode" class="pl-checkbox" @click.stop="togglePlaylistSelection(pl.id)">
          <span class="material-symbols-rounded" :class="{ filled: selectedPlaylists.has(pl.id) }" style="font-size: 22px">
            {{ selectedPlaylists.has(pl.id) ? 'check_circle' : 'radio_button_unchecked' }}
          </span>
        </div>
        <div class="pl-icon" :class="{ 'has-cover': pl.cover_url && !isLibraryCoverFailed('local', pl.id, pl.cover_url) }">
          <BilibiliCoverImage v-if="isBilibiliCover(pl.cover_url) && !isLibraryCoverFailed('local', pl.id, pl.cover_url)" :src="pl.cover_url!" class="pl-cover-img">
            <span class="material-symbols-rounded filled" style="font-size: 22px">library_music</span>
          </BilibiliCoverImage>
          <img
            v-else-if="pl.cover_url && !isLibraryCoverFailed('local', pl.id, pl.cover_url)"
            :src="toDisplayableLibraryCoverUrl(pl.cover_url)"
            referrerpolicy="no-referrer"
            class="pl-cover-img"
            @error="markLibraryCoverFailed('local', pl.id, pl.cover_url)"
          />
          <span v-else class="material-symbols-rounded filled" style="font-size: 22px">library_music</span>
        </div>
        <div class="pl-info">
          <div class="pl-name">{{ displayName(pl) }}</div>
          <div class="pl-count">{{ t('player.track_count', { count: pl.track_count }) }}</div>
        </div>
        <!-- 多选模式下显示排序摇杆（对齐 Android：仅拖此手柄可排序） -->
        <span
          v-if="isMultiSelectMode && !isProtectedPlaylist(pl)"
          class="pl-drag-handle material-symbols-rounded"
          style="font-size: 22px"
          :title="t('common.drag_to_reorder')"
          @pointerdown.stop="startPlaylistDrag($event, playlistDragKey(pl))"
          @click.stop
        >drag_handle</span>
        <!-- 受保护歌单不显示三点菜单 -->
        <button v-else-if="!isProtectedPlaylist(pl) && !isMultiSelectMode" class="pl-more" @click.stop="openContextMenu($event, pl)">
          <span class="material-symbols-rounded" style="font-size: 20px">more_vert</span>
        </button>
      </div>
      </TransitionGroup>
      </div>

      <!-- 多选模式底部操作栏 -->
      <Transition name="lib-bar">
      <div v-if="isMultiSelectMode" class="multi-select-bar">
        <span class="select-count">{{ t('common.selected_playlist_count', { count: selectedPlaylists.size }) }}</span>
        <button class="multi-select-action neutral" @click="selectAll">
          <span class="material-symbols-rounded" style="font-size: 18px">select_all</span>
          <span>{{ t('common.select_all') }}</span>
        </button>
        <button class="multi-select-action neutral" @click="invertSelection">
          <span class="material-symbols-rounded" style="font-size: 18px">flip</span>
          <span>{{ t('common.invert_selection') }}</span>
        </button>
        <button class="multi-select-action danger push-right" :disabled="selectedPlaylists.size === 0" @click="requestDeleteSelected">
          <span class="material-symbols-rounded" style="font-size: 18px">delete</span>
          <span>{{ t('common.delete_selected') }}</span>
        </button>
        <button class="multi-select-action ghost" @click="exitMultiSelect">
          <span>{{ t('common.cancel') }}</span>
        </button>
      </div>
      </Transition>

      <div v-if="playlists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">library_music</span></div>
        <p class="empty-title">{{ t('library.playlist_empty_title') }}</p>
        <p class="empty-desc">{{ t('library.playlist_empty_desc') }}</p>
      </div>
      <div v-else-if="filteredPlaylists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
        <p class="empty-title">{{ t('player.no_results') }}</p>
      </div>
      </div>
      </Transition>
    </div>

    <!-- Tab: 收藏（同步的收藏歌单） -->
    <div v-else-if="activeTab === 1" key="tab-favorites" class="playlist-list">
      <!-- 歌单 / 歌手分类 (对齐 Android 收藏页分类切换) -->
      <div class="local-category-bar">
        <button
          v-for="category in (['playlists', 'artists'] as const)"
          :key="category"
          class="local-category-chip"
          :class="{ active: favoriteCategory === category }"
          @click="favoriteCategory = category"
        >
          <span class="material-symbols-rounded" style="font-size: 17px">
            {{ category === 'playlists' ? 'queue_music' : 'account_circle' }}
          </span>
          <span>{{ category === 'playlists' ? t('library.local_category_playlists') : t('library.local_category_artists') }}</span>
        </button>
      </div>

      <section v-if="favoriteCategory === 'artists'" class="favorite-artist-header">
        <div class="favorite-artist-summary">
          <h2>{{ t('library.artist_following') }}</h2>
          <span>{{ t('library.artist_count', { count: artistFavorites.length }) }}</span>
        </div>
        <div class="favorite-artist-platforms" role="group" :aria-label="t('library.artist_following')">
          <button
            v-for="source in ARTIST_FAVORITE_SOURCES"
            :key="source"
            :class="{ active: favoriteArtistSource === source }"
            :aria-pressed="favoriteArtistSource === source"
            @click="favoriteArtistSource = source"
          >
            <span v-if="favoriteArtistSource === source" class="material-symbols-rounded">check</span>
            {{ favoriteArtistPlatformLabel(source) }}
          </button>
        </div>
        <button
          v-if="favoriteArtistSource !== 'biliArtist'"
          class="favorite-artist-import"
          :disabled="importingArtists"
          @click="importFollowedArtists"
        >
          <span class="material-symbols-rounded" :class="{ spinning: importingArtists }">
            {{ importingArtists ? 'progress_activity' : 'cloud_download' }}
          </span>
          {{ t(importingArtists ? 'library.artist_import_loading' : 'library.artist_import') }}
        </button>
      </section>

      <Transition name="fade" mode="out-in">
      <TransitionGroup
        v-if="filteredFavoritePlaylists.length > 0"
        :key="'fav-' + favoriteCategory + (favoriteCategory === 'artists' ? favoriteArtistSource : '')"
        tag="div"
        name="lib-list"
        class="lib-list"
      >
        <div
          v-for="fpl in visibleFavoritePlaylists"
          :key="favoriteKey(fpl)"
          class="playlist-item"
          :class="{ 'favorite-artist-item': favoriteCategory === 'artists' }"
          role="button"
          tabindex="0"
          @click="openFavorite(fpl)"
          @keydown.enter="openFavorite(fpl)"
          @keydown.space.prevent="openFavorite(fpl)"
        >
          <div class="pl-icon has-cover" v-if="fpl.coverUrl && !isLibraryCoverFailed('favorite', favoriteKey(fpl), fpl.coverUrl)">
            <BilibiliCoverImage
              :src="toDisplayableLibraryCoverUrl(fpl.coverUrl)"
              class="pl-cover-img"
              loading="lazy"
              @error="markLibraryCoverFailed('favorite', favoriteKey(fpl), fpl.coverUrl)"
            />
          </div>
          <div class="pl-icon" v-else>
            <span class="material-symbols-rounded filled" style="font-size: 22px">
              {{ favoriteCategory === 'artists' ? 'account_circle' : 'bookmark' }}
            </span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ fpl.name }}</div>
            <div class="pl-count" v-if="favoriteCategory === 'artists'">{{ fpl.subtitle || favoriteArtistPlatformLabel(fpl.source) }}</div>
            <div class="pl-count" v-else>{{ t('player.track_count', { count: fpl.trackCount }) }} · {{ favoriteSourceLabel(fpl.source) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
      </TransitionGroup>
      <div v-else class="empty-tab" :key="'fav-empty-' + favoriteCategory">
        <div class="empty-circle">
          <span class="material-symbols-rounded" style="font-size: 40px">
            {{ favoriteCategory === 'artists' ? 'account_circle' : 'bookmark' }}
          </span>
        </div>
        <p class="empty-title">{{ t(favoriteCategory === 'artists'
          ? (tabQuery.trim() ? 'library.artist_search_empty' : 'library.artist_empty')
          : (tabQuery.trim() ? 'player.no_results' : 'explore.no_playlists')) }}</p>
        <p v-if="!tabQuery.trim()" class="empty-desc">{{ t(favoriteCategory === 'artists' ? 'library.artist_empty_hint' : 'explore.login_for_playlists') }}</p>
      </div>
      </Transition>
      <button
        v-if="visibleFavoritePlaylists.length < filteredFavoritePlaylists.length"
        class="favorite-artist-import"
        @click="favoriteRenderCount += 100"
      >{{ t('player.artist_load_more') }}</button>
    </div>

    <!-- Tab: 下载 -->
    <DownloadsView v-else-if="activeTab === 2" key="tab-downloads" embedded :search-query="tabQuery" />

    <!-- Tab: 网易云-歌单 -->
    <div v-else-if="activeTab === 3" key="tab-netease" class="playlist-list">
      <!-- 歌单 / 专辑 分类（对齐 Android 网易云板块内部分类） -->
      <div class="local-category-bar">
        <button
          v-for="category in (['playlists', 'albums'] as const)"
          :key="category"
          class="local-category-chip"
          :class="{ active: neteaseCategory === category }"
          @click="neteaseCategory = category"
        >
          <span class="material-symbols-rounded" style="font-size: 17px">
            {{ category === 'playlists' ? 'queue_music' : 'album' }}
          </span>
          <span>{{ category === 'playlists' ? t('library.tab_netease_playlists_short') : t('library.tab_netease_albums_short') }}</span>
        </button>
        <CloudListState class="platform-sync" variant="banner" :status="neteaseRefreshStatus" @retry="retryNeteaseRefresh" />
      </div>

      <!-- 歌单 / 专辑分类切换同样走交叉淡入，与本地页保持一致 -->
      <Transition name="fade" mode="out-in">
      <div v-if="neteaseCategory === 'albums'" key="ne-albums" class="local-subview">
        <TransitionGroup v-if="filteredNeteaseAlbums.length > 0 && cloudSignedIn('netease')" tag="div" name="lib-list" class="lib-list">
          <div
            v-for="album in filteredNeteaseAlbums"
            :key="album.id"
            class="playlist-item"
            @click="router.push({ name: 'netease-album', params: { id: album.id } })"
          >
            <div class="pl-icon" :class="{ 'has-cover': album.coverUrl && !isLibraryCoverFailed('netease-album', album.id, album.coverUrl) }">
              <img
                v-if="album.coverUrl && !isLibraryCoverFailed('netease-album', album.id, album.coverUrl)"
                :src="toDisplayableLibraryCoverUrl(album.coverUrl)"
                class="pl-cover-img"
                loading="lazy"
                referrerpolicy="no-referrer"
                @error="markLibraryCoverFailed('netease-album', album.id, album.coverUrl)"
              />
              <span v-else class="material-symbols-rounded filled" style="font-size: 22px">album</span>
            </div>
            <div class="pl-info">
              <div class="pl-name">{{ album.name }}</div>
              <div class="pl-count">{{ album.artist }} · {{ t('player.track_count', { count: album.trackCount }) }}</div>
            </div>
            <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
          </div>
        </TransitionGroup>
        <div v-else-if="recommend.userAlbums.length > 0 && cloudSignedIn('netease')" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
          <p class="empty-title">{{ t('player.no_results') }}</p>
        </div>
        <CloudListState
          v-else-if="cloudSignedIn('netease')"
          variant="placeholder"
          :status="neteaseAlbumsStatus"
          :loading-text="t('library.cloud_albums_loading')"
          :failed-text="t('library.cloud_albums_load_failed')"
          @retry="recommend.fetchUserAlbums()"
        >
          <template #icon>
            <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">album</span></div>
          </template>
          <p class="empty-title">{{ t('library.empty_title', { type: t('library.albums') }) }}</p>
          <p class="empty-desc">{{ t('library.empty_desc') }}</p>
        </CloudListState>
        <div v-else class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">album</span></div>
          <p class="empty-title">{{ t('library.empty_title', { type: t('library.albums') }) }}</p>
          <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
        </div>
      </div>

      <div v-else key="ne-playlists" class="local-subview">
      <TransitionGroup v-if="filteredNeteasePlaylists.length > 0 && cloudSignedIn('netease')" tag="div" name="lib-list" class="lib-list">
        <div
          v-for="npl in filteredNeteasePlaylists"
          :key="'ne-' + npl.id"
          class="playlist-item"
          @click="router.push({ name: 'netease-playlist', params: { id: npl.id } })"
        >
          <div class="pl-icon netease">
            <img
              v-if="npl.coverUrl && !isLibraryCoverFailed('netease-playlist', npl.id, npl.coverUrl)"
              :src="toDisplayableLibraryCoverUrl(npl.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('netease-playlist', npl.id, npl.coverUrl)"
            />
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px">library_music</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ npl.name }}</div>
            <div class="pl-count">{{ t('library.track_count', { count: npl.trackCount || 0 }) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
      </TransitionGroup>
      <div v-else-if="neteasePlaylists.length > 0 && cloudSignedIn('netease')" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
        <p class="empty-title">{{ t('player.no_results') }}</p>
      </div>
      <CloudListState
        v-else-if="cloudSignedIn('netease')"
        variant="placeholder"
        :status="cloudPlaylistStatus('netease')"
        @retry="recommend.fetchUserPlaylists('netease')"
      >
        <template #icon>
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">cloud_queue</span></div>
        </template>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
      </CloudListState>
      <div v-else class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">cloud_queue</span></div>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
      </div>
      </Transition>
    </div>

    <!-- Tab: Bili 收藏夹 -->
    <div v-else-if="activeTab === 4" key="tab-bilibili" class="playlist-list">
      <template v-if="biliPlaylists.length > 0 && cloudSignedIn('bilibili')">
        <div class="platform-summary bilibili">
          <span class="platform-icon-mask" style="mask-image: url('/icons/ic_bilibili.svg')"></span>
          <div>
            <div class="platform-title">{{ t('library.bilibili_favorites') }}</div>
            <div class="platform-desc">{{ t('player.video_count', { count: biliPlaylists.reduce((sum, p) => sum + (p.trackCount || 0), 0) }) }}</div>
          </div>
          <CloudListState
            class="platform-sync"
            variant="banner"
            :status="recommend.userPlaylistsStatus.bilibili"
            @retry="recommend.fetchUserPlaylists('bilibili')"
          />
        </div>
        <TransitionGroup tag="div" name="lib-list" class="lib-list">
        <div
          v-for="bpl in filteredBiliPlaylists"
          :key="'bili-' + bpl.id"
          class="playlist-item"
          @click="router.push({ name: 'bili-playlist', params: { mediaId: bpl.id } })"
        >
          <div class="pl-icon bilibili" :class="{ 'has-cover': bpl.coverUrl }">
            <BilibiliCoverImage v-if="bpl.coverUrl" :src="bpl.coverUrl" class="pl-cover-img">
              <span class="material-symbols-rounded filled" style="font-size: 22px">video_library</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px">video_library</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ bpl.name }}</div>
            <div class="pl-count">{{ t('player.video_count', { count: bpl.trackCount || 0 }) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
        </TransitionGroup>
        <div v-if="filteredBiliPlaylists.length === 0" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
          <p class="empty-title">{{ t('player.no_results') }}</p>
        </div>
      </template>
      <CloudListState
        v-else-if="cloudSignedIn('bilibili')"
        variant="placeholder"
        :status="cloudPlaylistStatus('bilibili')"
        @retry="recommend.fetchUserPlaylists('bilibili')"
      >
        <template #icon>
          <div class="empty-circle platform-empty bilibili"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_bilibili.svg')"></span></div>
        </template>
        <p class="empty-title">{{ t('library.bilibili_favorites') }}</p>
        <p class="empty-desc">{{ t('explore.no_playlists') }}</p>
      </CloudListState>
      <div v-else class="empty-tab">
        <div class="empty-circle platform-empty bilibili"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_bilibili.svg')"></span></div>
        <p class="empty-title">{{ t('library.bilibili_favorites') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
    </div>

    <!-- Tab: YouTube Music 歌单 -->
    <div v-else-if="activeTab === 5" key="tab-youtube" class="playlist-list">
      <template v-if="youtubePlaylists.length > 0 && cloudSignedIn('youtube')">
        <div class="platform-summary youtube">
          <span class="platform-icon-mask" style="mask-image: url('/icons/ic_youtube.svg')"></span>
          <div>
            <div class="platform-title">YouTube Music</div>
            <div class="platform-desc">{{ t('library.playlist_count', { count: youtubePlaylists.length }) }}</div>
          </div>
          <CloudListState
            class="platform-sync"
            variant="banner"
            :status="recommend.userPlaylistsStatus.youtube"
            @retry="recommend.fetchUserPlaylists('youtube')"
          />
        </div>
        <TransitionGroup tag="div" name="lib-list" class="lib-list">
        <div
          v-for="ypl in filteredYoutubePlaylists"
          :key="'yt-' + ypl.id"
          class="playlist-item"
          @click="router.push({ name: 'youtube-playlist', params: { browseId: ypl.id } })"
        >
          <div class="pl-icon youtube" :class="{ 'has-cover': ypl.coverUrl && !isLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl) }">
            <img
              v-if="ypl.coverUrl && !isLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl)"
              :src="toDisplayableLibraryCoverUrl(ypl.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl)"
            />
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px">subscriptions</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ ypl.name }}</div>
            <div class="pl-count">{{ ypl.description || 'YouTube Music' }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
        </TransitionGroup>
        <div v-if="filteredYoutubePlaylists.length === 0" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
          <p class="empty-title">{{ t('player.no_results') }}</p>
        </div>
      </template>
      <CloudListState
        v-else-if="cloudSignedIn('youtube')"
        variant="placeholder"
        :status="cloudPlaylistStatus('youtube')"
        @retry="recommend.fetchUserPlaylists('youtube')"
      >
        <template #icon>
          <div class="empty-circle platform-empty youtube"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_youtube.svg')"></span></div>
        </template>
        <p class="empty-title">YouTube Music</p>
        <p class="empty-desc">{{ t('explore.no_playlists') }}</p>
      </CloudListState>
      <div v-else class="empty-tab">
        <div class="empty-circle platform-empty youtube"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_youtube.svg')"></span></div>
        <p class="empty-title">YouTube Music</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
    </div>
    </Transition>

    <!-- 创建播放列表对话框 -->
    <M3Dialog
      v-model:open="showCreateDialog"
      :title="t('library.create_playlist')"
      icon="playlist_add"
      :confirm-text="t('library.create_playlist')"
      :confirm-disabled="!newPlaylistName.trim()"
      @confirm="confirmCreate"
    >
      <M3Input
        ref="inputRef"
        v-model="newPlaylistName"
        :placeholder="t('library.playlist_name_placeholder')"
        :maxlength="50"
        @enter="confirmCreate"
      />
    </M3Dialog>

    <ContextMenu
      :open="contextMenu.show"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :items="playlistMenuItems"
      @update:open="contextMenu.show = $event"
      @click="handlePlaylistMenuClick"
    />

    <!-- 删除确认对话框 -->
    <M3Dialog
      v-model:open="showDeleteDialog"
      :title="t('library.delete_confirm_title')"
      icon="delete"
      :confirm-text="t('library.delete_playlist')"
      confirm-danger
      @confirm="confirmDelete"
    >
      <p class="dialog-msg">{{ t('library.delete_confirm_msg', { name: deleteTarget ? displayName(deleteTarget) : '' }) }}</p>
    </M3Dialog>

    <!-- 重命名对话框 -->
    <M3Dialog
      v-model:open="showRenameDialog"
      :title="t('library.rename_playlist')"
      icon="edit"
      :confirm-text="t('common.save')"
      :confirm-disabled="!renameValue.trim() || renameValue.trim() === renameTarget?.name"
      @confirm="confirmRename"
    >
      <M3Input
        ref="renameInputRef"
        v-model="renameValue"
        :placeholder="t('library.rename_placeholder')"
        :maxlength="50"
        @enter="confirmRename"
      />
    </M3Dialog>

    <!-- 批量删除歌单确认 -->
    <M3Dialog
      v-model:open="showBatchDeleteDialog"
      :title="t('library.delete_confirm_title')"
      icon="delete"
      :confirm-text="t('common.delete_selected')"
      confirm-danger
      @confirm="confirmDeleteSelected"
    >
      <p class="dialog-msg">{{ t('library.batch_delete_playlists_msg', { count: selectedPlaylists.size }) }}</p>
    </M3Dialog>
  </div>
</template>

<style scoped lang="scss">
.library-view { padding: 20px 28px 32px; }

.lib-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 20px;
}

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
}

.header-action {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
}

/* M3 Filter Chips */
.tab-bar {
  display: flex;
  gap: 8px;
  margin-bottom: 20px;
}

.tab-chip {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 16px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  background: transparent;
  border: 1px solid transparent;
  transition: all var(--duration-short) var(--ease-standard);

  &:hover:not(.active) {
    background: var(--md-surface-container);
  }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    font-weight: 600;
  }
}

/* 新建歌单行（对齐 Android） */
.new-playlist-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 12px;
  font-size: 14px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  cursor: pointer;
  border-radius: var(--radius-md);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container); }

  &.disabled {
    opacity: 0.6;
    cursor: progress;
    pointer-events: none;
  }
}

.new-playlist-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.new-playlist-sub {
  max-width: 100%;
  font-size: 11px;
  line-height: 1.2;
  color: var(--md-on-surface-variant);
  opacity: 0.75;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.scan-error {
  margin: 6px 12px 2px;
  font-size: 12px;
  color: var(--md-error);
}

.spinning {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.list-divider {
  height: 1px;
  background: var(--md-outline-variant);
  opacity: 0.3;
  margin: 4px 12px;
}

/* 播放列表 */
.playlist-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.subsection-label {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
}

.playlist-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  cursor: pointer;
  transform: translate3d(0, 0, 0);
  filter: blur(0) saturate(1);
  isolation: isolate;
  transition:
    filter 240ms cubic-bezier(0.2, 0, 0, 1),
    transform 180ms cubic-bezier(0.2, 0, 0, 1),
    background 180ms cubic-bezier(0.2, 0, 0, 1),
    box-shadow 180ms cubic-bezier(0.2, 0, 0, 1),
    opacity 220ms cubic-bezier(0.2, 0, 0, 1);
  will-change: transform;

  &:hover { background: var(--md-surface-container); }

  &.selected { background: color-mix(in srgb, var(--md-primary) 12%, transparent); }
}

.playlist-item > * {
  position: relative;
  z-index: 1;
}

/* 拖影毛玻璃层：常态透明，抬起后淡入（与歌单详情页歌曲拖拽同款） */
.playlist-item::before {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 0;
  border-radius: inherit;
  pointer-events: none;
  opacity: 0;
  background: color-mix(in srgb, var(--md-surface-container-highest) 36%, transparent);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.38),
    inset 0 -1px 0 rgba(255, 255, 255, 0.12);
  backdrop-filter: blur(0) saturate(1);
  -webkit-backdrop-filter: blur(0) saturate(1);
  transition:
    opacity 180ms cubic-bezier(0.2, 0, 0, 1),
    background 180ms cubic-bezier(0.2, 0, 0, 1),
    backdrop-filter 180ms cubic-bezier(0.2, 0, 0, 1),
    -webkit-backdrop-filter 180ms cubic-bezier(0.2, 0, 0, 1);
}

/* 拖拽中的行：跟随指针平移并浮起 */
.playlist-item.dragging {
  z-index: 6;
  opacity: 0.96;
  transform: translate3d(0, var(--drag-offset, 0px), 0) scale(1.012);
  background: color-mix(in srgb, var(--md-surface-container-highest) 18%, transparent);
  box-shadow:
    0 18px 38px rgba(0, 0, 0, 0.18),
    0 4px 10px color-mix(in srgb, var(--md-primary) 10%, transparent);
  transition:
    background 180ms cubic-bezier(0.2, 0, 0, 1),
    box-shadow 180ms cubic-bezier(0.2, 0, 0, 1),
    opacity 180ms cubic-bezier(0.2, 0, 0, 1);
}

.playlist-item.dragging.glass-active::before {
  opacity: 1;
  background: color-mix(in srgb, var(--md-surface-container-highest) 34%, transparent);
  backdrop-filter: blur(28px) saturate(1.42);
  -webkit-backdrop-filter: blur(28px) saturate(1.42);
}

.playlist-item.drag-under-glass {
  filter: blur(0) saturate(1);
  opacity: 1;
  will-change: filter, opacity;
}

.playlist-item.drag-under-glass-active {
  filter: blur(2.8px) saturate(0.82);
  opacity: 0.72;
}

.playlist-item.landing {
  animation: pl-glass-shadow-release 320ms cubic-bezier(0.2, 0, 0, 1) both;
}

.playlist-item.landing::before {
  animation: pl-glass-release 320ms cubic-bezier(0.2, 0, 0, 1) both;
}

/* 目标行 ±5px 让路位移，靠基础 transform 过渡回弹 */
.playlist-item.drag-over-before {
  transform: translate3d(0, 5px, 0);
}

.playlist-item.drag-over-after {
  transform: translate3d(0, -5px, 0);
}

/* 单实例落点指示线：换目标时整体平移而非逐行显隐，杜绝闪烁 */
.drop-indicator {
  position: absolute;
  top: 0;
  left: 12px;
  right: 12px;
  z-index: 2;
  height: 3px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-primary) 88%, white);
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--md-primary) 12%, transparent);
  opacity: 0;
  pointer-events: none;
  transition:
    transform 170ms cubic-bezier(0.2, 0, 0, 1),
    opacity 120ms ease-out;

  &.visible {
    opacity: 1;
  }

  /* 隐藏转显示的首帧直接落位，只保留淡入 */
  &.snap {
    transition: opacity 120ms ease-out;
  }
}

@keyframes pl-glass-release {
  0% {
    opacity: 1;
    background: color-mix(in srgb, var(--md-surface-container-highest) 34%, transparent);
    backdrop-filter: blur(28px) saturate(1.42);
    -webkit-backdrop-filter: blur(28px) saturate(1.42);
  }
  100% {
    opacity: 0;
    background: color-mix(in srgb, var(--md-surface-container-highest) 16%, transparent);
    backdrop-filter: blur(0) saturate(1);
    -webkit-backdrop-filter: blur(0) saturate(1);
  }
}

@keyframes pl-glass-shadow-release {
  0% {
    box-shadow:
      0 18px 38px rgba(0, 0, 0, 0.18),
      0 4px 10px color-mix(in srgb, var(--md-primary) 10%, transparent);
  }
  100% {
    box-shadow: none;
  }
}

/* 多选：复选框 */
.pl-checkbox {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--md-primary);

  .material-symbols-rounded.filled { font-variation-settings: 'FILL' 1; }
}

/* 多选：排序摇杆（拖此手柄可排序，对齐 Android DragHandle） */
.pl-drag-handle {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  opacity: 0.7;
  cursor: grab;
  touch-action: none;
  transition: background var(--duration-short), opacity var(--duration-short);

  &:hover { opacity: 1; background: var(--md-surface-container-high); }
  &:active { cursor: grabbing; }
}

/* 多选：底部操作栏 */
.multi-select-bar {
  position: sticky;
  bottom: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 8px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  /* 与歌单详情页选择工具条同一套毛玻璃配方，保持质感一致 */
  background: color-mix(in srgb, var(--md-surface-container-high) 70%, transparent);
  -webkit-backdrop-filter: blur(24px) saturate(1.5);
  backdrop-filter: blur(24px) saturate(1.5);
  border: 1px solid color-mix(in srgb, var(--md-outline-variant) 60%, transparent);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
}

/* Linux WebKitGTK 无 backdrop-filter 时给不透明底色，避免列表穿透 */
@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .multi-select-bar {
    background: var(--md-surface-container-high);
  }
}

.select-count {
  font-size: 13px;
  font-weight: 600;
  color: var(--md-on-surface);
}

.multi-select-action {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 16px;
  border: 0;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background var(--duration-short), color var(--duration-short), opacity var(--duration-short);

  /* 删除按钮起把后续动作推到右侧 */
  &.push-right { margin-left: auto; }

  &.neutral {
    background: var(--md-surface-container-highest);
    color: var(--md-on-surface-variant);

    &:hover:not(:disabled) { background: var(--md-secondary-container); color: var(--md-on-secondary-container); }
  }

  &.ghost {
    background: transparent;
    color: var(--md-on-surface-variant);

    &:hover:not(:disabled) { background: var(--md-surface-container-highest); }
  }

  &.danger {
    background: color-mix(in srgb, var(--md-error) 14%, transparent);
    color: var(--md-error);

    &:hover:not(:disabled) { background: color-mix(in srgb, var(--md-error) 22%, transparent); }
  }

  &:disabled { opacity: 0.4; cursor: not-allowed; }
}

/* 音乐库头部操作按钮容器 */
.header-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.new-playlist {
  margin-bottom: 4px;

  .pl-name {
    color: var(--md-primary);
    font-weight: 600;
  }
}

.system-playlist {
  .pl-name { font-weight: 600; }
}

.pl-icon {
  width: 48px;
  height: 48px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--md-on-surface-variant);

  &.create {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }

  &.favorite {
    background: var(--md-tertiary-container);
    color: var(--md-on-tertiary-container);
  }

  &.local-files {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
  }

  &.has-cover {
    overflow: hidden;
    background: var(--md-surface-container-highest);
  }
}

.pl-info { flex: 1; min-width: 0; }

.pl-name {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pl-count {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  margin-top: 2px;
}

/* 歌单封面图 */
.pl-cover-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
}

.pl-icon.netease {
  background: #e74c3c20;
  overflow: hidden;
}

.pl-icon.bilibili {
  background: #00a1d620;
  overflow: hidden;
}

.pl-icon.youtube {
  background: #ff003320;
  overflow: hidden;
}


.platform-summary {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 16px;
  margin-bottom: 8px;
  border-radius: 20px;
  border: 1px solid var(--md-outline-variant);
  background:
    radial-gradient(circle at 12% 20%, color-mix(in srgb, var(--platform-color) 22%, transparent), transparent 34%),
    var(--md-surface-container);

  &.bilibili { --platform-color: #00a1d6; }
  &.youtube { --platform-color: #ff0033; }
}

.platform-icon-mask {
  display: block;
  width: 26px;
  height: 26px;
  background: var(--platform-color, var(--md-primary));
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
  flex-shrink: 0;
}

.platform-title {
  font-size: 14px;
  font-weight: 800;
}

.platform-desc {
  margin-top: 2px;
  font-size: 12px;
  color: var(--md-on-surface-variant);
}

/* 云端列表的刷新状态贴在标题行右侧，出现和消失都不挤动下面的列表 */
.platform-sync {
  margin-left: auto;
  flex-shrink: 0;
  align-self: center;
}

.empty-circle.platform-empty {
  opacity: 0.85;

  &.bilibili { --platform-color: #00a1d6; }
  &.youtube { --platform-color: #ff0033; }

  .platform-icon-mask {
    width: 40px;
    height: 40px;
  }
}


/* 分组分割线 */
.section-divider {
  display: flex;
  align-items: center;
  padding: 16px 12px 8px;
  gap: 10px;
}

.divider-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  white-space: nowrap;
}

.section-divider::after {
  content: '';
  flex: 1;
  height: 1px;
  background: var(--md-outline-variant);
  opacity: 0.4;
}

.pl-more {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  opacity: 0;
  transition: opacity var(--duration-short), background var(--duration-short);

  .playlist-item:hover & { opacity: 1; }
  &:hover { background: var(--md-surface-container-high); }

  &.danger {
    opacity: 1;
    color: var(--md-error);
  }
}

.active-download-item {
  cursor: default;
}

.download-progress-bar {
  position: relative;
  height: 6px;
  margin-top: 8px;
  border-radius: 999px;
  overflow: hidden;
  background: var(--md-surface-container-highest);
}

.download-progress-fill {
  height: 100%;
  min-width: 4px;
  border-radius: inherit;
  background: linear-gradient(90deg, var(--md-primary), color-mix(in srgb, var(--md-primary) 70%, white));
  transition: width 180ms ease;
}

.download-progress-bar.indeterminate .download-progress-fill {
  width: 36% !important;
  animation: download-indeterminate 1.2s ease-in-out infinite;
}

.download-progress-bar.error .download-progress-fill {
  background: var(--md-error);
}

.download-progress-bar.muted .download-progress-fill {
  background: var(--md-outline);
}

@keyframes download-indeterminate {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(280%);
  }
}

/* 空状态 */
.empty-tab {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 80px 0;
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
  opacity: 0.5;
}

.empty-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  margin-bottom: 4px;
}

.empty-desc {
  font-size: 13px;
  color: var(--md-on-surface-variant);
  opacity: 0.5;
}

/* 对话框描述文本 */
.dialog-msg {
  font-size: 14px;
  color: var(--md-on-surface-variant);
  line-height: 1.5;
}

// 本地库：歌单 / 歌手 分类
.local-category-bar {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}

.local-category-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  border: 1px solid var(--md-outline-variant, rgba(255, 255, 255, 0.12));
  transition:
    background var(--duration-short) var(--easing-standard, ease),
    color var(--duration-short) var(--easing-standard, ease),
    border-color var(--duration-short) var(--easing-standard, ease);

  &:hover { background: var(--md-surface-container-high); }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    border-color: transparent;
  }
}

// 排序按钮常驻在分类 chips 行右侧，仅在歌手视图内出现
.artist-sort-btn {
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  margin-left: auto;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short), color var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }

  // 排序 chips 展开时按钮保持高亮，而不是消失
  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
  }
}

.artist-sort-options {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}

.artist-sort-option {
  padding: 6px 14px;
  border-radius: var(--radius-full);
  font-size: 12px;
  color: var(--md-on-surface-variant);
  background: var(--md-surface-container-high);
  transition: background var(--duration-short), color var(--duration-short);

  &:hover { background: var(--md-surface-container-highest); }
  &.active { background: var(--md-secondary-container); color: var(--md-on-secondary-container); }
}

.artist-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(124px, 1fr));
  gap: 16px 14px;
  padding-bottom: 12px;
}

.artist-card {
  cursor: pointer;
  min-width: 0;
  border-radius: var(--radius-lg, 16px);
  outline: none;

  &:focus-visible { box-shadow: 0 0 0 2px var(--md-primary); }
  &:hover .artist-play { opacity: 1; transform: translateY(0) scale(1); }
  &:hover .artist-cover { transform: translateY(-2px); }
}

.artist-cover {
  position: relative;
  aspect-ratio: 1;
  border-radius: var(--radius-full);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--md-surface-container-high);
  color: var(--md-on-surface-variant);
  // 只动 transform，避免触发布局
  transition: transform 200ms var(--easing-emphasized, cubic-bezier(0.2, 0, 0, 1));

  :deep(img) { width: 100%; height: 100%; object-fit: cover; }
}

.artist-play {
  position: absolute;
  right: 8px;
  bottom: 8px;
  width: 34px;
  height: 34px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--md-primary);
  color: var(--md-on-primary);
  opacity: 0;
  transform: translateY(6px) scale(0.9);
  transition:
    opacity 160ms var(--easing-standard, ease),
    transform 200ms var(--easing-emphasized, cubic-bezier(0.2, 0, 0, 1));

  &:focus-visible { opacity: 1; transform: translateY(0) scale(1); }
}

.artist-name {
  margin-top: 8px;
  font-size: 13px;
  font-weight: 500;
  text-align: center;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.artist-count {
  font-size: 11px;
  text-align: center;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
}

.favorite-artist-header {
  padding: 18px;
  margin-bottom: 12px;
  border: 1px solid var(--md-outline-variant);
  border-radius: 24px;
  background: var(--md-surface-container);
}

.favorite-artist-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;

  h2 { font-size: 16px; font-weight: 600; margin: 0; }
  span { color: var(--md-primary); font-size: 13px; font-weight: 600; }
}

.favorite-artist-platforms {
  display: flex;
  overflow: hidden;
  border: 1px solid var(--md-outline);
  border-radius: var(--radius-full);

  button {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 40px;
    font-size: 13px;
    color: var(--md-on-surface-variant);

    + button { border-left: 1px solid var(--md-outline); }
    &:hover { background: var(--md-surface-container-high); }
    &.active { background: var(--md-secondary-container); color: var(--md-on-secondary-container); }
    .material-symbols-rounded { font-size: 18px; }
  }
}

.favorite-artist-import {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  width: 100%;
  min-height: 40px;
  margin-top: 12px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-full);
  color: var(--md-primary);
  font-size: 13px;
  font-weight: 500;

  &:hover:not(:disabled) { background: var(--md-surface-container-high); }
  &:disabled { opacity: 0.5; cursor: progress; }
  .material-symbols-rounded { font-size: 19px; }
}

.favorite-artist-item {
  .pl-icon { width: 56px; height: 56px; border-radius: 50%; }
  .pl-name { font-size: 15px; }
}

@media (prefers-reduced-motion: reduce) {
  .artist-cover,
  .artist-play { transition: none; }
}

/* 各 tab 搜索栏 */
.tab-search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 40px;
  padding: 0 14px;
  margin-bottom: 14px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface-variant);

  input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    font-size: 13px;
    font-family: inherit;
    color: var(--md-on-surface);

    &::placeholder { color: var(--md-on-surface-variant); opacity: 0.7; }
    &::-webkit-search-cancel-button { -webkit-appearance: none; }
  }
}

.tab-search-clear {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-highest); }
}

/* ---- 页面统一过渡 ----
   时长/曲线复用全局动效 token（global.scss）：进入 --ease-decelerate、
   离开 --ease-accelerate，与全局 fade 过渡的节奏一致 */

// 可折叠行：grid-template-rows 0fr↔1fr 实现内容自适应高度的展开收起
.collapse-row {
  display: grid;
  grid-template-rows: 1fr;
}

.collapse-clip {
  min-height: 0;
  overflow: hidden;
}

.lib-collapse-enter-active,
.lib-collapse-leave-active {
  transition:
    grid-template-rows 240ms var(--ease-standard),
    opacity 240ms var(--ease-standard);

  .collapse-clip > * { transition: transform 240ms var(--ease-standard); }
}

.lib-collapse-enter-from,
.lib-collapse-leave-to {
  grid-template-rows: 0fr;
  opacity: 0;

  .collapse-clip > * { transform: translateY(-6px); }
}

// 容器级交叉淡入：用于大网格重排与整块内容的状态切换
.lib-fade-enter-active { transition: opacity 200ms var(--ease-decelerate); }
.lib-fade-leave-active { transition: opacity 120ms var(--ease-accelerate); }
.lib-fade-enter-from,
.lib-fade-leave-to { opacity: 0; }

// 多选底部操作栏：上滑淡入 / 下滑淡出
.lib-bar-enter-active,
.lib-bar-leave-active {
  transition:
    opacity 200ms var(--ease-standard),
    transform 200ms var(--ease-standard);
}
.lib-bar-enter-from,
.lib-bar-leave-to { opacity: 0; transform: translateY(12px); }

// 小控件出现 / 消失：淡入 + 缩放（排序按钮）
.lib-zoom-enter-active {
  transition:
    opacity 200ms var(--ease-decelerate),
    transform 200ms var(--ease-decelerate);
}
.lib-zoom-leave-active {
  transition:
    opacity 120ms var(--ease-accelerate),
    transform 120ms var(--ease-accelerate);
}
.lib-zoom-enter-from,
.lib-zoom-leave-to { opacity: 0; transform: scale(0.8); }

// 列表行：进入淡入、移除时脱离文档流让兄弟行平滑补位、重排走 FLIP move
.lib-list {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.lib-list-enter-active {
  transition:
    opacity 200ms var(--ease-decelerate),
    transform 200ms var(--ease-decelerate);
}
.lib-list-leave-active {
  position: absolute;
  width: 100%;
  transition: opacity 120ms var(--ease-accelerate);
}
.lib-list-enter-from { opacity: 0; transform: translateY(6px); }
.lib-list-leave-to { opacity: 0; }
.lib-list-move { transition: transform var(--duration-medium) var(--ease-standard); }

// 拖拽释放后行归位的 FLIP move 过渡（拖拽中切到 lib-static 无动画 name，
// 避免 move 位移与指针跟随打架），参数对齐歌单详情页歌曲拖拽
.playlist-item.lib-list-move { transition: transform 300ms cubic-bezier(0.2, 0, 0, 1); }

@media (prefers-reduced-motion: reduce) {
  .lib-collapse-enter-active,
  .lib-collapse-leave-active,
  .lib-collapse-enter-active .collapse-clip > *,
  .lib-fade-enter-active,
  .lib-fade-leave-active,
  .lib-bar-enter-active,
  .lib-bar-leave-active,
  .lib-zoom-enter-active,
  .lib-zoom-leave-active,
  .lib-list-enter-active,
  .lib-list-leave-active,
  .lib-list-move,
  .playlist-item.lib-list-move { transition: none; }
}
</style>
