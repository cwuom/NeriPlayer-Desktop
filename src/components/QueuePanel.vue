<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useI18n } from 'vue-i18n'
import { useIncrementalList } from '@/composables/useIncrementalList'
import BilibiliCoverImage from './BilibiliCoverImage.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import { useTrackDownloadMenu } from '@/composables/useTrackDownloadMenu'
import {
  createContextMenuItem,
  createContextMenuSeparator,
  type ContextMenuActionItem,
  type ContextMenuItem,
  type ContextMenuPosition,
} from '@/utils/contextMenu'
import { formatTrackDuration as formatDuration } from '@/utils/timeFormat'

const emit = defineEmits<{ close: [] }>()
const player = usePlayerStore()
const listenTogether = useListenTogetherStore()
const { t } = useI18n()

// ESC 只关本面板并消费事件, 阻止全局快捷键继续关闭下层 (对齐 Android 返回语义)
// 组件仅在面板打开时挂载, 不需要 open 状态判断
function handleEscKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return
  event.preventDefault()
  emit('close')
}
onMounted(() => document.addEventListener('keydown', handleEscKeydown))
onUnmounted(() => {
  document.removeEventListener('keydown', handleEscKeydown)
  cancelQueueDrag()
})
// 大队列分块渲染（队列可达上千行，WebKitGTK 一次性渲染会卡顿）
const showPlaybackOrder = ref(true)
const isPlaybackOrderView = computed(() => player.shuffleEnabled && showPlaybackOrder.value)
let queueEntryCache = new Map<number, { queueIndex: number; track: TrackInfo }>()
const displayQueue = computed(() => {
  const indices = isPlaybackOrderView.value
    ? player.queueDisplayIndices
    : player.queue.map((_, index) => index)
  const entries = new Map<number, { queueIndex: number; track: TrackInfo }>()
  const items = indices.map(queueIndex => {
    const track = player.queue[queueIndex]
    const cached = entries.get(queueIndex) ?? queueEntryCache.get(queueIndex)
    const entry = cached?.track.id === track.id && cached.track.playlistKey === track.playlistKey
      ? cached : { queueIndex, track }
    entry.track = track
    entries.set(queueIndex, entry)
    return entry
  })
  // 保留未变行的引用，元数据刷新才不会重置已展开的分块窗口
  queueEntryCache = entries
  return items
})
const currentDisplayIndex = computed(() => isPlaybackOrderView.value
  ? player.queueDisplayCurrentIndex
  : player.queueIndex)
const { visibleItems: visibleQueue, onScroll: onQueueScroll, expand, ensureIndex } = useIncrementalList(
  () => displayQueue.value,
)
const queueListRef = ref<HTMLElement | null>(null)
const canReorderQueue = computed(() => player.queue.length > 1
  && !listenTogether.localControlRestriction && !isPlaybackOrderView.value)
const queueDragIndex = ref(-1)
const queueDropIndex = ref(-1)
let queuePanelUnmounted = false

async function locateCurrentTrack(behavior: ScrollBehavior = 'smooth') {
  // 列表切换可能先重置分块窗口，等其完成后再扩展目标行
  await nextTick()
  const index = currentDisplayIndex.value
  if (queuePanelUnmounted || !queueListRef.value || !ensureIndex(index)) return
  await nextTick()
  if (queuePanelUnmounted || currentDisplayIndex.value !== index) return
  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  queueListRef.value?.querySelectorAll<HTMLElement>('.queue-item')[index]?.scrollIntoView({
    block: 'center', behavior: reducedMotion ? 'auto' : behavior,
  })
}

onMounted(() => { void locateCurrentTrack('auto') })
onUnmounted(() => { queuePanelUnmounted = true })
// 随机历史变化时显示位置可能不变，仍需跟随新的当前槽位
watch([currentDisplayIndex, () => player.queueIndex, () => player.queue.length, isPlaybackOrderView], () => {
  if (queueDragIndex.value < 0 && !queueContextMenuOpen.value) void locateCurrentTrack()
}, { flush: 'post' })
// The drag offset goes into a CSS variable on the dragged row instead of reactive state:
// pointermove fires hundreds of times per second while dragging, and every reactive write
// re-rendered the whole queue panel (visibly dropping frames with thousands of rows).
// This style object keeps one identity, so a re-render only rebinds the same transform.
const queueDragRowStyle = { transform: 'translateY(var(--queue-drag-offset, 0px))' }
let queueDragRow: HTMLElement | null = null
let queueDragPointerId: number | null = null
let queueDragHandle: HTMLElement | null = null
let queueDragSnapshot: typeof player.queue = []
let queueDragStartY = 0
let queueDragPointerY = 0
let queueDragStartScrollTop = 0
let queueDragFrame: number | null = null

