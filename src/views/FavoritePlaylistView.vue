<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { normalizeTrack, usePlayerStore, type TrackInfo } from '@/stores/player'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import LocateTrackFab from '@/components/LocateTrackFab.vue'
import { useLocateCurrentTrack } from '@/composables/useLocateCurrentTrack'
import { useIncrementalList } from '@/composables/useIncrementalList'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import { createLogger } from '@/utils/logger'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'

const log = createLogger('favorite-playlist-view')

const route = useRoute()
const router = useRouter()
const player = usePlayerStore()
const { t } = useI18n()

const loading = ref(true)
const name = ref('')
const source = ref('')
const coverUrl = ref('')
const tracks = ref<TrackInfo[]>([])

const favoriteId = computed(() => String(route.params.id ?? ''))

function platformLabel(value: string): string {
  const key = value.toLowerCase()
  if (key.includes('netease')) return t('player.source_netease')
  if (key.includes('bili')) return t('player.source_bilibili')
  if (key.includes('youtube')) return 'YouTube Music'
  if (key.includes('qq')) return t('player.source_qq')
  return value
}

async function load() {
  loading.value = true
  try {
    const raw = await invoke<any[]>('list_favorite_playlists')
    const found = (raw || []).find((item: any) => String(item?.id ?? '') === favoriteId.value)
    if (!found) {
      tracks.value = []
      return
    }
    name.value = found.name ?? ''
    source.value = found.source ?? ''
    coverUrl.value = found.coverUrl ?? found.cover_url ?? ''
    // 收藏项的曲目随同步数据一起落地，这里不需要再打平台接口，
    // 断网或平台接口异常时也能正常打开
    // tracks 由后端从同步内部模型转换而来 (id/title/coverUrl 可直接播放), songs 是原始同步数据
    tracks.value = (found.tracks || found.songs || [])
      .map(normalizeTrack)
      .filter((track: TrackInfo) => !!track.id && !!track.title)
  } catch (e) {
    log.error('load favorite playlist failed:', e)
    tracks.value = []
  } finally {
    loading.value = false
  }
}

function playAll() {
  if (tracks.value.length) player.playAll(tracks.value)
}

function shufflePlay() {
  if (tracks.value.length) player.shufflePlay(tracks.value)
}

function playTrack(index: number) {
  player.playAll(tracks.value, tracks.value[index]?.id)
}

// 行上右键：光标处弹出（对齐其它列表页）
const trackMenu = ref<{ show: boolean; x: number; y: number; track: TrackInfo | null }>({
  show: false,
  x: 0,
  y: 0,
  track: null,
})

function openTrackContextMenu(e: MouseEvent, track: TrackInfo) {
  trackMenu.value = { show: true, x: e.clientX, y: e.clientY, track }
}

function closeTrackMenu() {
  trackMenu.value.show = false
}

const trackMenuItems = computed<ContextMenuItem[]>(() => [
  createContextMenuItem(t('player.play_next'), { id: 'play-next', icon: 'queue_play_next' }),
  createContextMenuItem(t('player.add_to_queue'), { id: 'add-to-queue', icon: 'add_to_queue' }),
])

function handleTrackMenuClick(item: ContextMenuActionItem) {
  const track = trackMenu.value.track
  if (!track) return
  switch (item.id) {
    case 'play-next':
      player.addToQueueNext(track)
      break
    case 'add-to-queue':
      player.addToQueueEnd(track)
      break
  }
  closeTrackMenu()
}

// 大列表分块渲染（收藏歌单可能很长，WebKitGTK 一次性渲染会卡顿）
const { visibleItems: visibleTracks, onScroll: onTrackListScroll, ensureIndex: ensureTrackIndex } =
  useIncrementalList(() => tracks.value)

