<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { normalizeTrack, usePlayerStore, type TrackInfo } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { useArtistFavorite } from '@/modules/library/favoriteArtistState'
import { playlistDetailCacheKey, previewCachedDetail, writePlaylistDetailCache, type CachedDetailPreview } from '@/modules/library/playlistDetailCache'
import { recordPlaylistOpen } from '@/modules/library/playlistUsage'
import { formatTrackDuration } from '@/utils/timeFormat'

interface ArtistHeader { name: string; coverUrl: string; bannerUrl: string; description: string }
interface ArtistDetail { header: ArtistHeader; tracks: TrackInfo[]; page: number; total: number; hasMore: boolean }
interface ArtistContent { id: string; kind: 'collection' | 'series'; name: string; coverUrl: string; description: string; total: number }
interface ArtistContents { collections: ArtistContent[]; series: ArtistContent[]; page: number; hasMore: boolean }
interface ArtistCollection { tracks: TrackInfo[]; page: number; total: number; hasMore: boolean }
const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const player = usePlayerStore()
const toast = useToastStore()
const mid = computed(() => String(route.params.mid || ''))
const detail = ref<ArtistDetail | null>(null)
const loading = ref(false)
const loadingMore = ref(false)
const error = ref('')
const failedLoadMore = ref(false)
const activeTab = ref<'videos' | 'collections' | 'series'>('videos')
const contents = ref<ArtistContents | null>(null)
const loadingContents = ref(false)
const contentsError = ref('')
const selectedContent = ref<ArtistContent | null>(null)
const collection = ref<ArtistCollection | null>(null)
const loadingCollection = ref(false)
const collectionError = ref('')
let generation = 0
let contentsGeneration = 0
let collectionGeneration = 0
// detail 当前属于哪位 UP 主
let detailMid = ''
const header = computed(() => detail.value?.header || {
  name: String(route.query.name || ''), coverUrl: String(route.query.cover || ''), bannerUrl: '', description: '',
})
const tracks = computed(() => detail.value?.tracks || [])
const activeTracks = computed(() => selectedContent.value ? collection.value?.tracks || [] : tracks.value)
const visibleContents = computed(() => activeTab.value === 'collections'
  ? contents.value?.collections || [] : contents.value?.series || [])
const tabIcons = { videos: 'smart_display', collections: 'video_library', series: 'playlist_play' }
const { following, changing, toggle } = useArtistFavorite(computed(() => ({
  source: 'biliArtist', id: mid.value, name: header.value.name, coverUrl: header.value.coverUrl,
  trackCount: detail.value?.total || tracks.value.length,
})))

async function load(more = false) {
  const id = mid.value
  if (!/^[1-9]\d*$/.test(id) || (more && (loadingMore.value || !detail.value?.hasMore))) return
  const request = more ? generation : ++generation
  const cacheKey = playlistDetailCacheKey('bili-artist-v1', id)
  let cached: CachedDetailPreview | null = null
  if (!more) {
    // 换了 UP 主就先撤下上一位的内容，同一位重试时保留已显示的列表
    if (detailMid !== id) { detail.value = null; detailMid = '' }
    loading.value = true
    cached = previewCachedDetail<ArtistDetail>(cacheKey, (value) => {
      if (request !== generation || mid.value !== id) return false
      detail.value = value
      detailMid = id
    })
  } else loadingMore.value = true
  error.value = ''
  try {
    const loaded = await invoke<ArtistDetail>('get_bili_artist_detail', { mid: Number(id), page: more ? (detail.value?.page || 1) + 1 : 1 })
    cached?.markFresh()
    if (request !== generation || mid.value !== id) return
    const parsed = loaded.tracks.map(normalizeTrack)
    const seen = new Set<string>()
    loaded.tracks = (more ? [...tracks.value, ...parsed] : parsed).filter(track => {
      if (seen.has(track.id)) return false
      seen.add(track.id)
      return true
    })
    loaded.header.name ||= header.value.name
    loaded.header.coverUrl ||= header.value.coverUrl
    detail.value = loaded
    detailMid = id
    writePlaylistDetailCache(cacheKey, loaded)
  } catch (cause) {
    if (request === generation) {
      error.value = String(cause)
      failedLoadMore.value = more
    }
  } finally {
    if (request === generation) { loading.value = false; loadingMore.value = false }
  }
}
async function toggleFollow() {
  try { await toggle() } catch (cause) { toast.error(String(cause)) }
}
function playTrack(track: TrackInfo) { player.playAll(activeTracks.value, track.id) }

