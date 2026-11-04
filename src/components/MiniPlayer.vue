<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watchEffect } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useI18n } from 'vue-i18n'
import QueuePanel from './QueuePanel.vue'
import ListenTogetherPanel from './ListenTogetherPanel.vue'
import BilibiliCoverImage from './BilibiliCoverImage.vue'
import EditableRangeValue from './ui/EditableRangeValue.vue'
import TrackContextMenu from './TrackContextMenu.vue'
import { createContextMenuItem } from '@/utils/contextMenu'
import { wheelAdjustedVolume } from '@/utils/volume'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { formatTimeMs as formatTime } from '@/utils/timeFormat'
import { useEscapeClose } from '@/composables/useEscapeClose'
import { useToastStore } from '@/stores/toast'
import { closeDesktopLyricsWindow, desktopLyricsOpen, openDesktopLyricsWindow } from '@/modules/desktopLyrics/bridge'

type CoverSnapshot = {
  rect: { left: number; top: number; width: number; height: number }
  borderRadius: string
  src: string
}

const emit = defineEmits<{ expand: [] }>()
const props = withDefaults(defineProps<{
  coverHiddenByFlip?: boolean
  coverFallbackSrc?: string
}>(), {
  coverHiddenByFlip: false,
  coverFallbackSrc: '',
})
const player = usePlayerStore()
const lt = useListenTogetherStore()
const { t } = useI18n()
const toast = useToastStore()

async function toggleDesktopLyrics() {
  try {
    if (desktopLyricsOpen.value) await closeDesktopLyricsWindow()
    else await openDesktopLyricsWindow()
  } catch {
    toast.error(t('player.desktop_lyrics_failed'))
  }
}

const showQueue = ref(false)
const showLtPanel = ref(false)
const showVolumeSlider = ref(false)
const volumeWrapRef = ref<HTMLDivElement>()

// 进度条拖拽
const isDraggingProgress = ref(false)
const dragRatio = ref(0)
const progressTrackRef = ref<HTMLDivElement>()

// 悬浮时间提示
const isHoveringProgress = ref(false)
const hoverRatio = ref(0)
const hoverX = ref(0) // tooltip 的绝对 X 位置（相对于 progress-track）

const displayProgress = computed(() =>
  isDraggingProgress.value ? dragRatio.value : player.interpolatedProgress
)
const progressFillRef = ref<HTMLDivElement>()
const progressThumbRef = ref<HTMLDivElement>()

// 进度逐帧变化，直接写样式：放进模板绑定会让整个迷你播放器每帧重渲染
watchEffect(() => {
  const progress = Math.max(0, Math.min(1, displayProgress.value))
  if (progressFillRef.value) progressFillRef.value.style.transform = `scaleX(${progress})`
  if (progressThumbRef.value) progressThumbRef.value.style.transform = `translateX(${progress * 100}%)`
}, { flush: 'post' })

/** 悬浮位置对应的时间（ms） */
const hoverTimeMs = computed(() => {
  const ratio = isDraggingProgress.value ? dragRatio.value : hoverRatio.value
  return ratio * player.durationMs
})

const hoverTimeFormatted = computed(() => formatTime(hoverTimeMs.value))

/** 是否显示 tooltip（悬浮或拖拽中且有有效时长） */
const showTooltip = computed(() =>
  (isHoveringProgress.value || isDraggingProgress.value) && player.durationMs > 0
)

function onProgressPointerDown(e: PointerEvent) {
  e.stopPropagation()
  isDraggingProgress.value = true
  const el = progressTrackRef.value!
  el.setPointerCapture(e.pointerId)
  updateDragRatio(e)
}

function onProgressPointerMove(e: PointerEvent) {
  updateHoverPosition(e)
  if (!isDraggingProgress.value) return
  updateDragRatio(e)
}

function onProgressPointerUp(e: PointerEvent) {
  if (!isDraggingProgress.value) return
  updateDragRatio(e)
  player.seekTo(dragRatio.value * player.durationMs)
  isDraggingProgress.value = false
  progressTrackRef.value?.releasePointerCapture(e.pointerId)
}

