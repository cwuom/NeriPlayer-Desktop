<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { TrackInfo } from '@/stores/player'
import type { HomeSectionKind, HomeSectionState, HomePlaylist } from '@/modules/library/neteaseHome'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'

const props = defineProps<{
  definition: { key: string; kind: HomeSectionKind; titleKey: string; icon: string }
  section: HomeSectionState
  title?: string
  /** 并排榜单使用的单列紧凑样式 */
  compact?: boolean
}>()
const emit = defineEmits<{
  retry: []
  play: [songs: TrackInfo[], index: number]
  playlist: [playlist: HomePlaylist]
  songMenu: [event: MouseEvent, songs: TrackInfo[], index: number]
  playlistMenu: [event: MouseEvent, playlist: HomePlaylist]
}>()
const { t, locale } = useI18n()
const playCountFormat = computed(() => new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }))
const page = ref(0)
const columns = ref(3)
// 紧凑模式（榜单并排）固定单列 5 首，翻页浏览
const perPage = computed(() => props.compact ? 5 : columns.value * 3)
const pageCount = computed(() => Math.max(1, Math.ceil(props.section.songs.length / perPage.value)))
const pageSongs = computed(() => props.section.songs.slice(page.value * perPage.value, (page.value + 1) * perPage.value))
const hasItems = computed(() => props.section.songs.length > 0 || props.section.playlists.length > 0)

const sectionRef = ref<HTMLElement | null>(null)
const playlistRowRef = ref<HTMLElement | null>(null)
const canScrollPrev = ref(false)
const canScrollNext = ref(false)
let resizeObserver: ResizeObserver | null = null

// 歌单分区改为横向货架：宽度不足时滚动查看，而不是减列丢卡片
function updatePlaylistEdges() {
  const row = playlistRowRef.value
  canScrollPrev.value = !!row && row.scrollLeft > 4
  canScrollNext.value = !!row && row.scrollLeft + row.clientWidth < row.scrollWidth - 4
}

function scrollPlaylists(direction: 1 | -1) {
  const row = playlistRowRef.value
  row?.scrollBy({ left: direction * row.clientWidth * 0.85, behavior: 'smooth' })
}

// 按分区自身宽度分列：侧栏展开/折叠会改变可用宽度，窗口宽度并不准确
function updateColumns(width: number) {
  columns.value = props.compact ? 1 : width >= 760 ? 3 : width >= 500 ? 2 : 1
}

watch([() => props.section.songs, perPage], () => { page.value = 0 })
watch(() => props.section.playlists, () => void nextTick(updatePlaylistEdges))
onMounted(() => {
  if (!sectionRef.value) return
  updateColumns(sectionRef.value.clientWidth)
  resizeObserver = new ResizeObserver(entries => {
    updateColumns(entries[0]?.contentRect.width ?? 0)
    updatePlaylistEdges()
  })
  resizeObserver.observe(sectionRef.value)
})
onUnmounted(() => resizeObserver?.disconnect())
</script>