async function loadContents(more = false) {
  if (loadingContents.value) return
  const request = ++contentsGeneration
  const id = mid.value
  loadingContents.value = true
  contentsError.value = ''
  try {
    const loaded = await invoke<ArtistContents>('get_bili_artist_contents', { mid: Number(id), page: more ? (contents.value?.page || 1) + 1 : 1 })
    if (request !== contentsGeneration || mid.value !== id) return
    if (more && contents.value) {
      for (const key of ['collections', 'series'] as const) {
        const seen = new Set<string>()
        loaded[key] = [...contents.value[key], ...loaded[key]].filter(content => {
          if (seen.has(content.id)) return false
          seen.add(content.id)
          return true
        })
      }
    }
    contents.value = loaded
  } catch (cause) { if (request === contentsGeneration) contentsError.value = String(cause) }
  finally { if (request === contentsGeneration) loadingContents.value = false }
}

async function loadCollection(content: ArtistContent, more = false) {
  if (more && loadingCollection.value) return
  const request = more ? collectionGeneration : ++collectionGeneration
  const artistRequest = generation
  if (!more) { selectedContent.value = content; collection.value = null }
  loadingCollection.value = true
  collectionError.value = ''
  try {
    const loaded = await invoke<ArtistCollection>('get_bili_artist_collection', {
      mid: Number(mid.value), contentId: Number(content.id), kind: content.kind,
      page: more ? (collection.value?.page || 1) + 1 : 1,
    })
    if (request !== collectionGeneration || artistRequest !== generation) return
    const parsed = loaded.tracks.map(normalizeTrack).map(track => ({ ...track, artist: track.artist || header.value.name, album: content.name }))
    const seen = new Set<string>()
    loaded.tracks = [...(more ? collection.value?.tracks || [] : []), ...parsed].filter(track => {
      if (seen.has(track.id)) return false
      seen.add(track.id)
      return true
    })
    collection.value = loaded
    if (!more) {
      recordPlaylistOpen({
        source: 'bili',
        id: content.id,
        mid: mid.value,
        subtype: content.kind === 'collection' ? 'COLLECTION' : 'SERIES',
        name: content.name,
        coverUrl: content.coverUrl,
        trackCount: loaded.total || loaded.tracks.length,
      })
    }
  } catch (cause) { if (request === collectionGeneration && artistRequest === generation) collectionError.value = String(cause) }
  finally { if (request === collectionGeneration && artistRequest === generation) loadingCollection.value = false }
}

function selectTab(tab: 'videos' | 'collections' | 'series') {
  activeTab.value = tab
  selectedContent.value = null
  collectionGeneration++
  loadingCollection.value = false
}
watch(() => [mid.value, route.query.contentId, route.query.kind], () => {
  contentsGeneration++
  contents.value = null
  selectedContent.value = null
  collectionGeneration++
  activeTab.value = 'videos'
  loadingContents.value = false
  loadingCollection.value = false
  void load()
  void loadContents()
  const contentId = String(route.query.contentId || '')
  const kind = route.query.kind
  if (/^[1-9]\d*$/.test(contentId) && (kind === 'collection' || kind === 'series')) {
    activeTab.value = kind === 'collection' ? 'collections' : 'series'
    void loadCollection({
      id: contentId, kind, name: String(route.query.name || ''), coverUrl: String(route.query.cover || ''),
      description: '', total: Number(route.query.count || 0),
    })
  }
}, { immediate: true })
onUnmounted(() => { generation++; contentsGeneration++; collectionGeneration++ })
</script>