function onProgressMouseEnter() {
  isHoveringProgress.value = true
}

function onProgressMouseLeave() {
  isHoveringProgress.value = false
}

function updateHoverPosition(e: PointerEvent) {
  const el = progressTrackRef.value!
  const rect = el.getBoundingClientRect()
  const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
  hoverRatio.value = ratio
  hoverX.value = e.clientX - rect.left
}

function updateDragRatio(e: PointerEvent) {
  const el = progressTrackRef.value!
  const rect = el.getBoundingClientRect()
  dragRatio.value = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
}

const volumeIcon = computed(() => {
  if (player.volume === 0) return 'volume_off'
  if (player.volume < 0.5) return 'volume_down'
  return 'volume_up'
})

const volumePercent = computed(() => Math.round(player.volume * 100))
function onVolumeWheel(event: WheelEvent) {
  const next = wheelAdjustedVolume(player.volume, event.deltaY)
  if (next !== player.volume) void player.setVolume(next)
}

const trackMenuRef = ref<InstanceType<typeof TrackContextMenu> | null>(null)

function trackMenuExtras() {
  return [createContextMenuItem(t('tray.open_now_playing'), { id: 'open-now-playing', icon: 'open_in_full' })]
}

function openTrackMenu(event: MouseEvent) {
  if (player.currentTrack) trackMenuRef.value?.open(event, player.currentTrack)
}

function handleMiniPlayerPointerDown(e: PointerEvent) {
  if (!showVolumeSlider.value) return
  const target = e.target as Node | null
  if (target && volumeWrapRef.value?.contains(target)) return
  showVolumeSlider.value = false
}

onMounted(() => {
  document.addEventListener('pointerdown', handleMiniPlayerPointerDown, true)
})

onUnmounted(() => {
  document.removeEventListener('pointerdown', handleMiniPlayerPointerDown, true)
})

useEscapeClose(() => showVolumeSlider.value, () => { showVolumeSlider.value = false })

/** 当前播放时间（ms），拖拽时用拖拽位置 */
const currentTimeMs = computed(() =>
  isDraggingProgress.value
    ? dragRatio.value * player.durationMs
    : player.interpolatedProgress * player.durationMs
)

const currentTimeFormatted = computed(() => formatTime(currentTimeMs.value))
const durationFormatted = computed(() => formatTime(player.durationMs))
const miniTrackKey = computed(() => (
  player.currentTrack?.playlistKey || player.currentTrack?.id || 'empty'
))

const displayCoverUrl = computed(() => {
  const queuedTrack = player.queue.find(track => (
    player.currentTrack?.playlistKey
      ? track.playlistKey === player.currentTrack.playlistKey
      : track.id === player.currentTrack?.id
  ))
  return props.coverFallbackSrc
    || getTrackCoverUrl(player.currentTrack)
    || getTrackCoverUrl(queuedTrack)
    || ''
})
const coverRef = ref<HTMLDivElement>()

function getCoverSnapshot(): CoverSnapshot | null {
  const el = coverRef.value
  const src = displayCoverUrl.value
  if (!el || !src) return null
  const rect = el.getBoundingClientRect()
  return {
    rect: {
      left: rect.left,
      top: rect.top,
      width: rect.width,
      height: rect.height,
    },
    borderRadius: getComputedStyle(el).borderRadius,
    src,
  }
}

defineExpose({
  getCoverSnapshot,
})
</script>

