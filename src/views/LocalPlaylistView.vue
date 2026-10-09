<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  usePlayerStore,
  type TrackInfo,
  normalizeTrack,
  displayAlbum,
  tracePlaybackUi,
} from '@/stores/player'
import { useDownloadStore } from '@/stores/download'
import { useTrackDownloadMenu } from '@/composables/useTrackDownloadMenu'
import { useToastStore } from '@/stores/toast'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import AddToPlaylistDialog from '@/components/AddToPlaylistDialog.vue'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import LocateTrackFab from '@/components/LocateTrackFab.vue'
import TrackSelectionToolbar from '@/components/TrackSelectionToolbar.vue'
import { useLocateCurrentTrack } from '@/composables/useLocateCurrentTrack'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import { localPlaylistDisplayName } from '@/modules/library/localPlaylists'
import { recordPlaylistOpen } from '@/modules/library/playlistUsage'
import { createLogger } from '@/utils/logger'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'

const log = createLogger('local-playlist-view')

const route = useRoute()
const router = useRouter()
const player = usePlayerStore()
const downloadStore = useDownloadStore()
const toast = useToastStore()
const { t } = useI18n()

const isLoading = ref(true)
const error = ref<string | null>(null)
const playlistName = ref('')
const searchQuery = ref('')
const selectionMode = ref(false)
const selectedIds = ref<Set<string>>(new Set())
const isBatchRemoving = ref(false)
const dragTrackKey = ref<string | null>(null)
const dragOverTrackKey = ref<string | null>(null)
const dragInsertPosition = ref<'before' | 'after' | null>(null)
const dragLandingTrackKey = ref<string | null>(null)
const dragUnderGlassKeys = ref<Set<string>>(new Set())
const dragGlassActive = ref(false)
const isPersistingTrackOrder = ref(false)
const dragPointerId = ref<number | null>(null)
const trackListRef = ref<HTMLElement | null>(null)
const dragStartPointerY = ref(0)
const dragStartRowTop = ref(0)
const dragRowHeight = ref(0)
const dragOffsetY = ref(0)
let dragHandleElement: HTMLElement | null = null
let dragRowSnapshots: Array<{
  key: string
  top: number
  bottom: number
  midpoint: number
  layoutTop: number
  layoutHeight: number
}> = []
let dragListBounds: { top: number; bottom: number } | null = null
let dragLandingTimer: ReturnType<typeof window.setTimeout> | null = null
// 拖拽行的相邻行 key，用于屏蔽等价于原位的无效落点
let dragPrevSiblingKey: string | null = null
let dragNextSiblingKey: string | null = null
// 单实例落点指示线：位置用 transform 平移，隐藏时保留位置以便原地淡出
const dropIndicatorVisible = ref(false)
const dropIndicatorY = ref(0)
const dropIndicatorSnap = ref(false)
let dropIndicatorSnapFrame: number | null = null
let dragGlassStartFrame: number | null = null
let dragGlassActiveFrame: number | null = null

const tracks = ref<TrackInfo[]>([])

function trackSelectionKey(track: TrackInfo): string {
  return track.playlistKey || `${track.id}|${track.album}`
}

function trackOrderKey(track: TrackInfo): string {
  return track.playlistKey || track.id || trackSelectionKey(track)
}

function trackDragStyle(track: TrackInfo) {
  if (dragTrackKey.value !== trackSelectionKey(track)) return undefined
  return { '--track-drag-offset': `${dragOffsetY.value}px` }
}

function clearDragLandingState() {
  if (dragLandingTimer !== null) {
    window.clearTimeout(dragLandingTimer)
    dragLandingTimer = null
  }
  dragLandingTrackKey.value = null
}

function startDragLandingState(trackKey: string | null) {
  clearDragLandingState()
  if (!trackKey) return
  dragLandingTrackKey.value = trackKey
  dragLandingTimer = window.setTimeout(() => {
    dragLandingTrackKey.value = null
    dragLandingTimer = null
  }, 320)
}

function cancelDragGlassActivation() {
  if (dragGlassStartFrame !== null) {
    window.cancelAnimationFrame(dragGlassStartFrame)
    dragGlassStartFrame = null
  }
  if (dragGlassActiveFrame !== null) {
    window.cancelAnimationFrame(dragGlassActiveFrame)
    dragGlassActiveFrame = null
  }
}

function scheduleDragGlassActivation() {
  cancelDragGlassActivation()
  dragGlassActive.value = false
  dragGlassStartFrame = window.requestAnimationFrame(() => {
    dragGlassStartFrame = null
    dragGlassActiveFrame = window.requestAnimationFrame(() => {
      dragGlassActiveFrame = null
      dragGlassActive.value = dragTrackKey.value !== null
    })
  })
}

const filteredTracks = computed(() => {
  if (!searchQuery.value) return tracks.value
  const q = searchQuery.value.toLowerCase()
  return tracks.value.filter(t =>
    t.title.toLowerCase().includes(q)
    || t.artist.toLowerCase().includes(q)
    || displayAlbum(t.album || '').toLowerCase().includes(q)
  )
})

// 大歌单窗口渲染：首屏只画一批，滚动触底再扩，避免 800+ DOM 卡死
const RENDER_CHUNK = 100
const renderCount = ref(RENDER_CHUNK)
const visibleTracks = computed(() => filteredTracks.value.slice(0, renderCount.value))
const hasMoreTracks = computed(() => renderCount.value < filteredTracks.value.length)

// 删除、重排、收藏触发的静默刷新都会替换列表：保留已展开的行数，只有换关键词才回到首批
let renderedQuery = searchQuery.value
watch(filteredTracks, (list) => {
  const queryChanged = searchQuery.value !== renderedQuery
  renderedQuery = searchQuery.value
  renderCount.value = queryChanged
    ? Math.min(RENDER_CHUNK, list.length)
    : Math.min(list.length, Math.max(renderCount.value, RENDER_CHUNK))
})

