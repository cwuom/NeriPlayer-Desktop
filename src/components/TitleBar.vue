<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { UnlistenFn } from '@tauri-apps/api/event'

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
let unlistenResize: UnlistenFn | null = null

// macOS 使用系统原生红绿灯，隐藏自绘控制并在左侧留出安全区。
// 通过 navigator 判定平台（reqwest 层的 UA spoof 不影响 webview 的 navigator）。
const isMac = /Mac|iPhone|iPad/.test(
  (navigator as any).userAgentData?.platform || navigator.platform || navigator.userAgent
)

let appWindow: ReturnType<typeof getCurrentWindow> | null = null
try {
  appWindow = getCurrentWindow()
} catch {
  // Browser preview does not expose the Tauri window runtime.
}

async function refreshMaximized() {
  if (!appWindow) return
  try {
    isMaximized.value = await appWindow.isMaximized()
  } catch {
    isMaximized.value = false
  }
}

function minimize() {
  if (!appWindow) return
  appWindow.minimize().catch(() => {})
}

function toggleMaximize() {
  if (!appWindow) return
  appWindow.toggleMaximize().then(refreshMaximized).catch(() => {})
}

function close() {
  if (!appWindow) return
  appWindow.close().catch(() => {})
}

onMounted(async () => {
  await refreshMaximized()
  if (!appWindow) return
  try {
    unlistenResize = await appWindow.onResized(() => refreshMaximized())
  } catch {}
})

onUnmounted(() => {
  if (unlistenResize) unlistenResize()
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
    }"
    data-tauri-drag-region
  >
    <!-- macOS 原生红绿灯安全区（约 78px），避免内容压住系统交通灯 -->
    <div v-if="isMac" class="tb-traffic-safe-area" data-tauri-drag-region></div>

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
            <button class="tb-np-btn tb-np-collapse" type="button" data-tauri-drag-region="false" @click="emit('collapse')">
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
    <div v-if="isMac" class="tb-drag" data-tauri-drag-region></div>

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
            @click="emit('toggleMore')"
          >
            <span class="material-symbols-rounded">more_vert</span>
          </button>
        </transition>
      </div>
    </div>

    <!-- 窗口控制（macOS 使用系统原生红绿灯，此处隐藏） -->
    <div v-if="!isMac" class="tb-controls">
      <button class="tb-ctrl" type="button" @click="minimize" title="最小化">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <rect x="2" y="5.5" width="8" height="1" fill="currentColor" />
        </svg>
      </button>
      <button class="tb-ctrl" type="button" @click="toggleMaximize" :title="isMaximized ? '还原' : '最大化'">
        <svg v-if="!isMaximized" width="12" height="12" viewBox="0 0 12 12">
          <rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
        <svg v-else width="12" height="12" viewBox="0 0 12 12">
          <rect x="3.5" y="2" width="6.5" height="6.5" fill="none" stroke="currentColor" stroke-width="1" />
          <rect x="2" y="3.5" width="6.5" height="6.5" fill="var(--md-background)" stroke="currentColor" stroke-width="1" />
        </svg>
      </button>
      <button class="tb-ctrl tb-close" type="button" @click="close" title="关闭">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <line x1="2.5" y1="2.5" x2="9.5" y2="9.5" stroke="currentColor" stroke-width="1" />
          <line x1="9.5" y1="2.5" x2="2.5" y2="9.5" stroke="currentColor" stroke-width="1" />
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
  /* 固定 36px：对齐 macOS 红绿灯垂直中心，开关 NP 不改高度避免位移 */
  height: 36px;
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
    height: 36px;
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
  min-height: 36px;
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
  -webkit-app-region: drag;
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
  font-size: 12px;
  font-weight: 600;
  color: inherit;
  letter-spacing: 0.2px;
  pointer-events: none;
  opacity: 0.85;
}

/* 播放器模式 */
.tb-np-left {
  display: flex;
  align-items: center;
  padding: 0 8px;
  -webkit-app-region: no-drag;
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
  /* 水平留白；高度锁在 36px 内与红绿灯对齐，不做垂直撑高 */
  padding: 0 24px;
  box-sizing: border-box;
  width: min(56vw, 560px);
  max-width: calc(100% - 200px);
  height: 36px;
  pointer-events: auto;
  -webkit-app-region: drag;
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
}


.tb-np-more {
  margin: 0;
}

/* 播放器模式保持与普通顶栏同高，避免位移 */
.tb-np-mode .tb-ctrl {
  width: 46px;
  height: 100%;
  align-self: stretch;
  border-radius: 0;
}

.tb-np-mode .tb-ctrl svg {
  width: 12px;
  height: 12px;
}

.tb-np-mode .tb-controls {
  align-items: stretch;
  gap: 0;
  padding-right: 0;
}

/* 公共 */
.tb-drag {
  flex: 1;
  -webkit-app-region: drag;
  pointer-events: auto;
}

/* macOS 原生红绿灯安全区：系统交通灯位于左上 ~12px 起、宽约 78px */
.tb-traffic-safe-area {
  width: 78px;
  height: 100%;
  flex: 0 0 auto;
  -webkit-app-region: drag;
  pointer-events: auto;
}

/* mac：左槽已固定宽度，品牌/箭头不再改 padding 造成水平跳 */
.tb-mac.tb-np-mode {
  padding-top: 0;
}

.tb-mac.tb-np-mode .tb-traffic-safe-area {
  height: 36px;
}

.tb-controls {
  display: flex;
  align-items: stretch;
  -webkit-app-region: no-drag;
}

.tb-ctrl {
  width: 46px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: inherit;
  opacity: 0.7;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard),
              opacity var(--duration-short) var(--ease-standard);

  &:hover {
    background: rgba(255, 255, 255, 0.08);
    opacity: 1;
  }
}

.light-theme .title-bar:not(.tb-force-light) .tb-ctrl:hover {
  background: rgba(0, 0, 0, 0.06);
}

.light-theme .title-bar:not(.tb-force-light) .tb-np-btn:hover {
  background: rgba(0, 0, 0, 0.06);
}

.tb-close:hover {
  background: #e81123 !important;
  color: #fff;
  opacity: 1;
}

</style>
