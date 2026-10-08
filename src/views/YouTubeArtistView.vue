<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { useArtistFavorite } from '@/modules/library/favoriteArtistState'
import { parseYouTubeArtistDetail, parseYouTubeArtistItems, youtubeArtistItemTrack, type YouTubeArtistDetail, type YouTubeArtistItem, type YouTubeArtistSection } from '@/modules/library/youtubeArtistDetail'
import { playlistDetailCacheKey, previewCachedDetail, writePlaylistDetailCache } from '@/modules/library/playlistDetailCache'
import { formatTrackDuration } from '@/utils/timeFormat'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const player = usePlayerStore()
const toast = useToastStore()
const browseId = computed(() => String(route.params.browseId || ''))
const detail = ref<YouTubeArtistDetail | null>(null)
const loading = ref(false)
const error = ref('')
const sectionLoading = ref('')
const queueLoading = ref(false)
const query = ref('')
const sectionPages = ref<Record<string, { items: YouTubeArtistItem[]; continuation: string }>>({})
let generation = 0
// detail 当前属于哪位创作者
let detailId = ''
const header = computed(() => detail.value?.header || {
  name: String(route.query.name || ''), coverUrl: String(route.query.cover || ''), subtitle: String(route.query.subtitle || ''), description: '', subscribers: '', listeners: '',
})
const { following, changing, toggle } = useArtistFavorite(computed(() => ({
  source: 'youtubeMusicArtist', id: '', browseId: browseId.value, name: header.value.name,
  coverUrl: header.value.coverUrl, subtitle: header.value.subtitle, trackCount: 0,
})))
const firstPlayableSection = computed(() => detail.value?.sections.find(section => section.items.some(item => item.videoId)))
function sectionKey(section: YouTubeArtistSection) { return `${section.title}:${section.moreEndpoint?.browseId || ''}:${section.moreEndpoint?.params || ''}` }
function sectionItems(section: YouTubeArtistSection) { return sectionPages.value[sectionKey(section)]?.items || section.items }
function filteredItems(section: YouTubeArtistSection) {
  const search = query.value.trim().toLocaleLowerCase()
  return sectionItems(section).filter(item => !search || `${item.title} ${item.subtitle} ${item.artist}`.toLocaleLowerCase().includes(search))
}
function canLoadSection(section: YouTubeArtistSection) {
  const page = sectionPages.value[sectionKey(section)]
  return !!section.moreEndpoint && (!page || !!page.continuation)
}

async function load() {
  const id = browseId.value
  if (!id) return
  const request = ++generation
  const cacheKey = playlistDetailCacheKey('youtube-artist-v1', id)
  // 换了创作者就先撤下上一位的内容，同一位重试时保留已显示的列表
  if (detailId !== id) { detail.value = null; detailId = '' }
  const cached = previewCachedDetail<YouTubeArtistDetail>(cacheKey, (value) => {
    if (request !== generation) return false
    detail.value = value
    detailId = id
  })
  sectionPages.value = {}
  query.value = ''
  loading.value = true
  sectionLoading.value = ''
  queueLoading.value = false
  error.value = ''
  try {
    const raw = await invoke('get_youtube_artist_detail', { browseId: id })
    cached.markFresh()
    if (request !== generation) return
    detail.value = parseYouTubeArtistDetail(raw, header.value)
    detailId = id
    writePlaylistDetailCache(cacheKey, detail.value)
  } catch (cause) {
    if (request === generation) error.value = String(cause)
  } finally {
    if (request === generation) loading.value = false
  }
}

async function fetchSectionPage(section: YouTubeArtistSection) {
  const endpoint = section.moreEndpoint
  if (!endpoint) return sectionItems(section)
  const key = sectionKey(section)
  const previous = sectionPages.value[key]
  const request = generation
  const raw = await invoke('get_youtube_artist_items', {
    browseId: endpoint.browseId, params: endpoint.params || null, continuation: previous?.continuation || null,
  })
  if (request !== generation) return []
  const page = parseYouTubeArtistItems(raw)
  if (page.continuation && (!page.items.length || page.continuation === previous?.continuation)) {
    throw new Error('YouTube artist pagination did not advance')
  }
  const seen = new Set<string>()
  const items = [...(previous?.items || []), ...page.items].filter(item => {
    const identity = `${item.kind}:${item.videoId || item.browseId}`
    if (seen.has(identity)) return false
    seen.add(identity)
    return true
  })
  sectionPages.value = { ...sectionPages.value, [key]: { items, continuation: page.continuation } }
  return items
}

async function loadSection(section: YouTubeArtistSection) {
  if (sectionLoading.value || queueLoading.value) return
  const request = generation
  sectionLoading.value = sectionKey(section)
  try { await fetchSectionPage(section) } catch (cause) { if (request === generation) toast.error(String(cause)) }
  finally { if (request === generation) sectionLoading.value = '' }
}

async function playSection(section: YouTubeArtistSection, selected?: YouTubeArtistItem) {
  if (queueLoading.value || sectionLoading.value) return
  const request = generation
  queueLoading.value = true
  try {
    const key = sectionKey(section)
    if (section.moreEndpoint) {
      const visited = new Set<string>()
      if (!sectionPages.value[key]) await fetchSectionPage(section)
      if (request !== generation) return
      for (let page = 0; sectionPages.value[key]?.continuation; page++) {
        if (request !== generation) return
        const continuation = sectionPages.value[key]!.continuation
        if (page >= 200 || visited.has(continuation)) throw new Error('YouTube artist pagination exceeded budget')
        visited.add(continuation)
        await fetchSectionPage(section)
        if (request !== generation) return
      }
    }
    if (request !== generation) return
    const tracks = sectionItems(section).flatMap(item => {
      const track = youtubeArtistItemTrack(item, header.value.name)
      return track ? [track] : []
    })
    const selectedTrack = selected ? youtubeArtistItemTrack(selected, header.value.name) : null
    if (selectedTrack && !tracks.some(track => track.id === selectedTrack.id)) tracks.push(selectedTrack)
    if (tracks.length) player.playAll(tracks, selectedTrack?.id)
  } catch (cause) { if (request === generation) toast.error(String(cause)) }
  finally { if (request === generation) queueLoading.value = false }
}