function expandVisibleTracks(extra = RENDER_CHUNK) {
  if (!hasMoreTracks.value) return
  renderCount.value = Math.min(filteredTracks.value.length, renderCount.value + extra)
}

function onDetailScroll(e: Event) {
  const el = e.currentTarget as HTMLElement | null
  if (!el || !hasMoreTracks.value) return
  // 距底部 2400px 内预扩; 阈值太小时快速甩动会先看到底部空白再补渲染
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 2400) {
    expandVisibleTracks()
  }
}

// 定位到当前播放：窗口渐扩列表里目标行未渲染时先扩容再滚动
const detailViewRef = ref<HTMLElement | null>(null)
const currentRowKey = computed(() => {
  const current = player.currentTrack
  if (!current) return null
  const key = trackSelectionKey(current)
  return filteredTracks.value.some(track => trackSelectionKey(track) === key) ? key : null
})
const { fabVisible: locateFabVisible, locate: locateCurrentTrack } = useLocateCurrentTrack({
  containerRef: detailViewRef,
  currentKey: currentRowKey,
  suppressed: selectionMode,
  ensureRendered: key => {
    const index = filteredTracks.value.findIndex(track => trackSelectionKey(track) === key)
    if (index < 0) return false
    if (index >= renderCount.value) {
      // 多带一段缓冲，居中后目标行下方仍有内容
      renderCount.value = Math.min(filteredTracks.value.length, index + 20)
    }
    return true
  },
})

const selectedTracks = computed(() => tracks.value.filter(t => selectedIds.value.has(trackSelectionKey(t))))
const selectedCount = computed(() => selectedIds.value.size)
// 选择操作仍基于完整过滤结果，不限于当前窗口
const visibleSelectedCount = computed(() => filteredTracks.value.filter(t => selectedIds.value.has(trackSelectionKey(t))).length)
const allVisibleSelected = computed(() => filteredTracks.value.length > 0 && visibleSelectedCount.value === filteredTracks.value.length)

// 总时长
const totalDuration = computed(() => {
  const totalMs = tracks.value.reduce((sum, t) => sum + (t.durationMs || 0), 0)
  return formatTotalDuration(totalMs)
})

function formatTotalDuration(ms: number): string {
  const totalMin = Math.floor(ms / 60000)
  if (totalMin >= 60) {
    const h = Math.floor(totalMin / 60)
    const m = totalMin % 60
    return `${h}${t('common.hour_short')} ${m}${t('common.minute_short')}`
  }
  return `${totalMin}${t('common.minute_short')}`
}

async function loadDetail(options: { silent?: boolean } = {}) {
  const id = String(route.params.id || '')
  if (!/^-?[1-9]\d*$/.test(id)) return

  const loadStarted = performance.now()
  const silent = options.silent === true
  log.info('playlist load begin:', {
    playlistId: id,
    silent,
    loadingAudio: player.isLoadingAudio,
    currentTrackId: player.currentTrack?.id || '',
  })
  // 收藏切换触发的静默刷新不闪 loading，避免列表跳动
  if (!silent || tracks.value.length === 0) {
    isLoading.value = true
  }
  error.value = null

  try {
    const listStarted = performance.now()
    const playlists = await invoke<{ id: string; name: string }[]>('get_home_local_playlists')
    log.info('playlist list returned:', {
      playlistId: id,
      count: playlists.length,
      elapsedMs: Math.round(performance.now() - listStarted),
      loadingAudio: player.isLoadingAudio,
    })
    const pl = playlists.find(p => p.id === id)
    playlistName.value = pl
      ? localPlaylistDisplayName(pl, { favorites: t('library.liked_songs'), localFiles: t('library.local_files') })
      : ''

    const tracksStarted = performance.now()
    const trackList = await invoke<any[]>('get_playlist_tracks', { id })
    log.info('playlist tracks returned:', {
      playlistId: id,
      count: trackList.length,
      elapsedMs: Math.round(performance.now() - tracksStarted),
      loadingAudio: player.isLoadingAudio,
    })
    tracks.value = trackList.map(normalizeTrack)
    // 静默刷新不重复预取，减少 IO；也不算一次打开
    if (!silent) {
      player.prefetchPlaybackTracks(tracks.value)
      if (pl) {
        recordPlaylistOpen({
          source: 'local',
          id,
          name: pl.name,
          coverUrl: tracks.value.find(track => track.coverUrl)?.coverUrl,
          trackCount: tracks.value.length,
        })
      }
    }
    log.info('playlist load committed:', {
      playlistId: id,
      count: tracks.value.length,
      totalMs: Math.round(performance.now() - loadStarted),
      loadingAudio: player.isLoadingAudio,
      currentTrackId: player.currentTrack?.id || '',
    })
  } catch (e: any) {
    if (!silent) {
      error.value = e?.toString() || t('player.load_failed')
    }
    log.error('playlist load failed:', {
      playlistId: id,
      totalMs: Math.round(performance.now() - loadStarted),
      loadingAudio: player.isLoadingAudio,
      currentTrackId: player.currentTrack?.id || '',
      error: e,
    })
  } finally {
    isLoading.value = false
    log.info('playlist load finished:', {
      playlistId: id,
      totalMs: Math.round(performance.now() - loadStarted),
      loadingAudio: player.isLoadingAudio,
    })
  }
}

// 右键/三点菜单
const trackMenu = ref<{ show: boolean; x: number; y: number; track: TrackInfo | null; index: number }>({
  show: false, x: 0, y: 0, track: null, index: -1,
})
const showAddToPlaylist = ref(false)
const addToPlaylistTarget = ref<TrackInfo | null>(null)
const addToPlaylistTargets = ref<TrackInfo[]>([])