<template>
  <div class="detail-view">
    <header class="detail-header">
      <button class="back-btn" :aria-label="t('common.back')" @click="router.back()"><span class="material-symbols-rounded">arrow_back</span></button>
      <div class="creator-page-title">{{ header.name || t('player.source_bilibili') }}</div>
    </header>
    <section class="creator-hero">
      <div class="creator-identity">
        <div class="creator-avatar"><BilibiliCoverImage v-if="header.coverUrl" :src="header.coverUrl" /><span v-else class="material-symbols-rounded">account_circle</span></div>
        <div class="creator-name"><h1>{{ header.name }}</h1><p>{{ t('player.source_bilibili') }} · {{ t('player.track_count', { count: detail?.total || tracks.length }) }}</p></div>
        <button class="creator-follow" :class="{ active: following }" :disabled="changing || !header.name" @click="toggleFollow"><span class="material-symbols-rounded">{{ following ? 'check' : 'person_add' }}</span>{{ t(following ? 'player.artist_unsubscribe' : 'player.artist_subscribe') }}</button>
      </div>
      <p v-if="header.description" class="creator-description">{{ header.description }}</p>
      <button class="play-all-btn" :disabled="!activeTracks.length" @click="player.playAll(activeTracks)"><span class="material-symbols-rounded filled">play_arrow</span>{{ t('player.play_all') }}</button>
    </section>
    <div class="artist-tabs">
      <button v-for="tab in (['videos', 'collections', 'series'] as const)" :key="tab" class="artist-tab" :class="{ active: activeTab === tab }" @click="selectTab(tab)">
        <span class="material-symbols-rounded">{{ tabIcons[tab] }}</span>
        <span>{{ t(`player.artist_${tab}`) }}</span>
      </button>
    </div>
    <Transition name="fade" mode="out-in">
    <div :key="`${activeTab}:${selectedContent?.id || ''}`" class="creator-tab-content">
    <div v-if="activeTab === 'videos' && loading && !tracks.length" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
    <div v-else-if="activeTab === 'videos' && error && !tracks.length" class="state-center"><p>{{ error }}</p><button class="retry-btn" @click="load()">{{ t('player.retry') }}</button></div>
    <template v-else>
      <div class="creator-section-heading"><h2>{{ selectedContent?.name || t(`player.artist_${activeTab}`) }}</h2><button v-if="selectedContent" class="creator-more" @click="selectTab(activeTab)">{{ t('player.artist_collection_back') }}</button></div>
      <template v-if="activeTab !== 'videos' && !selectedContent">
        <div v-if="contentsError" class="creator-error"><span>{{ contentsError }}</span><button @click="loadContents()">{{ t('player.retry') }}</button></div>
        <div v-if="loadingContents && !contents" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
        <div v-else class="creator-content-list">
          <button v-for="content in visibleContents" :key="content.kind + content.id" class="creator-content-item" @click="loadCollection(content)">
            <div class="creator-content-cover"><BilibiliCoverImage v-if="content.coverUrl" :src="content.coverUrl" loading="lazy" /><span v-else class="material-symbols-rounded">video_library</span></div>
            <div class="creator-content-info"><div class="creator-content-title">{{ content.name }}</div><div class="creator-content-meta">{{ t('player.track_count', { count: content.total }) }}</div></div>
            <span class="material-symbols-rounded creator-content-arrow">chevron_right</span>
          </button>
          <p v-if="!loadingContents && !contentsError && !visibleContents.length" class="creator-empty">{{ t('library.empty_title', { type: t(`player.artist_${activeTab}`) }) }}</p>
        </div>
        <button v-if="contents?.hasMore" class="creator-more" :disabled="loadingContents" @click="loadContents(true)">{{ t(loadingContents ? 'common.loading' : 'player.artist_load_more') }}</button>
      </template>
      <template v-else>
      <div v-if="selectedContent ? collectionError : error" class="creator-error"><span>{{ selectedContent ? collectionError : error }}</span><button @click="selectedContent ? loadCollection(selectedContent, !!collection) : load(failedLoadMore)">{{ t('player.retry') }}</button></div>
      <div v-if="loadingCollection && !collection" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
      <div class="track-list">
        <button v-for="(track, index) in activeTracks" :key="track.id" class="track-item" :class="{ active: player.currentTrack?.id === track.id }" @click="playTrack(track)">
          <span class="track-index">{{ index + 1 }}</span><div class="track-cover"><BilibiliCoverImage :src="track.coverUrl" loading="lazy"><span class="material-symbols-rounded filled">music_note</span></BilibiliCoverImage></div><div class="track-info"><div class="track-title">{{ track.title }}</div><div class="track-meta">{{ track.artist }}</div></div><span class="track-duration">{{ formatTrackDuration(track.durationMs) }}</span>
        </button>
      </div>
      <p v-if="!loadingCollection && !activeTracks.length" class="creator-empty">{{ t('player.artist_songs_empty') }}</p>
      <button v-if="selectedContent ? collection?.hasMore : detail?.hasMore" class="creator-more" :disabled="loadingMore || loadingCollection" @click="selectedContent ? loadCollection(selectedContent, true) : load(true)">{{ t(loadingMore || loadingCollection ? 'common.loading' : 'player.artist_load_more') }}</button>
      </template>
    </template>
    </div>
    </Transition>
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;
@use '@/modules/library/artistDetail.scss' as *;
@use '@/modules/library/artistTabs.scss' as *;

.creator-hero {
  border-radius: 28px;
  margin-bottom: 16px;
}

.creator-content-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.creator-content-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 12px;
  border-radius: 16px;
  text-align: left;
  transition: background var(--duration-short, 150ms);

  &:hover { background: var(--md-surface-container); }
  &:focus-visible { outline: 2px solid var(--md-primary); }
}

.creator-content-cover {
  width: 56px;
  height: 56px;
  border-radius: 12px;
  overflow: hidden;
  flex-shrink: 0;
  background: var(--md-surface-variant);
  display: flex;
  align-items: center;
  justify-content: center;

  :deep(img) { width: 100%; height: 100%; object-fit: cover; }
  .material-symbols-rounded { font-size: 26px; opacity: 0.4; }
}

.creator-content-info { flex: 1; min-width: 0; }
.creator-content-title { font-size: 14px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.creator-content-meta { margin-top: 2px; font-size: 12px; color: var(--md-on-surface-variant); }
.creator-content-arrow { font-size: 18px; opacity: 0.3; }
</style>
