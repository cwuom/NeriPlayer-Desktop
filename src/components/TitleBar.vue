<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { isMacPlatform } from '@/modules/shortcuts/platform'

const props = defineProps<{
  forceLight?: boolean
  nowPlaying?: boolean
  hasPlaybackSession?: boolean
  trackName?: string
  albumName?: string
  transitioning?: boolean
  transitionState?: 'opening' | 'closing' | null
}>()

const emit = defineEmits<{
  collapse: []
  toggleMore: []
}>()

const { t } = useI18n()
const titleBarTrackText = computed(() => (
  props.hasPlaybackSession ? props.albumName || props.trackName || '' : ''
))
const titleBarLabel = computed(() => (
  props.hasPlaybackSession ? t('player.now_playing') : t('player.not_playing')
))
const titleBarTrackKey = computed(() => `${props.nowPlaying ? 'np' : 'base'}:${titleBarTrackText.value || 'empty'}`)

const isMaximized = ref(false)
const isFullscreen = ref(false)
let unlistenResize: UnlistenFn | null = null
let disposed = false
let windowStateVersion = 0

// macOS 使用系统原生红绿灯，隐藏自绘控制并在左侧留出安全区
const isMac = isMacPlatform
/** 拖动改变窗口大小时 resize 事件一秒几十次，窗口状态等停下来再查 */
const MAXIMIZED_REFRESH_DELAY_MS = 120
let maximizedRefreshTimer: ReturnType<typeof setTimeout> | null = null

const appWindow = getCurrentWindow()

async function refreshMaximized() {
  const version = ++windowStateVersion
  const [maximized, fullscreen] = await Promise.all([
    appWindow.isMaximized().catch(() => false),
    isMac ? appWindow.isFullscreen().catch(() => false) : Promise.resolve(false),
  ])
  if (disposed || version !== windowStateVersion) return
  isMaximized.value = maximized
  isFullscreen.value = fullscreen
  if (isMac) document.documentElement.classList.toggle('window-fullscreen', fullscreen)
}

function minimize() {
  appWindow.minimize().catch(() => {})
}

function toggleMaximize() {
  appWindow.toggleMaximize().then(refreshMaximized).catch(() => {})
}

function close() {
  appWindow.close().catch(() => {})
}

function scheduleMaximizedRefresh() {
  if (disposed) return
  if (maximizedRefreshTimer) clearTimeout(maximizedRefreshTimer)
  maximizedRefreshTimer = setTimeout(() => {
    maximizedRefreshTimer = null
    void refreshMaximized()
  }, MAXIMIZED_REFRESH_DELAY_MS)
}

onMounted(async () => {
  try {
    const release = await appWindow.onResized(scheduleMaximizedRefresh)
    if (disposed) { release(); return }
    unlistenResize = release
  } catch {}
  if (!disposed) await refreshMaximized()
})

onUnmounted(() => {
  disposed = true
  windowStateVersion++
  if (unlistenResize) unlistenResize()
  if (maximizedRefreshTimer) clearTimeout(maximizedRefreshTimer)
  if (isMac) document.documentElement.classList.remove('window-fullscreen')
})
</script>