<template>
  <section ref="sectionRef" class="home-feed-section" :data-home-source="definition.key">
    <div class="feed-header">
      <h2 class="feed-title"><span class="material-symbols-rounded">{{ definition.icon }}</span>{{ definition.titleKey ? t(definition.titleKey) : title }}</h2>
      <div v-if="definition.kind === 'songs' && pageCount > 1" class="feed-pages">
        <button :disabled="page === 0" :aria-label="t('home.previous_page')" @click="page--"><span class="material-symbols-rounded">chevron_left</span></button>
        <button :disabled="page >= pageCount - 1" :aria-label="t('home.next_page')" @click="page++"><span class="material-symbols-rounded">chevron_right</span></button>
      </div>
      <div v-else-if="definition.kind !== 'songs' && (canScrollPrev || canScrollNext)" class="feed-pages">
        <button :disabled="!canScrollPrev" :aria-label="t('home.previous_page')" @click="scrollPlaylists(-1)"><span class="material-symbols-rounded">chevron_left</span></button>
        <button :disabled="!canScrollNext" :aria-label="t('home.next_page')" @click="scrollPlaylists(1)"><span class="material-symbols-rounded">chevron_right</span></button>
      </div>
    </div>
    <div v-if="section.error" class="feed-state feed-error" role="status">
      <span>{{ t('home.recommend_load_failed') }}</span>
      <button @click="emit('retry')">{{ t('player.retry') }}</button>
    </div>
    <div v-if="hasItems && definition.kind === 'songs'" class="feed-songs" :class="{ compact }" :style="{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }">
      <button v-for="(song, index) in pageSongs" :key="song.playlistKey ?? song.id" type="button" class="feed-song" @click="emit('play', section.songs, page * perPage + index)" @contextmenu.prevent="emit('songMenu', $event, section.songs, page * perPage + index)">
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
    <div v-else-if="hasItems" ref="playlistRowRef" class="feed-playlists" @scroll.passive="updatePlaylistEdges">
      <button v-for="(playlist, index) in section.playlists" :key="playlist.id + ':' + index" type="button" class="feed-playlist" @click="emit('playlist', playlist)" @contextmenu.prevent="emit('playlistMenu', $event, playlist)">
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
.home-feed-section { margin-bottom: 36px; container-type: inline-size; }
.feed-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 16px; }
.feed-title { display: flex; align-items: center; gap: 8px; color: var(--md-on-surface); font-size: calc(19px * var(--text-scale)); font-weight: 700; letter-spacing: var(--tracking-title); }
/* Clean：分区标题只靠字重区分层级，图标弱化为辅助识别 */
.feed-title > .material-symbols-rounded { font-size: 20px; color: var(--md-on-surface-variant); }
.feed-pages { display: flex; gap: 4px; }
.feed-pages button { display: grid; place-items: center; width: 30px; height: 30px; border-radius: var(--radius-full); color: var(--md-on-surface-variant); background: var(--md-surface-container-low); box-shadow: var(--shadow-1); transition: background var(--duration-short), color var(--duration-short); }
.feed-pages button .material-symbols-rounded { font-size: 20px; }
.feed-pages button:disabled { opacity: 0.3; cursor: default; }
.feed-pages button:hover:not(:disabled) { background: var(--md-surface-container-high); color: var(--md-on-surface); }
.feed-songs { display: grid; grid-template-rows: repeat(3, minmax(68px, auto)); grid-auto-flow: column; gap: 6px 16px; }
.feed-songs.compact { grid-template-rows: none; grid-auto-flow: row; gap: 2px; padding: 6px; border-radius: var(--radius-lg); background: var(--md-surface-container-low); box-shadow: 0 0 0 1px var(--hairline); }
.feed-songs.compact .feed-song { padding: 6px 8px; }
.feed-songs.compact .song-cover { width: 44px; height: 44px; }
.feed-song { display: flex; align-items: center; gap: 12px; min-width: 0; padding: 8px; text-align: left; border-radius: var(--radius-md); transition: background var(--duration-short); }
.feed-song:hover { background: var(--state-hover); }
.feed-song:active { background: var(--state-pressed); }
.song-number { width: 20px; flex-shrink: 0; color: var(--md-on-surface-variant); font-size: var(--fs-13); font-variant-numeric: tabular-nums; text-align: center; }
.song-cover { position: relative; display: grid; place-items: center; width: 48px; height: 48px; flex-shrink: 0; overflow: hidden; border-radius: var(--radius-sm); background: var(--md-surface-variant); box-shadow: inset 0 0 0 1px var(--hairline); }
.song-cover > .material-symbols-rounded { opacity: 0.4; }
.song-cover img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
.song-info { display: flex; flex: 1; min-width: 0; flex-direction: column; gap: 3px; }
.song-title, .song-meta, .playlist-title, .playlist-meta { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.song-title { font-size: var(--fs-14); font-weight: 550; }
.song-meta, .playlist-meta { font-size: var(--fs-12); color: var(--md-on-surface-variant); }
.feed-playlists { display: grid; grid-auto-flow: column; grid-auto-columns: clamp(148px, 15cqi, 196px); gap: 18px; overflow-x: auto; overscroll-behavior-x: contain; scroll-snap-type: x proximity; scrollbar-width: none; padding: 4px 2px 10px; margin: -4px -2px -10px; }
.feed-playlists::-webkit-scrollbar { display: none; }
.feed-playlist { scroll-snap-align: start; }
.feed-playlist { min-width: 0; align-self: start; text-align: left; border-radius: var(--radius-md); }
.playlist-image { position: relative; display: grid; place-items: center; aspect-ratio: 1; overflow: hidden; border-radius: var(--radius-md); background: var(--md-surface-variant); box-shadow: var(--shadow-1), inset 0 0 0 1px var(--hairline); transition: transform 220ms var(--ease-emphasized), box-shadow 220ms var(--ease-emphasized); }
.feed-playlist:hover .playlist-image { transform: translateY(-3px); box-shadow: var(--shadow-2); }
.playlist-image > .material-symbols-rounded { font-size: 32px; opacity: 0.4; }
.playlist-image img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
.playlist-plays { position: absolute; top: 8px; right: 8px; padding: 2px 8px; border-radius: var(--radius-full); background: rgba(0, 0, 0, 0.38); -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px); color: white; font-size: var(--fs-11); font-weight: 600; font-variant-numeric: tabular-nums; }
.playlist-title { font-size: var(--fs-14); font-weight: 600; margin-top: 10px; }
.playlist-meta { margin-top: 3px; }
.feed-state { display: flex; align-items: center; justify-content: center; gap: 10px; min-height: 72px; font-size: var(--fs-13); color: var(--md-on-surface-variant); }
.feed-error { color: var(--md-error); }
.feed-error button { color: var(--md-on-surface); padding: 5px 12px; border-radius: var(--radius-full); background: var(--md-surface-container-high); font-weight: 600; }
.spinning { animation: spin 1s linear infinite; }
button:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (max-width: 700px) { .feed-title { font-size: calc(18px * var(--text-scale)); } }
@media (prefers-reduced-motion: reduce) { .spinning { animation: none; } }
</style>