// 定位到当前播放
const viewRef = ref<HTMLElement | null>(null)
const currentRowKey = computed(() => {
  const id = player.currentTrack?.id
  if (!id) return null
  return tracks.value.some(item => item.id === id) ? id : null
})
const { fabVisible: locateFabVisible, locate: locateCurrentTrack } = useLocateCurrentTrack({
  containerRef: viewRef,
  currentKey: currentRowKey,
  ensureRendered: key => ensureTrackIndex(tracks.value.findIndex(item => item.id === key)),
})

onMounted(load)
</script>

<template>
  <div ref="viewRef" class="detail-view" @scroll="onTrackListScroll">
    <div class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="header-title">{{ name || t('library.tab_favorites') }}</div>
    </div>

    <div v-if="loading" class="empty-state">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <div v-else-if="tracks.length === 0" class="empty-state">
      <span class="material-symbols-rounded">bookmark</span>
      <p>{{ t('library.favorite_empty_songs') }}</p>
    </div>

    <template v-else>
      <div class="fav-summary">
        <span>{{ t('player.track_count', { count: tracks.length }) }}</span>
        <span class="dot">·</span>
        <span>{{ platformLabel(source) }}</span>
      </div>

      <div class="fav-actions">
        <button class="primary-action" @click="playAll">
          <span class="material-symbols-rounded filled" style="font-size: 20px">play_arrow</span>
          <span>{{ t('player.play_all') }}</span>
        </button>
        <button class="secondary-action" @click="shufflePlay">
          <span class="material-symbols-rounded" style="font-size: 20px">shuffle</span>
          <span>{{ t('player.shuffle_play') }}</span>
        </button>
      </div>

      <div class="track-list">
        <div
          v-for="(track, index) in visibleTracks"
          :key="track.id + '-' + index"
          class="track-item"
          :class="{ active: player.currentTrack?.id === track.id }"
          :data-track-key="track.id"
          @click="playTrack(index)"
          @contextmenu.prevent.stop="openTrackContextMenu($event, track)"
        >
          <div class="track-index">
            <div
              v-if="player.currentTrack?.id === track.id && player.isPlaying"
              class="equalizer-bars"
            >
              <span class="bar" /><span class="bar" /><span class="bar" />
            </div>
            <span v-else class="index-num">{{ index + 1 }}</span>
          </div>
          <div class="track-cover">
            <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy" />
            <span v-else class="material-symbols-rounded filled">music_note</span>
          </div>
          <div class="track-info">
            <div class="track-title">{{ track.title }}</div>
            <div class="track-meta">{{ track.artist }}</div>
          </div>
          <div class="track-duration">{{ formatDuration(track.durationMs) }}</div>
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
      @update:open="(value: boolean) => { if (!value) closeTrackMenu() }"
      @click="handleTrackMenuClick"
    />
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;

.header-title {
  font-size: 18px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fav-summary {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--md-on-surface-variant);
  margin-bottom: 16px;

  .dot { opacity: 0.5; }
}

.fav-actions {
  display: flex;
  gap: 10px;
  margin-bottom: 20px;
}

.primary-action,
.secondary-action {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 40px;
  padding: 0 20px;
  border-radius: var(--radius-full);
  font-size: 14px;
  font-weight: 500;
  transition:
    background var(--duration-short) var(--easing-standard, ease),
    transform 120ms var(--easing-emphasized, cubic-bezier(0.2, 0, 0, 1));

  &:active { transform: scale(0.97); }
}

.primary-action {
  background: var(--md-primary);
  color: var(--md-on-primary);

  &:hover { filter: brightness(1.06); }
}

.secondary-action {
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);

  &:hover { background: var(--md-surface-container-highest); }
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 64px 0;
  color: var(--md-on-surface-variant);

  .material-symbols-rounded { font-size: 44px; opacity: 0.4; }
  p { font-size: 14px; opacity: 0.7; }
}

.spinning { animation: fav-spin 1s linear infinite; }

@keyframes fav-spin {
  to { transform: rotate(360deg); }
}
</style>
