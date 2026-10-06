<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import {
  playlistDetailCacheKey,
  readPlaylistDetailCache,
  writePlaylistDetailCache,
} from '@/modules/library/playlistDetailCache'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'
import { resolveNeteaseCover } from '@/utils/neteaseCover'
import { createLogger } from '@/utils/logger'
import { useToastStore } from '@/stores/toast'
import { useArtistFavorite } from '@/modules/library/favoriteArtistState'

const log = createLogger('netease-artist-view')

const route = useRoute()
const router = useRouter()
const player = usePlayerStore()
const { t } = useI18n()
const toast = useToastStore()

interface ArtistHeader {
  name: string
  alias: string
  coverUrl: string
  avatarUrl: string
  briefDesc: string
  musicSize: number
  albumSize: number
}

interface ArtistAlbum {
  id: number
  name: string
  coverUrl: string
  publishYear: string
  size: number
}

interface ArtistDetailCache {
  header: ArtistHeader
  tracks: TrackInfo[]
  albums: ArtistAlbum[]
}

const isLoading = ref(true)
const error = ref<string | null>(null)
const header = ref<ArtistHeader | null>(null)
const tracks = ref<TrackInfo[]>([])
const albums = ref<ArtistAlbum[]>([])
const activeTab = ref<'songs' | 'albums'>('songs')

const artistId = computed(() => Number(route.params.id) || 0)
let generation = 0
const { following, changing, toggle } = useArtistFavorite(computed(() => ({
  source: 'neteaseArtist', id: String(artistId.value),
  name: header.value?.name || String(route.query.name || ''),
  coverUrl: header.value?.avatarUrl || header.value?.coverUrl || String(route.query.cover || ''),
  trackCount: header.value?.musicSize || tracks.value.length,
  subtitle: header.value?.alias || String(route.query.subtitle || ''),
})))
async function toggleFollow() {
  try { await toggle() } catch (cause) { toast.error(String(cause)) }
}

// 大列表窗口渲染, 与歌单详情页同策略
const RENDER_CHUNK = 100
const renderCount = ref(RENDER_CHUNK)
const visibleTracks = computed(() => tracks.value.slice(0, renderCount.value))
const hasMoreTracks = computed(() => renderCount.value < tracks.value.length)

function onViewScroll(e: Event) {
  const el = e.currentTarget as HTMLElement | null
  if (!el || !hasMoreTracks.value || activeTab.value !== 'songs') return
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 2400) {
    renderCount.value = Math.min(tracks.value.length, renderCount.value + RENDER_CHUNK)
  }
}

function parseHeader(raw: any): ArtistHeader {
  const artist = raw?.data?.artist || raw?.artist || {}
  const aliasList = Array.isArray(artist.alias) ? artist.alias.filter(Boolean) : []
  return {
    name: String(artist.name || route.query.name || ''),
    alias: aliasList.join(' / ') || String(route.query.subtitle || ''),
    coverUrl: resolveNeteaseCover(artist.cover, artist.picUrl) || String(route.query.cover || ''),
    avatarUrl: resolveNeteaseCover(artist.avatar, artist.img1v1Url),
    briefDesc: String(artist.briefDesc || ''),
    musicSize: Number(artist.musicSize) || 0,
    albumSize: Number(artist.albumSize) || 0,
  }
}