function openTrackMenu(e: MouseEvent, track: TrackInfo, index: number) {
  if (selectionMode.value) {
    toggleSelected(trackSelectionKey(track))
    return
  }
  const btn = e.currentTarget as HTMLElement
  const rect = btn.getBoundingClientRect()
  let x = rect.left - 204
  if (x < 8) x = rect.right + 4
  trackMenu.value = { show: true, x, y: rect.top, track, index }
}

function openTrackContextMenu(e: MouseEvent, track: TrackInfo, index: number) {
  if (selectionMode.value) {
    toggleSelected(trackSelectionKey(track))
    return
  }
  trackMenu.value = {
    show: true,
    x: e.clientX,
    y: e.clientY,
    track,
    index,
  }
}

function closeTrackMenu() {
  trackMenu.value.show = false
}

const { downloadMenuItem, downloadFromMenu } = useTrackDownloadMenu(() => trackMenu.value.track, closeTrackMenu)

function openAddToPlaylist(track: TrackInfo) {
  closeTrackMenu()
  addToPlaylistTarget.value = track
  addToPlaylistTargets.value = []
  showAddToPlaylist.value = true
}

function openBatchAddToPlaylist() {
  if (selectedTracks.value.length === 0) return
  closeTrackMenu()
  addToPlaylistTarget.value = null
  addToPlaylistTargets.value = selectedTracks.value
  showAddToPlaylist.value = true
}

// 删除确认
const showRemoveDialog = ref(false)
const removeTarget = ref<TrackInfo | null>(null)
const removeMode = ref<'single' | 'batch'>('single')

function requestRemove(track: TrackInfo) {
  closeTrackMenu()
  removeMode.value = 'single'
  removeTarget.value = track
  showRemoveDialog.value = true
}

function requestBatchRemove() {
  if (selectedCount.value === 0) return
  closeTrackMenu()
  removeMode.value = 'batch'
  removeTarget.value = null
  showRemoveDialog.value = true
}

async function confirmRemove() {
  const id = String(route.params.id || '')
  try {
    if (removeMode.value === 'batch') {
      isBatchRemoving.value = true
      const ids = [...selectedIds.value]
      await invoke('remove_tracks_from_playlist', { playlistId: id, trackIds: ids })
      tracks.value = tracks.value.filter(t => !selectedIds.value.has(trackSelectionKey(t)))
      toast.success(t('library.removed_tracks', { count: ids.length }))
      leaveSelectionMode()
    } else if (removeTarget.value) {
      const trackKey = trackSelectionKey(removeTarget.value)
      await invoke('remove_from_playlist', { playlistId: id, trackId: trackKey })
      tracks.value = tracks.value.filter(t => trackSelectionKey(t) !== trackKey)
    }
  } catch (e) {
    log.error('Remove failed:', e)
    toast.error(t('library.remove_tracks_failed'))
  } finally {
    isBatchRemoving.value = false
    showRemoveDialog.value = false
    removeTarget.value = null
  }
}

function addToQueueNext(track: TrackInfo) {
  closeTrackMenu()
  player.addToQueueNext(track)
}

function addToQueueEnd(track: TrackInfo) {
  closeTrackMenu()
  player.addToQueueEnd(track)
}

const trackMenuItems = computed<ContextMenuItem[]>(() => [
  createContextMenuItem(t('common.multi_select'), { id: 'select', icon: 'checklist' }),
  createContextMenuItem(t('player.play_next'), { id: 'play-next', icon: 'queue_play_next' }),
  createContextMenuItem(t('player.add_to_queue'), { id: 'add-to-queue', icon: 'add_to_queue' }),
  createContextMenuItem(t('player.add_to_playlist'), { id: 'add-to-playlist', icon: 'playlist_add' }),
  downloadMenuItem.value,
  createContextMenuItem(t('library.remove_from_playlist'), {
    id: 'remove-from-playlist',
    icon: 'delete',
    danger: true,
  }),
])

function handleTrackMenuClick(item: ContextMenuActionItem) {
  const track = trackMenu.value.track
  if (!track) return

  switch (item.id) {
    case 'download':
      void downloadFromMenu()
      break
    case 'select':
      closeTrackMenu()
      enterSelectionMode(track)
      break
    case 'play-next':
      addToQueueNext(track)
      break
    case 'add-to-queue':
      addToQueueEnd(track)
      break
    case 'add-to-playlist':
      openAddToPlaylist(track)
      break
    case 'remove-from-playlist':
      requestRemove(track)
      break
  }
}

function enterSelectionMode(track?: TrackInfo) {
  selectionMode.value = true
  if (track) selectedIds.value = new Set(selectedIds.value).add(trackSelectionKey(track))
}

function leaveSelectionMode() {
  cancelTrackDrag()
  selectionMode.value = false
  selectedIds.value = new Set()
}

function toggleSelected(trackId: string) {
  const next = new Set(selectedIds.value)
  if (next.has(trackId)) next.delete(trackId)
  else next.add(trackId)
  selectedIds.value = next
  if (selectionMode.value && next.size === 0) selectionMode.value = false
}

function toggleSelectAllVisible() {
  if (allVisibleSelected.value) {
    const next = new Set(selectedIds.value)
    for (const track of filteredTracks.value) next.delete(trackSelectionKey(track))
    selectedIds.value = next
    if (next.size === 0) selectionMode.value = false
    return
  }
  const next = new Set(selectedIds.value)
  for (const track of filteredTracks.value) next.add(trackSelectionKey(track))
  selectedIds.value = next
  if (next.size > 0) selectionMode.value = true
}

