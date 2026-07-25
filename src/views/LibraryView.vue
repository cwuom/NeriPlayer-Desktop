<script setup lang="ts">
import { ref, reactive, computed, onMounted, onUnmounted, nextTick, watch } from 'vue'
import { RouterLink, useRouter, useRoute } from 'vue-router'

defineOptions({ name: 'LibraryView' })
import { useI18n } from 'vue-i18n'
import { useLibraryStore } from '@/stores/library'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useRecommendStore } from '@/stores/recommend'
import { useAuthStore } from '@/stores/auth'
import { useDownloadStore } from '@/stores/download'
import { useToastStore } from '@/stores/toast'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import { createLogger } from '@/utils/logger'
import {
  buildLibraryQuery,
  isCanonicalLibraryLocation,
  resolveLibraryLocation,
  type LibraryTabKey,
  type NeteaseLibraryCategory,
} from '@/modules/library/libraryRoute'
import { filterNeteaseAlbums, filterNeteasePlaylists } from '@/modules/library/neteaseLibraryFilter'
import { NeteaseLibraryRequestCoordinator } from '@/modules/library/neteaseLibraryRequest'
import {
  favoritePlaylistKey,
  favoritePlaylistLocation,
  normalizeFavoritePlaylist,
  type FavoritePlaylist,
} from '@/modules/library/favoritePlaylists'

const log = createLogger('library-view')

const router = useRouter()
const route = useRoute()
const { t } = useI18n()
const library = useLibraryStore()
const player = usePlayerStore()
const recommend = useRecommendStore()
const auth = useAuthStore()
const downloadStore = useDownloadStore()
const toast = useToastStore()

// 喜欢的歌曲计数
const likedCount = computed(() => recommend.likedSongIds.size)

interface LibraryTabDefinition {
  label: string
  icon: string
  key: LibraryTabKey
}

interface NeteaseCategoryDefinition {
  label: string
  key: NeteaseLibraryCategory
}

const tabs = computed<LibraryTabDefinition[]>(() => [
  { label: t('library.tab_local'), icon: 'folder_open', key: 'local' },
  { label: t('library.tab_favorites'), icon: 'favorite', key: 'favorites' },
  { label: t('library.tab_downloads'), icon: 'download', key: 'downloads' },
  { label: t('library.tab_netease'), icon: 'cloud', key: 'netease' },
  { label: t('settings.bilibili_account'), icon: 'video_library', key: 'bilibili' },
  { label: t('settings.youtube_account'), icon: 'subscriptions', key: 'youtube' },
])

const neteaseCategories = computed<NeteaseCategoryDefinition[]>(() => [
  { label: t('library.netease_category_playlists'), key: 'playlists' },
  { label: t('library.netease_category_albums'), key: 'albums' },
])

const initialLocation = resolveLibraryLocation(route.query.tab, route.query.category)
const activeTab = ref<LibraryTabKey>(initialLocation.tab)
const neteaseCategory = ref<NeteaseLibraryCategory>(initialLocation.category)

function writeLibraryLocation(location: { tab: LibraryTabKey; category: NeteaseLibraryCategory }) {
  activeTab.value = location.tab
  neteaseCategory.value = location.category

  const { tab: _tab, category: _category, ...unrelatedQuery } = route.query
  void router.replace({
    query: {
      ...unrelatedQuery,
      ...buildLibraryQuery(location),
    },
  })
}

function activateTab(tab: LibraryTabKey) {
  writeLibraryLocation({ tab, category: neteaseCategory.value })
}

function activateNeteaseCategory(category: NeteaseLibraryCategory) {
  writeLibraryLocation({ tab: 'netease', category })
}

function handleNeteaseCategoryKeydown(event: KeyboardEvent, current: NeteaseLibraryCategory) {
  let nextCategory: NeteaseLibraryCategory

  switch (event.key) {
    case 'ArrowRight':
    case 'ArrowDown':
      nextCategory = current === 'playlists' ? 'albums' : 'playlists'
      break
    case 'ArrowLeft':
    case 'ArrowUp':
      nextCategory = current === 'albums' ? 'playlists' : 'albums'
      break
    case 'Home':
      nextCategory = 'playlists'
      break
    case 'End':
      nextCategory = 'albums'
      break
    default:
      return
  }

  event.preventDefault()
  activateNeteaseCategory(nextCategory)
  nextTick(() => document.getElementById(`netease-category-${nextCategory}`)?.focus())
}

// 监听路由 query 变化（同页面内导航）
watch(
  () => [route.query.tab, route.query.category] as const,
  ([tab, category]) => {
    const location = resolveLibraryLocation(tab, category)
    activeTab.value = location.tab
    if (location.tab === 'netease') {
      neteaseCategory.value = location.category
    }

    if (!isCanonicalLibraryLocation(tab, category)) writeLibraryLocation(location)
  },
  { immediate: true },
)

// 真实播放列表
interface PlaylistInfo { id: number; name: string; track_count: number; modified_at: number; cover_url: string | null }
const playlists = ref<PlaylistInfo[]>([])

// 多选模式
const isMultiSelectMode = ref(false)
const selectedPlaylists = ref<Set<number>>(new Set())

watch(activeTab, (tab) => {
  if (tab !== 'local' && isMultiSelectMode.value) exitMultiSelect()
})

function enterMultiSelect() {
  isMultiSelectMode.value = true
  selectedPlaylists.value.clear()
}
function exitMultiSelect() {
  isMultiSelectMode.value = false
  selectedPlaylists.value.clear()
}
function togglePlaylistSelection(id: number) {
  const playlist = playlists.value.find(pl => pl.id === id)
  if (!playlist || isProtectedPlaylist(playlist)) return
  const set = selectedPlaylists.value
  if (set.has(id)) set.delete(id)
  else set.add(id)
}
function selectAll() {
  for (const pl of playlists.value) {
    if (!isProtectedPlaylist(pl)) {
      selectedPlaylists.value.add(pl.id)
    }
  }
}
function invertSelection() {
  for (const pl of playlists.value) {
    if (isProtectedPlaylist(pl)) continue
    if (selectedPlaylists.value.has(pl.id)) selectedPlaylists.value.delete(pl.id)
    else selectedPlaylists.value.add(pl.id)
  }
  log.info('invert selection ->', selectedPlaylists.value.size, 'selected')
}

// 拖拽排序
const dragIndex = ref<number | null>(null)
const dragOverIndex = ref<number | null>(null)

