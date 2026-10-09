<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import {
  groupLocalArtists,
  loadArtistSourceTracks,
  localArtistStableKey,
} from '@/modules/library/localArtists'
import { recordPlaylistOpen } from '@/modules/library/playlistUsage'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { createLogger } from '@/utils/logger'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'

const log = createLogger('local-artist-view')

const route = useRoute()
const router = useRouter()
const player = usePlayerStore()
const { t } = useI18n()

const loading = ref(true)
const tracks = ref<TrackInfo[]>([])
const searchQuery = ref('')
const filteredTracks = computed(() => {
  const keyword = searchQuery.value.trim().toLocaleLowerCase()
  if (!keyword) return tracks.value
  return tracks.value.filter(track =>
    [track.title, track.artist, track.album].some(value =>
      (value || '').toLocaleLowerCase().includes(keyword)),
  )
})

const artistName = computed(() => String(route.params.name ?? ''))
const totalDurationMs = computed(() =>
  tracks.value.reduce((sum, track) => sum + (track.durationMs || 0), 0),
)

function formatTotal(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000)
  const hours = Math.floor(minutes / 60)
  // 使用本地化时长单位（对齐 LocalPlaylistView），避免中日文界面显示 '3h 12m'
  return hours > 0
    ? `${hours}${t('common.hour_short')} ${minutes % 60}${t('common.minute_short')}`
    : `${minutes}${t('common.minute_short')}`
}

async function load() {
  loading.value = true
  try {
    const all = await loadArtistSourceTracks()
    const wanted = localArtistStableKey(artistName.value)
    const artist = groupLocalArtists(all, t('library.local_artist_unknown')).find(
      (entry) => entry.key === wanted,
    )
    tracks.value = artist?.tracks ?? []
    recordPlaylistOpen({
      source: 'localArtist',
      name: artist?.name || artistName.value,
      coverUrl: tracks.value.find(track => track.coverUrl)?.coverUrl,
      trackCount: tracks.value.length,
    })
  } catch (e) {
    log.error('load local artist failed:', e)
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

function playTrack(track: TrackInfo) {
  player.playAll(filteredTracks.value, track.id)
}

onMounted(load)
</script>

<template>
  <div class="detail-view">
    <div class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="header-title">{{ artistName }}</div>
      <div v-if="!loading && tracks.length > 0" class="header-search">
        <span class="material-symbols-rounded search-icon" aria-hidden="true">search</span>
        <input
          v-model="searchQuery"
          class="search-input"
          :placeholder="t('player.search_tracks')"
          :aria-label="t('player.search_tracks')"
          @keydown.esc="searchQuery = ''"
        />
        <button
          v-if="searchQuery"
          class="search-clear"
          :aria-label="t('common.clear')"
          :title="t('common.clear')"
          @click="searchQuery = ''"
        >
          <span class="material-symbols-rounded" aria-hidden="true">close</span>
        </button>
      </div>
    </div>

    <div v-if="loading" class="empty-state">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <template v-else-if="tracks.length === 0">
      <div class="empty-state">
        <span class="material-symbols-rounded">account_circle</span>
        <p>{{ t('library.local_artist_empty') }}</p>
      </div>
    </template>

    <template v-else>
      <div class="artist-summary">
        <span>{{ t('player.track_count', { count: tracks.length }) }}</span>
        <span class="dot">·</span>
        <span>{{ formatTotal(totalDurationMs) }}</span>
      </div>

      <div class="artist-actions">
        <button class="primary-action" @click="playAll">
          <span class="material-symbols-rounded filled" style="font-size: 20px">play_arrow</span>
          <span>{{ t('player.play_all') }}</span>
        </button>
        <button class="secondary-action" @click="shufflePlay">
          <span class="material-symbols-rounded" style="font-size: 20px">shuffle</span>
          <span>{{ t('player.shuffle_play') }}</span>
        </button>
      </div>

      <div v-if="filteredTracks.length === 0" class="empty-state">
        <span class="material-symbols-rounded" aria-hidden="true">search_off</span>
        <p>{{ t('player.no_results') }}</p>
      </div>
      <div v-else class="track-list">
        <div
          v-for="(track, index) in filteredTracks"
          :key="track.id"
          class="track-item"
          :class="{ active: player.currentTrack?.id === track.id }"
          @click="playTrack(track)"
          @pointerenter="player.prefetchIntent(track)"
          @focusin="player.prefetchIntent(track)"
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
            <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy">
              <span class="material-symbols-rounded filled">music_note</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled">music_note</span>
          </div>
          <div class="track-info">
            <div class="track-title">{{ track.title }}</div>
            <div class="track-meta">{{ track.album }}</div>
          </div>
          <div class="track-duration">{{ formatDuration(track.durationMs) }}</div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;

.header-title {
  flex: 1;
  min-width: 0;
  font-size: 18px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.header-search {
  flex: 0 1 320px;
  min-width: 0;

  &:focus-within { outline: 2px solid var(--md-primary); }
}

.search-input { min-width: 0; }

.search-clear {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);

  &:hover { background: var(--md-surface-container-high); }
  .material-symbols-rounded { font-size: 18px; }
}

@media (max-width: 600px) {
  .detail-header { flex-wrap: wrap; }
  .header-search { flex-basis: 100%; max-width: none; }
}

.artist-summary {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--md-on-surface-variant);
  margin-bottom: 16px;

  .dot { opacity: 0.5; }
}

.artist-actions {
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

.spinning { animation: artist-spin 1s linear infinite; }

@keyframes artist-spin {
  to { transform: rotate(360deg); }
}

@media (prefers-reduced-motion: reduce) {
  .primary-action,
  .secondary-action { transition: none; }
  .spinning { animation: none; }
}
</style>
