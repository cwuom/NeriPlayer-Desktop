<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useDownloadStore } from '@/stores/download'
import { useDelayedFlag } from '@/composables/useDelayedFlag'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import AddToPlaylistDialog from '@/components/AddToPlaylistDialog.vue'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import TrackSelectionToolbar from '@/components/TrackSelectionToolbar.vue'
import LocateTrackFab from '@/components/LocateTrackFab.vue'
import { useTrackSelection } from '@/composables/useTrackSelection'
import { useLocateCurrentTrack } from '@/composables/useLocateCurrentTrack'
import { useIncrementalList } from '@/composables/useIncrementalList'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import {
  playlistDetailCacheKey,
  readPlaylistDetailCache,
  writePlaylistDetailCache,
} from '@/modules/library/playlistDetailCache'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'

const route = useRoute()
const router = useRouter()
const player = usePlayerStore()
const downloadStore = useDownloadStore()
const { t } = useI18n()

const isLoading = ref(true)
// 慢加载才显示 spinner，避免快速加载时一闪而过（UI-016）
const isLoadingSlow = useDelayedFlag(isLoading)
const error = ref<string | null>(null)
const folderName = ref('')
const coverUrl = ref('')
const mediaCount = ref(0)
const searchQuery = ref('')

const tracks = ref<TrackInfo[]>([])

interface BiliDetailCache {
  folderName: string
  coverUrl: string
  mediaCount: number
  tracks: TrackInfo[]
}

function applyDetailCache(cache: BiliDetailCache) {
  folderName.value = cache.folderName
  coverUrl.value = cache.coverUrl
  mediaCount.value = cache.mediaCount
  tracks.value = cache.tracks
}

function saveDetailCache(cacheKey: string) {
  writePlaylistDetailCache<BiliDetailCache>(cacheKey, {
    folderName: folderName.value,
    coverUrl: coverUrl.value,
    mediaCount: mediaCount.value,
    tracks: tracks.value,
  })
}

const filteredTracks = computed(() => {
  if (!searchQuery.value) return tracks.value
  const q = searchQuery.value.toLowerCase()
  return tracks.value.filter(t =>
    t.title.toLowerCase().includes(q) || t.artist.toLowerCase().includes(q)
  )
})

// 大列表分块渲染（WebKitGTK 上千行一次性渲染会卡顿）
const { visibleItems: visibleTracks, onScroll: onTrackListScroll, ensureIndex: ensureTrackIndex } =
  useIncrementalList(() => filteredTracks.value)

const {
  selectionMode,
  selectedIds,
  selectedItems: selectedTracks,
  visibleSelectedCount,
  allVisibleSelected,
  enterSelectionMode,
  leaveSelectionMode,
  toggleSelected,
  toggleSelectAllVisible,
  invertSelectionVisible,
} = useTrackSelection(tracks, filteredTracks)

// 定位到当前播放
const viewRef = ref<HTMLElement | null>(null)
const currentRowKey = computed(() => {
  const id = player.currentTrack?.id
  if (!id) return null
  return filteredTracks.value.some(item => item.id === id) ? id : null
})
const { fabVisible: locateFabVisible, locate: locateCurrentTrack } = useLocateCurrentTrack({
  containerRef: viewRef,
  currentKey: currentRowKey,
  suppressed: selectionMode,
  ensureRendered: key => ensureTrackIndex(filteredTracks.value.findIndex(item => item.id === key)),
})

async function loadDetail() {
  const mediaId = Number(route.params.mediaId)
  if (!mediaId) return

  const cacheKey = playlistDetailCacheKey('bilibili-favorite', mediaId)
  const cached = readPlaylistDetailCache<BiliDetailCache>(cacheKey)
  if (cached) {
    applyDetailCache(cached)
    isLoading.value = false
  } else {
    isLoading.value = true
  }
  error.value = null

  try {
    // 获取收藏夹信息
    const infoData = await invoke<any>('get_bili_fav_folder_info', { mediaId })
    const info = infoData?.data || {}
    folderName.value = info.title || ''
    coverUrl.value = info.cover || ''
    mediaCount.value = info.media_count || 0

    // 获取收藏夹内容（分页加载所有）
    let page = 1
    const allItems: any[] = []
    let hasMore = true

    while (hasMore) {
      const data = await invoke<any>('get_bili_favorite_items', { mediaId, page })
      const items = data?.data?.medias || []
      allItems.push(...items)
      hasMore = data?.data?.has_more || false
      page++
      if (page > 50) break // 安全上限
    }

    tracks.value = allItems
      .filter((item: any) => item.type === 2) // 仅视频
      .map((item: any) => ({
        id: `bilibili:${item.bvid || item.bv_id}`,
        title: item.title || '',
        artist: item.upper?.name || '',
        album: '',
        durationMs: (item.duration || 0) * 1000,
        coverUrl: item.cover || '',
        audioUrl: '',
      }))
    saveDetailCache(cacheKey)
  } catch (e: any) {
    if (!cached) {
      error.value = e?.toString() || t('player.load_failed')
    }
  } finally {
    isLoading.value = false
  }
}