function onDragStart(e: DragEvent, index: number) {
  dragIndex.value = index
  // 让拖拽 ghost 显示整行而非仅手柄，贴近 Android 抬起整行的手感
  const handle = e.currentTarget as HTMLElement | null
  const row = handle?.closest('.playlist-item') as HTMLElement | null
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    if (row) e.dataTransfer.setDragImage(row, 24, row.offsetHeight / 2)
  }
}
function onDragOver(e: DragEvent, index: number) {
  e.preventDefault()
  dragOverIndex.value = index
}
function onDragEnd() {
  if (dragIndex.value !== null && dragOverIndex.value !== null && dragIndex.value !== dragOverIndex.value) {
    const arr = [...playlists.value]
    const [moved] = arr.splice(dragIndex.value, 1)
    arr.splice(dragOverIndex.value, 0, moved)
    playlists.value = arr
    savePlaylistOrder(arr)
  }
  dragIndex.value = null
  dragOverIndex.value = null
}

function savePlaylistOrder(ordered: PlaylistInfo[]) {
  const orderIds = ordered.map(p => p.id)
  localStorage.setItem('neri:playlist-order', JSON.stringify(orderIds))
  log.info('Playlist order saved:', orderIds)
}
function loadPlaylistOrder(): number[] | null {
  try {
    const raw = localStorage.getItem('neri:playlist-order')
    return raw ? JSON.parse(raw) : null
  } catch { return null }
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
    const raw = await invoke<PlaylistInfo[]>('list_playlists')
    // 排序：「我喜欢的音乐」置顶，「本地音乐」置底，其余保持原序
    const liked: PlaylistInfo[] = []
    const localFiles: PlaylistInfo[] = []
    const normal: PlaylistInfo[] = []
    for (const pl of raw) {
      if (isLikedPlaylist(pl)) liked.push(pl)
      else if (isLocalFilesPlaylist(pl)) localFiles.push(pl)
      else normal.push(pl)
    }
    // 应用用户自定义排序
    const savedOrder = loadPlaylistOrder()
    if (savedOrder && savedOrder.length > 0) {
      const orderMap = new Map(savedOrder.map((id, idx) => [id, idx]))
      normal.sort((a, b) => {
        const ai = orderMap.get(a.id) ?? Number.MAX_SAFE_INTEGER
        const bi = orderMap.get(b.id) ?? Number.MAX_SAFE_INTEGER
        return ai - bi
      })
    }
    playlists.value = [...liked, ...normal, ...localFiles]
  } catch (e) {
    log.error('Load playlists failed:', e)
  }
}

function inferTrackSource(track: TrackInfo) {
  if (track.id.startsWith('netease:')) return 'netease'
  if (track.id.startsWith('qq:')) return 'qq'
  if (track.id.startsWith('bilibili:')) return 'bilibili'
  if (track.id.startsWith('youtube:')) return 'youtube'
  return 'local'
}

function toBackendTrack(track: TrackInfo) {
  return {
    id: track.id,
    title: track.title,
    artist: track.artist,
    album: track.album || '',
    duration_ms: track.durationMs || 0,
    source: inferTrackSource(track),
    url: track.audioUrl || '',
    cover_url: track.coverUrl || null,
    added_at: Math.max(0, Math.round(track.addedAt || 0)),
    sync_payload: track.syncPayload ?? null,
    playlist_key: track.playlistKey ?? null,
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
  }
}

// 上下文菜单
const contextMenu = ref<{ show: boolean; x: number; y: number; playlist: PlaylistInfo | null }>({
  show: false, x: 0, y: 0, playlist: null,
})

// 特殊歌单：跨语言匹配（同步数据可能是任何语言的名称）
const SYSTEM_LIKED_PLAYLIST_ID = -1001
const SYSTEM_LOCAL_PLAYLIST_ID = -1002
const LIKED_NAMES = ['我喜欢的音乐', '我喜歡的音樂', 'お気に入りの曲', 'Liked Songs', 'My Favorite Music']
const LOCAL_NAMES = ['本地文件', '本地音乐', '本機音樂', 'ローカルファイル', 'ローカル音楽', 'Local Files', 'Local Music']

function isLikedPlaylist(pl: PlaylistInfo): boolean {
  return pl.id === SYSTEM_LIKED_PLAYLIST_ID || LIKED_NAMES.includes(pl.name)
}

function isLocalFilesPlaylist(pl: PlaylistInfo): boolean {
  return pl.id === SYSTEM_LOCAL_PLAYLIST_ID || LOCAL_NAMES.includes(pl.name)
}

async function ensureLocalPlaylistId(): Promise<number> {
  const raw = await invoke<PlaylistInfo[]>('list_playlists')
  const localPlaylist = raw.find(isLocalFilesPlaylist)
  if (localPlaylist) return localPlaylist.id

  const created = await invoke<PlaylistInfo>('create_playlist', { name: t('home.local_music') })
  return created.id
}

async function syncScannedTracksToLocalPlaylist(scannedTracks: TrackInfo[]) {
  const localPlaylistId = await ensureLocalPlaylistId()

  const existingTracks = await invoke<any[]>('get_playlist_tracks', { id: localPlaylistId })
  const existingIds = (existingTracks || [])
    .map((track: any) => String(track?.id || ''))
    .filter(Boolean)

  if (existingIds.length > 0) {
    await invoke('remove_tracks_from_playlist', {
      playlistId: localPlaylistId,
      trackIds: existingIds,
    })
  }

  if (scannedTracks.length > 0) {
    await invoke('add_tracks_to_playlist', {
      playlistId: localPlaylistId,
      tracks: scannedTracks.map(toBackendTrack),
    })
  }

  await loadPlaylists()
}

async function selectAndScanLocalMusic() {
  if (library.isScanning) return
  try {
    const result = await dialogOpen({ directory: true, multiple: false })
    if (!result) return

    const dir = typeof result === 'string' ? result : (result as any).path || String(result)
    if (!dir || dir === '[object Object]') return

    await library.scanDirectory(dir)
    if (library.scanError) {
      toast.error(t('library.scan_failed'))
      return
    }

    await syncScannedTracksToLocalPlaylist(library.tracks)
    toast.success(t('library.scan_success', { count: library.tracks.length }))
  } catch (e) {
    log.error('Scan local music failed:', e)
    toast.error(t('library.scan_failed'))
  }
}

function isProtectedPlaylist(pl: PlaylistInfo) {
  return isLikedPlaylist(pl) || isLocalFilesPlaylist(pl)
}