function invertSelectionVisible() {
  const next = new Set(selectedIds.value)
  for (const track of filteredTracks.value) {
    const key = trackSelectionKey(track)
    if (next.has(key)) next.delete(key)
    else next.add(key)
  }
  selectedIds.value = next
  selectionMode.value = next.size > 0
}

function onTrackDragPointerDown(e: PointerEvent, track: TrackInfo) {
  if (!selectionMode.value || isPersistingTrackOrder.value) return
  if (e.pointerType === 'mouse' && e.button !== 0) return
  e.preventDefault()
  clearDragLandingState()
  dragTrackKey.value = trackSelectionKey(track)
  scheduleDragGlassActivation()
  dragPointerId.value = e.pointerId
  dragHandleElement = e.currentTarget as HTMLElement
  const row = dragHandleElement.closest('.track-item') as HTMLElement | null
  const rowRect = row?.getBoundingClientRect()
  dragStartPointerY.value = e.clientY
  dragStartRowTop.value = rowRect?.top ?? e.clientY
  dragRowHeight.value = rowRect?.height ?? 0
  dragOffsetY.value = 0
  captureTrackDragSnapshot()
  dragHandleElement.setPointerCapture?.(e.pointerId)
  updateTrackDragTarget(e)
  window.addEventListener('pointermove', onTrackDragPointerMove)
  window.addEventListener('pointerup', onTrackDragPointerUp)
  window.addEventListener('pointercancel', cancelTrackDrag)
}

function onTrackDragPointerMove(e: PointerEvent) {
  if (dragPointerId.value !== e.pointerId) return
  e.preventDefault()
  autoScrollWhileDragging(e)
  // 滚动后重新采样行位置，保证目标插入点准确
  captureTrackDragSnapshot()
  updateTrackDragOffset(e)
  updateDragUnderGlassKeys()
  updateTrackDragTarget(e)
}

function autoScrollWhileDragging(e: PointerEvent) {
  // 优先滚详情页容器，找不到再滚主内容区
  const scroller =
    trackListRef.value?.closest('.detail-view') as HTMLElement | null
    || document.querySelector('.content') as HTMLElement | null
  if (!scroller) return

  const rect = scroller.getBoundingClientRect()
  const edge = 72
  const maxStep = 28
  let delta = 0
  if (e.clientY < rect.top + edge) {
    const t = 1 - Math.max(0, e.clientY - rect.top) / edge
    delta = -Math.ceil(maxStep * t)
  } else if (e.clientY > rect.bottom - edge) {
    const t = 1 - Math.max(0, rect.bottom - e.clientY) / edge
    delta = Math.ceil(maxStep * t)
  }
  if (delta === 0) return

  const prev = scroller.scrollTop
  scroller.scrollTop = prev + delta
  const actual = scroller.scrollTop - prev
  if (actual === 0) return
  // 列表滚动后补偿拖拽起点，保持行跟随手指
  dragStartPointerY.value -= actual
  dragStartRowTop.value -= actual
}

function updateTrackDragOffset(e: PointerEvent) {
  // 允许拖出当前视口，配合自动滚动完成跨屏排序
  const requestedOffset = e.clientY - dragStartPointerY.value
  dragOffsetY.value = requestedOffset
}

function updateDragUnderGlassKeys() {
  const draggedKey = dragTrackKey.value
  if (!draggedKey || dragRowHeight.value <= 0) {
    dragUnderGlassKeys.value = new Set()
    return
  }

  const top = dragStartRowTop.value + dragOffsetY.value
  const bottom = top + dragRowHeight.value
  const overlapInset = 8
  dragUnderGlassKeys.value = new Set(
    dragRowSnapshots
      .filter(row => row.bottom > top + overlapInset && row.top < bottom - overlapInset)
      .map(row => row.key),
  )
}

function captureTrackDragSnapshot() {
  const list = trackListRef.value
  const draggedKey = dragTrackKey.value
  if (!list || !draggedKey) {
    dragRowSnapshots = []
    dragListBounds = null
    return
  }

  const listRect = list.getBoundingClientRect()
  dragListBounds = { top: listRect.top, bottom: listRect.bottom }
  const rows = [...list.querySelectorAll<HTMLElement>('.track-item')]
  const draggedIndex = rows.findIndex(row => (row.dataset.trackKey || '') === draggedKey)
  dragPrevSiblingKey = draggedIndex > 0 ? rows[draggedIndex - 1].dataset.trackKey || null : null
  dragNextSiblingKey = draggedIndex >= 0 && draggedIndex < rows.length - 1
    ? rows[draggedIndex + 1].dataset.trackKey || null
    : null
  // 用布局位置（offsetTop）而非 getBoundingClientRect：目标行的位移过渡带 transform，
  // 若采样含 transform 会把视觉位移反馈进目标计算，造成边界处目标来回翻转闪烁
  dragRowSnapshots = rows
    .map(row => {
      const layoutTop = row.offsetTop
      const layoutHeight = row.offsetHeight
      const top = listRect.top + layoutTop
      return {
        key: row.dataset.trackKey || '',
        top,
        bottom: top + layoutHeight,
        midpoint: top + layoutHeight / 2,
        layoutTop,
        layoutHeight,
      }
    })
    .filter(row => row.key && row.key !== draggedKey)
}

function resolveTrackInsertTarget(e: PointerEvent): { key: string; position: 'before' | 'after' } | null {
  if (dragRowSnapshots.length === 0) return null

  // 允许拖到视口外时仍按最近行判定，配合自动滚动
  for (const row of dragRowSnapshots) {
    if (e.clientY < row.midpoint) {
      return { key: row.key, position: 'before' }
    }
  }

  const lastRow = dragRowSnapshots[dragRowSnapshots.length - 1]
  return lastRow ? { key: lastRow.key, position: 'after' } : null
}

