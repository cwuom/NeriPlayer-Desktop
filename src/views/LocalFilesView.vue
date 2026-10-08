<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import { useI18n } from 'vue-i18n'
import { useLibraryStore } from '@/stores/library'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { useTrackSelection } from '@/composables/useTrackSelection'
import { duplicateScanTrackIds, existingScanTrackIds, filterScanTracks } from '@/modules/library/localScanPreview'
import AddToPlaylistDialog from '@/components/AddToPlaylistDialog.vue'
import TrackSelectionToolbar from '@/components/TrackSelectionToolbar.vue'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'

const props = withDefaults(defineProps<{ embedded?: boolean; searchQuery?: string }>(), { embedded: false, searchQuery: '' })
const { t } = useI18n()
const library = useLibraryStore()
const player = usePlayerStore()
const toast = useToastStore()
const { tracks } = storeToRefs(library)
const localQuery = ref('')
const query = computed(() => props.embedded ? props.searchQuery : localQuery.value)
const metadataOnly = ref(false)
const hideExisting = ref(false)
const hideDuplicates = ref(false)
const visibleLimit = ref(100)
const showAddToPlaylist = ref(false)
const importTargets = ref<TrackInfo[]>([])
const failedCovers = ref(new Set<string>())
const showTagEditor = ref(false)
const editingTrack = ref<TrackInfo | null>(null)
const editTitle = ref('')
const editArtist = ref('')
const editAlbum = ref('')
const tagEditError = ref<string | null>(null)
let unlistenPlaylists: UnlistenFn | undefined
let disposed = false

const existingIds = computed(() => existingScanTrackIds(tracks.value, library.playlistTracks))
const duplicateIds = computed(() => duplicateScanTrackIds(tracks.value))
const filteredTracks = computed(() => filterScanTracks(tracks.value, {
  query: query.value, metadataOnly: metadataOnly.value,
  hideExisting: hideExisting.value && !library.playlistIndexError,
  hideDuplicates: hideDuplicates.value,
  existingIds: existingIds.value, duplicateIds: duplicateIds.value,
}))
const displayedTracks = computed(() => filteredTracks.value.slice(0, visibleLimit.value))
const {
  selectedIds, selectedItems, visibleSelectedCount, allVisibleSelected,
  toggleSelected, toggleSelectAllVisible, invertSelectionVisible, leaveSelectionMode, pruneSelection,
} = useTrackSelection(tracks, filteredTracks)

// 标签保存、歌单索引刷新也会替换结果；只有筛选条件变化才收起「显示更多」
watch([query, metadataOnly, hideExisting, hideDuplicates], () => {
  visibleLimit.value = 100
})
watch(filteredTracks, () => {
  // 隐藏后的文件不应继续被导入，否则筛选按钮的含义会变得不明确
  const visibleIds = new Set(filteredTracks.value.map(track => track.id))
  selectedIds.value = new Set([...selectedIds.value].filter(id => visibleIds.has(id)))
})
watch(tracks, () => {
  pruneSelection()
  failedCovers.value = new Set()
})

async function chooseDirectory() {
  if (library.isScanning) return
  try {
    const dir = await dialogOpen({ directory: true, multiple: false, defaultPath: library.lastScanDir || undefined })
    if (typeof dir === 'string' && dir) await library.scanDirectory(dir)
  } catch (error) {
    toast.error(`${t('library.scan_failed')}: ${String(error)}`)
  }
}

function importSelected() {
  if (!selectedItems.value.length) return
  importTargets.value = [...selectedItems.value]
  showAddToPlaylist.value = true
}

function playSelected() {
  if (selectedItems.value.length) player.playAll(selectedItems.value)
}

function queueSelected() {
  for (const track of selectedItems.value) player.addToQueueEnd(track)
}

function openTagEditor(track: TrackInfo) {
  editingTrack.value = track
  editTitle.value = track.title
  editArtist.value = track.artist
  editAlbum.value = track.album
  tagEditError.value = null
  showTagEditor.value = true
}