function playAll() {
  if (tracks.value.length === 0) return
  player.playAll(tracks.value)
}

function shufflePlay() {
  if (tracks.value.length === 0) return
  player.shufflePlay(tracks.value)
}

function playTrack(track: TrackInfo) {
  if (selectionMode.value) {
    toggleSelected(track.id)
    return
  }
  player.playAll(filteredTracks.value, track.id)
}

function playSelected() {
  if (selectedTracks.value.length === 0) return
  player.playAll(selectedTracks.value)
  leaveSelectionMode()
}

function queueSelected() {
  for (const track of selectedTracks.value) player.addToQueueEnd(track)
  leaveSelectionMode()
}

const trackMenu = ref<{ show: boolean; x: number; y: number; track: TrackInfo | null }>({
  show: false, x: 0, y: 0, track: null,
})
const showAddToPlaylist = ref(false)
const addToPlaylistTarget = ref<TrackInfo | null>(null)
const addToPlaylistTargets = ref<TrackInfo[]>([])

function openTrackMenu(e: MouseEvent, track: TrackInfo) {
  if (selectionMode.value) {
    toggleSelected(track.id)
    return
  }
  const btn = e.currentTarget as HTMLElement
  const rect = btn.getBoundingClientRect()
  let x = rect.left - 204
  if (x < 8) x = rect.right + 4
  trackMenu.value = { show: true, x, y: rect.top, track }
}

function openTrackContextMenu(e: MouseEvent, track: TrackInfo) {
  if (selectionMode.value) {
    toggleSelected(track.id)
    return
  }
  trackMenu.value = { show: true, x: e.clientX, y: e.clientY, track }
}

function closeTrackMenu() {
  trackMenu.value.show = false
}

function openAddToPlaylist(track: TrackInfo) {
  closeTrackMenu()
  addToPlaylistTarget.value = track
  addToPlaylistTargets.value = []
  showAddToPlaylist.value = true
}

function openBatchAddToPlaylist() {
  if (selectedTracks.value.length === 0) return
  const targets = [...selectedTracks.value]
  leaveSelectionMode()
  addToPlaylistTarget.value = null
  addToPlaylistTargets.value = targets
  showAddToPlaylist.value = true
}

function downloadSelected() {
  const targets = [...selectedTracks.value]
  leaveSelectionMode()
  for (const track of targets) void downloadStore.downloadTrack(track)
}

function addToQueueNext(track: TrackInfo) {
  closeTrackMenu()
  player.addToQueueNext(track)
}

function addToQueueEnd(track: TrackInfo) {
  closeTrackMenu()
  player.addToQueueEnd(track)
}

function downloadTaskStatusText(status?: string) {
  switch (status) {
    case 'resolving': return t('download.resolving')
    case 'downloading': return t('download.downloading')
    case 'cancelling': return t('download.cancelling')
    case 'cancelled': return t('download.cancelled')
    case 'error': return t('download.download_failed')
    case 'already_exists': return t('download.already_exists')
    default: return t('download.downloading')
  }
}

function trackDownloadLabel(track: TrackInfo) {
  const task = downloadStore.downloading.get(track.id)
  if (task) return downloadTaskStatusText(task.status)
  if (downloadStore.isDownloaded(track.id)) return t('download.redownload')
  return t('download.download')
}

function isTrackDownloadDisabled(track: TrackInfo) {
  if (downloadStore.isDownloading(track.id)) return true
  return downloadStore.isDownloaded(track.id)
    && player.currentTrack?.id === track.id
    && player.isPlayingFromDownload
}

async function handleTrackDownload(track: TrackInfo) {
  closeTrackMenu()
  if (isTrackDownloadDisabled(track)) return
  if (downloadStore.isDownloaded(track.id)) {
    await downloadStore.redownloadTrack(track)
  } else {
    await downloadStore.downloadTrack(track)
  }
}

const trackMenuItems = computed<ContextMenuItem[]>(() => {
  const track = trackMenu.value.track
  return [
    createContextMenuItem(t('common.multi_select'), { id: 'select', icon: 'checklist' }),
    createContextMenuItem(t('player.play_next'), { id: 'play-next', icon: 'queue_play_next' }),
    createContextMenuItem(t('player.add_to_queue'), { id: 'add-to-queue', icon: 'add_to_queue' }),
    createContextMenuItem(t('player.add_to_playlist'), { id: 'add-to-playlist', icon: 'playlist_add' }),
    createContextMenuItem(
      track ? trackDownloadLabel(track) : t('download.download'),
      {
        id: 'download',
        icon: 'download',
        disabled: !track || isTrackDownloadDisabled(track),
      },
    ),
  ]
})

function handleTrackMenuClick(item: ContextMenuActionItem) {
  const track = trackMenu.value.track
  if (!track) return

  switch (item.id) {
    case 'select':
      closeTrackMenu()
      enterSelectionMode(track)
      break
    case 'play-next':
      addToQueueNext(track)
      break
    case 'add-to-queue':
      addToQueueEnd(track)
      break
    case 'add-to-playlist':
      openAddToPlaylist(track)
      break
    case 'download':
      void handleTrackDownload(track)
      break
  }
}