// 队列或房间权限变化后，旧手势不能移动新列表里的同一下标
watch(() => player.queue.slice(), cancelQueueDrag, { flush: 'sync' })
watch(canReorderQueue, allowed => { if (!allowed) cancelQueueDrag() }, { flush: 'sync' })
watch(isPlaybackOrderView, () => { cancelQueueDrag(); closeQueueContextMenu() }, { flush: 'sync' })

function cancelQueueDrag() {
  if (queueDragFrame !== null) cancelAnimationFrame(queueDragFrame)
  queueDragFrame = null
  window.removeEventListener('pointermove', moveQueueDrag)
  window.removeEventListener('pointerup', finishQueueDrag)
  window.removeEventListener('pointercancel', cancelQueueDrag)
  if (queueDragPointerId !== null && queueDragHandle?.hasPointerCapture(queueDragPointerId)) {
    queueDragHandle.releasePointerCapture(queueDragPointerId)
  }
  queueDragPointerId = null
  queueDragHandle = null
  queueDragRow?.style.removeProperty('--queue-drag-offset')
  queueDragRow = null
  queueDragSnapshot = []
  queueDragIndex.value = -1
  queueDropIndex.value = -1
}

function updateQueueDragTarget() {
  const list = queueListRef.value
  if (!list || queueDragIndex.value < 0) return
  const offset = queueDragPointerY - queueDragStartY + list.scrollTop - queueDragStartScrollTop
  queueDragRow?.style.setProperty('--queue-drag-offset', `${offset}px`)
  let targetIndex = 0
  const rows = list.querySelectorAll<HTMLElement>('.queue-item')
  rows.forEach((row, index) => {
    if (index === queueDragIndex.value) return
    const rect = row.getBoundingClientRect()
    if (queueDragPointerY > rect.top + rect.height / 2) targetIndex++
  })
  queueDropIndex.value = targetIndex
}

function tickQueueDrag() {
  const list = queueListRef.value
  if (!list || queueDragPointerId === null) return
  const rect = list.getBoundingClientRect()
  const edge = 48
  const maxStep = 16
  let delta = 0
  if (queueDragPointerY < rect.top + edge) {
    delta = -Math.ceil(maxStep * Math.min(1, (rect.top + edge - queueDragPointerY) / edge))
  } else if (queueDragPointerY > rect.bottom - edge) {
    delta = Math.ceil(maxStep * Math.min(1, (queueDragPointerY - rect.bottom + edge) / edge))
  }
  if (delta !== 0) {
    list.scrollTop += delta
    if (delta > 0 && list.scrollTop + list.clientHeight >= list.scrollHeight - edge) expand()
    updateQueueDragTarget()
  }
  queueDragFrame = requestAnimationFrame(tickQueueDrag)
}

function startQueueDrag(event: PointerEvent, index: number) {
  if (!canReorderQueue.value || !player.queue[index]) return
  if (event.pointerType === 'mouse' && event.button !== 0) return
  event.preventDefault()
  cancelQueueDrag()
  closeQueueContextMenu()
  queueDragSnapshot = [...player.queue]
  queueDragIndex.value = index
  queueDragPointerId = event.pointerId
  queueDragHandle = event.currentTarget as HTMLElement
  queueDragRow = queueDragHandle.closest<HTMLElement>('.queue-item')
  queueDragStartY = queueDragPointerY = event.clientY
  queueDragStartScrollTop = queueListRef.value?.scrollTop ?? 0
  queueDragHandle.setPointerCapture(event.pointerId)
  updateQueueDragTarget()
  window.addEventListener('pointermove', moveQueueDrag)
  window.addEventListener('pointerup', finishQueueDrag)
  window.addEventListener('pointercancel', cancelQueueDrag)
  queueDragFrame = requestAnimationFrame(tickQueueDrag)
}

function moveQueueDrag(event: PointerEvent) {
  if (event.pointerId !== queueDragPointerId) return
  event.preventDefault()
  queueDragPointerY = event.clientY
  updateQueueDragTarget()
}

