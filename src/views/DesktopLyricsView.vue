<script setup lang="ts">
// 桌面歌词窗口：按主窗口发来的锚点本地插值播放进度；悬停出工具栏，锁定后点击穿透、只留解锁按钮
// 播放、字号、布局、锁定等操作都发给主窗口执行，这里不碰播放状态和设置
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import DesktopLyricsStage from '@/components/desktopLyrics/DesktopLyricsStage.vue'
import { emptyDesktopLyricsFrame, type DesktopLyricsFrame } from '@/modules/desktopLyrics/frame'
import { normalizeDesktopLyricsStyle } from '@/modules/desktopLyrics/style'
import { anchoredPositionMs } from '@/modules/desktopLyrics/timeline'

const { t } = useI18n()
const frame = ref<DesktopLyricsFrame>(emptyDesktopLyricsFrame())
const pointerInside = ref(false)
/** 锁定时 DOM 收不到鼠标事件，悬停状态由后端按光标位置通知 */
const lockedHover = ref(false)
const lockedByEvent = ref<boolean | null>(null)
const unlockButton = ref<HTMLElement | null>(null)
const releases: UnlistenFn[] = []
let disposed = false
let eventVersion = 0
let resizeObserver: ResizeObserver | null = null

const style = computed(() => frame.value.style)
const locked = computed(() => lockedByEvent.value ?? style.value.locked)
const hovered = computed(() => (locked.value ? lockedHover.value : pointerInside.value))
const showToolbar = computed(() => hovered.value && !locked.value)
const showPanel = computed(() => style.value.background === 'always' || (style.value.background === 'hover' && showToolbar.value))
const hasLyrics = computed(() => frame.value.lines.length > 0)
/** 前奏或没有歌词时显示「歌名 - 歌手」 */
const placeholder = computed(() => {
  if (!frame.value.trackId) return 'NeriPlayer'
  return [frame.value.title, frame.value.artist].filter(Boolean).join(' - ') || t('player.no_lyrics')
})
const panelStyle = computed(() => ({ '--dl-background': `rgba(16, 15, 24, ${style.value.backgroundOpacity})` }))

function clock(): number {
  const current = frame.value
  return anchoredPositionMs(current, Date.now()) + current.offsetMs
}

function accept(payload: DesktopLyricsFrame) {
  frame.value = {
    ...emptyDesktopLyricsFrame(),
    ...payload,
    lines: Array.isArray(payload.lines) ? payload.lines : [],
    style: normalizeDesktopLyricsStyle(payload.style),
  }
  if (lockedByEvent.value !== null && lockedByEvent.value === frame.value.style.locked) lockedByEvent.value = null
}

function act(action: string) {
  if (!isTauri()) return
  void invoke('desktop_lyrics_action', { action }).catch(() => {})
}

async function close() {
  if (!isTauri()) return
  await getCurrentWindow().close()
}

function drag(event: PointerEvent) {
  if (!isTauri() || locked.value) return
  if (event.button !== 0 || (event.target as HTMLElement).closest('button, .dl-resize')) return
  void getCurrentWindow().startDragging().catch(() => {})
}

type ResizeDirection = 'North' | 'South' | 'East' | 'West' | 'NorthEast' | 'NorthWest' | 'SouthEast' | 'SouthWest'
const RESIZE_EDGES: Array<{ id: string, direction: ResizeDirection }> = [
  { id: 'n', direction: 'North' }, { id: 's', direction: 'South' },
  { id: 'e', direction: 'East' }, { id: 'w', direction: 'West' },
  { id: 'ne', direction: 'NorthEast' }, { id: 'nw', direction: 'NorthWest' },
  { id: 'se', direction: 'SouthEast' }, { id: 'sw', direction: 'SouthWest' },
]

function resize(event: PointerEvent, direction: ResizeDirection) {
  if (!isTauri() || event.button !== 0) return
  event.stopPropagation()
  void getCurrentWindow().startResizeDragging(direction).catch(() => {})
}

/** 锁定时只有解锁按钮这一块接收点击，把它的位置告诉后端 */
function reportHitRegion() {
  if (!isTauri()) return
  const button = unlockButton.value
  const rect = locked.value && button ? button.getBoundingClientRect() : null
  const region = rect ? { x: rect.left - 4, y: rect.top - 4, width: rect.width + 8, height: rect.height + 8 } : null
  void invoke('desktop_lyrics_hit_region', { region }).catch(() => {})
}