// 显示名：特殊歌单用当前语言翻译，其他原样
function displayName(pl: PlaylistInfo): string {
  if (isLikedPlaylist(pl)) return t('library.liked_songs')
  if (isLocalFilesPlaylist(pl)) return t('library.local_files')
  return pl.name
}

function openContextMenu(e: MouseEvent, pl: PlaylistInfo) {
  if (isProtectedPlaylist(pl)) return
  const btn = e.currentTarget as HTMLElement
  const rect = btn.getBoundingClientRect()
  let x = rect.left - 204
  if (x < 8) x = rect.right + 4
  contextMenu.value = { show: true, x, y: rect.top, playlist: pl }
}

function closeContextMenu() {
  contextMenu.value.show = false
}

// 删除确认
const showDeleteDialog = ref(false)
const deleteTarget = ref<PlaylistInfo | null>(null)

function requestDelete(pl: PlaylistInfo) {
  if (isProtectedPlaylist(pl)) return
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
  }
}

// 重命名
const showRenameDialog = ref(false)
const renameTarget = ref<PlaylistInfo | null>(null)
const renameValue = ref('')
const renameInputRef = ref<InstanceType<typeof M3Input>>()

function requestRename(pl: PlaylistInfo) {
  if (isProtectedPlaylist(pl)) return
  closeContextMenu()
  renameTarget.value = pl
  renameValue.value = pl.name
  showRenameDialog.value = true
  nextTick(() => renameInputRef.value?.focus())
}

const playlistMenuItems = computed<ContextMenuItem[]>(() => {
  const playlist = contextMenu.value.playlist
  return [
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
  }
}

// 批量删除确认
const showBatchDeleteDialog = ref(false)

function requestDeleteSelected() {
  if (selectedPlaylists.value.size === 0) return
  showBatchDeleteDialog.value = true
}

async function confirmDeleteSelected() {
  const protectedIds = new Set(playlists.value.filter(isProtectedPlaylist).map(pl => pl.id))
  const ids = [...selectedPlaylists.value].filter(id => !protectedIds.has(id))
  try {
    for (const id of ids) {
      await invoke('delete_playlist', { id })
    }
    showBatchDeleteDialog.value = false
    exitMultiSelect()
    await loadPlaylists()
  } catch (e) {
    log.error('Batch delete playlists failed:', e)
  }
}

// 收藏歌单（从同步数据中获取）
// 网易云用户歌单及收藏专辑
const neteasePlaylists = computed(() => recommend.userPlaylists['netease'] || [])
const neteasePlaylistSearchQuery = ref('')
const neteaseAlbumSearchQuery = ref('')
const neteaseFilteredPlaylists = computed(() => filterNeteasePlaylists(neteasePlaylists.value, neteasePlaylistSearchQuery.value))
const neteaseFilteredAlbums = computed(() => filterNeteaseAlbums(recommend.userAlbums, neteaseAlbumSearchQuery.value))
const activeNeteaseSearchQuery = computed({
  get: () => neteaseCategory.value === 'playlists' ? neteasePlaylistSearchQuery.value : neteaseAlbumSearchQuery.value,
  set: (value: string) => {
    if (neteaseCategory.value === 'playlists') neteasePlaylistSearchQuery.value = value
    else neteaseAlbumSearchQuery.value = value
  },
})
const neteasePlaylistLoading = ref(false)
const neteaseAlbumLoading = ref(false)
const neteasePlaylistError = ref<string | null>(null)
const neteaseAlbumError = ref<string | null>(null)
const neteaseLibraryRequestCoordinator = new NeteaseLibraryRequestCoordinator()

async function loadNeteaseLibrary() {
  if (!auth.netease.loggedIn) return

  const request = neteaseLibraryRequestCoordinator.run(
    () => recommend.fetchUserPlaylists('netease'),
    () => recommend.fetchUserAlbums(),
  )
  if (!request.started) return request.promise

  neteasePlaylistLoading.value = true
  neteaseAlbumLoading.value = true
  neteasePlaylistError.value = null
  neteaseAlbumError.value = null

  const result = await request.promise
  if (!result.current || !auth.netease.loggedIn) return result

  neteasePlaylistLoading.value = false
  neteaseAlbumLoading.value = false
  neteasePlaylistError.value = result.playlistsOk ? null : t('player.load_failed')
  neteaseAlbumError.value = result.albumsOk ? null : t('player.load_failed')
  return result
}

type CloudLibraryPlatform = 'bilibili' | 'youtube'

const bilibiliPlaylists = computed(() => recommend.userPlaylists.bilibili || [])
const youtubePlaylists = computed(() => recommend.userPlaylists.youtube || [])
const cloudLibraryLoading = reactive<Record<CloudLibraryPlatform, boolean>>({
  bilibili: false,
  youtube: false,
})
const cloudLibraryError = reactive<Record<CloudLibraryPlatform, string | null>>({
  bilibili: null,
  youtube: null,
})
const cloudLibraryRequestGeneration: Record<CloudLibraryPlatform, number> = {
  bilibili: 0,
  youtube: 0,
}

function isCloudPlatformLoggedIn(platform: CloudLibraryPlatform): boolean {
  return platform === 'bilibili' ? auth.bilibili.loggedIn : auth.youtube.loggedIn
}

async function loadCloudLibrary(platform: CloudLibraryPlatform) {
  if (!isCloudPlatformLoggedIn(platform)) return

  const requestGeneration = ++cloudLibraryRequestGeneration[platform]
  cloudLibraryLoading[platform] = true
  cloudLibraryError[platform] = null
  const loaded = await recommend.fetchUserPlaylists(platform)
  if (
    requestGeneration !== cloudLibraryRequestGeneration[platform]
    || !isCloudPlatformLoggedIn(platform)
  ) return

  cloudLibraryLoading[platform] = false
  cloudLibraryError[platform] = loaded ? null : t('player.load_failed')
  if (!loaded && (recommend.userPlaylists[platform]?.length || 0) > 0) {
    toast.error(t('player.load_failed'))
  }
}

function resetCloudLibrary(platform: CloudLibraryPlatform) {
  cloudLibraryRequestGeneration[platform]++
  cloudLibraryLoading[platform] = false
  cloudLibraryError[platform] = null
  recommend.userPlaylists[platform] = []
  if (isCloudPlatformLoggedIn(platform)) void loadCloudLibrary(platform)
}

const favoritePlaylists = ref<FavoritePlaylist[]>([])
const favoritePlaylistRows = computed(() => favoritePlaylists.value.map(favorite => ({
  ...favorite,
  key: favoritePlaylistKey(favorite),
  location: favoritePlaylistLocation(favorite),
})))
let favoritePlaylistsRequestGeneration = 0

async function loadFavorites() {
  const requestGeneration = ++favoritePlaylistsRequestGeneration
  try {
    const raw = await invoke<any[]>('list_favorite_playlists')
    if (requestGeneration !== favoritePlaylistsRequestGeneration) return
    favoritePlaylists.value = (raw || [])
      .map(normalizeFavoritePlaylist)
      .filter(favorite => !favorite.isDeleted)
  } catch (e) {
    log.error('Load favorites failed:', e)
  }
}

onMounted(loadPlaylists)
onMounted(() => downloadStore.loadDownloads())

// 下载相关
function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}