function updateTrackDragTarget(e: PointerEvent) {
  const target = resolveTrackInsertTarget(e)
  // 与原位等价的落点不提示：拖拽行下一行的 before / 上一行的 after 都是无效移动
  const isNoopTarget =
    !!target
    && ((target.position === 'before' && target.key === dragNextSiblingKey)
      || (target.position === 'after' && target.key === dragPrevSiblingKey))
  if (!target || isNoopTarget || target.key === dragTrackKey.value) {
    dragOverTrackKey.value = null
    dragInsertPosition.value = null
    dropIndicatorVisible.value = false
    return
  }
  // 同一目标不重复更新，避免每帧刷新样式
  if (target.key === dragOverTrackKey.value && target.position === dragInsertPosition.value) return
  dragOverTrackKey.value = target.key
  dragInsertPosition.value = target.position
  moveDropIndicatorToTarget(target)
}

function moveDropIndicatorToTarget(target: { key: string; position: 'before' | 'after' }) {
  const snap = dragRowSnapshots.find(row => row.key === target.key)
  if (!snap) {
    dropIndicatorVisible.value = false
    return
  }
  // 指示线中心对准两行之间 2px 间隙的中点，线高 3px
  const boundaryCenter = target.position === 'before'
    ? snap.layoutTop - 1
    : snap.layoutTop + snap.layoutHeight + 1
  if (!dropIndicatorVisible.value) {
    // 从隐藏转为显示时直接落位，避免从上一次的旧位置滑入
    scheduleDropIndicatorSnap()
  }
  dropIndicatorY.value = boundaryCenter - 1.5
  dropIndicatorVisible.value = true
}

function scheduleDropIndicatorSnap() {
  if (dropIndicatorSnapFrame !== null) window.cancelAnimationFrame(dropIndicatorSnapFrame)
  dropIndicatorSnap.value = true
  dropIndicatorSnapFrame = window.requestAnimationFrame(() => {
    dropIndicatorSnapFrame = null
    dropIndicatorSnap.value = false
  })
}

function cleanupTrackDrag() {
  window.removeEventListener('pointermove', onTrackDragPointerMove)
  window.removeEventListener('pointerup', onTrackDragPointerUp)
  window.removeEventListener('pointercancel', cancelTrackDrag)
  if (dragHandleElement && dragPointerId.value !== null) {
    if (dragHandleElement.hasPointerCapture?.(dragPointerId.value)) {
      dragHandleElement.releasePointerCapture(dragPointerId.value)
    }
  }
  dragHandleElement = null
  dragPointerId.value = null
  dragTrackKey.value = null
  dragOverTrackKey.value = null
  dragInsertPosition.value = null
  // 只翻可见标志、保留 dropIndicatorY，让指示线原地淡出而非瞬间消失
  dropIndicatorVisible.value = false
  if (dropIndicatorSnapFrame !== null) {
    window.cancelAnimationFrame(dropIndicatorSnapFrame)
    dropIndicatorSnapFrame = null
    dropIndicatorSnap.value = false
  }
  dragPrevSiblingKey = null
  dragNextSiblingKey = null
  cancelDragGlassActivation()
  dragGlassActive.value = false
  dragUnderGlassKeys.value = new Set()
  dragStartPointerY.value = 0
  dragStartRowTop.value = 0
  dragRowHeight.value = 0
  dragOffsetY.value = 0
  dragRowSnapshots = []
  dragListBounds = null
}

function cancelTrackDrag() {
  startDragLandingState(dragTrackKey.value)
  cleanupTrackDrag()
}

async function onTrackDragPointerUp(e: PointerEvent) {
  if (dragPointerId.value !== e.pointerId) return
  e.preventDefault()
  const fromKey = dragTrackKey.value
  const toKey = dragOverTrackKey.value
  const insertPosition = dragInsertPosition.value
  startDragLandingState(fromKey)
  cleanupTrackDrag()

  if (!fromKey || !toKey || !insertPosition || fromKey === toKey) return

  const previousTracks = tracks.value
  const nextTracks = [...tracks.value]
  const fromIndex = nextTracks.findIndex(track => trackSelectionKey(track) === fromKey)
  let toIndex = nextTracks.findIndex(track => trackSelectionKey(track) === toKey)
  if (fromIndex < 0 || toIndex < 0 || fromIndex === toIndex) return

  const [moved] = nextTracks.splice(fromIndex, 1)
  if (toIndex > fromIndex) toIndex -= 1
  const insertIndex = insertPosition === 'after' ? toIndex + 1 : toIndex
  nextTracks.splice(insertIndex, 0, moved)

  const previousOrder = previousTracks.map(trackSelectionKey).join('\n')
  const nextOrder = nextTracks.map(trackSelectionKey).join('\n')
  if (previousOrder === nextOrder) return

  tracks.value = nextTracks

  try {
    isPersistingTrackOrder.value = true
    await invoke('reorder_playlist_tracks', {
      playlistId: String(route.params.id),
      orderedKeys: nextTracks.map(trackOrderKey),
      expectedKeys: previousTracks.map(trackOrderKey),
    })
    player.prefetchPlaybackTracks(tracks.value)
  } catch (e) {
    tracks.value = previousTracks
    if (String(e).includes('PLAYLIST_ORDER_CHANGED')) {
      toast.error(t('library.track_order_changed'))
      void loadDetail({ silent: true })
    } else {
      log.error('Reorder playlist tracks failed:', e)
      toast.error(t('library.track_order_save_failed'))
    }
  } finally {
    isPersistingTrackOrder.value = false
  }
}

function playSelected() {
  if (selectedTracks.value.length === 0) return
  player.playAll(selectedTracks.value, undefined, undefined, String(route.params.id))
}

function addSelectedToQueueEnd() {
  for (const track of selectedTracks.value) player.addToQueueEnd(track)
  toast.success(t('player.added_to_queue_count', { count: selectedTracks.value.length }))
}