onMounted(() => {
  downloadStore.initEvents()
  void downloadStore.loadDownloads()
  void loadDetail()
})
</script>

<template>
  <div ref="viewRef" class="detail-view" @scroll="onTrackListScroll">
    <header class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="header-search" v-if="!isLoading && tracks.length > 0">
        <span class="material-symbols-rounded search-icon">search</span>
        <input v-model="searchQuery" :placeholder="t('player.search_tracks')" class="search-input" />
      </div>
    </header>

    <div v-if="isLoading" class="state-center">
      <template v-if="isLoadingSlow">
        <span class="material-symbols-rounded spinning">progress_activity</span>
        <p>{{ t('player.loading') }}</p>
      </template>
    </div>

    <div v-else-if="error" class="state-center">
      <span class="material-symbols-rounded" style="font-size: 48px; opacity: 0.3">error</span>
      <p>{{ error }}</p>
      <button class="retry-btn" @click="loadDetail">{{ t('player.retry') }}</button>
    </div>

    <template v-else>
      <div class="detail-hero">
        <div class="hero-cover">
          <BilibiliCoverImage v-if="coverUrl" :src="coverUrl">
            <span class="material-symbols-rounded filled" style="font-size: 48px; opacity: 0.3">video_library</span>
          </BilibiliCoverImage>
          <span v-else class="material-symbols-rounded filled" style="font-size: 48px; opacity: 0.3">video_library</span>
        </div>
        <div class="hero-info">
          <h1 class="hero-title">{{ folderName }}</h1>
          <p class="hero-meta">{{ t('player.video_count', { count: mediaCount }) }}</p>
          <div class="hero-actions">
            <button class="play-all-btn" @click="playAll">
              <span class="material-symbols-rounded filled">play_arrow</span>
              {{ t('player.play_all') }}
            </button>
            <button class="hero-icon-btn" :title="t('player.shuffle_play')" @click="shufflePlay">
              <span class="material-symbols-rounded">shuffle</span>
            </button>
            <button class="hero-icon-btn" :title="t('common.multi_select')" @click="enterSelectionMode()">
              <span class="material-symbols-rounded">checklist</span>
            </button>
          </div>
        </div>
      </div>

      <div v-if="filteredTracks.length === 0" class="state-center">
        <p>{{ t('player.empty_playlist') }}</p>
      </div>
      <div v-else>
        <TrackSelectionToolbar
          v-if="selectionMode"
          :selected-count="selectedTracks.length"
          :visible-selected-count="visibleSelectedCount"
          :all-visible-selected="allVisibleSelected"
          @select-all="toggleSelectAllVisible"
          @invert-selection="invertSelectionVisible"
          @play="playSelected"
          @queue="queueSelected"
          @playlist="openBatchAddToPlaylist"
          @download="downloadSelected"
          @exit="leaveSelectionMode"
        />
        <div class="track-list">
          <div
            v-for="(track, index) in visibleTracks"
            :key="track.id"
            class="track-item"
            :class="{ active: player.currentTrack?.id === track.id, selected: selectionMode && selectedIds.has(track.id), 'selection-mode': selectionMode }"
            :data-track-key="track.id"
            @click="playTrack(track)"
            @contextmenu.prevent.stop="openTrackContextMenu($event, track)"
          >
            <button v-if="selectionMode" class="track-select" @click.stop="toggleSelected(track.id)">
              <span class="material-symbols-rounded filled">{{ selectedIds.has(track.id) ? 'check_circle' : 'radio_button_unchecked' }}</span>
            </button>
            <div v-else class="track-index">
              <div v-if="player.currentTrack?.id === track.id && player.isPlaying" class="equalizer-bars"><span class="bar"/><span class="bar"/><span class="bar"/></div>
              <span v-else class="index-num">{{ index + 1 }}</span>
            </div>
            <div class="track-cover-wide">
              <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy">
                <span class="material-symbols-rounded filled">movie</span>
              </BilibiliCoverImage>
              <span v-else class="material-symbols-rounded filled">movie</span>
            </div>
            <div class="track-info">
              <div class="track-title">{{ track.title }}</div>
              <div class="track-meta">{{ track.artist }}</div>
            </div>
            <div class="track-duration">{{ formatDuration(track.durationMs) }}</div>
            <button v-if="!selectionMode" class="track-more" @click.stop="openTrackMenu($event, track)">
              <span class="material-symbols-rounded">more_vert</span>
            </button>
          </div>
        </div>
      </div>
    </template>

    <LocateTrackFab
      :visible="locateFabVisible"
      :label="t('player.locate_current')"
      @click="locateCurrentTrack"
    />

    <ContextMenu
      :open="trackMenu.show"
      :x="trackMenu.x"
      :y="trackMenu.y"
      :items="trackMenuItems"
      @update:open="trackMenu.show = $event"
      @click="handleTrackMenuClick"
    />

    <AddToPlaylistDialog v-model:open="showAddToPlaylist" :track="addToPlaylistTarget" :tracks="addToPlaylistTargets" />
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;
</style>
