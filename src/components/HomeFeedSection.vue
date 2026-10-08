<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { TrackInfo } from '@/stores/player'
import type { HomeSectionKind, HomeSectionState, HomePlaylist } from '@/modules/library/neteaseHome'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'

const props = defineProps<{
  definition: { key: string; kind: HomeSectionKind; titleKey: string; icon: string }
  section: HomeSectionState
  title?: string
}>()
const emit = defineEmits<{
  retry: []
  play: [songs: TrackInfo[], index: number]
  playlist: [playlist: HomePlaylist]
}>()
const { t, locale } = useI18n()
const playCountFormat = computed(() => new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }))
const page = ref(0)
const columns = ref(3)
const perPage = computed(() => columns.value * 3)
const pageCount = computed(() => Math.max(1, Math.ceil(props.section.songs.length / perPage.value)))
const pageSongs = computed(() => props.section.songs.slice(page.value * perPage.value, (page.value + 1) * perPage.value))
const hasItems = computed(() => props.section.songs.length > 0 || props.section.playlists.length > 0)

function updateColumns() {
  columns.value = window.innerWidth >= 840 ? 3 : window.innerWidth >= 600 ? 2 : 1
}

watch([() => props.section.songs, perPage], () => { page.value = 0 })
onMounted(() => {
  updateColumns()
  window.addEventListener('resize', updateColumns)
})
onUnmounted(() => window.removeEventListener('resize', updateColumns))
</script>

<template>
  <section class="home-feed-section" :data-home-source="definition.key">
    <div class="feed-header">
      <h2 class="feed-title"><span class="material-symbols-rounded">{{ definition.icon }}</span>{{ definition.titleKey ? t(definition.titleKey) : title }}</h2>
      <div v-if="definition.kind === 'songs' && pageCount > 1" class="feed-pages">
        <button :disabled="page === 0" :aria-label="t('home.previous_page')" @click="page--"><span class="material-symbols-rounded">chevron_left</span></button>
        <button :disabled="page >= pageCount - 1" :aria-label="t('home.next_page')" @click="page++"><span class="material-symbols-rounded">chevron_right</span></button>
      </div>
    </div>
    <div v-if="section.error" class="feed-state feed-error" role="status">
      <span>{{ t('home.recommend_load_failed') }}</span>
      <button @click="emit('retry')">{{ t('player.retry') }}</button>
    </div>
    <div v-if="hasItems && definition.kind === 'songs'" class="feed-songs" :style="{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }">
      <button v-for="(song, index) in pageSongs" :key="song.playlistKey ?? song.id" type="button" class="feed-song" @click="emit('play', section.songs, page * perPage + index)">
        <span class="song-number">{{ page * perPage + index + 1 }}</span>
        <span class="song-cover">
          <span class="material-symbols-rounded">music_note</span>
          <BilibiliCoverImage v-if="song.coverUrl" :src="song.coverUrl" :alt="song.title" loading="lazy" />
        </span>
        <span class="song-info">
          <span class="song-title">{{ song.title }}</span>
          <span class="song-meta">{{ song.artist }}<template v-if="song.album"> · {{ song.album }}</template></span>
        </span>
      </button>
    </div>
    <div v-else-if="hasItems" class="feed-playlists" :class="{ 'radar-playlists': definition.kind === 'radar' }">
      <button v-for="(playlist, index) in section.playlists" :key="playlist.id + ':' + index" type="button" class="feed-playlist" @click="emit('playlist', playlist)">
        <div class="playlist-image">
          <span class="material-symbols-rounded">queue_music</span>
          <BilibiliCoverImage v-if="playlist.coverUrl" :src="playlist.coverUrl" :alt="playlist.name" loading="lazy" />
          <span v-if="playlist.playCount > 0" class="playlist-plays">{{ playCountFormat.format(playlist.playCount) }}</span>
        </div>
        <div class="playlist-title">{{ playlist.name }}</div>
        <div v-if="playlist.trackCount > 0" class="playlist-meta">{{ t('player.track_count', { count: playlist.trackCount }) }}</div>
      </button>
    </div>
    <div v-else-if="section.loading" class="feed-state" role="status">
      <span class="material-symbols-rounded spinning">progress_activity</span>{{ t('player.loading') }}
    </div>
  </section>