<template>
  <header
    class="title-bar"
    :class="{
      'tb-force-light': forceLight,
      'tb-np-mode': nowPlaying,
      'tb-transitioning': transitioning,
      'tb-opening': transitionState === 'opening',
      'tb-closing': transitionState === 'closing',
      'tb-mac': isMac,
      'tb-fullscreen': isFullscreen,
    }"
    data-tauri-drag-region
  >
    <!-- macOS 原生红绿灯安全区（92px，红绿灯 x=20 起），避免内容压住系统交通灯 -->
    <div v-if="isMac && !isFullscreen" class="tb-traffic-safe-area" data-tauri-drag-region></div>

    <div class="tb-left-slot">
      <!-- 同槽绝对叠层：宽度固定，避免品牌/箭头切换时整栏右移 -->
      <div class="tb-left-stack" data-tauri-drag-region>
        <transition name="tb-brand-fade">
          <div v-if="!nowPlaying" key="brand" class="tb-brand" data-tauri-drag-region>
            <img src="/app-icon.png" alt="logo" class="tb-icon" />
            <span class="tb-title">NeriPlayer</span>
          </div>
        </transition>
        <transition name="tb-arrow-fade">
          <div v-if="nowPlaying" key="np-left" class="tb-np-left">
            <button
              class="tb-np-btn tb-np-collapse"
              type="button"
              data-tauri-drag-region="false"
              :title="t('common.back')"
              :aria-label="t('common.back')"
              @click="emit('collapse')"
            >
              <span class="material-symbols-rounded">keyboard_arrow_down</span>
            </button>
          </div>
        </transition>
      </div>
    </div>

    <!-- 播放器模式：居中播放信息（等品牌淡完再出现） -->
    <transition name="tb-center-fade">
      <div v-if="nowPlaying" class="tb-np-center" data-tauri-drag-region>
        <span class="tb-np-label">{{ titleBarLabel }}</span>
        <transition name="tb-track-swap" mode="out-in">
          <span :key="titleBarTrackKey" class="tb-np-track">{{ titleBarTrackText }}</span>
        </transition>
      </div>
    </transition>

    <!-- 拖拽占位 -->
    <div class="tb-drag" data-tauri-drag-region></div>
    <div v-if="isMac && !isFullscreen" class="tb-drag" data-tauri-drag-region></div>

    <div class="tb-right-slot">
      <!-- 固定宽度槽，避免 more 显隐时右侧跳动 -->
      <div class="tb-right-stack">
        <transition name="tb-arrow-fade">
          <button
            v-if="nowPlaying && hasPlaybackSession"
            key="more"
            class="tb-np-btn tb-np-more"
            type="button"
            data-tauri-drag-region="false"
            :title="t('player.more_options')"
            :aria-label="t('player.more_options')"
            @click="emit('toggleMore')"
          >
            <span class="material-symbols-rounded">more_vert</span>
          </button>
        </transition>
      </div>
    </div>

    <!-- 窗口控制（macOS 使用系统原生红绿灯，此处隐藏） -->
    <div v-if="!isMac" class="tb-controls">
      <button class="tb-ctrl" type="button" @click="minimize" :title="t('common.minimize')" :aria-label="t('common.minimize')">
        <svg viewBox="0 0 12 12" aria-hidden="true">
          <path d="M2.5 6h7" />
        </svg>
      </button>
      <button
        class="tb-ctrl"
        type="button"
        @click="toggleMaximize"
        :title="isMaximized ? t('common.restore') : t('common.maximize')"
        :aria-label="isMaximized ? t('common.restore') : t('common.maximize')"
      >
        <Transition name="tb-ctrl-swap" mode="out-in">
          <svg v-if="!isMaximized" key="max" viewBox="0 0 12 12" aria-hidden="true">
            <rect x="2.5" y="2.5" width="7" height="7" rx="1.6" />
          </svg>
          <!-- 还原：后窗只画露出的两条边，不靠填色遮挡（自定义背景下底色是透明的） -->
          <svg v-else key="restore" viewBox="0 0 12 12" aria-hidden="true">
            <path d="M4.5 3V2.9A1.4 1.4 0 0 1 5.9 1.5h3.2A1.4 1.4 0 0 1 10.5 2.9v3.2A1.4 1.4 0 0 1 9.1 7.5H9" />
            <rect x="1.5" y="4" width="6.5" height="6.5" rx="1.4" />
          </svg>
        </Transition>
      </button>
      <button class="tb-ctrl tb-close" type="button" @click="close" :title="t('common.close')" :aria-label="t('common.close')">
        <svg viewBox="0 0 12 12" aria-hidden="true">
          <path d="M3 3l6 6M9 3L3 9" />
        </svg>
      </button>
    </div>
  </header>
</template>

<style scoped lang="scss">
.title-bar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  /* 高度走全局变量：Windows/Linux 36px，macOS 52px（红绿灯垂直居中），
     开关 NP 不改高度避免位移 */
  height: var(--titlebar-height, 36px);
  display: flex;
  align-items: stretch;
  background: transparent;
  color: var(--md-on-surface);
  user-select: none;
  z-index: 1000;
  pointer-events: none;
  transition: color 280ms var(--ease-standard);

  &.tb-np-mode {
    /* 与普通模式同高，只做透明度交叉 */
    height: var(--titlebar-height, 36px);
    padding-top: 0;
  }
}