function downloadSelected() {
  const targets = [...selectedTracks.value]
  leaveSelectionMode()
  for (const track of targets) void downloadStore.downloadTrack(track)
}

function playAll() {
  if (tracks.value.length === 0) return
  player.playAll(tracks.value, undefined, undefined, String(route.params.id))
}

function shufflePlay() {
  if (tracks.value.length === 0) return
  player.shufflePlay(tracks.value, String(route.params.id))
}

function playTrack(track: TrackInfo) {
  tracePlaybackUi(
    'local_playlist_click',
    track,
    `selectionMode=${selectionMode.value}, filteredTracks=${filteredTracks.value.length}`,
  )
  if (selectionMode.value) {
    toggleSelected(trackSelectionKey(track))
    return
  }
  player.playAll(filteredTracks.value, track.id, trackSelectionKey(track), String(route.params.id))
}

function prefetchTrack(track: TrackInfo) {
  player.prefetchIntent(track)
}

// 歌单封面：取第一首有 cover 的曲目
const playlistCover = computed(() => {
  for (const t of tracks.value) {
    if (t.coverUrl) return t.coverUrl
  }
  return ''
})

let unlistenPlaylistsChanged: UnlistenFn | null = null
// 组件已卸载标志：若在 await listen() resolve 前就卸载（路由切换，本视图不在 keep-alive
// 列表），onUnmounted 会先拿到 null；据此在 listen resolve 后立即解绑，防监听器滞留（MK-04）
let disposed = false
let playlistRefreshTimer: ReturnType<typeof setTimeout> | null = null

function schedulePlaylistRefresh() {
  // 收藏/移除会连发事件，合并刷新避免闪烁
  if (playlistRefreshTimer) clearTimeout(playlistRefreshTimer)
  playlistRefreshTimer = setTimeout(() => {
    playlistRefreshTimer = null
    void loadDetail({ silent: true })
  }, 120)
}

onMounted(async () => {
  loadDetail()
  try {
    const unlisten = await listen('playlists-changed', () => {
      schedulePlaylistRefresh()
    })
    if (disposed) {
      unlisten()
    } else {
      unlistenPlaylistsChanged = unlisten
    }
  } catch (e) {
    log.error('listen playlists-changed failed:', e)
  }
})

onUnmounted(() => {
  disposed = true
  cleanupTrackDrag()
  clearDragLandingState()
  if (playlistRefreshTimer) {
    clearTimeout(playlistRefreshTimer)
    playlistRefreshTimer = null
  }
  if (unlistenPlaylistsChanged) {
    unlistenPlaylistsChanged()
    unlistenPlaylistsChanged = null
  }
})
</script>