async function load() {
  const id = artistId.value
  if (!id) return
  const request = ++generation

  const cacheKey = playlistDetailCacheKey('netease-artist-v2', id)
  const cached = readPlaylistDetailCache<ArtistDetailCache>(cacheKey)
  if (cached) {
    header.value = cached.header
    tracks.value = cached.tracks
    albums.value = cached.albums
    isLoading.value = false
  } else {
    header.value = parseHeader(null)
    tracks.value = []
    albums.value = []
    isLoading.value = true
  }
  error.value = null

  try {
    // 头部/歌曲/专辑并行加载 (对齐 Android loadInitial)
    const [detailRes, songsRes, albumsRes] = await Promise.allSettled([
      invoke<any>('get_netease_artist_detail', { artistId: id }),
      invoke<any>('get_netease_artist_songs', { artistId: id }),
      invoke<any>('get_netease_artist_albums', { artistId: id }),
    ])
    if (request !== generation) return

    if (detailRes.status === 'fulfilled') {
      header.value = parseHeader(detailRes.value)
    } else if (!header.value) {
      header.value = parseHeader(null)
    }

    if (songsRes.status === 'fulfilled') {
      const songs = songsRes.value?.songs || []
      tracks.value = songs.map((s: any) => ({
        id: `netease:${s.id}`,
        title: s.name || '',
        artist: (s.ar || []).map((a: any) => a.name).join(', '),
        album: s.al?.name || '',
        durationMs: s.dt || 0,
        coverUrl: resolveNeteaseCover(s.al?.picUrl, s.al?.pic),
        audioUrl: '',
      }))
      renderCount.value = Math.min(RENDER_CHUNK, tracks.value.length)
    }

    if (albumsRes.status === 'fulfilled') {
      const rawAlbums = albumsRes.value?.hotAlbums || []
      albums.value = rawAlbums
        .filter((al: any) => al?.id && al?.name)
        .map((al: any) => ({
          id: Number(al.id),
          name: String(al.name),
          coverUrl: resolveNeteaseCover(al.picUrl, al.blurPicUrl, al.pic),
          publishYear: al.publishTime ? String(new Date(al.publishTime).getFullYear()) : '',
          size: Number(al.size) || 0,
        }))
    }

    if (songsRes.status === 'rejected' && albumsRes.status === 'rejected' && !cached) {
      error.value = String(songsRes.reason || albumsRes.reason)
      return
    }

    if (header.value) {
      writePlaylistDetailCache<ArtistDetailCache>(cacheKey, {
        header: header.value,
        tracks: tracks.value,
        albums: albums.value,
      })
    }
  } catch (e: any) {
    if (request !== generation) return
    if (!cached) error.value = e?.toString() || t('player.load_failed')
    log.error('load artist failed:', e)
  } finally {
    if (request === generation) isLoading.value = false
  }
}

function playAll() {
  if (tracks.value.length) player.playAll(tracks.value)
}

function playTrack(index: number) {
  player.playAll(tracks.value, tracks.value[index]?.id)
}

function openAlbum(album: ArtistAlbum) {
  router.push({ name: 'netease-album', params: { id: String(album.id) } })
}

const songCountLabel = computed(() =>
  t('player.track_count', { count: header.value?.musicSize || tracks.value.length }))
const albumCountLabel = computed(() =>
  t('player.artist_album_count', { count: header.value?.albumSize || albums.value.length }))

watch(artistId, () => { void load() }, { immediate: true })
onUnmounted(() => { generation++ })
</script>

