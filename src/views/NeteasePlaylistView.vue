<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useDownloadStore } from '@/stores/download'
import { useAuthStore } from '@/stores/auth'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import AddToPlaylistDialog from '@/components/AddToPlaylistDialog.vue'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import TrackSelectionToolbar from '@/components/TrackSelectionToolbar.vue'
import { useTrackSelection } from '@/composables/useTrackSelection'
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
import { createNeteaseDetailCacheScope } from '@/modules/library/neteaseDetailCacheScope'

const props = defineProps<{ isAlbum?: boolean }>()
const route = useRoute()
const router = useRouter()
const player = usePlayerStore()
const downloadStore = useDownloadStore()
const auth = useAuthStore()
const { t } = useI18n()

const isLoading = ref(true)
const error = ref<string | null>(null)
const playlistName = ref('')
const coverUrl = ref('')
const trackCount = ref(0)
const playCount = ref(0)
const description = ref('')
const creator = ref('')
const searchQuery = ref('')

const tracks = ref<TrackInfo[]>([])
let detailRequestGeneration = 0

const neteaseDetailCacheScope = computed(() =>
  createNeteaseDetailCacheScope(auth.netease, auth.neteaseSessionVersion),
)
const neteaseSessionFingerprint = computed(
  () => `${auth.netease.loggedIn ? '1' : '0'}:${auth.neteaseSessionVersion}`,
)

interface NeteaseDetailCache {
  playlistName: string
  coverUrl: string
  trackCount: number
  playCount: number
  description: string
  creator: string
  tracks: TrackInfo[]
}

function applyDetailCache(cache: NeteaseDetailCache) {
  playlistName.value = cache.playlistName
  coverUrl.value = cache.coverUrl
  trackCount.value = cache.trackCount
  playCount.value = cache.playCount
  description.value = cache.description
  creator.value = cache.creator
  tracks.value = cache.tracks
}

function saveDetailCache(cacheKey: string) {
  writePlaylistDetailCache<NeteaseDetailCache>(cacheKey, {
    playlistName: playlistName.value,
    coverUrl: coverUrl.value,
    trackCount: trackCount.value,
    playCount: playCount.value,
    description: description.value,
    creator: creator.value,
    tracks: tracks.value,
  })
}

function resetDetailState() {
  playlistName.value = ''
  coverUrl.value = ''
  trackCount.value = 0
  playCount.value = 0
  description.value = ''
  creator.value = ''
  tracks.value = []
  error.value = null
}

const filteredTracks = computed(() => {
  if (!searchQuery.value) return tracks.value
  const q = searchQuery.value.toLowerCase()
  return tracks.value.filter(t =>
    t.title.toLowerCase().includes(q) || t.artist.toLowerCase().includes(q)
  )
})

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

// 总时长
const totalDuration = computed(() => {
  const totalMs = tracks.value.reduce((sum, t) => sum + (t.durationMs || 0), 0)
  return formatTotalDuration(totalMs)
})

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000)
  return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, '0')}`
}

function formatTotalDuration(ms: number): string {
  const totalMin = Math.floor(ms / 60000)
  if (totalMin >= 60) {
    const h = Math.floor(totalMin / 60)
    const m = totalMin % 60
    return `${h}${t('common.hour_short')} ${m}${t('common.minute_short')}`
  }
  return `${totalMin}${t('common.minute_short')}`
}

/** 网易云封面字段兼容：picUrl / blurPicUrl / coverImgUrl / pic 数字 ID */
function resolveNeteaseCover(...candidates: unknown[]): string {
  for (const raw of candidates) {
    if (raw == null) continue
    if (typeof raw === 'string') {
      const value = raw.trim()
      if (!value) continue
      if (value.startsWith('http://') || value.startsWith('https://') || value.startsWith('//')) {
        return value.startsWith('//') ? `https:${value}` : value
      }
      // 少数接口只返回 pic 哈希/数字串
      if (/^[A-Za-z0-9_-]+$/.test(value) && value.length >= 8) {
        return `https://p1.music.126.net/${value}.jpg`
      }
      continue
    }
    if (typeof raw === 'number' && Number.isFinite(raw) && raw > 0) {
      // 纯数字 pic 字段无法稳定还原 URL，跳过
      continue
    }
  }
  return ''
}