<template>
  <div ref="detailViewRef" class="detail-view" @scroll.passive="onDetailScroll">
    <header class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="header-search" v-if="!isLoading && tracks.length > 0">
        <span class="material-symbols-rounded search-icon">search</span>
        <input v-model="searchQuery" :placeholder="t('player.search_tracks')" class="search-input" />
      </div>
    </header>

    <div v-if="isLoading" class="state-center">
      <span class="material-symbols-rounded spinning">progress_activity</span>
      <p>{{ t('player.loading') }}</p>
    </div>

    <div v-else-if="error" class="state-center">
      <span class="material-symbols-rounded" style="font-size: 48px; opacity: 0.3">error</span>
      <p>{{ error }}</p>
      <button class="retry-btn" @click="() => loadDetail()">{{ t('player.retry') }}</button>
    </div>

    <template v-else>
      <!-- Hero 封面 + 信息（对齐 NeteasePlaylistView） -->
      <div class="detail-hero">
        <div class="hero-cover">
          <BilibiliCoverImage v-if="playlistCover" :src="playlistCover">
            <span class="material-symbols-rounded filled" style="font-size: 48px; opacity: 0.3">queue_music</span>
          </BilibiliCoverImage>
          <span v-else class="material-symbols-rounded filled" style="font-size: 48px; opacity: 0.3">queue_music</span>
        </div>
        <div class="hero-info">
          <h1 class="hero-title">{{ playlistName }}</h1>
          <p class="hero-meta">
            {{ t('player.track_count', { count: tracks.length }) }} · {{ totalDuration }}
          </p>
          <div class="hero-actions" v-if="tracks.length > 0">
            <button class="play-all-btn" @click="playAll">
              <span class="material-symbols-rounded filled">play_arrow</span>
              {{ t('player.play_all') }}
            </button>
            <button class="hero-icon-btn" :title="t('player.shuffle_play')" @click="shufflePlay">
              <span class="material-symbols-rounded">shuffle</span>
            </button>
            <button class="hero-icon-btn" :title="t('common.multi_select')" @click="enterSelectionMode()">
              <span class="material-symbols-rounded">checklist</span>
            </button>
          </div>
        </div>
      </div>

      <div v-if="filteredTracks.length === 0" class="state-center">
        <p>{{ searchQuery.trim() && tracks.length > 0 ? t('player.no_results') : t('player.empty_playlist') }}</p>
      </div>
      <template v-else>
        <TrackSelectionToolbar
          v-if="selectionMode"
          class="sticky-selection-toolbar"
          :selected-count="selectedCount"
          :visible-selected-count="visibleSelectedCount"
          :all-visible-selected="allVisibleSelected"
          show-delete
          :delete-label="t('common.remove_selected')"
          @select-all="toggleSelectAllVisible"
          @invert-selection="invertSelectionVisible"
          @play="playSelected"
          @queue="addSelectedToQueueEnd"
          @playlist="openBatchAddToPlaylist"
          @download="downloadSelected"
          @delete="requestBatchRemove"
          @exit="leaveSelectionMode"
        />
        <div ref="trackListRef" class="track-list">
        <div
          class="drop-indicator"
          :class="{ visible: dropIndicatorVisible, snap: dropIndicatorSnap }"
          :style="{ transform: `translate3d(0, ${dropIndicatorY}px, 0)` }"
          aria-hidden="true"
        />
        <!-- 拖拽中切到无动画 name，释放后行归位走 move 过渡，避免与指针跟随打架 -->
        <TransitionGroup :name="dragTrackKey ? 'track-static' : 'track-move'">
        <div
          v-for="(track, index) in visibleTracks"
          :key="trackSelectionKey(track)"
          class="track-item"
          :class="{
            active: player.currentTrack && trackSelectionKey(player.currentTrack) === trackSelectionKey(track),
            selected: selectedIds.has(trackSelectionKey(track)),
            'selection-mode': selectionMode,
            dragging: dragTrackKey === trackSelectionKey(track),
            'glass-active': dragGlassActive && dragTrackKey === trackSelectionKey(track),
            landing: dragLandingTrackKey === trackSelectionKey(track),
            'drag-under-glass': dragUnderGlassKeys.has(trackSelectionKey(track)),
            'drag-under-glass-active': dragGlassActive && dragUnderGlassKeys.has(trackSelectionKey(track)),
            'drag-over-before': dragOverTrackKey === trackSelectionKey(track) && dragInsertPosition === 'before',
            'drag-over-after': dragOverTrackKey === trackSelectionKey(track) && dragInsertPosition === 'after',
          }"
          :data-track-key="trackSelectionKey(track)"
          :style="trackDragStyle(track)"
          @click="playTrack(track)"
          @pointerenter="prefetchTrack(track)"
          @focusin="prefetchTrack(track)"
          @contextmenu.prevent.stop="openTrackContextMenu($event, track, index)"
        >
          <button v-if="selectionMode" class="track-select" @click.stop="toggleSelected(trackSelectionKey(track))">
            <span class="material-symbols-rounded filled">{{ selectedIds.has(trackSelectionKey(track)) ? 'check_circle' : 'radio_button_unchecked' }}</span>
          </button>
          <div v-else class="track-index">
            <div v-if="player.currentTrack && trackSelectionKey(player.currentTrack) === trackSelectionKey(track) && player.isPlaying" class="equalizer-bars"><span class="bar"/><span class="bar"/><span class="bar"/></div>
            <span v-else class="index-num">{{ index + 1 }}</span>
          </div>
          <div class="track-cover">
            <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy">
              <span class="material-symbols-rounded filled">music_note</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled">music_note</span>
          </div>
          <div class="track-info">
            <div class="track-title">{{ track.title }}</div>
            <div class="track-meta">{{ track.artist }}<template v-if="displayAlbum(track.album)"> · {{ displayAlbum(track.album) }}</template></div>
          </div>
          <span
            v-if="downloadStore.isDownloaded(track.id)"
            class="track-download-badge material-symbols-rounded filled"
            :title="t('download.downloaded')"
          >download_done</span>
          <button
            v-if="selectionMode"
            class="track-drag-handle material-symbols-rounded"
            type="button"
            :disabled="isPersistingTrackOrder"
            :title="t('common.drag_to_reorder')"
            @pointerdown.stop="onTrackDragPointerDown($event, track)"
            @click.stop
          >drag_handle</button>
          <div v-else class="track-duration">{{ formatDuration(track.durationMs) }}</div>
          <button v-if="!selectionMode" class="track-more" @click.stop="openTrackMenu($event, track, index)">
            <span class="material-symbols-rounded">more_vert</span>
          </button>
        </div>
        </TransitionGroup>
        </div>
      </template>
    </template>

    <LocateTrackFab
      :visible="locateFabVisible"
      :label="t('player.locate_current')"
      @click="locateCurrentTrack"
    />

    <ContextMenu
      :open="trackMenu.show"
      :x="trackMenu.x"
      :y="trackMenu.y"
      :items="trackMenuItems"
      @update:open="trackMenu.show = $event"
      @click="handleTrackMenuClick"
    />

    <!-- 删除确认对话框 -->
    <M3Dialog
      v-model:open="showRemoveDialog"
      :title="removeMode === 'batch' ? t('library.batch_remove_title') : t('library.remove_from_playlist')"
      icon="delete"
      :confirm-text="removeMode === 'batch' ? t('common.remove_selected') : t('library.remove_from_playlist')"
      :confirm-disabled="isBatchRemoving"
      confirm-danger
      @confirm="confirmRemove"
    >
      <p class="dialog-msg">{{ removeMode === 'batch' ? t('library.batch_remove_confirm', { count: selectedCount }) : t('library.remove_confirm_msg', { name: removeTarget?.title || '' }) }}</p>
    </M3Dialog>

    <AddToPlaylistDialog v-model:open="showAddToPlaylist" :track="addToPlaylistTarget" :tracks="addToPlaylistTargets" />
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;


.sticky-selection-toolbar {
  position: sticky;
  /* 排在常驻 detail-header（56px 高）之下，避免两个 sticky 叠死在同一位置 */
  top: 60px;
  z-index: 10;
}

.track-select {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-full);
  color: var(--md-primary);
  flex-shrink: 0;

  .material-symbols-rounded { font-size: 22px; }
  &:hover { background: color-mix(in srgb, var(--md-primary) 10%, transparent); }
}

:deep(.track-item.selected),
.track-item.selected {
  background: color-mix(in srgb, var(--md-primary) 14%, transparent);
}

:deep(.track-item),
.track-item {
  position: relative;
  transform: translate3d(0, 0, 0);
  filter: blur(0) saturate(1);
  isolation: isolate;
  transition:
    filter 240ms cubic-bezier(0.2, 0, 0, 1),
    transform 180ms cubic-bezier(0.2, 0, 0, 1),
    background 180ms cubic-bezier(0.2, 0, 0, 1),
    box-shadow 180ms cubic-bezier(0.2, 0, 0, 1),
    opacity 220ms cubic-bezier(0.2, 0, 0, 1);
  will-change: transform;
}

