<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import { useI18n } from 'vue-i18n'
import { useLibraryStore } from '@/stores/library'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { useTrackSelection } from '@/composables/useTrackSelection'
import { useTrackActivation } from '@/composables/useTrackActivation'
import { duplicateScanTrackIds, existingScanTrackIds, filterScanTracks } from '@/modules/library/localScanPreview'
import { sortLocalTracks } from '@/modules/library/localTrackSort'
import AddToPlaylistDialog from '@/components/AddToPlaylistDialog.vue'
import TrackSelectionToolbar from '@/components/TrackSelectionToolbar.vue'
import TrackContextMenu from '@/components/TrackContextMenu.vue'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import { invoke } from '@tauri-apps/api/core'
import { createContextMenuItem, createContextMenuSeparator, type ContextMenuActionItem } from '@/utils/contextMenu'

const props = withDefaults(defineProps<{ embedded?: boolean; searchQuery?: string }>(), { embedded: false, searchQuery: '' })
const { t } = useI18n()
const library = useLibraryStore()
const player = usePlayerStore()
const settings = useSettingsStore()
const { onTrackClick, onTrackDoubleClick } = useTrackActivation()
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
const trackMenuRef = ref<InstanceType<typeof TrackContextMenu> | null>(null)
const showEmptyState = computed(() => !tracks.value.length && library.isLoaded && !library.isScanning && !library.scanError)

function trackMenuExtras() {
  return [createContextMenuItem(t('library.local_tags_edit'), { id: 'edit-tags', icon: 'edit', disabled: library.isSavingTags })]
}

function handleTrackMenuAction(id: string, track: TrackInfo) {
  if (id === 'edit-tags') openTagEditor(track)
}

const folderMenu = ref<{ open: boolean; x: number; y: number; path: string; available: boolean }>({
  open: false, x: 0, y: 0, path: '', available: false,
})
const folderMenuItems = computed(() => [
  createContextMenuItem(t('library.show_in_folder'), { id: 'reveal', icon: 'folder_open', disabled: !folderMenu.value.available }),
  createContextMenuItem(t('library.local_scan_rescan'), { id: 'rescan', icon: 'refresh', disabled: library.isScanning }),
  createContextMenuSeparator('folder-danger'),
  createContextMenuItem(t('library.local_remove_folder'), { id: 'remove', icon: 'folder_delete', danger: true }),
])

function openFolderMenu(event: MouseEvent, folder: { path: string; available: boolean }) {
  event.preventDefault()
  folderMenu.value = { open: true, x: event.clientX, y: event.clientY, path: folder.path, available: folder.available }
}

async function handleFolderMenuClick(item: ContextMenuActionItem) {
  const path = folderMenu.value.path
  if (item.id === 'remove') return removeFolder(path)
  if (item.id === 'rescan') return library.rescan()
  if (item.id === 'reveal') {
    try {
      await invoke('reveal_in_file_manager', { path })
    } catch (error) {
      toast.error(String(error))
    }
  }
}

const existingIds = computed(() => existingScanTrackIds(tracks.value, library.playlistTracks))
const duplicateIds = computed(() => duplicateScanTrackIds(tracks.value))
const sortOptions = computed(() => (['default', 'modified', 'title', 'artist', 'album', 'duration'] as const)
  .map(value => ({ value, label: t(`library.local_sort_${value}`) })))
const sortedTracks = computed(() => sortLocalTracks(tracks.value, settings.localLibrarySortBy, settings.localLibrarySortDescending))
const filteredTracks = computed(() => filterScanTracks(sortedTracks.value, {
  query: query.value, metadataOnly: metadataOnly.value,
  hideExisting: hideExisting.value && !library.playlistIndexError,
  hideDuplicates: hideDuplicates.value,
  existingIds: existingIds.value, duplicateIds: duplicateIds.value,
}))
const displayedTracks = computed(() => filteredTracks.value.slice(0, visibleLimit.value))
const {
  selectedIds, selectedItems, visibleSelectedCount, allVisibleSelected,
  toggleSelected, toggleSelectAllVisible, invertSelectionVisible, leaveSelectionMode, pruneSelection,
} = useTrackSelection(sortedTracks, filteredTracks)