<template>
  <div class="mini-player">
    <!-- 顶部进度条（通栏） -->
    <div
      ref="progressTrackRef"
      class="progress-track"
      :class="{ dragging: isDraggingProgress, hovering: isHoveringProgress }"
      @pointerdown="onProgressPointerDown"
      @pointermove="onProgressPointerMove"
      @pointerup="onProgressPointerUp"
      @pointercancel="onProgressPointerUp"
      @mouseenter="onProgressMouseEnter"
      @mouseleave="onProgressMouseLeave"
      @click.stop
    >
      <div ref="progressFillRef" class="progress-fill" />
      <div ref="progressThumbRef" class="progress-thumb-track"><span class="progress-thumb" /></div>
      <!-- 悬浮时间提示 -->
      <div
        v-if="showTooltip"
        class="progress-tooltip"
        :style="{ left: `${hoverX}px` }"
      >{{ hoverTimeFormatted }}</div>
    </div>

    <!-- 三栏主体 -->
    <div class="mp-body">
      <!-- 左：封面 + 歌曲信息 -->
      <div class="mp-left" @click="emit('expand')" @contextmenu="openTrackMenu">
        <div ref="coverRef" class="mp-cover" :class="{ playing: player.isPlaying, 'mp-cover-hidden': props.coverHiddenByFlip }">
          <BilibiliCoverImage
            v-if="displayCoverUrl"
            :src="displayCoverUrl"
            class="mp-cover-img"
          >
            <span class="material-symbols-rounded filled" style="font-size: 20px; opacity: 0.6">music_note</span>
          </BilibiliCoverImage>
          <span
            v-else
            class="material-symbols-rounded filled"
            style="font-size: 20px; opacity: 0.6"
          >music_note</span>
        </div>
        <div class="mp-info">
          <transition name="mp-meta-swap" mode="out-in">
            <div :key="miniTrackKey" class="mp-meta">
              <div class="mp-title">{{ player.currentTrack?.title || t('player.not_playing') }}</div>
              <div class="mp-artist">{{ player.currentTrack?.artist || '' }}</div>
              <!-- 始终占位，避免切歌时时长行出现/消失导致上下跳动 -->
              <div class="mp-time">{{ player.durationMs > 0 ? `${currentTimeFormatted} / ${durationFormatted}` : ' ' }}</div>
            </div>
          </transition>
        </div>
      </div>

      <!-- 中：播放控制 -->
      <div class="mp-center">
        <button
          class="mp-ctrl-btn small"
          :class="{ active: player.shuffleEnabled }"
          :title="t('shortcuts.shuffle')"
          :aria-label="t('shortcuts.shuffle')"
          :aria-pressed="player.shuffleEnabled"
          @click="player.toggleShuffle()"
        >
          <span class="material-symbols-rounded">shuffle</span>
        </button>
        <button class="mp-ctrl-btn" :title="t('tray.previous')" :aria-label="t('tray.previous')" @click="player.previous()">
          <span class="material-symbols-rounded filled">skip_previous</span>
        </button>
        <button
          class="mp-play-btn"
          :title="player.isPlaying ? t('tray.pause') : t('tray.play')"
          :aria-label="player.isPlaying ? t('tray.pause') : t('tray.play')"
          :disabled="player.isLoadingAudio"
          @click="player.togglePlayPause()"
        >
          <transition name="mp-icon">
            <span
              v-if="player.isLoadingAudioSlow"
              class="material-symbols-rounded mp-icon-abs spinning"
              key="loading"
            >progress_activity</span>
            <span
              v-else
              class="material-symbols-rounded mp-icon-abs filled"
              :key="player.isPlaying ? 'p' : 'r'"
            >{{ player.isPlaying ? 'pause' : 'play_arrow' }}</span>
          </transition>
        </button>
        <button class="mp-ctrl-btn" :title="t('tray.next')" :aria-label="t('tray.next')" @click="player.next()">
          <span class="material-symbols-rounded filled">skip_next</span>
        </button>
        <button
          class="mp-ctrl-btn small"
          :class="{ active: player.repeatMode !== 'off' }"
          :title="t('shortcuts.repeat')"
          :aria-label="t('shortcuts.repeat')"
          @click="player.toggleRepeatMode()"
        >
          <span class="material-symbols-rounded">{{ player.repeatMode === 'one' ? 'repeat_one' : 'repeat' }}</span>
        </button>
      </div>

      <!-- 右：桌面歌词 + 音量 + 一起听 + 队列 + 展开 -->
      <div class="mp-right">
        <button
          class="mp-tool-btn"
          :class="{ active: desktopLyricsOpen }"
          :title="t('player.desktop_lyrics')"
          :aria-label="t('player.desktop_lyrics')"
          :aria-pressed="desktopLyricsOpen"
          @click="toggleDesktopLyrics"
        >
          <span class="material-symbols-rounded" :class="{ filled: desktopLyricsOpen }">lyrics</span>
        </button>
        <div ref="volumeWrapRef" class="mp-volume-wrap">
          <button
            class="mp-tool-btn"
            :class="{ active: showVolumeSlider }"
            :title="t('player.volume')"
            :aria-label="t('player.volume')"
            :aria-expanded="showVolumeSlider"
            @click="showVolumeSlider = !showVolumeSlider"
            @wheel.prevent="onVolumeWheel"
          >
            <span class="material-symbols-rounded">{{ volumeIcon }}</span>
          </button>
          <Transition name="mp-popover">
            <div v-if="showVolumeSlider" class="mp-volume-popover">
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                :value="player.volume"
                class="mp-volume-slider"
                :style="{ background: `linear-gradient(to right, var(--md-primary) ${volumePercent}%, var(--md-surface-container-highest) ${volumePercent}%)` }"
                @input="player.setVolume(parseFloat(($event.target as HTMLInputElement).value))"
              />
              <EditableRangeValue
                :model-value="player.volume"
                class="mp-volume-label"
                :min="0"
                :max="1"
                :step="0.01"
                :input-scale="100"
                :input-width="32"
                :display-value="`${volumePercent}%`"
                input-suffix="%"
                :aria-label="t('player.volume')"
                @update:model-value="player.setVolume($event)"
              />
            </div>
          </Transition>
        </div>
        <button
          class="mp-tool-btn"
          :class="{ active: lt.isConnected }"
          :title="t('listen_together.title')"
          :aria-label="t('listen_together.title')"
          @click="showLtPanel = !showLtPanel; if (showLtPanel) showQueue = false"
        >
          <span class="material-symbols-rounded">group</span>
        </button>
        <button
          class="mp-tool-btn"
          :title="t('player.queue')"
          :aria-label="t('player.queue')"
          @click="showQueue = !showQueue; if (showQueue) showLtPanel = false"
        >
          <span class="material-symbols-rounded">queue_music</span>
        </button>
        <button class="mp-tool-btn" :title="t('tray.open_now_playing')" :aria-label="t('tray.open_now_playing')" @click="emit('expand')">
          <span class="material-symbols-rounded">keyboard_arrow_up</span>
        </button>
      </div>
    </div>

    <!-- 队列面板（Teleport 到 body，避免被 .mini-player 的 backdrop-filter 包含块限制） -->
    <Teleport to="body">
      <Transition name="queue-sheet">
        <QueuePanel v-if="showQueue" @close="showQueue = false" />
      </Transition>
    </Teleport>

    <!-- 一起听弹窗（组件内部自带 Teleport） -->
    <ListenTogetherPanel v-model:open="showLtPanel" />
    <TrackContextMenu
      ref="trackMenuRef"
      :playable="false"
      :extra-items="trackMenuExtras"
      @action="id => id === 'open-now-playing' && emit('expand')"
    />
  </div>