function keydown(event: KeyboardEvent) {
  if (event.key === 'Escape') void close().catch(() => {})
}

watch(locked, () => {
  lockedHover.value = false
  void nextTick(reportHitRegion)
})

onMounted(async () => {
  document.documentElement.classList.add('desktop-lyrics-page')
  window.addEventListener('keydown', keydown)
  resizeObserver = new ResizeObserver(() => reportHitRegion())
  resizeObserver.observe(document.documentElement)
  try {
    const subscriptions = await Promise.all([
      listen<DesktopLyricsFrame>('desktop-lyrics:frame', event => {
        if (disposed) return
        eventVersion++
        accept(event.payload)
      }),
      listen<{ inside: boolean }>('desktop-lyrics:hover', event => {
        if (!disposed) lockedHover.value = event.payload.inside === true
      }),
      listen<{ locked: boolean }>('desktop-lyrics:lock', event => {
        if (!disposed) lockedByEvent.value = event.payload.locked === true
      }),
    ])
    if (disposed) { subscriptions.forEach(release => release()); return }
    releases.push(...subscriptions)
    const version = eventVersion
    const snapshot = await invoke<DesktopLyricsFrame>('get_desktop_lyrics_snapshot')
    if (!disposed && version === eventVersion) accept(snapshot)
  } catch {
    // 只读显示窗的连接失败保持空态，不初始化任何播放状态
  } finally {
    if (!disposed && isTauri()) void getCurrentWindow().show().catch(() => {})
    void nextTick(reportHitRegion)
  }
})

onUnmounted(() => {
  disposed = true
  releases.forEach(release => release())
  resizeObserver?.disconnect()
  window.removeEventListener('keydown', keydown)
  document.documentElement.classList.remove('desktop-lyrics-page')
})
</script>

<template>
  <main
    class="desktop-lyrics"
    :class="{ panel: showPanel, hovered, locked }"
    :style="panelStyle"
    @pointerdown="drag"
    @pointerenter="pointerInside = true"
    @pointerleave="pointerInside = false"
  >
    <header class="dl-toolbar" :class="{ visible: showToolbar }">
      <div class="dl-track" :title="[frame.title, frame.artist].filter(Boolean).join(' · ')">
        <template v-if="style.showTrackInfo && frame.title">
          <span class="material-symbols-rounded">{{ frame.isPlaying ? 'graphic_eq' : 'music_note' }}</span>
          <span class="dl-track-text">{{ frame.title }}<template v-if="frame.artist"> · {{ frame.artist }}</template></span>
        </template>
      </div>
      <div class="dl-controls">
        <button type="button" :title="t('desktop_lyrics.previous')" @click="act('previous')"><span class="material-symbols-rounded">skip_previous</span></button>
        <button type="button" class="dl-play" :title="t(frame.isPlaying ? 'desktop_lyrics.pause' : 'desktop_lyrics.play')" @click="act('toggle-play')">
          <span class="material-symbols-rounded filled">{{ frame.isPlaying ? 'pause' : 'play_arrow' }}</span>
        </button>
        <button type="button" :title="t('desktop_lyrics.next')" @click="act('next')"><span class="material-symbols-rounded">skip_next</span></button>
        <span class="dl-divider" />
        <button type="button" :title="t('desktop_lyrics.font_smaller')" @click="act('font-smaller')"><span class="material-symbols-rounded">text_decrease</span></button>
        <button type="button" :title="t('desktop_lyrics.font_larger')" @click="act('font-larger')"><span class="material-symbols-rounded">text_increase</span></button>
        <button type="button" :title="t('desktop_lyrics.cycle_layout')" @click="act('cycle-layout')"><span class="material-symbols-rounded">view_agenda</span></button>
        <button type="button" :title="t('desktop_lyrics.lock')" @click="act('lock')"><span class="material-symbols-rounded">lock</span></button>
        <button type="button" :title="t('desktop_lyrics.settings')" @click="act('open-settings')"><span class="material-symbols-rounded">tune</span></button>
        <button type="button" :title="t('common.close')" :aria-label="t('common.close')" @click="close"><span class="material-symbols-rounded">close</span></button>
      </div>
    </header>

    <section class="dl-body" aria-live="off">
      <DesktopLyricsStage
        :lines="frame.lines"
        :first-index="frame.firstIndex"
        :clock="clock"
        :playing="frame.isPlaying"
        :clock-key="`${frame.positionMs}|${frame.offsetMs}`"
        :style="style"
        :accent="frame.accent || null"
        :placeholder="placeholder"
      />
    </section>

    <button
      v-if="locked"
      ref="unlockButton"
      type="button"
      class="dl-unlock"
      :class="{ visible: lockedHover }"
      :title="t('desktop_lyrics.unlock')"
      @click="act('unlock')"
    >
      <span class="material-symbols-rounded">lock_open</span>
    </button>

    <template v-if="showToolbar">
      <div
        v-for="edge in RESIZE_EDGES"
        :key="edge.id"
        class="dl-resize"
        :class="`dl-resize-${edge.id}`"
        @pointerdown="resize($event, edge.direction)"
      />
    </template>
  </main>