<template>
  <div class="detail-view" @scroll.passive="onViewScroll">
    <header class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="artist-header-title">{{ header?.name || String(route.query.name || '') }}</div>
    </header>

    <div v-if="isLoading && !header" class="state-center">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <div v-else-if="error && !tracks.length && !albums.length" class="state-center">
      <span class="material-symbols-rounded" style="font-size: 48px; opacity: 0.3">error</span>
      <p>{{ error }}</p>
      <button class="retry-btn" @click="() => load()">{{ t('player.retry') }}</button>
    </div>

    <template v-else>
      <!-- Hero 卡片 (对齐 Android ArtistHeaderCard: 封面大图 + 底部渐变 + 圆头像/名称/别名) -->
      <div class="artist-hero-card">
        <div class="artist-hero-media">
          <BilibiliCoverImage
            v-if="header?.coverUrl || header?.avatarUrl"
            :src="header.coverUrl || header.avatarUrl"
            class="artist-hero-img"
          />
          <div v-else class="artist-hero-img artist-hero-img--empty">
            <span class="material-symbols-rounded" style="font-size: 56px; opacity: 0.3">account_circle</span>
          </div>
          <div class="artist-hero-scrim" />
          <div class="artist-hero-identity">
            <div class="artist-avatar">
              <BilibiliCoverImage
                v-if="header?.avatarUrl || header?.coverUrl"
                :src="header.avatarUrl || header.coverUrl"
              />
              <span v-else class="material-symbols-rounded">account_circle</span>
            </div>
            <div class="artist-identity-text">
              <h1>{{ header?.name || String(route.query.name || '') }}</h1>
              <p v-if="header?.alias">{{ header.alias }}</p>
            </div>
          </div>
        </div>
        <div class="artist-hero-body">
          <div class="artist-stat-chips">
            <span class="artist-stat-chip">{{ songCountLabel }}</span>
            <span class="artist-stat-chip">{{ albumCountLabel }}</span>
          </div>
          <p v-if="header?.briefDesc" class="artist-brief">{{ header.briefDesc }}</p>
          <div class="artist-hero-actions">
            <button class="play-all-btn" :disabled="!tracks.length" @click="playAll">
              <span class="material-symbols-rounded filled">play_arrow</span>
              {{ t('player.play_all') }}
            </button>
            <button class="artist-follow-btn" :class="{ active: following }" :disabled="changing || !header?.name" @click="toggleFollow">
              <span class="material-symbols-rounded">{{ following ? 'check' : 'person_add' }}</span>
              {{ t(following ? 'player.artist_unsubscribe' : 'player.artist_subscribe') }}
            </button>
          </div>
        </div>
      </div>

      <!-- 歌曲 / 专辑 Tab (对齐 Android PrimaryTabRow) -->
      <div class="artist-tabs">
        <button
          class="artist-tab"
          :class="{ active: activeTab === 'songs' }"
          @click="activeTab = 'songs'"
        >
          <span class="material-symbols-rounded">music_note</span>
          <span>{{ t('player.artist_tab_songs') }}</span>
        </button>
        <button
          class="artist-tab"
          :class="{ active: activeTab === 'albums' }"
          @click="activeTab = 'albums'"
        >
          <span class="material-symbols-rounded">library_music</span>
          <span>{{ t('player.artist_tab_albums') }}</span>
        </button>
      </div>

      <Transition name="fade" mode="out-in">
      <!-- 歌曲列表 -->
      <div v-if="activeTab === 'songs'" key="artist-songs" class="track-list">
        <div v-if="tracks.length === 0" class="state-center">
          <p>{{ t('player.artist_songs_empty') }}</p>
        </div>
        <div
          v-for="(track, index) in visibleTracks"
          :key="track.id"
          class="track-item"
          :class="{ active: player.currentTrack?.id === track.id }"
          @click="playTrack(index)"
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
            <div class="track-meta">{{ track.album || track.artist }}</div>
          </div>
          <div class="track-duration">{{ formatDuration(track.durationMs) }}</div>
        </div>
      </div>

      <!-- 专辑列表 -->
      <div v-else key="artist-albums" class="artist-album-list">
        <div v-if="albums.length === 0" class="state-center">
          <p>{{ t('player.artist_albums_empty') }}</p>
        </div>
        <div
          v-for="album in albums"
          :key="album.id"
          class="artist-album-item"
          role="button"
          tabindex="0"
          @click="openAlbum(album)"
          @keydown.enter="openAlbum(album)"
        >
          <div class="artist-album-cover">
            <BilibiliCoverImage v-if="album.coverUrl" :src="album.coverUrl" loading="lazy" />
            <span v-else class="material-symbols-rounded filled">album</span>
          </div>
          <div class="artist-album-info">
            <div class="artist-album-name">{{ album.name }}</div>
            <div class="artist-album-meta">
              {{ [album.publishYear, album.size ? t('player.track_count', { count: album.size }) : '']
                .filter(Boolean).join(' · ') }}
            </div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
      </div>
      </Transition>
    </template>
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;