async function finishQueueDrag(event: PointerEvent) {
  if (event.pointerId !== queueDragPointerId) return
  event.preventDefault()
  queueDragPointerY = event.clientY
  updateQueueDragTarget()
  const fromIndex = queueDragIndex.value
  const toIndex = queueDropIndex.value
  const snapshot = queueDragSnapshot
  cancelQueueDrag()
  if (player.moveQueueItem(fromIndex, toIndex, snapshot)) {
    // 首行换位会重置分块窗口，提交后保留拖动目标的可见范围
    await nextTick()
    ensureIndex(Math.max(fromIndex, toIndex))
  }
}

async function moveQueueByKeyboard(event: KeyboardEvent, index: number) {
  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
  event.preventDefault()
  event.stopPropagation()
  if (!canReorderQueue.value) return
  closeQueueContextMenu()
  const toIndex = index + (event.key === 'ArrowUp' ? -1 : 1)
  if (player.moveQueueItem(index, toIndex)) {
    await nextTick()
    ensureIndex(Math.max(index, toIndex))
    await nextTick()
    queueListRef.value?.querySelectorAll<HTMLElement>('.qi-drag-handle')[toIndex]?.focus()
  }
}

const queueContextMenuOpen = ref(false)
const queueContextMenuPosition = ref<ContextMenuPosition>({ x: 0, y: 0 })
const queueContextMenuIndex = ref(-1)

const { downloadMenuItem, downloadFromMenu } = useTrackDownloadMenu(
  () => player.queue[queueContextMenuIndex.value],
  closeQueueContextMenu,
)

const queueContextMenuItems = computed<readonly ContextMenuItem[]>(() => {
  if (queueContextMenuIndex.value < 0 || !player.queue[queueContextMenuIndex.value]) return []

  return [
    createContextMenuItem(t('player.play_all'), {
      id: 'play',
      icon: 'play_arrow',
    }),
    createContextMenuItem(t('player.play_next'), {
      id: 'play-next',
      icon: 'queue_play_next',
    }),
    createContextMenuItem(t('player.add_to_queue'), {
      id: 'queue-end',
      icon: 'add_to_queue',
    }),
    downloadMenuItem.value,
    createContextMenuSeparator('queue-actions'),
    createContextMenuItem(t('common.delete'), {
      id: 'remove',
      icon: 'delete',
      danger: true,
    }),
  ]
})

function playFromQueue(index: number) {
  void player.playQueueItem(index)
}

function removeFromQueue(index: number) {
  player.removeFromQueue(index)
}

function clearQueue() {
  player.clearQueue()
  player.queueIndex = -1
}

function openQueueContextMenu(event: MouseEvent, index: number) {
  const target = event.currentTarget instanceof HTMLElement ? event.currentTarget : null
  const rect = target?.getBoundingClientRect()
  const isMoreButton = event.type === 'click' && rect

  queueContextMenuPosition.value = isMoreButton
    ? { x: rect.right, y: rect.bottom }
    : { x: event.clientX, y: event.clientY }
  queueContextMenuIndex.value = index
  queueContextMenuOpen.value = true
}

function closeQueueContextMenu() {
  queueContextMenuOpen.value = false
  queueContextMenuIndex.value = -1
}

function handleQueueContextMenuClick(item: ContextMenuActionItem) {
  const index = queueContextMenuIndex.value
  const track = player.queue[index]
  if (!track) return

  switch (item.id) {
    case 'play':
      void player.playQueueItem(index)
      break
    case 'play-next':
      player.addToQueueNext(track)
      break
    case 'queue-end':
      player.addToQueueEnd(track)
      break
    case 'download':
      void downloadFromMenu()
      break
    case 'remove':
      removeFromQueue(index)
      break
  }
}
</script>