function assertNeteaseDetailResponse(data: any) {
  const code = Number(data?.code)
  if (code === 200) return
  const message = typeof data?.message === 'string' && data.message.trim()
    ? data.message.trim()
    : `Netease API code: ${Number.isFinite(code) ? code : 'invalid'}`
  throw new Error(message)
}

async function loadDetail() {
  const requestGeneration = ++detailRequestGeneration
  resetDetailState()
  const id = Number(route.params.id)
  if (!Number.isSafeInteger(id) || id <= 0) {
    error.value = t('player.load_failed')
    isLoading.value = false
    return
  }

  if (!auth.netease.loggedIn) {
    error.value = t('player.load_failed')
    isLoading.value = false
    return
  }
  const cacheScope = neteaseDetailCacheScope.value
  const cacheKey = cacheScope
    ? playlistDetailCacheKey(
      `${props.isAlbum ? 'netease-album' : 'netease-playlist'}:${cacheScope}`,
      id,
    )
    : null
  const cached = cacheKey
    ? readPlaylistDetailCache<NeteaseDetailCache>(cacheKey)
    : null
  if (cached) {
    applyDetailCache(cached)
    if (props.isAlbum) playCount.value = 0
    isLoading.value = false
  } else {
    isLoading.value = true
  }
  error.value = null

  try {
    if (props.isAlbum) {
      const data = await invoke<any>('get_album_detail', { albumId: id })
      if (requestGeneration !== detailRequestGeneration) return
      assertNeteaseDetailResponse(data)
      const album = data?.album || {}
      playlistName.value = album.name || ''
      const albumCover = resolveNeteaseCover(
        album.picUrl,
        album.blurPicUrl,
        album.picUrl_str,
        album.coverImgUrl,
        album.pic,
      )
      coverUrl.value = albumCover
      description.value = album.description || ''
      creator.value = album.artist?.name || ''

      const songs = data?.songs || []
      tracks.value = songs.map((s: any) => ({
        id: `netease:${s.id}`,
        title: s.name || '',
        artist: (s.ar || []).map((a: any) => a.name).join(', '),
        album: s.al?.name || album.name || '',
        durationMs: s.dt || 0,
        // 专辑曲目 al.picUrl 常为空，回退到专辑封面
        coverUrl: resolveNeteaseCover(s.al?.picUrl, s.al?.pic, albumCover) || albumCover,
        audioUrl: '',
      }))
      trackCount.value = tracks.value.length
      playCount.value = 0
    } else {
      const data = await invoke<any>('get_netease_playlist_detail', { playlistId: id })
      if (requestGeneration !== detailRequestGeneration) return
      assertNeteaseDetailResponse(data)
      const pl = data?.playlist || {}
      playlistName.value = pl.name || ''
      coverUrl.value = resolveNeteaseCover(pl.coverImgUrl, pl.picUrl, pl.cover)
      trackCount.value = pl.trackCount || 0
      playCount.value = pl.playCount || 0
      description.value = pl.description || ''
      creator.value = pl.creator?.nickname || ''

      const songs = pl.tracks || []
      tracks.value = songs.map((s: any) => ({
        id: `netease:${s.id}`,
        title: s.name || '',
        artist: (s.ar || []).map((a: any) => a.name).join(', '),
        album: s.al?.name || '',
        durationMs: s.dt || 0,
        coverUrl: resolveNeteaseCover(s.al?.picUrl, s.al?.pic),
        audioUrl: '',
      }))
    }
    if (cacheKey) saveDetailCache(cacheKey)
  } catch (e: any) {
    if (requestGeneration === detailRequestGeneration) {
      resetDetailState()
      error.value = e?.toString() || t('player.load_failed')
    }
  } finally {
    if (requestGeneration === detailRequestGeneration) {
      isLoading.value = false
    }
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

function playTrack(track: TrackInfo, index: number) {
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

function formatPlayCount(count: number): string {
  if (count >= 100000000) return (count / 100000000).toFixed(1) + t('common.hundred_million')
  if (count >= 10000) return (count / 10000).toFixed(1) + t('common.ten_thousand')
  return count.toString()
}

// 曲目右键菜单
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
  trackMenu.value = {
    show: true,
    x: e.clientX,
    y: e.clientY,
    track,
  }
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
  closeTrackMenu()
  addToPlaylistTarget.value = null
  addToPlaylistTargets.value = [...selectedTracks.value]
  leaveSelectionMode()
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

watch(() => [route.params.id, props.isAlbum], () => {
  searchQuery.value = ''
  leaveSelectionMode()
  void loadDetail()
})

watch(neteaseSessionFingerprint, () => {
  searchQuery.value = ''
  leaveSelectionMode()
  resetDetailState()
  void loadDetail()
})
</script>

<template>
  <div class="detail-view">
    <header class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="header-search" v-if="!isLoading && tracks.length > 0">
        <span class="material-symbols-rounded search-icon">search</span>
        <input
          v-model="searchQuery"
          :placeholder="t('player.search_tracks')"
          class="search-input"
        />
      </div>
    </header>

    <!-- 加载状态 -->
    <div v-if="isLoading" class="state-center">
      <span class="material-symbols-rounded spinning">progress_activity</span>
      <p>{{ t('player.loading') }}</p>
    </div>

    <!-- 错误状态 -->
    <div v-else-if="error" class="state-center">
      <span class="material-symbols-rounded" style="font-size: 48px; opacity: 0.3">error</span>
      <p>{{ error }}</p>
      <button class="retry-btn" @click="loadDetail">{{ t('player.retry') }}</button>
    </div>

    <template v-else>
      <!-- 歌单 / 专辑 信息头 -->
      <div class="detail-hero">
        <div class="hero-cover">
          <BilibiliCoverImage v-if="coverUrl" :src="coverUrl" />
          <span v-else class="material-symbols-rounded filled" style="font-size: 48px; opacity: 0.3">queue_music</span>
        </div>
        <div class="hero-info">
          <h1 class="hero-title">{{ playlistName }}</h1>
          <p v-if="creator" class="hero-creator">{{ creator }}</p>
          <p class="hero-meta">
            {{ t('player.track_count', { count: trackCount }) }} · {{ totalDuration }}
            <span v-if="playCount"> · {{ formatPlayCount(playCount) }}</span>
          </p>
          <p v-if="description" class="hero-desc">{{ description }}</p>
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

      <!-- 歌曲列表 -->
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
          v-for="(track, index) in filteredTracks"
          :key="track.id"
          class="track-item"
          :class="{ active: player.currentTrack?.id === track.id, selected: selectionMode && selectedIds.has(track.id), 'selection-mode': selectionMode }"
          @click="playTrack(track, index)"
          @contextmenu.prevent.stop="openTrackContextMenu($event, track)"
        >
          <button v-if="selectionMode" class="track-select" @click.stop="toggleSelected(track.id)">
            <span class="material-symbols-rounded filled">{{ selectedIds.has(track.id) ? 'check_circle' : 'radio_button_unchecked' }}</span>
          </button>
          <div v-else class="track-index">
            <div v-if="player.currentTrack?.id === track.id && player.isPlaying" class="equalizer-bars"><span class="bar"/><span class="bar"/><span class="bar"/></div>
            <span v-else class="index-num">{{ index + 1 }}</span>
          </div>
          <div class="track-cover">
            <BilibiliCoverImage
              v-if="track.coverUrl || (props.isAlbum && coverUrl)"
              :src="track.coverUrl || coverUrl"
              loading="lazy"
            />
            <span v-else class="material-symbols-rounded filled">music_note</span>
          </div>
          <div class="track-info">
            <div class="track-title">{{ track.title }}</div>
            <div class="track-meta">{{ track.artist }}<template v-if="track.album"> · {{ track.album }}</template></div>
          </div>
          <div class="track-duration">{{ formatDuration(track.durationMs) }}</div>
          <button v-if="!selectionMode" class="track-more" @click.stop="openTrackMenu($event, track)">
            <span class="material-symbols-rounded">more_vert</span>
          </button>
        </div>
        </div>
      </div>
    </template>

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

.track-more {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  opacity: 0;
  transition: opacity var(--duration-short), background var(--duration-short);

  .track-item:hover & { opacity: 0.6; }
  &:hover { opacity: 1 !important; background: var(--md-surface-container-high); }
  .material-symbols-rounded { font-size: 18px; }
}
</style>