</template>

<style scoped lang="scss">
.mini-player {
  position: fixed;
  bottom: 0;
  left: 80px;
  right: 0;
  height: var(--mini-player-height, 76px);
  background: var(--md-surface-container-low);
  z-index: 100;
  backdrop-filter: blur(18px) saturate(1.04);
  transition: transform 320ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease, backdrop-filter 320ms ease;
}

/* 顶部进度条（通栏）：静置时降低强调，悬浮/拖拽时恢复全亮 */
.progress-track {
  height: 5px;
  background: color-mix(in srgb, var(--md-surface-container-highest) 60%, transparent);
  position: relative;
  cursor: pointer;
  touch-action: none;
  transition: background 200ms ease;

  &.hovering,
  &.dragging {
    background: var(--md-surface-container-highest);
  }

  // 扩大点击区域
  &::before {
    content: '';
    position: absolute;
    top: -8px;
    left: 0;
    right: 0;
    bottom: -6px;
  }
}

/* 进度逐帧变化：填充用 scaleX、圆点用 translateX，只走合成层，不触发布局 */
.progress-fill {
  position: absolute;
  inset: 0;
  /* 静置时降低已播放段亮度，避免通栏主题色过于扎眼 */
  background: color-mix(in srgb, var(--md-primary) 60%, transparent);
  transform: scaleX(0);
  transform-origin: left center;
  transition: background 200ms ease;
  pointer-events: none;

  // 交互时需要清晰：恢复全亮
  .progress-track.hovering &,
  .progress-track.dragging & {
    background: var(--md-primary);
  }
}