<template>
  <div class="queue-overlay" @click.self="emit('close')">
    <div class="queue-panel">
      <header class="queue-header">
        <h3>{{ t('player.queue') }}</h3>
        <span class="queue-count">{{ player.queue.length }}</span>
        <div style="flex: 1" />
        <button
          class="queue-locate"
          :disabled="currentDisplayIndex < 0 || currentDisplayIndex >= displayQueue.length"
          :title="t('player.locate_current')"
          :aria-label="t('player.locate_current')"
          @click="locateCurrentTrack()"
        >
          <span class="material-symbols-rounded">my_location</span>
        </button>
        <button v-if="player.queue.length > 0" class="queue-clear" @click="clearQueue">
          {{ t('player.clear_queue') }}
        </button>
        <button class="queue-close" @click="emit('close')">
          <span class="material-symbols-rounded">close</span>
        </button>
      </header>

      <div v-if="player.shuffleEnabled && player.queue.length > 0" class="queue-order-toolbar">
        <span>{{ t(isPlaybackOrderView ? 'player.queue_playback_order' : 'player.queue_original_order') }}</span>
        <button
          class="queue-order-toggle"
          :aria-pressed="isPlaybackOrderView"
          @click="showPlaybackOrder = !showPlaybackOrder"
        >
          <span class="material-symbols-rounded">{{ isPlaybackOrderView ? 'format_list_numbered' : 'shuffle' }}</span>
          {{ t(isPlaybackOrderView ? 'player.queue_show_original' : 'player.queue_show_playback') }}
        </button>
      </div>

      <div v-if="player.queue.length === 0" class="queue-empty">
        <span class="material-symbols-rounded" style="font-size: 36px; opacity: 0.2">queue_music</span>
        <p>{{ t('player.no_queue') }}</p>
      </div>

      <div v-else ref="queueListRef" class="queue-list" @scroll="onQueueScroll">
        <div
          v-for="(entry, index) in visibleQueue"
          :key="`${index}:${entry.queueIndex}:${entry.track.id}`"
          class="queue-item"
          :class="{
            active: index === currentDisplayIndex,
            dragging: index === queueDragIndex,
            'drop-before': queueDragIndex >= 0 && queueDropIndex < queueDragIndex && index === queueDropIndex,
            'drop-after': queueDragIndex >= 0 && queueDropIndex > queueDragIndex && index === queueDropIndex,
          }"
          :style="index === queueDragIndex ? queueDragRowStyle : undefined"
          @click="playFromQueue(entry.queueIndex)"
          @contextmenu.prevent.stop="openQueueContextMenu($event, entry.queueIndex)"
        >
          <div class="qi-index">
            <div v-if="index === currentDisplayIndex && player.isPlaying" class="equalizer-bars"><span class="bar"/><span class="bar"/><span class="bar"/></div>
            <span v-else class="qi-num">{{ index + 1 }}</span>
          </div>
          <div class="qi-cover">
            <BilibiliCoverImage v-if="entry.track.coverUrl" :src="entry.track.coverUrl" loading="lazy">
              <span class="material-symbols-rounded filled">music_note</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled">music_note</span>
          </div>
          <div class="qi-info">
            <div class="qi-title">{{ entry.track.title }}</div>
            <div class="qi-meta">{{ entry.track.artist }}</div>
          </div>
          <div class="qi-duration">{{ formatDuration(entry.track.durationMs) }}</div>
          <button
            class="qi-drag-handle"
            :disabled="!canReorderQueue"
            :title="t(isPlaybackOrderView ? 'player.queue_reorder_original' : 'common.drag_to_reorder')"
            :aria-label="t('common.drag_to_reorder')"
            @pointerdown.stop="startQueueDrag($event, index)"
            @keydown="moveQueueByKeyboard($event, index)"
            @click.stop
          >
            <span class="material-symbols-rounded">drag_handle</span>
          </button>
          <button class="qi-more" :title="t('player.more_options')" @click.stop="openQueueContextMenu($event, entry.queueIndex)">
            <span class="material-symbols-rounded">more_vert</span>
          </button>
          <button class="qi-remove" @click.stop="removeFromQueue(entry.queueIndex)">
            <span class="material-symbols-rounded">close</span>
          </button>
        </div>
      </div>
    </div>

    <ContextMenu
      v-model:open="queueContextMenuOpen"
      :x="queueContextMenuPosition.x"
      :y="queueContextMenuPosition.y"
      :items="queueContextMenuItems"
      @click="handleQueueContextMenuClick"
      @close="closeQueueContextMenu"
    />
  </div>
</template>

<style scoped lang="scss">
.queue-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  z-index: 250;
  display: flex;
  justify-content: flex-end;
}

.queue-panel {
  width: 380px;
  max-width: 90vw;
  height: 100%;
  background: var(--md-surface-container);
  display: flex;
  flex-direction: column;
  box-shadow: -4px 0 24px rgba(0, 0, 0, 0.3);
}