.track-item > * {
  position: relative;
  z-index: 1;
}

.track-item::before {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 0;
  border-radius: inherit;
  pointer-events: none;
  opacity: 0;
  background: color-mix(in srgb, var(--md-surface-container-highest) 36%, transparent);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.38),
    inset 0 -1px 0 rgba(255, 255, 255, 0.12);
  backdrop-filter: blur(0) saturate(1);
  -webkit-backdrop-filter: blur(0) saturate(1);
  transition:
    opacity 180ms cubic-bezier(0.2, 0, 0, 1),
    background 180ms cubic-bezier(0.2, 0, 0, 1),
    backdrop-filter 180ms cubic-bezier(0.2, 0, 0, 1),
    -webkit-backdrop-filter 180ms cubic-bezier(0.2, 0, 0, 1);
}

.track-item.selection-mode .track-more {
  opacity: 0;
  pointer-events: none;
}

.track-item.selection-mode .track-duration {
  opacity: 0.45;
}

.track-item.dragging {
  z-index: 6;
  opacity: 0.96;
  transform: translate3d(0, var(--track-drag-offset, 0px), 0) scale(1.012);
  background: color-mix(in srgb, var(--md-surface-container-highest) 18%, transparent);
  box-shadow:
    0 18px 38px rgba(0, 0, 0, 0.18),
    0 4px 10px color-mix(in srgb, var(--md-primary) 10%, transparent);
  transition:
    background 180ms cubic-bezier(0.2, 0, 0, 1),
    box-shadow 180ms cubic-bezier(0.2, 0, 0, 1),
    opacity 180ms cubic-bezier(0.2, 0, 0, 1);
}

.track-item.dragging.glass-active::before {
  opacity: 1;
  background: color-mix(in srgb, var(--md-surface-container-highest) 34%, transparent);
  backdrop-filter: blur(28px) saturate(1.42);
  -webkit-backdrop-filter: blur(28px) saturate(1.42);
}

.track-item.drag-under-glass {
  filter: blur(0) saturate(1);
  opacity: 1;
  will-change: filter, opacity;
}

.track-item.drag-under-glass-active {
  filter: blur(2.8px) saturate(0.82);
  opacity: 0.72;
}

.track-item.landing {
  animation: track-glass-shadow-release 320ms cubic-bezier(0.2, 0, 0, 1) both;
}

.track-item.landing::before {
  animation: track-glass-release 320ms cubic-bezier(0.2, 0, 0, 1) both;
}

.track-item.drag-over-before {
  transform: translate3d(0, 5px, 0);
}

.track-item.drag-over-after {
  transform: translate3d(0, -5px, 0);
}

/* 单实例落点指示线：换目标时整体平移而非逐行显隐，杜绝闪烁 */
.track-list {
  position: relative;
}

.drop-indicator {
  position: absolute;
  top: 0;
  left: 44px;
  right: 44px;
  z-index: 2;
  height: 3px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-primary) 88%, white);
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--md-primary) 12%, transparent);
  opacity: 0;
  pointer-events: none;
  transition:
    transform 170ms cubic-bezier(0.2, 0, 0, 1),
    opacity 120ms ease-out;

  &.visible {
    opacity: 1;
  }

  /* 隐藏转显示的首帧直接落位，只保留淡入 */
  &.snap {
    transition: opacity 120ms ease-out;
  }
}

/* 拖拽释放后行归位的 FLIP move 过渡 */
.track-item.track-move-move {
  transition: transform 300ms cubic-bezier(0.2, 0, 0, 1);
}

@keyframes track-glass-release {
  0% {
    opacity: 1;
    background: color-mix(in srgb, var(--md-surface-container-highest) 34%, transparent);
    backdrop-filter: blur(28px) saturate(1.42);
    -webkit-backdrop-filter: blur(28px) saturate(1.42);
  }
  100% {
    opacity: 0;
    background: color-mix(in srgb, var(--md-surface-container-highest) 16%, transparent);
    backdrop-filter: blur(0) saturate(1);
    -webkit-backdrop-filter: blur(0) saturate(1);
  }
}

@keyframes track-glass-shadow-release {
  0% {
    box-shadow:
      0 18px 38px rgba(0, 0, 0, 0.18),
      0 4px 10px color-mix(in srgb, var(--md-primary) 10%, transparent);
  }
  100% {
    box-shadow: none;
  }
}

/* 已下载标识：在音乐库列表中标记本地已缓存的曲目 */
.track-download-badge {
  flex-shrink: 0;
  color: var(--md-primary);
  opacity: 0.85;
  font-size: 18px;
}

.track-drag-handle {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  opacity: 0.72;
  cursor: grab;
  touch-action: none;
  user-select: none;
  transition: background var(--duration-short), opacity var(--duration-short), transform var(--duration-short);

  &:hover:not(:disabled) {
    opacity: 1;
    background: var(--md-surface-container-high);
  }

  &:active:not(:disabled) {
    cursor: grabbing;
    transform: scale(0.97);
  }

  &:disabled {
    cursor: wait;
    opacity: 0.36;
  }
}

.track-more {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  opacity: 0;
  transition: opacity var(--duration-short), background var(--duration-short);

  .track-item:hover & { opacity: 0.6; }
  &:hover { opacity: 1 !important; background: var(--md-surface-container-high); }
  .material-symbols-rounded { font-size: 18px; }
}

.dialog-msg {
  font-size: 14px;
  color: var(--md-on-surface-variant);
  line-height: 1.5;
}
</style>