function playDownloadedTrack(dl: any) {
  player.play({
    id: dl.id,
    title: dl.title,
    artist: dl.artist,
    album: dl.album,
    durationMs: dl.durationMs,
    coverUrl: dl.coverUrl || '',
    audioUrl: dl.filePath,
  })
}

// 下载列表右键菜单
const dlContextMenu = ref<{ show: boolean; x: number; y: number; track: any | null }>({
  show: false, x: 0, y: 0, track: null,
})

function openDlContextMenu(e: MouseEvent, track: any) {
  const btn = e.currentTarget as HTMLElement
  const rect = btn.getBoundingClientRect()
  let x = rect.left - 204
  if (x < 8) x = rect.right + 4
  dlContextMenu.value = { show: true, x, y: rect.top, track }
}

function closeDlContextMenu() {
  dlContextMenu.value.show = false
}

// 删除下载确认
const showDlDeleteDialog = ref(false)
const dlDeleteTarget = ref<any>(null)

async function cancelActiveDownload(trackId: string) {
  await downloadStore.cancelDownload(trackId)
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

  const downloaded = typeof task.downloadedBytes === 'number' ? formatFileSize(task.downloadedBytes) : null
  const total = typeof task.totalBytes === 'number' && task.totalBytes > 0 ? formatFileSize(task.totalBytes) : null
  const percent = typeof task.progress === 'number' ? `${task.progress}%` : null

  if (downloaded && total && percent) return `${activeDownloadStatusText(task.status)} · ${downloaded} / ${total} · ${percent}`
  if (downloaded && total) return `${activeDownloadStatusText(task.status)} · ${downloaded} / ${total}`
  if (downloaded && percent) return `${activeDownloadStatusText(task.status)} · ${downloaded} · ${percent}`
  return activeDownloadStatusText(task.status)
}

function requestDlDelete(track: any) {
  if (isDownloadedTrackInUse(track)) return
  closeDlContextMenu()
  dlDeleteTarget.value = track
  showDlDeleteDialog.value = true
}

async function revealDownloadFile(track: any) {
  closeDlContextMenu()
  try {
    await invoke('reveal_file', { path: track.filePath })
  } catch (e) {
    log.error('Failed to reveal file:', e)
  }
}

async function confirmDlDelete() {
  if (!dlDeleteTarget.value) return
  const deleted = dlDeleteTarget.value
  await downloadStore.deleteDownload(deleted.id)
  player.handleDownloadedFileRemoved(deleted.id, deleted.filePath)
  showDlDeleteDialog.value = false
  dlDeleteTarget.value = null
}

function isDownloadedTrackInUse(track: any) {
  return player.isPlayingFromDownload && player.currentTrack?.id === track?.id
}

async function redownloadDownloadedTrack(track: any) {
  if (isDownloadedTrackInUse(track)) return
  closeDlContextMenu()
  player.handleDownloadedFileRemoved(track.id, track.filePath)
  await downloadStore.redownloadTrack({
    id: track.id,
    title: track.title,
    artist: track.artist,
    album: track.album || '',
    durationMs: track.durationMs || 0,
    coverUrl: track.coverUrl || '',
    audioUrl: '',
  })
}

const downloadMenuItems = computed<ContextMenuItem[]>(() => {
  const track = dlContextMenu.value.track
  const isInUse = track ? isDownloadedTrackInUse(track) : true
  return [
    createContextMenuItem(t('download.open_folder'), {
      id: 'reveal',
      icon: 'folder_open',
      disabled: !track,
    }),
    createContextMenuItem(t('download.redownload'), {
      id: 'redownload',
      icon: 'refresh',
      disabled: !track || isInUse,
    }),
    createContextMenuItem(t('common.delete'), {
      id: 'delete',
      icon: 'delete',
      danger: true,
      disabled: !track || isInUse,
    }),
  ]
})

function handleDownloadMenuClick(item: ContextMenuActionItem) {
  const track = dlContextMenu.value.track
  if (!track) return

  switch (item.id) {
    case 'reveal':
      void revealDownloadFile(track)
      break
    case 'redownload':
      void redownloadDownloadedTrack(track)
      break
    case 'delete':
      requestDlDelete(track)
      break
  }
}

const neteaseSessionFingerprint = computed(() => `${auth.netease.loggedIn ? '1' : '0'}:${auth.neteaseSessionVersion}`)
const bilibiliSessionFingerprint = computed(() => `${auth.bilibili.loggedIn ? '1' : '0'}:${auth.bilibiliSessionVersion}`)
const youtubeSessionFingerprint = computed(() => `${auth.youtube.loggedIn ? '1' : '0'}:${auth.youtubeSessionVersion}`)

// Login restoration and account changes must invalidate stale Netease library requests together.
watch(neteaseSessionFingerprint, () => {
  neteaseLibraryRequestCoordinator.invalidate()
  neteasePlaylistLoading.value = false
  neteaseAlbumLoading.value = false
  neteasePlaylistError.value = null
  neteaseAlbumError.value = null
  recommend.userPlaylists['netease'] = []
  recommend.userAlbums = []

  if (auth.netease.loggedIn) {
    void loadNeteaseLibrary()
  }
}, { immediate: true })

watch(bilibiliSessionFingerprint, () => resetCloudLibrary('bilibili'), { immediate: true })
watch(youtubeSessionFingerprint, () => resetCloudLibrary('youtube'), { immediate: true })