</template>

<style scoped lang="scss">
.home-feed-section { margin-bottom: 32px; }
.feed-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 16px; }
.feed-title { display: flex; align-items: center; gap: 8px; color: var(--md-primary); font-size: 20px; font-weight: 700; }
.feed-title > .material-symbols-rounded { font-size: 23px; }
.feed-pages { display: flex; gap: 4px; }
.feed-pages button { display: grid; place-items: center; width: 32px; height: 32px; border: 1px solid var(--md-outline-variant); border-radius: var(--radius-full); }
.feed-pages button:disabled { opacity: 0.3; cursor: default; }
.feed-pages button:hover:not(:disabled) { background: var(--md-surface-container-high); }
.feed-songs { display: grid; grid-template-rows: repeat(3, minmax(68px, auto)); grid-auto-flow: column; gap: 6px 16px; }
.feed-song { display: flex; align-items: center; gap: 12px; min-width: 0; padding: 8px; text-align: left; border-radius: var(--radius-md); }
.feed-song:hover { background: var(--md-surface-container-high); }
.song-number { width: 20px; flex-shrink: 0; color: var(--md-primary); font-size: 13px; text-align: center; }
.song-cover { position: relative; display: grid; place-items: center; width: 48px; height: 48px; flex-shrink: 0; overflow: hidden; border-radius: var(--radius-sm); background: var(--md-surface-variant); }
.song-cover > .material-symbols-rounded { opacity: 0.4; }
.song-cover img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
.song-info { display: flex; flex: 1; min-width: 0; flex-direction: column; gap: 3px; }
.song-title, .song-meta, .playlist-title, .playlist-meta { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.song-title { font-size: 14px; font-weight: 550; }
.song-meta, .playlist-meta { font-size: 12px; color: var(--md-on-surface-variant); }
.feed-playlists { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 20px; }
.feed-playlist { min-width: 0; align-self: start; text-align: left; border-radius: var(--radius-md); }
.playlist-image { position: relative; display: grid; place-items: center; aspect-ratio: 1; overflow: hidden; border-radius: var(--radius-md); background: var(--md-surface-variant); }
.playlist-image > .material-symbols-rounded { font-size: 32px; opacity: 0.4; }
.playlist-image img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
.playlist-plays { position: absolute; top: 8px; right: 8px; padding: 3px 7px; border-radius: var(--radius-full); background: rgba(0, 0, 0, 0.45); color: white; font-size: 11px; }
.playlist-title { font-size: 14px; font-weight: 550; margin-top: 10px; }
.playlist-meta { margin-top: 3px; }
.radar-playlists { display: flex; overflow-x: auto; scrollbar-width: none; padding-bottom: 4px; }
.radar-playlists::-webkit-scrollbar { display: none; }
.radar-playlists .feed-playlist { flex: 0 0 calc((100% - 100px) / 6); min-width: 150px; }
.feed-state { display: flex; align-items: center; justify-content: center; gap: 10px; min-height: 72px; font-size: 13px; color: var(--md-on-surface-variant); }
.feed-error { color: var(--md-error); }
.feed-error button { color: var(--md-primary); padding: 5px 10px; border-radius: var(--radius-full); background: var(--md-surface-container); }
.spinning { animation: spin 1s linear infinite; }
button:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (max-width: 1000px) { .feed-playlists:not(.radar-playlists) { grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 16px; } }
@media (max-width: 700px) { .feed-playlists:not(.radar-playlists) { grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; } .feed-title { font-size: 18px; } }
@media (max-width: 480px) { .feed-playlists:not(.radar-playlists) { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (prefers-reduced-motion: reduce) { .spinning { animation: none; } }
</style>