function changeSortBy(value: string) {
  const option = sortOptions.value.find(option => option.value === value)
  if (option) settings.localLibrarySortBy = option.value
}

function activateTrack(track: TrackInfo) {
  if (selectedIds.value.size) toggleSelected(track.id)
  else playTrack(track)
}

// 标签保存和索引刷新保留已展开的曲目数，筛选或排序变化时重新分页
watch([query, metadataOnly, hideExisting, hideDuplicates, () => settings.localLibrarySortBy, () => settings.localLibrarySortDescending], () => {
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

async function addFolders() {
  try {
    const picked = await dialogOpen({ directory: true, multiple: true, defaultPath: library.folders[0]?.path })
    const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : []
    for (const path of paths) await library.addFolder(path)
  } catch (error) {
    toast.error(`${t('library.scan_failed')}: ${String(error)}`)
  }
}

async function removeFolder(path: string) {
  try {
    await library.removeFolder(path)
    toast.show(t('library.local_folder_removed'), 'info')
  } catch (error) {
    toast.error(String(error))
  }
}

function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path
}

function playTrack(track: TrackInfo) {
  player.playAll(filteredTracks.value, track.id)
}

function playAllFiltered() {
  if (filteredTracks.value.length) player.playAll(filteredTracks.value)
}

function shuffleAllFiltered() {
  if (!filteredTracks.value.length) return
  const start = filteredTracks.value[Math.floor(Math.random() * filteredTracks.value.length)]
  if (!player.shuffleEnabled) player.toggleShuffle()
  player.playAll(filteredTracks.value, start.id)
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

onMounted(() => {
  void library.ensureStarted()
  void library.refreshPlaylistIndex()
})
</script>

<template>
  <section class="local-files-view" :class="{ embedded }">
    <div class="scan-heading">
      <div>
        <h2>{{ t('library.local_library_title') }}</h2>
        <p>{{ t('library.local_library_hint') }}</p>
      </div>
      <div class="scan-actions">
        <template v-if="tracks.length">
          <button class="scan-button" @click="playAllFiltered">
            <span class="material-symbols-rounded filled">play_arrow</span>{{ t('player.play_all') }}
          </button>
          <button class="scan-icon-button outlined" :title="t('player.shuffle_play')" :aria-label="t('player.shuffle_play')" @click="shuffleAllFiltered">
            <span class="material-symbols-rounded">shuffle</span>
          </button>
        </template>
        <button v-if="library.hasFolders && !library.isScanning" class="scan-icon-button outlined" :title="t('library.local_scan_rescan')" :aria-label="t('library.local_scan_rescan')" @click="library.rescan()">
          <span class="material-symbols-rounded">refresh</span>
        </button>
        <button v-if="!showEmptyState || library.hasFolders" class="scan-button secondary" @click="addFolders">
          <span class="material-symbols-rounded">create_new_folder</span>{{ t('library.local_add_folder') }}
        </button>
      </div>
    </div>

    <TransitionGroup v-if="library.hasFolders" tag="div" name="folder-chip" class="folder-chips">
      <div
        v-for="folder in library.folders"
        :key="folder.path"
        class="folder-chip"
        :class="{ unavailable: !folder.available }"
        :title="folder.path"
        @contextmenu="openFolderMenu($event, folder)"
      >
        <span class="material-symbols-rounded">{{ folder.available ? 'folder' : 'folder_off' }}</span>
        <span class="folder-chip-name">{{ folderName(folder.path) }}</span>
        <span class="folder-chip-count">{{ folder.available ? folder.trackCount : t('library.local_folder_unavailable') }}</span>
        <button class="folder-chip-remove" :title="t('library.local_remove_folder')" :aria-label="t('library.local_remove_folder')" @click="removeFolder(folder.path)">
          <span class="material-symbols-rounded">close</span>
        </button>
      </div>
    </TransitionGroup>

    <Transition name="scan-banner">
      <div v-if="library.isScanning" class="scan-status" role="status" aria-live="polite">
        <span class="material-symbols-rounded scanning-icon">progress_activity</span>
        <div>
          <strong>{{ t('library.scanning') }}</strong>
          <p v-if="library.scanProgress">{{ t('library.local_scan_progress', { visited: library.scanProgress.visitedEntries, count: library.scanProgress.tracks, skipped: library.scanProgress.skipped }) }}</p>
          <p v-if="library.scanProgress" class="scan-current-path">{{ library.scanProgress.currentPath }}</p>
        </div>
        <button class="scan-button secondary scan-cancel" @click="library.cancelScan()">{{ t('common.cancel') }}</button>
      </div>
    </Transition>
    <p v-if="library.scanError" class="scan-error" role="alert">{{ t('library.scan_failed') }}: {{ library.scanError }}</p>

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
      <div class="scan-sort">
        <CustomSelect :model-value="settings.localLibrarySortBy" :options="sortOptions" :label="t('library.local_sort')" @update:model-value="changeSortBy" />
        <button
          class="scan-icon-button outlined"
          :title="t(settings.localLibrarySortDescending ? 'download.sort_ascending' : 'download.sort_descending')"
          :aria-label="t(settings.localLibrarySortDescending ? 'download.sort_ascending' : 'download.sort_descending')"
          @click="settings.localLibrarySortDescending = !settings.localLibrarySortDescending"
        ><span class="material-symbols-rounded">{{ settings.localLibrarySortDescending ? 'south' : 'north' }}</span></button>
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
        <div
          v-for="track in displayedTracks"
          :key="track.id"
          class="scan-track"
          :class="{ selected: selectedIds.has(track.id), playing: player.currentTrack?.audioUrl === track.audioUrl }"
          @contextmenu="trackMenuRef?.open($event, track)"
        >
          <label class="scan-track-select">
            <input type="checkbox" :checked="selectedIds.has(track.id)" :aria-label="track.title" @change="toggleSelected(track.id)" />
          </label>
          <div class="scan-cover">
            <img v-if="track.coverUrl && !failedCovers.has(track.id)" :src="track.coverUrl" loading="lazy" alt="" @error="failedCovers.add(track.id)" />
            <span v-else class="material-symbols-rounded">music_note</span>
          </div>
          <button
            class="scan-track-info"
            :class="{ playing: player.currentTrack?.audioUrl === track.audioUrl }"
            @click="onTrackClick($event, () => activateTrack(track), selectedIds.size > 0)"
            @dblclick="onTrackDoubleClick($event, () => playTrack(track), selectedIds.size > 0)"
          >
            <strong>{{ track.title }}</strong>
            <span>{{ track.artist }}<template v-if="track.album"> · {{ track.album }}</template></span>
            <small :title="track.audioUrl">{{ track.audioUrl }}</small>
          </button>
          <span v-if="existingIds.has(track.id)" class="scan-badge">{{ t('library.local_scan_existing') }}</span>
          <span v-if="duplicateIds.has(track.id)" class="scan-badge">{{ t('library.local_scan_duplicate') }}</span>
          <span class="scan-duration">{{ formatDuration(track.durationMs) }}</span>
          <button class="scan-icon-button" :disabled="library.isSavingTags" :title="t('library.local_tags_edit')" @click="openTagEditor(track)"><span class="material-symbols-rounded">edit</span></button>
          <button class="scan-icon-button" :title="t('common.play_selected')" @click="playTrack(track)"><span class="material-symbols-rounded">play_arrow</span></button>
        </div>
      </div>
      <p v-if="!filteredTracks.length" class="scan-empty">{{ t('library.local_scan_no_matches') }}</p>
      <button v-if="filteredTracks.length > visibleLimit" class="scan-button secondary scan-show-more" @click="visibleLimit += 100">
        {{ t('library.local_scan_show_more', { count: filteredTracks.length - visibleLimit }) }}
      </button>
    </template>
    <div v-else-if="showEmptyState" class="scan-empty scan-empty--state">
      <span class="material-symbols-rounded">{{ library.hasFolders ? 'music_off' : 'library_music' }}</span>
      <p>{{ t(library.hasFolders ? 'library.local_library_no_audio' : 'library.local_library_empty') }}</p>
      <button v-if="!library.hasFolders" class="scan-button" @click="addFolders">
        <span class="material-symbols-rounded">create_new_folder</span>{{ t('library.local_add_folder') }}
      </button>
    </div>

    <details v-if="library.scanSkipped.length" class="scan-skipped">
      <summary>{{ t('library.local_scan_skipped', { count: library.scanSkipped.length }) }}</summary>
      <div v-for="(item, index) in library.scanSkipped.slice(0, 100)" :key="index"><strong>{{ item.path }}</strong><p>{{ item.reason }}</p></div>
    </details>
    <AddToPlaylistDialog v-model:open="showAddToPlaylist" :tracks="importTargets" />
    <ContextMenu
      v-model:open="folderMenu.open"
      :x="folderMenu.x"
      :y="folderMenu.y"
      :items="folderMenuItems"
      @click="handleFolderMenuClick"
    />
    <TrackContextMenu
      ref="trackMenuRef"
      :extra-items="trackMenuExtras"
      @play="playTrack"
      @action="handleTrackMenuAction"
    />
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
.scan-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.scan-button { display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 10px 18px; border-radius: 24px; color: var(--md-on-primary); background: var(--md-primary); font-size: 13px; font-weight: 600; cursor: pointer; &.secondary { background: var(--md-surface-container-high); color: var(--md-on-surface); } &:disabled { opacity: .4; cursor: default; } }
.scan-directory { color: var(--md-on-surface-variant); font-size: 12px; overflow-wrap: anywhere; margin: 12px 0 20px; }
.scan-icon-button.outlined { width: 40px; height: 40px; border: 1px solid var(--md-outline-variant); }
.folder-chips { display: flex; flex-wrap: wrap; gap: 8px; margin: 18px 0 4px; }
.folder-chip {
  display: inline-flex; align-items: center; gap: 8px; max-width: 100%; min-height: 36px; padding: 0 4px 0 12px;
  border-radius: var(--radius-sm); background: var(--md-surface-container); color: var(--md-on-surface); font-size: 13px;
  > .material-symbols-rounded { font-size: 18px; color: var(--md-primary); }
  &.unavailable { color: var(--md-on-surface-variant); > .material-symbols-rounded { color: var(--md-error); } }
}
.folder-chip-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.folder-chip-count { color: var(--md-on-surface-variant); font-size: 12px; font-variant-numeric: tabular-nums; }
.folder-chip-remove {
  width: 28px; height: 28px; display: grid; place-items: center; border-radius: 50%; color: var(--md-on-surface-variant);
  transition: background var(--duration-short) var(--ease-standard), color var(--duration-short) var(--ease-standard);
  .material-symbols-rounded { font-size: 16px; }
  &:hover { background: var(--md-surface-container-highest); color: var(--md-error); }
}
.folder-chip-enter-active, .folder-chip-leave-active { transition: opacity 180ms var(--ease-standard), transform 180ms var(--ease-standard); }
.folder-chip-enter-from, .folder-chip-leave-to { opacity: 0; transform: scale(0.94); }
.folder-chip-leave-active { position: absolute; }
.scan-status { display: flex; align-items: center; gap: 16px; background: var(--md-surface-container); border-radius: 16px; padding: 14px 16px; margin: 16px 0; min-width: 0; > div { min-width: 0; flex: 1; } p { color: var(--md-on-surface-variant); font-size: 13px; margin: 4px 0 0; } .scan-current-path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; } }
.scan-cancel { flex-shrink: 0; padding: 8px 14px; }
.scan-banner-enter-active, .scan-banner-leave-active { transition: opacity 200ms var(--ease-standard), transform 200ms var(--ease-standard); }
.scan-banner-enter-from, .scan-banner-leave-to { opacity: 0; transform: translateY(-4px); }
.scan-track-info.playing strong { color: var(--md-primary); }
.scanning-icon { color: var(--md-primary); animation: local-scan-spin 1s linear infinite; }
.scan-error { color: var(--md-error); overflow-wrap: anywhere; font-size: 13px; }
.scan-message, .scan-summary { color: var(--md-on-surface-variant); font-size: 13px; margin: 16px 0; }
.scan-search { display: flex; align-items: center; gap: 12px; background: var(--md-surface-container-high); border-radius: 28px; padding: 12px 18px; margin: 20px 0 16px; color: var(--md-on-surface-variant); input { width: 100%; border: 0; outline: none; background: none; font-size: 14px; color: var(--md-on-surface); } }
.scan-filters {
  display: flex; gap: 8px 16px; flex-wrap: wrap; margin-top: 8px;
  label {
    display: inline-flex; align-items: center; gap: 10px; min-height: 40px; padding: 8px 12px;
    border-radius: var(--radius-sm); font-size: 13px; cursor: pointer;
    transition: background var(--duration-short) var(--ease-standard);
    &:hover:not(:has(input:disabled)) { background: var(--md-surface-container); }
    &:has(input:disabled) { cursor: default; color: var(--md-outline); }
  }
}
.scan-sort { display: flex; align-items: center; gap: 8px; margin-top: 12px; }
.scan-track {
  display: flex; align-items: center; gap: 14px; padding: 10px 12px; border-radius: 14px;
  &:hover, &.selected { background: var(--md-surface-container); }
  &.playing {
    background: color-mix(in srgb, var(--md-primary) 10%, var(--md-surface));
    box-shadow: inset 0 0 0 1px var(--md-primary);
  }
}
.scan-track-select {
  display: grid; place-items: center; width: 40px; height: 40px; flex-shrink: 0;
  margin: 0 -4px 0 -8px; border-radius: var(--radius-full); cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard);
  &:hover { background: color-mix(in srgb, var(--md-primary) 10%, transparent); }
}
.scan-cover { width: 48px; height: 48px; flex-shrink: 0; border-radius: 10px; overflow: hidden; display: grid; place-items: center; background: var(--md-surface-container-high); color: var(--md-on-surface-variant); img { width: 100%; height: 100%; object-fit: cover; } }
.scan-track-info { text-align: left; flex: 1; min-width: 0; color: var(--md-on-surface); cursor: pointer; strong, span, small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } strong { font-size: 14px; } span { font-size: 12px; color: var(--md-on-surface-variant); margin-top: 4px; } small { font-size: 11px; color: var(--md-on-surface-variant); margin-top: 4px; opacity: .7; } }
.scan-badge { font-size: 11px; color: var(--md-on-surface-variant); background: var(--md-surface-container-high); border-radius: 8px; padding: 4px 7px; }
.scan-duration { font-size: 12px; color: var(--md-on-surface-variant); }
.scan-icon-button { width: 36px; height: 36px; border-radius: 50%; display: grid; place-items: center; color: var(--md-on-surface-variant); cursor: pointer; &:hover { color: var(--md-primary); background: var(--md-primary-container); } }
.scan-empty { text-align: center; padding: 64px 16px; color: var(--md-on-surface-variant); > .material-symbols-rounded { font-size: 54px; opacity: .5; } }
.scan-empty--state {
  display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 72px 16px 56px;
  p { margin: 0; font-size: 14px; }
  .scan-button { margin-top: 8px; }
}
.scan-show-more { display: flex; margin: 20px auto; }
.scan-skipped { margin: 24px 0; font-size: 12px; color: var(--md-on-surface-variant); summary { cursor: pointer; } > div { padding: 12px 0; overflow-wrap: anywhere; } p { margin: 5px 0 0; } }
@keyframes local-scan-spin { to { transform: rotate(360deg); } }
@media (max-width: 700px) { .scan-badge, .scan-duration { display: none; } .scan-track { gap: 8px; padding: 8px 4px; } .scan-track-select { margin: 0; } }
@media (prefers-reduced-motion: reduce) { .scanning-icon { animation: none; } .scan-filters label, .scan-track-select { transition: none; } }
</style>
