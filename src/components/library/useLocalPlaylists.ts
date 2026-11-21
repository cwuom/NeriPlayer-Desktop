import { computed, onDeactivated, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePointerListReorder } from '@/composables/usePointerListReorder'
import { searchMatches } from '@/modules/search/textMatcher'
import {
  LEGACY_PLAYLIST_ORDER_KEY,
  localPlaylistDisplayName,
  playlistOrderIds,
  readLegacyPlaylistOrder,
  visibleSelection,
} from '@/modules/library/localPlaylists'
import { useToastStore } from '@/stores/toast'
import { createLogger } from '@/utils/logger'
import {
  isProtectedPlaylist,
  movePlaylist,
  orderLibraryPlaylists,
  playlistDragKey,
  type LocalPlaylistSummary,
} from './libraryPlaylists'

const log = createLogger('library-view')

/** UI 层直接 invoke 受 ui_invoke_baseline 约束，后端调用由页面注入 */
export interface LocalPlaylistsIpc {
  listPlaylists: () => Promise<LocalPlaylistSummary[]>
  reorderPlaylists: (orderedIds: string[]) => Promise<unknown>
}

export interface LocalPlaylistsOptions {
  ipc: LocalPlaylistsIpc
  activeTab: Ref<number>
  /** 当前 tab 的搜索关键词 */
  query: Ref<string>
  /** 拖拽列表容器（模板 ref） */
  listRef: Ref<HTMLElement | null>
}

