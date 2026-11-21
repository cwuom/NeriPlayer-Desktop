import { computed, nextTick, ref, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useToastStore } from '@/stores/toast'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import { createLogger } from '@/utils/logger'
import { deletablePlaylistIds, isProtectedPlaylist, type LocalPlaylistSummary } from './libraryPlaylists'
import type { LocalPlaylistsState } from './useLocalPlaylists'

const log = createLogger('library-view')

export interface PlaylistActionsIpc {
  createPlaylist: (name: string) => Promise<unknown>
  renamePlaylist: (id: number, name: string) => Promise<unknown>
  deletePlaylist: (id: number) => Promise<unknown>
}

interface FocusableInput {
  focus: () => void
}

export interface PlaylistActionsOptions {
  ipc: PlaylistActionsIpc
  local: LocalPlaylistsState
  /** 对话框打开后要聚焦的输入框（模板 ref） */
  createInputRef: Ref<FocusableInput | undefined>
  renameInputRef: Ref<FocusableInput | undefined>
}

/** 本地歌单的新建 / 重命名 / 删除 / 批量删除对话框与行菜单 */
export function usePlaylistActions(options: PlaylistActionsOptions) {
  const { ipc, local, createInputRef, renameInputRef } = options
  const {
    playlists,
    filteredPlaylists,
    isMultiSelectMode,
    selectedPlaylists,
    enterMultiSelect,
    exitMultiSelect,
    togglePlaylistSelection,
    loadPlaylists,
  } = local
  const { t } = useI18n()
  const toast = useToastStore()

  // M3 Dialog 创建播放列表
  const showCreateDialog = ref(false)
  const newPlaylistName = ref('')

  function openCreateDialog() {
    newPlaylistName.value = ''
    showCreateDialog.value = true
    nextTick(() => createInputRef.value?.focus())
  }

  async function confirmCreate() {
    if (!newPlaylistName.value.trim()) return
    try {
      await ipc.createPlaylist(newPlaylistName.value.trim())
      showCreateDialog.value = false
      await loadPlaylists()
    } catch (e) {
      log.error('Create playlist failed:', e)
      toast.error(t('library.create_playlist_failed'))
    }
  }

  // 上下文菜单
  const contextMenu = ref<{ show: boolean; x: number; y: number; playlist: LocalPlaylistSummary | null }>({
    show: false, x: 0, y: 0, playlist: null,
  })

  function openContextMenu(e: MouseEvent, pl: LocalPlaylistSummary) {
    if (isProtectedPlaylist(pl)) return
    const btn = e.currentTarget as HTMLElement
    const rect = btn.getBoundingClientRect()
    let x = rect.left - 204
    if (x < 8) x = rect.right + 4
    contextMenu.value = { show: true, x, y: rect.top, playlist: pl }
  }

  // 行上右键：菜单在光标处弹出（复用 ContextMenu 的边缘翻转与重定位）
  function openPlaylistContextMenu(e: MouseEvent, pl: LocalPlaylistSummary) {
    if (isMultiSelectMode.value) {
      togglePlaylistSelection(pl.id)
      return
    }
    if (isProtectedPlaylist(pl)) return
    contextMenu.value = { show: true, x: e.clientX, y: e.clientY, playlist: pl }
  }

  function closeContextMenu() {
    contextMenu.value.show = false
  }

  // 删除确认
  const showDeleteDialog = ref(false)
  const deleteTarget = ref<LocalPlaylistSummary | null>(null)

  function requestDelete(pl: LocalPlaylistSummary) {
    closeContextMenu()
    deleteTarget.value = pl
    showDeleteDialog.value = true
  }

  async function confirmDelete() {
    if (!deleteTarget.value) return
    try {
      await ipc.deletePlaylist(deleteTarget.value.id)
      showDeleteDialog.value = false
      deleteTarget.value = null
      await loadPlaylists()
    } catch (e) {
      log.error('Delete playlist failed:', e)
      toast.error(t('library.delete_playlist_failed'))
    }
  }

  // 重命名
  const showRenameDialog = ref(false)
  const renameTarget = ref<LocalPlaylistSummary | null>(null)
  const renameValue = ref('')

  function requestRename(pl: LocalPlaylistSummary) {
    closeContextMenu()
    renameTarget.value = pl
    renameValue.value = pl.name
    showRenameDialog.value = true
    nextTick(() => renameInputRef.value?.focus())
  }

  const playlistMenuItems = computed<ContextMenuItem[]>(() => {
    const playlist = contextMenu.value.playlist
    return [
      createContextMenuItem(t('common.multi_select'), { id: 'select', icon: 'checklist' }),
      createContextMenuItem(t('library.rename_playlist'), { id: 'rename', icon: 'edit' }),
      createContextMenuItem(t('library.delete_playlist'), {
        id: 'delete',
        icon: 'delete',
        danger: true,
        disabled: !playlist || isProtectedPlaylist(playlist),
      }),
    ]
  })

  function handlePlaylistMenuClick(item: ContextMenuActionItem) {
    const playlist = contextMenu.value.playlist
    if (!playlist) return

    switch (item.id) {
      case 'select':
        closeContextMenu()
        enterMultiSelect(playlist)
        break
      case 'rename':
        requestRename(playlist)
        break
      case 'delete':
        requestDelete(playlist)
        break
    }
  }

  async function confirmRename() {
    if (!renameTarget.value || !renameValue.value.trim()) return
    try {
      await ipc.renamePlaylist(renameTarget.value.id, renameValue.value.trim())
      showRenameDialog.value = false
      renameTarget.value = null
      await loadPlaylists()
    } catch (e) {
      log.error('Rename playlist failed:', e)
      toast.error(t('library.rename_playlist_failed'))
    }
  }

  // 批量删除确认
  const showBatchDeleteDialog = ref(false)

  function requestDeleteSelected() {
    if (selectedPlaylists.value.size === 0) return
    showBatchDeleteDialog.value = true
  }

  async function confirmDeleteSelected() {
    const ids = deletablePlaylistIds(selectedPlaylists.value, filteredPlaylists.value, playlists.value)
    try {
      for (const id of ids) {
        await ipc.deletePlaylist(id)
      }
      showBatchDeleteDialog.value = false
      exitMultiSelect()
      await loadPlaylists()
    } catch (e) {
      log.error('Batch delete playlists failed:', e)
      toast.error(t('library.delete_playlist_failed'))
      await loadPlaylists()
    }
  }

  return {
    showCreateDialog,
    newPlaylistName,
    openCreateDialog,
    confirmCreate,
    contextMenu,
    openContextMenu,
    openPlaylistContextMenu,
    showDeleteDialog,
    deleteTarget,
    confirmDelete,
    showRenameDialog,
    renameTarget,
    renameValue,
    playlistMenuItems,
    handlePlaylistMenuClick,
    confirmRename,
    showBatchDeleteDialog,
    requestDeleteSelected,
    confirmDeleteSelected,
  }
}