function openItem(section: YouTubeArtistSection, item: YouTubeArtistItem) {
  if (item.videoId) { void playSection(section, item); return }
  if (!item.browseId) return
  router.push(item.kind === 'artist'
    ? { name: 'youtube-artist', params: { browseId: item.browseId }, query: { name: item.title, cover: item.coverUrl, subtitle: item.subtitle } }
    : { name: 'youtube-playlist', params: { browseId: item.browseId } })
}
async function toggleFollow() { try { await toggle() } catch (cause) { toast.error(String(cause)) } }
watch(browseId, () => { queueLoading.value = false; void load() }, { immediate: true })
onUnmounted(() => { generation++ })
</script>

<template>
  <div class="detail-view">
    <header class="detail-header">
      <button class="back-btn" :aria-label="t('common.back')" @click="router.back()"><span class="material-symbols-rounded">arrow_back</span></button>
      <div class="creator-page-title">{{ header.name || 'YouTube' }}</div>
      <div class="header-search"><span class="material-symbols-rounded search-icon">search</span><input v-model="query" class="search-input" :placeholder="t('library.tab_search_hint')" :aria-label="t('library.tab_search_hint')" /></div>
    </header>
    <section class="creator-hero">
      <div class="creator-identity">
        <div class="creator-avatar"><BilibiliCoverImage v-if="header.coverUrl" :src="header.coverUrl" /><span v-else class="material-symbols-rounded">account_circle</span></div>
        <div class="creator-name"><h1>{{ header.name }}</h1><p>{{ [header.subtitle, header.subscribers, header.listeners].filter(Boolean).join(' · ') }}</p></div>
        <button class="creator-follow" :class="{ active: following }" :disabled="changing || !header.name" @click="toggleFollow"><span class="material-symbols-rounded">{{ following ? 'check' : 'person_add' }}</span>{{ t(following ? 'player.artist_unsubscribe' : 'player.artist_subscribe') }}</button>
      </div>
      <p v-if="header.description" class="creator-description">{{ header.description }}</p>
      <button class="play-all-btn" :disabled="!firstPlayableSection || queueLoading" @click="firstPlayableSection && playSection(firstPlayableSection)"><span class="material-symbols-rounded filled">play_arrow</span>{{ t(queueLoading ? 'common.loading' : 'player.play_all') }}</button>
    </section>
    <div v-if="loading && !detail" class="state-center"><span class="material-symbols-rounded spinning">progress_activity</span></div>
    <div v-else-if="error && !detail" class="state-center"><p>{{ error }}</p><button class="retry-btn" @click="load()">{{ t('player.retry') }}</button></div>
    <template v-else>
      <div v-if="error" class="creator-error"><span>{{ error }}</span><button @click="load()">{{ t('player.retry') }}</button></div>
      <section v-for="section in detail?.sections || []" :key="sectionKey(section)" class="creator-section">
        <div class="creator-section-heading"><h2>{{ section.title }}</h2><button v-if="canLoadSection(section)" class="creator-more" :disabled="!!sectionLoading || queueLoading" @click="loadSection(section)">{{ t(sectionLoading === sectionKey(section) ? 'common.loading' : (sectionPages[sectionKey(section)] ? 'player.artist_load_more' : 'player.artist_section_more')) }}</button></div>
        <div v-if="section.items.some(item => item.videoId)" class="track-list">
          <button v-for="(item, index) in filteredItems(section)" :key="item.videoId || item.browseId" class="track-item" :disabled="queueLoading" :class="{ active: player.currentTrack?.id === `youtube:${item.videoId}` }" @click="openItem(section, item)">
            <span class="track-index">{{ index + 1 }}</span><div class="track-cover"><BilibiliCoverImage :src="item.coverUrl" loading="lazy"><span class="material-symbols-rounded filled">music_note</span></BilibiliCoverImage></div><div class="track-info"><div class="track-title">{{ item.title }}</div><div class="track-meta">{{ item.subtitle || item.artist }}</div></div><span class="track-duration">{{ formatTrackDuration(item.durationMs) }}</span>
          </button>
        </div>
        <div v-else class="creator-grid">
          <button v-for="item in filteredItems(section)" :key="item.browseId || item.videoId" class="creator-card" @click="openItem(section, item)"><div class="creator-card-cover" :class="{ round: item.kind === 'artist' }"><BilibiliCoverImage v-if="item.coverUrl" :src="item.coverUrl" loading="lazy" /><span v-else class="material-symbols-rounded">{{ item.kind === 'artist' ? 'account_circle' : 'album' }}</span></div><span class="creator-card-title">{{ item.title }}</span><span class="creator-card-subtitle">{{ item.subtitle }}</span></button>
        </div>
      </section>
      <p v-if="detail && !detail.sections.length" class="creator-empty">{{ t('player.artist_songs_empty') }}</p>
    </template>
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;
@use '@/modules/library/artistDetail.scss' as *;
</style>