/* 由父级 <Transition name="queue-sheet"> 驱动：开合都是 transition，半路反向会从当前位置收回 */
.queue-overlay.queue-sheet-enter-active,
.queue-overlay.queue-sheet-leave-active {
  transition: background-color 260ms var(--ease-standard);
  .queue-panel { transition: transform 300ms var(--ease-emphasized-decel); }
}
.queue-overlay.queue-sheet-leave-active .queue-panel {
  transition-duration: 200ms;
  transition-timing-function: var(--ease-emphasized-accel);
}
.queue-overlay.queue-sheet-enter-from,
.queue-overlay.queue-sheet-leave-to {
  background-color: transparent;
  .queue-panel { transform: translateX(100%); }
}

.queue-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px 20px;
  border-bottom: 1px solid var(--md-outline-variant);
  flex-shrink: 0;

  h3 {
    font-size: 18px;
    font-weight: 600;
  }
}

.queue-count {
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-primary);
  background: var(--md-primary);
  padding: 2px 8px;
  border-radius: var(--radius-full);
  min-width: 24px;
  text-align: center;
}

.queue-clear {
  font-size: 12px;
  font-weight: 500;
  color: var(--md-error, #FFB4AB);
  padding: 6px 12px;
  border-radius: var(--radius-full);
  transition: background var(--duration-short);

  &:hover { background: color-mix(in srgb, var(--md-error, #FFB4AB) 10%, transparent); }
}

.queue-close,
.queue-locate {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
  &:disabled { opacity: 0.3; cursor: default; }
  &:focus-visible { outline: 2px solid var(--md-primary); }
}

.queue-order-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 20px;
  border-bottom: 1px solid var(--md-outline-variant);
  flex-shrink: 0;
  font-size: 12px;
  color: var(--md-on-surface-variant);
}

.queue-order-toggle {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 8px;
  border-radius: var(--radius-full);
  color: var(--md-primary);

  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: var(--md-surface-container-high); }
  &:focus-visible { outline: 2px solid var(--md-primary); }
}

.queue-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  gap: 10px;
  color: var(--md-on-surface-variant);
  font-size: 14px;
}

.queue-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}

.queue-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
  &.active { background: color-mix(in srgb, var(--md-primary) 10%, transparent); }
  &.active .qi-title { color: var(--md-primary); }
  &.dragging {
    z-index: 1;
    background: var(--md-surface-container-highest);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
    cursor: grabbing;
    user-select: none;
  }
  &.drop-before::before,
  &.drop-after::after {
    content: '';
    position: absolute;
    left: 10px;
    right: 10px;
    height: 2px;
    background: var(--md-primary);
    border-radius: var(--radius-full);
  }
  &.drop-before::before { top: -1px; }
  &.drop-after::after { bottom: -1px; }
}

.qi-index {
  width: 24px;
  text-align: center;
  flex-shrink: 0;
}

.qi-num {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  font-weight: 500;
}

.qi-playing {
  font-size: 18px;
  color: var(--md-primary);
}

.equalizer-bars {
  display: flex;
  align-items: flex-end;
  justify-content: center;
  gap: 2px;
  width: 16px;
  height: 16px;

  .bar {
    width: 3px;
    border-radius: 1.5px;
    background: var(--md-primary);
    animation: eq-bounce 0.8s ease-in-out infinite alternate;

    &:nth-child(1) { height: 30%; animation-delay: 0s; }
    &:nth-child(2) { height: 60%; animation-delay: 0.2s; }
    &:nth-child(3) { height: 45%; animation-delay: 0.4s; }
  }
}

@keyframes eq-bounce {
  0%   { height: 20%; }
  50%  { height: 90%; }
  100% { height: 30%; }
}

.qi-cover {
  width: 38px;
  height: 38px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-variant);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  overflow: hidden;

  img { width: 100%; height: 100%; object-fit: cover; }
  .material-symbols-rounded { font-size: 20px; opacity: 0.5; }
}

.qi-info {
  flex: 1;
  min-width: 0;
}

.qi-title {
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: color var(--duration-short);
}

.qi-meta {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.qi-duration {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.qi-drag-handle,
.qi-more,
.qi-remove {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  opacity: 0;
  transition: opacity var(--duration-short), background var(--duration-short);

  .queue-item:hover & { opacity: 0.6; }
  &:hover { opacity: 1 !important; background: var(--md-surface-container-highest); }
  .material-symbols-rounded { font-size: 16px; }
}

.qi-drag-handle {
  opacity: 0.6;
  cursor: grab;
  touch-action: none;

  &:active { cursor: grabbing; }
  &:disabled { opacity: 0.2; cursor: default; }
  &:focus-visible { opacity: 1; outline: 2px solid var(--md-primary); }
}
</style>
