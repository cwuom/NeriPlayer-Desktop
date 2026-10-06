<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { normalizeTrack, usePlayerStore, type TrackInfo } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { useArtistFavorite } from '@/modules/library/favoriteArtistState'
import { playlistDetailCacheKey, readPlaylistDetailCache, writePlaylistDetailCache } from '@/modules/library/playlistDetailCache'
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
const query = ref('')
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
const header = computed(() => detail.value?.header || {
  name: String(route.query.name || ''), coverUrl: String(route.query.cover || ''), bannerUrl: '', description: '',
})
const tracks = computed(() => detail.value?.tracks || [])
const activeTracks = computed(() => selectedContent.value ? collection.value?.tracks || [] : tracks.value)
const filteredTracks = computed(() => {
  const search = query.value.trim().toLocaleLowerCase()
  return activeTracks.value.filter(track => !search || `${track.title} ${track.artist}`.toLocaleLowerCase().includes(search))
})
const visibleContents = computed(() => {
  const pool = activeTab.value === 'collections' ? contents.value?.collections || [] : contents.value?.series || []
  const search = query.value.trim().toLocaleLowerCase()
  return pool.filter(content => !search || `${content.name} ${content.description}`.toLocaleLowerCase().includes(search))
})
const { following, changing, toggle } = useArtistFavorite(computed(() => ({
  source: 'biliArtist', id: mid.value, name: header.value.name, coverUrl: header.value.coverUrl,
  trackCount: detail.value?.total || tracks.value.length,
})))

async function load(more = false) {
  const id = mid.value
  if (!/^[1-9]\d*$/.test(id) || (more && (loadingMore.value || !detail.value?.hasMore))) return
  const request = more ? generation : ++generation
  const cacheKey = playlistDetailCacheKey('bili-artist-v1', id)
  if (!more) {
    detail.value = readPlaylistDetailCache<ArtistDetail>(cacheKey)
    query.value = ''
    loading.value = true
  } else loadingMore.value = true
  error.value = ''
  try {
    const loaded = await invoke<ArtistDetail>('get_bili_artist_detail', { mid: Number(id), page: more ? (detail.value?.page || 1) + 1 : 1 })
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
function playTrack(track: TrackInfo) { player.playAll(filteredTracks.value, track.id) }

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
  if (!more) { selectedContent.value = content; collection.value = null; query.value = '' }
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
  } catch (cause) { if (request === collectionGeneration && artistRequest === generation) collectionError.value = String(cause) }
  finally { if (request === collectionGeneration && artistRequest === generation) loadingCollection.value = false }
}

function selectTab(tab: 'videos' | 'collections' | 'series') {
  activeTab.value = tab
  selectedContent.value = null
  collectionGeneration++
  loadingCollection.value = false
  query.value = ''
}
watch(mid, () => {
  contentsGeneration++
  contents.value = null
  selectedContent.value = null
  collectionGeneration++
  activeTab.value = 'videos'
  loadingContents.value = false
  loadingCollection.value = false
  void load()
  void loadContents()
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
      <div v-if="header.bannerUrl" class="creator-banner"><BilibiliCoverImage :src="header.bannerUrl" /></div>
      <div class="creator-identity">
        <div class="creator-avatar"><BilibiliCoverImage v-if="header.coverUrl" :src="header.coverUrl" /><span v-else class="material-symbols-rounded">account_circle</span></div>
        <div class="creator-name"><h1>{{ header.name }}</h1><p>{{ t('player.source_bilibili') }} · {{ t('player.track_count', { count: detail?.total || tracks.length }) }}</p></div>
        <button class="creator-follow" :class="{ active: following }" :disabled="changing || !header.name" @click="toggleFollow"><span class="material-symbols-rounded">{{ following ? 'check' : 'person_add' }}</span>{{ t(following ? 'player.artist_unsubscribe' : 'player.artist_subscribe') }}</button>
      </div>
      <p v-if="header.description" class="creator-description">{{ header.description }}</p>
      <button class="play-all-btn" :disabled="!activeTracks.length" @click="player.playAll(activeTracks)"><span class="material-symbols-rounded filled">play_arrow</span>{{ t('player.play_all') }}</button>
    </section>
    <div class="creator-tabs">
      <button v-for="tab in (['videos', 'collections', 'series'] as const)" :key="tab" :class="{ active: activeTab === tab }" @click="selectTab(tab)">{{ t(`player.artist_${tab}`) }}</button>
    </div>
    <div v-if="activeTab === 'videos' && loading && !tracks.length" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
    <div v-else-if="activeTab === 'videos' && error && !tracks.length" class="state-center"><p>{{ error }}</p><button class="retry-btn" @click="load()">{{ t('player.retry') }}</button></div>
    <template v-else>
      <div class="creator-section-heading"><h2>{{ selectedContent?.name || t(`player.artist_${activeTab}`) }}</h2><button v-if="selectedContent" class="creator-more" @click="selectTab(activeTab)">{{ t('player.artist_collection_back') }}</button><div class="header-search"><span class="material-symbols-rounded search-icon">search</span><input v-model="query" class="search-input" :placeholder="t('library.tab_search_hint')" :aria-label="t('library.tab_search_hint')" /></div></div>
      <template v-if="activeTab !== 'videos' && !selectedContent">
        <div v-if="contentsError" class="creator-error"><span>{{ contentsError }}</span><button @click="loadContents()">{{ t('player.retry') }}</button></div>
        <div v-if="loadingContents && !contents" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
        <div v-else class="creator-grid"><button v-for="content in visibleContents" :key="content.kind + content.id" class="creator-card" @click="loadCollection(content)"><div class="creator-card-cover"><BilibiliCoverImage v-if="content.coverUrl" :src="content.coverUrl" loading="lazy" /><span v-else class="material-symbols-rounded">video_library</span></div><span class="creator-card-title">{{ content.name }}</span><span class="creator-card-subtitle">{{ t('player.track_count', { count: content.total }) }}</span></button></div>
        <button v-if="contents?.hasMore" class="creator-more" :disabled="loadingContents" @click="loadContents(true)">{{ t(loadingContents ? 'common.loading' : 'player.artist_load_more') }}</button>
      </template>
      <template v-else>
      <div v-if="selectedContent ? collectionError : error" class="creator-error"><span>{{ selectedContent ? collectionError : error }}</span><button @click="selectedContent ? loadCollection(selectedContent, !!collection) : load(failedLoadMore)">{{ t('player.retry') }}</button></div>
      <div v-if="loadingCollection && !collection" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
      <div class="track-list">
        <button v-for="(track, index) in filteredTracks" :key="track.id" class="track-item" :class="{ active: player.currentTrack?.id === track.id }" @click="playTrack(track)">
          <span class="track-index">{{ index + 1 }}</span><div class="track-cover"><BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy" /></div><div class="track-info"><div class="track-title">{{ track.title }}</div><div class="track-meta">{{ track.artist }}</div></div><span class="track-duration">{{ formatTrackDuration(track.durationMs) }}</span>
        </button>
      </div>
      <p v-if="!loadingCollection && !filteredTracks.length" class="creator-empty">{{ t('player.artist_songs_empty') }}</p>
      <button v-if="selectedContent ? collection?.hasMore : detail?.hasMore" class="creator-more" :disabled="loadingMore || loadingCollection" @click="selectedContent ? loadCollection(selectedContent, true) : load(true)">{{ t(loadingMore || loadingCollection ? 'common.loading' : 'player.artist_load_more') }}</button>
      </template>
    </template>
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;
@use '@/modules/library/artistDetail.scss' as *;
</style>