</template>

<style scoped>
:global(html.desktop-lyrics-page),
:global(html.desktop-lyrics-page body),
:global(html.desktop-lyrics-page #app) {
  background: transparent !important;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

.desktop-lyrics {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  padding: 6px 16px 10px;
  border: 1px solid transparent;
  border-radius: 18px;
  color: #fff;
  box-sizing: border-box;
  user-select: none;
  cursor: grab;
  transition: background-color 0.2s ease, border-color 0.2s ease;
}

.desktop-lyrics.panel {
  background: var(--dl-background, rgba(16, 15, 24, 0.55));
  border-color: rgb(255 255 255 / 14%);
}

.desktop-lyrics.locked { cursor: default; }

.dl-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex: 0 0 32px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.18s ease;
}

.dl-toolbar.visible { opacity: 1; pointer-events: auto; }

.dl-track {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  color: rgb(255 255 255 / 70%);
  font-size: 12px;
  text-shadow: 0 1px 4px rgb(0 0 0 / 70%);
}

.dl-track .material-symbols-rounded { font-size: 16px; }
.dl-track-text { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }

.dl-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  padding: 2px;
  border-radius: 999px;
  background: rgb(0 0 0 / 35%);
}

.dl-controls button,
.dl-unlock {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: rgb(255 255 255 / 82%);
  cursor: pointer;
}

.dl-controls .material-symbols-rounded,
.dl-unlock .material-symbols-rounded { font-size: 18px; }
.dl-controls .dl-play .material-symbols-rounded { font-size: 22px; }
.dl-controls button:hover,
.dl-controls button:focus-visible { background: rgb(255 255 255 / 16%); color: #fff; }
.dl-controls button:focus-visible { outline: 2px solid #d0bcff; outline-offset: 1px; }
.dl-divider { width: 1px; height: 16px; margin: 0 4px; background: rgb(255 255 255 / 22%); }

.dl-body {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
}

.dl-unlock {
  position: absolute;
  top: 8px;
  right: 12px;
  background: rgb(0 0 0 / 50%);
  opacity: 0;
  transition: opacity 0.18s ease;
}

.dl-unlock.visible { opacity: 1; }

.dl-resize { position: absolute; z-index: 2; }
.dl-resize-n, .dl-resize-s { left: 12px; right: 12px; height: 6px; cursor: ns-resize; }
.dl-resize-n { top: 0; }
.dl-resize-s { bottom: 0; }
.dl-resize-e, .dl-resize-w { top: 12px; bottom: 12px; width: 6px; cursor: ew-resize; }
.dl-resize-e { right: 0; }
.dl-resize-w { left: 0; }
.dl-resize-ne, .dl-resize-nw, .dl-resize-se, .dl-resize-sw { width: 12px; height: 12px; }
.dl-resize-ne { top: 0; right: 0; cursor: nesw-resize; }
.dl-resize-sw { bottom: 0; left: 0; cursor: nesw-resize; }
.dl-resize-nw { top: 0; left: 0; cursor: nwse-resize; }
.dl-resize-se { bottom: 0; right: 0; cursor: nwse-resize; }
</style>