.progress-thumb-track {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

// 圆点保持实心，避免进度条透过圆点
.progress-thumb {
  position: absolute;
  left: 0;
  top: 50%;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--md-primary);
  box-shadow: 0 0 4px rgba(0,0,0,0.15);
  transform: translate(-50%, -50%);
  transition: transform 150ms var(--ease-standard);

  .progress-track.hovering &,
  .progress-track.dragging & {
    transform: translate(-50%, -50%) scale(1.3);
  }
}

/* 时间提示 tooltip */
.progress-tooltip {
  position: absolute;
  bottom: calc(100% + 8px);
  transform: translateX(-50%);
  background: var(--md-inverse-surface, #313033);
  color: var(--md-inverse-on-surface, #F4EFF4);
  font-size: 11px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  padding: 3px 8px;
  border-radius: 6px;
  white-space: nowrap;
  pointer-events: none;
  box-shadow: 0 2px 8px rgba(0,0,0,0.2);
  animation: tooltip-fade-in 100ms var(--ease-decelerate);
}

@keyframes tooltip-fade-in {
  from { opacity: 0; transform: translateX(-50%) translateY(4px); }
  to { opacity: 1; transform: translateX(-50%) translateY(0); }
}

/* 三栏主体 */
.mp-body {
  display: flex;
  align-items: center;
  padding: 0 16px;
  height: 71px; // 76 - 5px 进度条
  gap: 16px;
  transition: transform 280ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease;
}

/* 左：封面 + 信息 */
.mp-left {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  min-width: 0;
  cursor: pointer;
  padding: 4px;
  border-radius: var(--radius-md);
  transition: background 150ms, transform 260ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease;

  &:hover { background: var(--md-surface-container-high); }
}

.mp-cover {
  width: 48px;
  height: 48px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-variant);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: border-radius 500ms var(--ease-standard), transform 320ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease;
  overflow: hidden;

  &.playing { border-radius: var(--radius-md); }
}

.mp-cover-hidden {
  opacity: 0;
}

.mp-cover-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
}

.mp-info {
  flex: 1;
  min-width: 0;
  position: relative;
  /* 固定三行信息高度，切歌时不上下弹 */
  min-height: 48px;
  transition: opacity 220ms ease;
}

.mp-meta {
  min-width: 0;
  min-height: 48px;
}