.artist-header-title {
  font-size: 18px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

// Hero 卡片
.artist-hero-card {
  border-radius: 28px;
  overflow: hidden;
  background: var(--md-surface-container);
  margin-bottom: 16px;
}

.artist-hero-media {
  position: relative;
  height: 240px;
}

.artist-hero-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;

  :deep(img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  &--empty {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--md-surface-variant);
  }
}

.artist-hero-scrim {
  position: absolute;
  inset: 0;
  background: linear-gradient(to bottom, transparent 40%, rgba(0, 0, 0, 0.55));
}

.artist-hero-identity {
  position: absolute;
  left: 20px;
  right: 20px;
  bottom: 16px;
  display: flex;
  align-items: center;
  gap: 14px;
}

.artist-avatar {
  width: 64px;
  height: 64px;
  border-radius: 50%;
  overflow: hidden;
  flex-shrink: 0;
  background: var(--md-surface-variant);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.35);

  :deep(img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .material-symbols-rounded { font-size: 40px; opacity: 0.5; }
}

.artist-identity-text {
  min-width: 0;

  h1 {
    margin: 0;
    font-size: 24px;
    font-weight: 700;
    color: #fff;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-shadow: 0 1px 8px rgba(0, 0, 0, 0.4);
  }

  p {
    margin: 2px 0 0;
    font-size: 13px;
    color: rgba(255, 255, 255, 0.82);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.artist-hero-body {
  padding: 16px 20px 20px;
}

.artist-stat-chips {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.artist-stat-chip {
  display: inline-flex;
  align-items: center;
  height: 30px;
  padding: 0 14px;
  border-radius: var(--radius-full);
  border: 1px solid var(--md-outline-variant);
  color: var(--md-on-surface-variant);
  font-size: 12px;
  font-weight: 600;
}

.artist-brief {
  margin: 12px 0 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--md-on-surface-variant);
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.artist-hero-actions {
  margin-top: 14px;
  display: flex;
  gap: 10px;

  .play-all-btn { margin-top: 0; }
  .play-all-btn:disabled { opacity: 0.4; pointer-events: none; }
}

.artist-follow-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 10px 18px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-full);
  font-size: 13px;
  color: var(--md-primary);

  &.active { background: var(--md-secondary-container); color: var(--md-on-secondary-container); }
  &:disabled { opacity: 0.5; }
  .material-symbols-rounded { font-size: 18px; }
}

// 歌曲 / 专辑 Tab
.artist-tabs {
  display: flex;
  padding: 4px;
  border-radius: 24px;
  background: var(--md-surface-container);
  margin-bottom: 12px;
}

.artist-tab {
  flex: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 42px;
  border-radius: 20px;
  font-size: 14px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short, 150ms), color var(--duration-short, 150ms);

  .material-symbols-rounded { font-size: 19px; }

  &:hover { background: color-mix(in srgb, var(--md-on-surface) 6%, transparent); }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
  }
}

// 专辑行
.artist-album-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.artist-album-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 12px;
  border-radius: 16px;
  cursor: pointer;
  transition: background var(--duration-short, 150ms);

  &:hover { background: var(--md-surface-container); }
}

.artist-album-cover {
  width: 56px;
  height: 56px;
  border-radius: 12px;
  overflow: hidden;
  flex-shrink: 0;
  background: var(--md-surface-variant);
  display: flex;
  align-items: center;
  justify-content: center;

  :deep(img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .material-symbols-rounded { font-size: 26px; opacity: 0.4; }
}

.artist-album-info {
  flex: 1;
  min-width: 0;
}

.artist-album-name {
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.artist-album-meta {
  margin-top: 2px;
  font-size: 12px;
  color: var(--md-on-surface-variant);
}
</style>