async function saveTags() {
  const track = editingTrack.value
  if (!track || library.isSavingTags) return
  tagEditError.value = null
  try {
    await library.saveTrackTags(track, { title: editTitle.value.trim(), artist: editArtist.value.trim(), album: editAlbum.value.trim() })
    showTagEditor.value = false
    toast.success(t('library.local_tags_saved'))
  } catch (error) {
    tagEditError.value = `${t('library.local_tags_failed')}: ${String(error)}`
  }
}

function formatDuration(milliseconds: number) {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

onMounted(async () => {
  void library.refreshPlaylistIndex()
  try {
    const stop = await listen('playlists-changed', () => { void library.refreshPlaylistIndex() })
    if (disposed) stop()
    else unlistenPlaylists = stop
  } catch { /* 浏览器预览没有 Tauri 事件桥 */ }
})
onBeforeUnmount(() => {
  disposed = true
  unlistenPlaylists?.()
})
</script>

<template>
  <section class="local-files-view" :class="{ embedded }">
    <div class="scan-heading">
      <div>
        <h2>{{ t('library.local_scan_title') }}</h2>
        <p>{{ t('library.local_scan_hint') }}</p>
      </div>
      <div class="scan-actions">
        <button v-if="library.lastScanDir && !library.isScanning" class="scan-button secondary" :disabled="library.isSavingTags" @click="library.scanDirectory(library.lastScanDir)">
          <span class="material-symbols-rounded">refresh</span>{{ t('library.local_scan_rescan') }}
        </button>
        <button v-if="library.isScanning" class="scan-button secondary" :disabled="library.isCancelling" @click="library.cancelScan()">
          <span class="material-symbols-rounded">stop</span>{{ t('common.cancel') }}
        </button>
        <button class="scan-button" :disabled="library.isScanning || library.isSavingTags" @click="chooseDirectory">
          <span class="material-symbols-rounded">folder_open</span>{{ t('library.local_scan_choose') }}
        </button>
      </div>
    </div>
    <p v-if="library.scanDir || library.lastScanDir" class="scan-directory">{{ library.scanDir || library.lastScanDir }}</p>

    <div v-if="library.isScanning && library.scanProgress" class="scan-status" role="status" aria-live="polite">
      <span class="material-symbols-rounded scanning-icon">progress_activity</span>
      <div>
        <strong>{{ library.isCancelling ? t('library.local_scan_cancelling') : t('library.scanning') }}</strong>
        <p>{{ t('library.local_scan_progress', { visited: library.scanProgress.visitedEntries, count: library.scanProgress.tracks, skipped: library.scanProgress.skipped }) }}</p>
        <p class="scan-current-path">{{ library.scanProgress.currentPath }}</p>
      </div>
    </div>
    <p v-if="library.scanError" class="scan-error" role="alert">{{ t('library.scan_failed') }}: {{ library.scanError }}</p>
    <p v-if="library.scanCancelled" class="scan-message" role="status">{{ t('library.local_scan_cancelled') }}</p>

    <template v-if="tracks.length">
      <label v-if="!embedded" class="scan-search">
        <span class="material-symbols-rounded">search</span>
        <input v-model="localQuery" :placeholder="t('library.local_scan_search')" :aria-label="t('library.local_scan_search')" />
      </label>
      <div class="scan-filters">
        <label><input v-model="metadataOnly" type="checkbox" />{{ t('library.local_scan_metadata_only') }}</label>
        <label><input v-model="hideExisting" type="checkbox" :disabled="!!library.playlistIndexError" />{{ t('library.local_scan_hide_existing') }}</label>
        <label><input v-model="hideDuplicates" type="checkbox" />{{ t('library.local_scan_hide_duplicates') }}</label>
      </div>
      <p v-if="library.playlistIndexError" class="scan-message">{{ t('library.local_scan_index_failed') }}</p>
      <p class="scan-summary">{{ t('library.local_scan_summary', { visible: filteredTracks.length, total: tracks.length, existing: existingIds.size, duplicates: duplicateIds.size }) }}</p>
      <TrackSelectionToolbar
        :selected-count="selectedItems.length" :visible-selected-count="visibleSelectedCount"
        :all-visible-selected="allVisibleSelected" :show-download="false"
        @select-all="toggleSelectAllVisible" @invert-selection="invertSelectionVisible" @exit="leaveSelectionMode"
        @play="playSelected" @queue="queueSelected" @playlist="importSelected"
      />
      <div class="scan-track-list">
        <div v-for="track in displayedTracks" :key="track.id" class="scan-track" :class="{ selected: selectedIds.has(track.id) }">
          <input type="checkbox" :checked="selectedIds.has(track.id)" :aria-label="track.title" @change="toggleSelected(track.id)" />
          <div class="scan-cover">
            <img v-if="track.coverUrl && !failedCovers.has(track.id)" :src="track.coverUrl" loading="lazy" alt="" @error="failedCovers.add(track.id)" />
            <span v-else class="material-symbols-rounded">music_note</span>
          </div>
          <button class="scan-track-info" @click="toggleSelected(track.id)">
            <strong>{{ track.title }}</strong>
            <span>{{ track.artist }}<template v-if="track.album"> · {{ track.album }}</template></span>
            <small :title="track.audioUrl">{{ track.audioUrl }}</small>
          </button>
          <span v-if="existingIds.has(track.id)" class="scan-badge">{{ t('library.local_scan_existing') }}</span>
          <span v-if="duplicateIds.has(track.id)" class="scan-badge">{{ t('library.local_scan_duplicate') }}</span>
          <span class="scan-duration">{{ formatDuration(track.durationMs) }}</span>
          <button class="scan-icon-button" :disabled="library.isSavingTags" :title="t('library.local_tags_edit')" @click="openTagEditor(track)"><span class="material-symbols-rounded">edit</span></button>
          <button class="scan-icon-button" :title="t('common.play_selected')" @click="player.playAll(filteredTracks, track.id)"><span class="material-symbols-rounded">play_arrow</span></button>
        </div>
      </div>
      <p v-if="!filteredTracks.length" class="scan-empty">{{ t('library.local_scan_no_matches') }}</p>
      <button v-if="filteredTracks.length > visibleLimit" class="scan-button secondary scan-show-more" @click="visibleLimit += 100">
        {{ t('library.local_scan_show_more', { count: filteredTracks.length - visibleLimit }) }}
      </button>
    </template>
    <div v-else-if="!library.isScanning && !library.scanError && !library.scanCancelled" class="scan-empty">
      <span class="material-symbols-rounded">folder_open</span>
      <p>{{ t(library.scanDir ? 'library.local_scan_no_matches' : 'library.local_scan_empty') }}</p>
    </div>

    <details v-if="library.scanSkipped.length" class="scan-skipped">
      <summary>{{ t('library.local_scan_skipped', { count: library.scanSkipped.length }) }}</summary>
      <div v-for="(item, index) in library.scanSkipped.slice(0, 100)" :key="index"><strong>{{ item.path }}</strong><p>{{ item.reason }}</p></div>
    </details>
    <AddToPlaylistDialog v-model:open="showAddToPlaylist" :tracks="importTargets" />
    <M3Dialog v-model:open="showTagEditor" :title="t('library.local_tags_edit')" icon="edit" :confirm-text="t('library.local_tags_save')" :confirm-disabled="library.isSavingTags || !editTitle.trim() || !editArtist.trim() || !editAlbum.trim()" @confirm="saveTags">
      <p>{{ t('library.local_tags_hint') }}</p>
      <p class="scan-directory">{{ editingTrack?.audioUrl }}</p>
      <M3Input v-model="editTitle" :label="t('library.local_tags_title')" :maxlength="1000" />
      <M3Input v-model="editArtist" :label="t('library.local_tags_artist')" :maxlength="1000" />
      <M3Input v-model="editAlbum" :label="t('library.local_tags_album')" :maxlength="1000" />
      <p v-if="tagEditError" class="scan-error" role="alert">{{ tagEditError }}</p>
    </M3Dialog>
  </section>
</template>

<style scoped lang="scss">
.local-files-view { padding: 24px 32px 100px; &.embedded { padding: 8px 0 32px; } }
.scan-heading { display: flex; align-items: center; justify-content: space-between; gap: 20px; flex-wrap: wrap; h2 { font-size: 22px; margin: 0 0 8px; } p { margin: 0; color: var(--md-on-surface-variant); font-size: 13px; } }
.scan-actions { display: flex; gap: 8px; flex-wrap: wrap; }
.scan-button { display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 10px 18px; border-radius: 24px; color: var(--md-on-primary); background: var(--md-primary); font-size: 13px; font-weight: 600; cursor: pointer; &.secondary { background: var(--md-surface-container-high); color: var(--md-on-surface); } &:disabled { opacity: .4; cursor: default; } }
.scan-directory { color: var(--md-on-surface-variant); font-size: 12px; overflow-wrap: anywhere; margin: 12px 0 20px; }
.scan-status { display: flex; align-items: center; gap: 16px; background: var(--md-surface-container); border-radius: 16px; padding: 20px; margin: 20px 0; min-width: 0; > div { min-width: 0; } p { color: var(--md-on-surface-variant); font-size: 13px; margin: 6px 0 0; } .scan-current-path { overflow-wrap: anywhere; font-size: 12px; } }
.scanning-icon { color: var(--md-primary); animation: local-scan-spin 1s linear infinite; }
.scan-error { color: var(--md-error); overflow-wrap: anywhere; font-size: 13px; }
.scan-message, .scan-summary { color: var(--md-on-surface-variant); font-size: 13px; margin: 16px 0; }
.scan-search { display: flex; align-items: center; gap: 12px; background: var(--md-surface-container-high); border-radius: 28px; padding: 12px 18px; margin: 20px 0 16px; color: var(--md-on-surface-variant); input { width: 100%; border: 0; outline: none; background: none; font-size: 14px; color: var(--md-on-surface); } }
.scan-filters { display: flex; gap: 12px 24px; flex-wrap: wrap; label { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; cursor: pointer; } }
input[type='checkbox'] { width: 18px; height: 18px; accent-color: var(--md-primary); flex-shrink: 0; cursor: pointer; }
.scan-track { display: flex; align-items: center; gap: 14px; padding: 10px 12px; border-radius: 14px; &:hover, &.selected { background: var(--md-surface-container); } }
.scan-cover { width: 48px; height: 48px; flex-shrink: 0; border-radius: 10px; overflow: hidden; display: grid; place-items: center; background: var(--md-surface-container-high); color: var(--md-on-surface-variant); img { width: 100%; height: 100%; object-fit: cover; } }
.scan-track-info { text-align: left; flex: 1; min-width: 0; color: var(--md-on-surface); cursor: pointer; strong, span, small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } strong { font-size: 14px; } span { font-size: 12px; color: var(--md-on-surface-variant); margin-top: 4px; } small { font-size: 11px; color: var(--md-on-surface-variant); margin-top: 4px; opacity: .7; } }
.scan-badge { font-size: 11px; color: var(--md-on-surface-variant); background: var(--md-surface-container-high); border-radius: 8px; padding: 4px 7px; }
.scan-duration { font-size: 12px; color: var(--md-on-surface-variant); }
.scan-icon-button { width: 36px; height: 36px; border-radius: 50%; display: grid; place-items: center; color: var(--md-on-surface-variant); cursor: pointer; &:hover { color: var(--md-primary); background: var(--md-primary-container); } }
.scan-empty { text-align: center; padding: 64px 16px; color: var(--md-on-surface-variant); > .material-symbols-rounded { font-size: 54px; opacity: .5; } }
.scan-show-more { display: flex; margin: 20px auto; }
.scan-skipped { margin: 24px 0; font-size: 12px; color: var(--md-on-surface-variant); summary { cursor: pointer; } > div { padding: 12px 0; overflow-wrap: anywhere; } p { margin: 5px 0 0; } }
@keyframes local-scan-spin { to { transform: rotate(360deg); } }
@media (max-width: 700px) { .scan-badge, .scan-duration { display: none; } .scan-track { gap: 8px; padding: 8px 4px; } }
@media (prefers-reduced-motion: reduce) { .scanning-icon { animation: none; } }
</style>