.title-bar.tb-transitioning {
  background: transparent;
  backdrop-filter: none;
}

.title-bar.tb-opening,
.title-bar.tb-closing {
  animation: none;
}

.title-bar.tb-force-light {
  color: rgba(255, 255, 255, 0.9);
}

.tb-transitioning .tb-np-center {
  animation: none;
}

.tb-transitioning .tb-np-btn,
.tb-transitioning .tb-ctrl {
  transition:
    background var(--duration-short) var(--ease-standard),
    opacity var(--duration-short) var(--ease-standard);
  transform: none;
}

.tb-opening .tb-np-btn,
.tb-opening .tb-ctrl,
.tb-closing .tb-np-btn,
.tb-closing .tb-ctrl {
  transform: none;
}

/* 品牌：慢淡出（约半秒），不位移 */
.tb-brand-fade-enter-active {
  transition: opacity 320ms cubic-bezier(0.2, 0, 0, 1);
  transition-delay: 180ms;
}
.tb-brand-fade-leave-active {
  transition: opacity 520ms cubic-bezier(0.2, 0, 0, 1);
}
.tb-brand-fade-enter-from,
.tb-brand-fade-leave-to {
  opacity: 0;
}

/* 箭头/更多：等品牌淡完大半再出现 */
.tb-arrow-fade-enter-active {
  transition: opacity 300ms cubic-bezier(0.2, 0, 0, 1);
  transition-delay: 360ms;
}
.tb-arrow-fade-leave-active {
  transition: opacity 240ms cubic-bezier(0.2, 0, 0, 1);
}
.tb-arrow-fade-enter-from,
.tb-arrow-fade-leave-to {
  opacity: 0;
}

/* 中间信息：等左侧淡完再出 */
.tb-center-fade-enter-active {
  transition: opacity 320ms cubic-bezier(0.2, 0, 0, 1);
  transition-delay: 380ms;
}
.tb-center-fade-leave-active {
  transition: opacity 240ms cubic-bezier(0.2, 0, 0, 1);
}
.tb-center-fade-enter-from,
.tb-center-fade-leave-to {
  opacity: 0;
}

.tb-track-swap-enter-active,
.tb-track-swap-leave-active {
  transition: opacity 220ms cubic-bezier(0.2, 0, 0, 1);
}
.tb-track-swap-enter-from,
.tb-track-swap-leave-to {
  opacity: 0;
}

.tb-brand,
.tb-controls,
.tb-ctrl,
.tb-left-slot,
.tb-np-left,
.tb-np-btn,
.tb-np-more,
.tb-right-slot {
  pointer-events: auto;
}

/* 左右槽固定宽度，切换内容绝对叠层 -> 整栏不左右跳 */
.tb-left-slot {
  display: flex;
  align-items: center;
  flex: 0 0 auto;
  width: 128px;
  min-width: 128px;
  position: relative;
}

.tb-right-slot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex: 0 0 auto;
  width: 40px;
  min-width: 40px;
  position: relative;
}

.tb-left-stack,
.tb-right-stack {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: var(--titlebar-height, 36px);
}

.tb-left-stack > .tb-brand,
.tb-left-stack > .tb-np-left {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
}

.tb-right-stack > .tb-np-btn {
  position: absolute;
  top: 50%;
  right: 0;
  transform: translateY(-50%);
}

/* 普通模式 */
.tb-brand {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  width: 100%;
  box-sizing: border-box;
}

.tb-icon {
  width: 18px;
  height: 18px;
  border-radius: 4px;
  object-fit: contain;
  pointer-events: none;
  opacity: 0.95;
}

.tb-title {
  font-family: var(--font-display);
  font-size: 13px;
  font-weight: 600;
  color: inherit;
  letter-spacing: 0.3px;
  pointer-events: none;
  opacity: 0.85;
}

/* 播放器模式 */
.tb-np-left {
  display: flex;
  align-items: center;
  padding: 0 8px;
}

.tb-np-btn {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: var(--radius-full);
  color: inherit;
  opacity: 0.8;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard),
              opacity var(--duration-short) var(--ease-standard);

  &:hover {
    background: rgba(255, 255, 255, 0.1);
    opacity: 1;
  }

  &:disabled {
    cursor: default;
    opacity: 0.36;
  }

  &:disabled:hover {
    background: transparent;
  }

  .material-symbols-rounded {
    font-size: 24px;
  }
}