/** 资料库本地歌单：加载与自定义顺序、多选，以及多选模式下的拖拽重排 */
export function useLocalPlaylists(options: LocalPlaylistsOptions) {
  const { ipc, activeTab, query, listRef } = options
  const { t } = useI18n()
  const toast = useToastStore()

  const playlists = ref<LocalPlaylistSummary[]>([])

  // 多选模式
  const isMultiSelectMode = ref(false)
  const selectedPlaylists = ref<Set<number>>(new Set())

  function enterMultiSelect(pl?: LocalPlaylistSummary) {
    isMultiSelectMode.value = true
    selectedPlaylists.value.clear()
    if (pl && !isProtectedPlaylist(pl)) selectedPlaylists.value.add(pl.id)
  }
  function exitMultiSelect() {
    cancelPlaylistDrag()
    isMultiSelectMode.value = false
    selectedPlaylists.value.clear()
  }
  // 多选只属于本地歌单列表：切 tab 或离开资料库（KeepAlive 停用）都要退出
  watch(activeTab, () => {
    if (isMultiSelectMode.value) exitMultiSelect()
  })
  onDeactivated(() => {
    if (isMultiSelectMode.value) exitMultiSelect()
  })
  function togglePlaylistSelection(id: number) {
    // 受保护歌单（我喜欢的音乐/本地文件）不允许进入选择集，防止被批量删除
    const pl = playlists.value.find(p => p.id === id)
    if (pl && isProtectedPlaylist(pl)) return
    const set = selectedPlaylists.value
    if (set.has(id)) set.delete(id)
    else set.add(id)
  }
  // 全选 / 反选只作用于当前可见（搜索过滤后）的歌单，被筛掉的歌单不会被批量删除
  function selectAll() {
    for (const pl of filteredPlaylists.value) {
      if (!isProtectedPlaylist(pl)) selectedPlaylists.value.add(pl.id)
    }
  }
  function invertSelection() {
    for (const pl of filteredPlaylists.value) {
      if (isProtectedPlaylist(pl)) continue
      if (selectedPlaylists.value.has(pl.id)) selectedPlaylists.value.delete(pl.id)
      else selectedPlaylists.value.add(pl.id)
    }
    log.info('invert selection ->', selectedPlaylists.value.size, 'selected')
  }

  // 拖拽排序：与歌单详情页歌曲拖拽同一套指针交互（composable 抽取自 LocalPlaylistView）
  const {
    dragKey: plDragKey,
    dragOverKey: plDragOverKey,
    dragInsertPosition: plDragInsertPosition,
    dragLandingKey: plDragLandingKey,
    dragUnderGlassKeys: plDragUnderGlassKeys,
    dragGlassActive: plDragGlassActive,
    dropIndicatorVisible: plDropIndicatorVisible,
    dropIndicatorY: plDropIndicatorY,
    dropIndicatorSnap: plDropIndicatorSnap,
    dragItemStyle: plDragItemStyle,
    startDrag: startPlaylistDrag,
    cancelDrag: cancelPlaylistDrag,
  } = usePointerListReorder({
    listRef,
    itemSelector: '.playlist-item',
    canDrag: () => isMultiSelectMode.value,
    // 「我喜欢的音乐」「本地文件」等系统歌单固定首尾，不参与落点
    isValidTargetKey: (key) => {
      const pl = playlists.value.find(p => playlistDragKey(p) === key)
      return !!pl && !isProtectedPlaylist(pl)
    },
    onReorder: (fromKey, toKey, position) => {
      const arr = movePlaylist(playlists.value, fromKey, toKey, position)
      if (!arr) return
      playlists.value = arr
      void savePlaylistOrder(arr)
    },
  })

  // 自定义顺序落库并随同步跨端保留（对齐 Android reorderPlaylists）
  async function savePlaylistOrder(ordered: LocalPlaylistSummary[]) {
    const orderedIds = playlistOrderIds(ordered, isProtectedPlaylist)
    try {
      await ipc.reorderPlaylists(orderedIds)
      log.info('Playlist order saved:', orderedIds)
    } catch (e) {
      log.error('Save playlist order failed:', e)
      toast.error(t('library.playlist_order_save_failed'))
      void loadPlaylists()
    }
  }

  let legacyPlaylistOrderMigrated = false
  async function migrateLegacyPlaylistOrder() {
    if (legacyPlaylistOrderMigrated) return
    legacyPlaylistOrderMigrated = true
    const legacyOrder = readLegacyPlaylistOrder(localStorage)
    if (legacyOrder === null) return
    try {
      if (legacyOrder.length > 0) await ipc.reorderPlaylists(legacyOrder)
      localStorage.removeItem(LEGACY_PLAYLIST_ORDER_KEY)
    } catch (e) {
      legacyPlaylistOrderMigrated = false
      log.warn('Migrate legacy playlist order failed:', e)
    }
  }

  async function loadPlaylists() {
    try {
      await migrateLegacyPlaylistOrder()
      playlists.value = orderLibraryPlaylists(await ipc.listPlaylists())
    } catch (e) {
      log.error('Load playlists failed:', e)
    }
  }

  function displayName(pl: LocalPlaylistSummary): string {
    return localPlaylistDisplayName(pl, { favorites: t('library.liked_songs'), localFiles: t('library.local_files') })
  }

  const filteredPlaylists = computed(() =>
    playlists.value.filter((pl) => searchMatches(query.value, displayName(pl))),
  )
  watch(filteredPlaylists, (visible) => {
    if (selectedPlaylists.value.size > 0) selectedPlaylists.value = visibleSelection(selectedPlaylists.value, visible)
  })

  return {
    playlists,
    filteredPlaylists,
    displayName,
    isMultiSelectMode,
    selectedPlaylists,
    enterMultiSelect,
    exitMultiSelect,
    togglePlaylistSelection,
    selectAll,
    invertSelection,
    plDragKey,
    plDragOverKey,
    plDragInsertPosition,
    plDragLandingKey,
    plDragUnderGlassKeys,
    plDragGlassActive,
    plDropIndicatorVisible,
    plDropIndicatorY,
    plDropIndicatorSnap,
    plDragItemStyle,
    startPlaylistDrag,
    loadPlaylists,
  }
}

export type LocalPlaylistsState = ReturnType<typeof useLocalPlaylists>