.mp-title {
  font-size: 14px;
  font-weight: 600;
  line-height: 1.45;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mp-artist {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  line-height: 1.45;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mp-time {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
  line-height: 1.45;
  font-variant-numeric: tabular-nums;
  margin-top: 1px;
  min-height: 1.45em;
}

/* 中：播放控制 */
.mp-center {
  display: flex;
  align-items: center;
  gap: 4px;
  transition: transform 260ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease;
}

.mp-ctrl-btn {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: color 150ms, background 150ms;

  .material-symbols-rounded { font-size: 24px; }

  &:hover { background: var(--md-surface-variant); color: var(--md-on-surface); }
  &:active { transform: scale(0.9); }
  &.active { color: var(--md-primary); }

  &.small {
    width: 32px;
    height: 32px;
    .material-symbols-rounded { font-size: 20px; }
  }
}

.mp-play-btn {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0 4px;
  transition: transform 150ms, box-shadow 150ms;
  overflow: hidden;
  position: relative;

  .material-symbols-rounded { font-size: 26px; }

  &:hover { transform: scale(1.06); box-shadow: 0 2px 12px rgba(0,0,0,0.2); }
  &:active { transform: scale(0.92); }
  &:disabled { opacity: 0.5; }
}

// 绝对定位图标，新旧同时存在，消除 out-in 空白间隙
.mp-icon-abs {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* 右：工具按钮 */
.mp-right {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
  transition: transform 260ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease;
}

.mp-tool-btn {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: color 150ms, background 150ms;

  .material-symbols-rounded { font-size: 20px; }

  &:hover { background: var(--md-surface-variant); color: var(--md-on-surface); }
  &:active { transform: scale(0.9); }
  &.active { color: var(--md-primary); }
}

/* 音量弹窗 */
.mp-volume-wrap {
  position: relative;
}

.mp-volume-popover {
  position: absolute;
  bottom: 44px;
  left: 50%;
  transform: translateX(-50%);
  background: var(--md-surface-container-high);
  border-radius: 12px;
  box-sizing: border-box;
  width: 56px;
  min-width: 56px;
  max-width: 56px;
  padding: 14px 10px;
  box-shadow: 0 4px 24px rgba(0,0,0,0.25);
  border: 1px solid var(--md-outline-variant);
  z-index: 10;
  display: flex;
  flex-direction: column;
  align-items: center;
}

/* 音量弹层进出场过渡（保留 translateX(-50%) 居中，只叠加纵向位移与缩放） */
.mp-popover-enter-from,
.mp-popover-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(8px) scale(0.96);
}

.mp-popover-enter-active,
.mp-popover-leave-active {
  transition: opacity 140ms var(--ease-decelerate), transform 140ms var(--ease-decelerate);
}

.mp-volume-slider {
  // 竖排滑条不能用 writing-mode（WebKitGTK 圆点错位且拖不动），
  // 用原生横向滑条整体旋转：几何/命中计算保持原生，视觉为竖向。
  // value = volume：rotate(-90deg) 使 min 在下，即静音在下、最大音量在上
  appearance: none;
  width: 100px;
  height: 4px;
  transform: rotate(-90deg);
  margin: 48px 0;
  background: var(--md-surface-container-highest);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--md-primary);
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2);
  }
}

.mp-volume-label {
  width: 38px;
  font-size: 11px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  margin-top: 8px;
  font-variant-numeric: tabular-nums;
  text-align: center;
  white-space: nowrap;
}

.spinning {
  animation: spin 1s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

/* 图标切换动画 */
.mp-icon-enter-active {
  transition: transform 150ms var(--ease-decelerate), opacity 100ms var(--ease-decelerate);
}
.mp-icon-leave-active {
  transition: transform 80ms var(--ease-accelerate), opacity 60ms var(--ease-accelerate);
}
.mp-icon-enter-from { transform: scale(0.5); opacity: 0; }
.mp-icon-leave-to { transform: scale(0.5); opacity: 0; }

.mp-cover-swap-enter-active,
.mp-cover-swap-leave-active {
  transition: opacity 220ms ease, transform 280ms cubic-bezier(0.22, 1, 0.36, 1);
}
.mp-cover-swap-enter-from,
.mp-cover-swap-leave-to {
  opacity: 0;
  transform: scale(0.92);
}

.mp-meta-swap-enter-active,
.mp-meta-swap-leave-active {
  transition: opacity 180ms ease;
}
.mp-meta-swap-enter-from {
  opacity: 0;
}
.mp-meta-swap-leave-to {
  opacity: 0;
}

</style>