.tb-np-collapse .material-symbols-rounded {
  font-size: 30px;
}

.tb-np-center {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1px;
  /* 水平留白；高度锁在顶栏高度内与红绿灯对齐，不做垂直撑高 */
  padding: 0 24px;
  box-sizing: border-box;
  width: min(56vw, 560px);
  max-width: calc(100% - 200px);
  height: var(--titlebar-height, 36px);
  pointer-events: auto;
  min-width: 180px;
}

.tb-np-label {
  font-size: 10px;
  font-weight: 600;
  color: inherit;
  opacity: 0.45;
  text-transform: uppercase;
  letter-spacing: 1.2px;
  line-height: 1.2;
  padding: 0 8px;
  /* 事件冒给带 data-tauri-drag-region 的 .tb-np-center，否则中心信息区不可拖拽窗口（UI-010） */
  pointer-events: none;
}

.tb-np-track {
  font-size: 12px;
  font-weight: 600;
  color: inherit;
  opacity: 0.85;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  width: 100%;
  max-width: 100%;
  line-height: 1.25;
  text-align: center;
  padding: 0 8px;
  box-sizing: border-box;
  pointer-events: none;
}


.tb-np-more {
  margin: 0;
}

/* 公共 */
.tb-drag {
  flex: 1;
  pointer-events: auto;
}

/* macOS 原生红绿灯安全区：trafficLightPosition x=20，按钮组宽 ~52px，
   右侧再留 ~20px 呼吸空间 -> 92px */
.tb-traffic-safe-area {
  width: 92px;
  height: 100%;
  flex: 0 0 auto;
  pointer-events: auto;
}

/* mac：左槽已固定宽度，品牌/箭头不再改 padding 造成水平跳 */
.tb-mac.tb-np-mode {
  padding-top: 0;
}

.tb-mac.tb-np-mode .tb-traffic-safe-area {
  height: var(--titlebar-height, 36px);
}

/* mac：更高顶栏下放大播放信息两行字号，与红绿灯/菜单按钮同轴垂直居中 */
.tb-mac:not(.tb-fullscreen) .tb-np-center {
  gap: 3px;
}

.tb-mac:not(.tb-fullscreen) .tb-np-label {
  font-size: 12px;
  letter-spacing: 1px;
  line-height: 1.25;
}

.tb-mac:not(.tb-fullscreen) .tb-np-track {
  font-size: 15px;
  line-height: 1.35;
}

/* 窗口控制：与应用其余按钮一致的圆角悬停块，普通 / 播放器模式同尺寸，开合播放页不位移 */
.tb-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 0 6px 0 4px;
}

.tb-ctrl {
  width: 40px;
  height: 28px;
  display: grid;
  place-items: center;
  background: transparent;
  border: none;
  border-radius: 8px;
  color: inherit;
  opacity: 0.72;
  cursor: pointer;
  transition: background 140ms var(--ease-standard),
              color 140ms var(--ease-standard),
              opacity 140ms var(--ease-standard),
              transform 120ms var(--ease-standard);

  svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.15;
    stroke-linecap: round;
    stroke-linejoin: round;
    overflow: visible;
  }

  &:hover {
    background: color-mix(in srgb, currentColor 12%, transparent);
    opacity: 1;
  }

  &:active {
    background: color-mix(in srgb, currentColor 18%, transparent);
    transform: scale(0.94);
  }
}

.tb-close:hover {
  background: #e81123;
  color: #fff;
  opacity: 1;
}

.tb-close:active {
  background: #c50f1f;
}

.tb-ctrl-swap-enter-active,
.tb-ctrl-swap-leave-active {
  transition: opacity 120ms var(--ease-standard), transform 160ms var(--ease-emphasized-decel);
}
.tb-ctrl-swap-enter-from,
.tb-ctrl-swap-leave-to {
  opacity: 0;
  transform: scale(0.7);
}

.light-theme .title-bar:not(.tb-force-light) .tb-np-btn:hover {
  background: rgba(0, 0, 0, 0.06);
}

</style>