// 监听同步完成后的歌单变更事件
let unlistenPlaylistsChanged: UnlistenFn | null = null
let unlistenFavoritePlaylistsChanged: UnlistenFn | null = null
onMounted(async () => {
  try {
    unlistenPlaylistsChanged = await listen('playlists-changed', () => {
      void loadPlaylists()
    })
  } catch {
    // Browser preview has no Tauri event bridge.
  }
  try {
    unlistenFavoritePlaylistsChanged = await listen('favorite-playlists-changed', () => {
      void loadFavorites()
    })
  } catch {
    // Browser preview has no Tauri event bridge.
  }
  void loadFavorites()
})
onUnmounted(() => {
  favoritePlaylistsRequestGeneration++
  cloudLibraryRequestGeneration.bilibili++
  cloudLibraryRequestGeneration.youtube++
  unlistenPlaylistsChanged?.()
  unlistenFavoritePlaylistsChanged?.()
})
</script>

<template>
  <div class="library-view">
    <header class="lib-header">
      <h1 class="page-title">{{ t('library.title') }}</h1>
      <div class="header-actions">
        <button v-if="activeTab === 'local' && !isMultiSelectMode" class="header-action" @click="enterMultiSelect" :title="t('common.multi_select')">
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
        v-for="tab in tabs"
        :key="tab.key"
        class="tab-chip"
        :class="{ active: activeTab === tab.key }"
        :aria-pressed="activeTab === tab.key"
        @click="activateTab(tab.key)"
      >
        <span class="material-symbols-rounded tab-chip-icon" :class="{ filled: activeTab === tab.key }">{{ tab.icon }}</span>
        <span class="tab-chip-label">{{ tab.label }}</span>
      </button>
    </div>

    <!-- Tab: 本地 -->
    <div v-if="activeTab === 'local'" class="playlist-list">
      <div class="new-playlist-row" :class="{ disabled: library.isScanning }" @click="selectAndScanLocalMusic">
        <span class="material-symbols-rounded" :class="{ spinning: library.isScanning }" style="font-size: 20px">
          {{ library.isScanning ? 'progress_activity' : 'folder_open' }}
        </span>
        <div class="new-playlist-copy">
          <span>{{ library.isScanning ? t('library.scanning') : t('library.scan_local_music') }}</span>
          <small v-if="library.lastScanDir" class="new-playlist-sub">{{ library.lastScanDir }}</small>
        </div>
      </div>
      <!-- 新建歌单（对齐 Android：+ 新建歌单 行） -->
      <div class="new-playlist-row" @click="openCreateDialog">
        <span class="material-symbols-rounded" style="font-size: 20px">add</span>
        <span>{{ t('library.create_playlist') }}</span>
      </div>
      <div class="list-divider" />
      <p v-if="library.scanError" class="scan-error">{{ t('library.scan_failed') }}</p>

      <!-- 歌单列表 -->
      <div
        v-for="(pl, index) in playlists"
        :key="pl.id"
        class="playlist-item"
        :class="{
          'drag-over': dragOverIndex === index,
          dragging: dragIndex === index,
          selected: selectedPlaylists.has(pl.id),
        }"
        @dragover="onDragOver($event, index)"
        @click="isMultiSelectMode ? togglePlaylistSelection(pl.id) : router.push({ name: 'local-playlist', params: { id: pl.id } })"
        @contextmenu.prevent.stop="openContextMenu($event, pl)"
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
          draggable="true"
          :title="t('common.drag_to_reorder')"
          @dragstart.stop="onDragStart($event, index)"
          @dragend.stop="onDragEnd()"
          @click.stop
        >drag_handle</span>
        <!-- 受保护歌单不显示三点菜单 -->
        <button v-else-if="!isProtectedPlaylist(pl) && !isMultiSelectMode" class="pl-more" @click.stop="openContextMenu($event, pl)">
          <span class="material-symbols-rounded" style="font-size: 20px">more_vert</span>
        </button>
      </div>

      <!-- 多选模式底部操作栏 -->
      <div v-if="isMultiSelectMode" class="multi-select-bar">
        <span class="select-count">{{ t('common.selected_count', { count: selectedPlaylists.size }) }}</span>
        <button class="multi-select-action danger" :disabled="selectedPlaylists.size === 0" @click="requestDeleteSelected">
          <span class="material-symbols-rounded" style="font-size: 18px">delete</span>
          <span>{{ t('common.delete_selected') }}</span>
        </button>
      </div>

      <div v-if="playlists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">library_music</span></div>
        <p class="empty-title">{{ t('library.playlist_empty_title') }}</p>
        <p class="empty-desc">{{ t('library.playlist_empty_desc') }}</p>
      </div>
    </div>

    <!-- Tab: 收藏（同步的收藏歌单） -->
    <div v-else-if="activeTab === 'favorites'" class="playlist-list">
      <template v-if="favoritePlaylists.length > 0">
        <component
          v-for="row in favoritePlaylistRows"
          :is="row.location ? RouterLink : 'div'"
          :key="row.key"
          :to="row.location ?? undefined"
          class="playlist-item"
          :class="row.location ? 'favorite-detail-link' : 'favorite-static-row'"
        >
          <div class="pl-icon has-cover" v-if="row.coverUrl && !isLibraryCoverFailed('favorite', row.id, row.coverUrl)">
            <img
              :src="toDisplayableLibraryCoverUrl(row.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('favorite', row.id, row.coverUrl)"
            />
          </div>
          <div class="pl-icon" v-else>
            <span class="material-symbols-rounded filled" style="font-size: 22px">bookmark</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ row.name }}</div>
            <div class="pl-count">{{ t('player.track_count', { count: row.trackCount }) }} · {{ platformLabel(row.source) }}</div>
          </div>
          <span v-if="row.location" class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </component>
      </template>
      <div v-else class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">bookmark</span></div>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
    </div>

    <!-- Tab: 下载 -->
    <div v-else-if="activeTab === 'downloads'" class="playlist-list">
      <template v-if="downloadStore.activeDownloads.length > 0">
        <div class="subsection-label">
          <span class="material-symbols-rounded" style="font-size: 18px">downloading</span>
          <span>{{ t('download.active_tasks', { count: downloadStore.activeDownloads.length }) }}</span>
        </div>
        <div
          v-for="task in downloadStore.activeDownloads"
          :key="'active-' + task.trackId"
          class="playlist-item active-download-item"
        >
          <div class="pl-icon">
            <span class="material-symbols-rounded filled" style="font-size: 22px">
              {{ task.status === 'resolving' ? 'network_node' : task.status === 'error' ? 'error' : task.status === 'cancelled' ? 'cancel' : 'downloading' }}
            </span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ task.title }}</div>
            <div class="pl-count">{{ task.artist }} · {{ activeDownloadProgressText(task) }}</div>
            <div
              class="download-progress-bar"
              :class="{
                indeterminate: task.status === 'downloading' && !task.totalBytes,
                error: task.status === 'error',
                muted: task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'already_exists',
              }"
            >
              <div
                class="download-progress-fill"
                :style="{ width: `${Math.max(4, task.status === 'cancelled' || task.status === 'already_exists' ? 100 : task.progress ?? 0)}%` }"
              />
            </div>
          </div>
          <button
            class="pl-more danger"
            :disabled="task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'error' || task.status === 'already_exists'"
            @click.stop="cancelActiveDownload(task.trackId)"
          >
            <span class="material-symbols-rounded" style="font-size: 20px">close</span>
          </button>
        </div>
        <div v-if="downloadStore.downloads.length > 0" class="subsection-label" style="margin-top: 10px;">
          <span class="material-symbols-rounded" style="font-size: 18px">download_done</span>
          <span>{{ t('download.downloaded_items') }}</span>
        </div>
      </template>

      <template v-if="downloadStore.downloads.length > 0">
        <div
          v-for="dl in downloadStore.downloads"
          :key="'dl-' + dl.id"
          class="playlist-item"
          @click="playDownloadedTrack(dl)"
          @contextmenu.prevent.stop="openDlContextMenu($event, dl)"
        >
          <div class="pl-icon has-cover" v-if="dl.coverUrl && !isLibraryCoverFailed('download', dl.id, dl.coverUrl)">
            <img
              :src="toDisplayableLibraryCoverUrl(dl.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('download', dl.id, dl.coverUrl)"
            />
          </div>
          <div class="pl-icon" v-else>
            <span class="material-symbols-rounded filled" style="font-size: 22px">music_note</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ dl.title }}</div>
            <div class="pl-count">{{ dl.artist }} · {{ formatFileSize(dl.fileSize) }} · {{ platformLabel(dl.source) }}</div>
          </div>
          <button class="pl-more" @click.stop="openDlContextMenu($event, dl)">
            <span class="material-symbols-rounded" style="font-size: 20px">more_vert</span>
          </button>
        </div>
      </template>
      <div v-else-if="downloadStore.activeDownloads.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">download</span></div>
        <p class="empty-title">{{ t('library.downloads_empty_title') }}</p>
        <p class="empty-desc">{{ t('library.downloads_empty_desc') }}</p>
      </div>
    </div>

    <!-- Tab: Bilibili favorites -->
    <div v-else-if="activeTab === 'bilibili'" class="playlist-list">
      <div v-if="auth.bilibili.loggedIn" class="cloud-library-tools">
        <button
          class="netease-refresh"
          type="button"
          :disabled="cloudLibraryLoading.bilibili"
          :aria-label="t('common.refresh')"
          :title="t('common.refresh')"
          @click="loadCloudLibrary('bilibili')"
        >
          <span class="material-symbols-rounded" :class="{ spinning: cloudLibraryLoading.bilibili }" aria-hidden="true">refresh</span>
        </button>
      </div>
      <div v-if="!auth.bilibili.loggedIn" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">video_library</span></div>
        <p class="empty-title">{{ t('settings.bilibili_account') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
        <button
          class="retry-btn netease-login-button"
          type="button"
          :disabled="auth.loggingIn === 'bilibili'"
          @click="auth.loginBilibili"
        >
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">login</span>
          <span>{{ t('settings.sign_in') }}</span>
        </button>
      </div>
      <div v-else-if="cloudLibraryLoading.bilibili && bilibiliPlaylists.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded spinning" style="font-size: 40px" aria-hidden="true">progress_activity</span></div>
        <p class="empty-title">{{ t('player.loading') }}</p>
      </div>
      <div v-else-if="cloudLibraryError.bilibili && bilibiliPlaylists.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">cloud_off</span></div>
        <p class="empty-title">{{ t('player.load_failed') }}</p>
        <p class="empty-desc">{{ cloudLibraryError.bilibili }}</p>
        <button class="retry-btn" type="button" @click="loadCloudLibrary('bilibili')">
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">refresh</span>
          <span>{{ t('common.retry') }}</span>
        </button>
      </div>
      <template v-else-if="bilibiliPlaylists.length > 0">
        <RouterLink
          v-for="playlist in bilibiliPlaylists"
          :key="`bilibili-${playlist.id}`"
          class="playlist-item netease-result-link"
          :to="{ name: 'bili-playlist', params: { mediaId: playlist.id } }"
        >
          <div class="pl-icon" :class="{ 'has-cover': playlist.coverUrl }">
            <BilibiliCoverImage v-if="playlist.coverUrl" :src="playlist.coverUrl" class="pl-cover-img">
              <span class="material-symbols-rounded filled" style="font-size: 22px" aria-hidden="true">video_library</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px" aria-hidden="true">video_library</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ playlist.name }}</div>
            <div class="pl-count">{{ t('library.track_count', { count: playlist.trackCount || 0 }) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3" aria-hidden="true">chevron_right</span>
        </RouterLink>
      </template>
      <div v-else class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">video_library</span></div>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
      </div>
    </div>

    <!-- Tab: YouTube Music playlists -->
    <div v-else-if="activeTab === 'youtube'" class="playlist-list">
      <div v-if="auth.youtube.loggedIn" class="cloud-library-tools">
        <button
          class="netease-refresh"
          type="button"
          :disabled="cloudLibraryLoading.youtube"
          :aria-label="t('common.refresh')"
          :title="t('common.refresh')"
          @click="loadCloudLibrary('youtube')"
        >
          <span class="material-symbols-rounded" :class="{ spinning: cloudLibraryLoading.youtube }" aria-hidden="true">refresh</span>
        </button>
      </div>
      <div v-if="!auth.youtube.loggedIn" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">subscriptions</span></div>
        <p class="empty-title">{{ t('settings.youtube_account') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
        <button
          class="retry-btn netease-login-button"
          type="button"
          :disabled="auth.loggingIn === 'youtube'"
          @click="auth.loginYoutube"
        >
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">login</span>
          <span>{{ t('settings.sign_in') }}</span>
        </button>
      </div>
      <div v-else-if="cloudLibraryLoading.youtube && youtubePlaylists.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded spinning" style="font-size: 40px" aria-hidden="true">progress_activity</span></div>
        <p class="empty-title">{{ t('player.loading') }}</p>
      </div>
      <div v-else-if="cloudLibraryError.youtube && youtubePlaylists.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">cloud_off</span></div>
        <p class="empty-title">{{ t('player.load_failed') }}</p>
        <p class="empty-desc">{{ cloudLibraryError.youtube }}</p>
        <button class="retry-btn" type="button" @click="loadCloudLibrary('youtube')">
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">refresh</span>
          <span>{{ t('common.retry') }}</span>
        </button>
      </div>
      <template v-else-if="youtubePlaylists.length > 0">
        <RouterLink
          v-for="playlist in youtubePlaylists"
          :key="`youtube-${playlist.id}`"
          class="playlist-item netease-result-link"
          :to="{ name: 'youtube-playlist', params: { browseId: playlist.id } }"
        >
          <div class="pl-icon" :class="{ 'has-cover': playlist.coverUrl }">
            <img
              v-if="playlist.coverUrl && !isLibraryCoverFailed('youtube-playlist', playlist.id, playlist.coverUrl)"
              :src="toDisplayableLibraryCoverUrl(playlist.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('youtube-playlist', playlist.id, playlist.coverUrl)"
            />
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px" aria-hidden="true">subscriptions</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ playlist.name }}</div>
            <div class="pl-count" :title="playlist.description || undefined">{{ playlist.description || t('library.track_count', { count: playlist.trackCount || 0 }) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3" aria-hidden="true">chevron_right</span>
        </RouterLink>
      </template>
      <div v-else class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">subscriptions</span></div>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
      </div>
    </div>

    <!-- Tab: 网易云-歌单 -->
    <div v-else-if="activeTab === 'netease'" class="playlist-list">
      <div class="netease-category-bar" role="tablist" :aria-label="t('library.tab_netease')">
        <button
          v-for="category in neteaseCategories"
          :key="category.key"
          :id="`netease-category-${category.key}`"
          class="netease-category-tab"
          :class="{ active: neteaseCategory === category.key }"
          type="button"
          role="tab"
          :aria-selected="neteaseCategory === category.key"
          aria-controls="netease-category-panel"
          :tabindex="neteaseCategory === category.key ? 0 : -1"
          @click="activateNeteaseCategory(category.key)"
          @keydown="handleNeteaseCategoryKeydown($event, category.key)"
        >
          {{ category.label }}
        </button>
      </div>
      <div
        id="netease-category-panel"
        role="tabpanel"
        :aria-labelledby="`netease-category-${neteaseCategory}`"
        tabindex="0"
        class="netease-content"
      >
      <div v-if="auth.netease.loggedIn" class="netease-tools">
        <label class="netease-search">
          <span class="material-symbols-rounded" style="font-size: 20px" aria-hidden="true">search</span>
          <input
            v-model="activeNeteaseSearchQuery"
            type="search"
            :aria-label="t('library.netease_search_placeholder')"
            :placeholder="t('library.netease_search_placeholder')"
          />
        </label>
        <button
          class="netease-refresh"
          type="button"
          :disabled="neteasePlaylistLoading || neteaseAlbumLoading"
          :aria-label="t('common.refresh')"
          :title="t('common.refresh')"
          @click="loadNeteaseLibrary"
        >
          <span class="material-symbols-rounded" :class="{ spinning: neteasePlaylistLoading || neteaseAlbumLoading }" aria-hidden="true">refresh</span>
        </button>
      </div>
      <div v-if="!auth.netease.loggedIn" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">cloud_queue</span></div>
        <p class="empty-title">{{ t('library.netease_login') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_recommend') }}</p>
        <button
          class="retry-btn netease-login-button"
          type="button"
          :disabled="auth.loggingIn === 'netease'"
          @click="auth.loginNetease"
        >
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">login</span>
          <span>{{ t('library.netease_login') }}</span>
        </button>
      </div>
      <template v-else>
      <template v-if="neteaseCategory === 'playlists'">
      <div v-if="neteasePlaylistLoading && neteasePlaylists.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded spinning" style="font-size: 40px" aria-hidden="true">progress_activity</span></div>
        <p class="empty-title">{{ t('player.loading') }}</p>
      </div>
      <div v-else-if="neteasePlaylistError && neteasePlaylists.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">cloud_off</span></div>
        <p class="empty-title">{{ t('player.load_failed') }}</p>
        <p class="empty-desc">{{ neteasePlaylistError }}</p>
        <button class="retry-btn" type="button" @click="loadNeteaseLibrary">
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">refresh</span>
          <span>{{ t('common.retry') }}</span>
        </button>
      </div>
      <template v-else-if="neteasePlaylists.length > 0 && neteaseFilteredPlaylists.length > 0">
        <RouterLink
          v-for="npl in neteaseFilteredPlaylists"
          :key="'ne-' + npl.id"
          class="playlist-item netease-result-link"
          :to="{ name: 'netease-playlist', params: { id: npl.id } }"
        >
          <div class="pl-icon netease">
            <img
              v-if="npl.coverUrl && !isLibraryCoverFailed('netease-playlist', npl.id, npl.coverUrl)"
              :src="toDisplayableLibraryCoverUrl(npl.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('netease-playlist', npl.id, npl.coverUrl)"
            />
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px" aria-hidden="true">library_music</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ npl.name }}</div>
            <div class="pl-count">
              {{ t('library.netease_play_count', { count: npl.playCount || 0 }) }}
              <span aria-hidden="true">·</span>
              {{ t('library.track_count', { count: npl.trackCount || 0 }) }}
            </div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3" aria-hidden="true">chevron_right</span>
        </RouterLink>
      </template>
      <div v-else-if="neteasePlaylists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">cloud_queue</span></div>
        <p class="empty-title">{{ t('library.netease_playlist_empty') }}</p>
      </div>
      <div v-else-if="neteasePlaylists.length > 0 && neteaseFilteredPlaylists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">search_off</span></div>
        <p class="empty-title">{{ t('library.netease_search_empty_title') }}</p>
        <p class="empty-desc">{{ t('library.netease_search_empty_desc') }}</p>
      </div>
      </template>

    <!-- Tab: 网易云-专辑 -->
      <template v-else-if="neteaseCategory === 'albums'">
      <div v-if="neteaseAlbumLoading && recommend.userAlbums.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded spinning" style="font-size: 40px" aria-hidden="true">progress_activity</span></div>
        <p class="empty-title">{{ t('player.loading') }}</p>
      </div>
      <div v-else-if="neteaseAlbumError && recommend.userAlbums.length === 0" class="empty-tab library-state">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">cloud_off</span></div>
        <p class="empty-title">{{ t('player.load_failed') }}</p>
        <p class="empty-desc">{{ neteaseAlbumError }}</p>
        <button class="retry-btn" type="button" @click="loadNeteaseLibrary">
          <span class="material-symbols-rounded" style="font-size: 18px" aria-hidden="true">refresh</span>
          <span>{{ t('common.retry') }}</span>
        </button>
      </div>
      <template v-else-if="recommend.userAlbums.length > 0 && neteaseFilteredAlbums.length > 0">
        <RouterLink
          v-for="album in neteaseFilteredAlbums"
          :key="album.id"
          class="playlist-item netease-result-link"
          :to="{ name: 'netease-album', params: { id: album.id } }"
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
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px" aria-hidden="true">album</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ album.name }}</div>
            <div class="pl-count">
              <template v-if="album.artist">
                {{ album.artist }} <span aria-hidden="true">·</span>
              </template>
              {{ t('player.track_count', { count: album.trackCount || 0 }) }}
            </div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3" aria-hidden="true">chevron_right</span>
        </RouterLink>
      </template>
      <div v-else-if="recommend.userAlbums.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">album</span></div>
        <p class="empty-title">{{ t('library.netease_album_empty') }}</p>
      </div>
      <div v-else-if="recommend.userAlbums.length > 0 && neteaseFilteredAlbums.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px" aria-hidden="true">search_off</span></div>
        <p class="empty-title">{{ t('library.netease_search_empty_title') }}</p>
        <p class="empty-desc">{{ t('library.netease_search_empty_desc') }}</p>
      </div>
      </template>
      </template>
      </div>
    </div>

    <ContextMenu
      :open="dlContextMenu.show"
      :x="dlContextMenu.x"
      :y="dlContextMenu.y"
      :items="downloadMenuItems"
      @update:open="dlContextMenu.show = $event"
      @click="handleDownloadMenuClick"
    />

    <!-- 删除下载确认对话框 -->
    <M3Dialog
      v-model:open="showDlDeleteDialog"
      :title="t('download.delete_confirm')"
      icon="delete"
      :confirm-text="t('common.delete')"
      confirm-danger
      @confirm="confirmDlDelete"
    >
      <p class="dialog-msg">{{ t('library.delete_confirm_msg', { name: dlDeleteTarget?.title || '' }) }}</p>
    </M3Dialog>

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
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: 4px;
  min-height: 44px;
  padding: 4px;
  margin-bottom: 20px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
}

.tab-chip {
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  min-height: 36px;
  gap: 6px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  font-size: 13px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  background: transparent;
  border: 1px solid transparent;
  transition: all var(--duration-short) var(--ease-standard);

  &:hover:not(.active) {
    background: var(--md-surface-container-high);
  }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    font-weight: 600;
  }
}

.tab-chip-icon {
  flex: 0 0 auto;
  font-size: 18px;
}

.tab-chip-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.netease-category-bar {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 4px;
  width: 100%;
  min-height: 44px;
  margin-bottom: 12px;
  padding: 4px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
}

.netease-category-tab {
  min-width: 0;
  min-height: 36px;
  padding: 6px 10px;
  overflow: hidden;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--md-on-surface-variant);
  font-size: 13px;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: background var(--duration-short) var(--ease-standard);

  &:hover:not(.active) {
    background: var(--md-surface-container-high);
  }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    font-weight: 600;
  }
}

.netease-content {
  min-width: 0;
}

.netease-tools {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 40px;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}

.cloud-library-tools {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 12px;
}

.netease-search {
  display: flex;
  align-items: center;
  min-width: 0;
  min-height: 40px;
  gap: 8px;
  padding: 0 12px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-md);
  color: var(--md-on-surface-variant);
  background: var(--md-surface-container);
  transition: border-color var(--duration-short), background var(--duration-short);

  &:focus-within {
    border-color: var(--md-primary);
    outline: 2px solid color-mix(in srgb, var(--md-primary) 24%, transparent);
    outline-offset: 0;
  }

  input {
    width: 100%;
    min-width: 0;
    border: 0;
    outline: 0;
    color: var(--md-on-surface);
    background: transparent;
    font: inherit;

    &::placeholder {
      color: var(--md-on-surface-variant);
      opacity: 0.72;
    }
  }
}

.netease-refresh {
  display: grid;
  place-items: center;
  width: 40px;
  height: 40px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  background: transparent;
  cursor: pointer;
  transition: background var(--duration-short), color var(--duration-short), opacity var(--duration-short);

  &:hover:not(:disabled) {
    background: var(--md-surface-container-high);
    color: var(--md-primary);
  }

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: 2px;
  }

  &:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
}

.netease-result-link {
  color: inherit;
  text-decoration: none;

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: 2px;
    background: var(--md-surface-container);
  }
}

.favorite-detail-link {
  color: inherit;
  text-decoration: none;

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: 2px;
    background: var(--md-surface-container);
  }
}

.playlist-item.favorite-static-row {
  cursor: default;
}

.netease-login-button {
  margin-top: 16px;
}

@media (max-width: 680px) {
  .library-view {
    padding-inline: 16px;
  }

  .tab-bar {
    gap: 2px;
    min-height: 62px;
  }

  .tab-chip {
    min-height: 54px;
    flex-direction: column;
    gap: 2px;
    padding: 4px 2px;
    font-size: 11px;
    line-height: 1.15;
  }

  .tab-chip-label {
    display: -webkit-box;
    overflow: hidden;
    white-space: normal;
    text-align: center;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
  }

  .netease-category-bar {
    gap: 2px;
  }

  .netease-category-tab {
    min-height: 42px;
    padding-inline: 6px;
    font-size: 12px;
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
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--duration-short), box-shadow var(--duration-short), opacity var(--duration-short);

  &:hover { background: var(--md-surface-container); }

  &.selected { background: color-mix(in srgb, var(--md-primary) 12%, transparent); }
  &.dragging { opacity: 0.4; }
  &.drag-over { box-shadow: inset 0 2px 0 0 var(--md-primary); }
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
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 8px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
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
  margin-left: auto;
  padding: 8px 16px;
  border: 0;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background var(--duration-short), opacity var(--duration-short);

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
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
.library-state {
  min-height: 260px;
  padding-block: 56px;
}

.library-state .retry-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 36px;
  margin-top: 16px;
  padding: 7px 14px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-sm);
  background: var(--md-surface-container);
  color: var(--md-primary);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard);
}

.library-state .retry-btn:hover {
  background: var(--md-surface-container-high);
}

.dialog-msg {
  font-size: 14px;
  color: var(--md-on-surface-variant);
  line-height: 1.5;
}
</style>
