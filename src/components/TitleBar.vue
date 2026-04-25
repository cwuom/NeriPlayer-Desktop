<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { UnlistenFn } from '@tauri-apps/api/event'

defineProps<{
  forceLight?: boolean
}>()

const isMaximized = ref(false)
let unlistenResize: UnlistenFn | null = null

const appWindow = getCurrentWindow()

async function refreshMaximized() {
  try {
    isMaximized.value = await appWindow.isMaximized()
  } catch {
    isMaximized.value = false
  }
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

onMounted(async () => {
  await refreshMaximized()
  try {
    unlistenResize = await appWindow.onResized(() => refreshMaximized())
  } catch {}
})

onUnmounted(() => {
  if (unlistenResize) unlistenResize()
})
</script>

<template>
  <header class="title-bar" :class="{ 'tb-force-light': forceLight }" data-tauri-drag-region>
    <div class="tb-brand" data-tauri-drag-region>
      <img src="/app-icon.png" alt="logo" class="tb-icon" />
      <span class="tb-title">NeriPlayer</span>
    </div>

    <div class="tb-drag" data-tauri-drag-region></div>

    <div class="tb-controls">
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
  height: 36px;
  display: flex;
  align-items: stretch;
  background: transparent;
  color: var(--md-on-surface);
  user-select: none;
  z-index: 1000;
  pointer-events: none;
  transition: color 300ms var(--ease-standard);
}

/* NowPlaying 打开时，强制使用亮色文字（深色背景） */
.title-bar.tb-force-light {
  color: rgba(255, 255, 255, 0.9);
}

.tb-brand,
.tb-controls,
.tb-ctrl {
  pointer-events: auto;
}

.tb-brand {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 14px;
  -webkit-app-region: drag;
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

.tb-drag {
  flex: 1;
  -webkit-app-region: drag;
  pointer-events: auto;
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

/* 浅色主题 + 非 forceLight 时，hover 用深色遮罩 */
.light-theme .title-bar:not(.tb-force-light) .tb-ctrl:hover {
  background: rgba(0, 0, 0, 0.06);
}

.tb-close:hover {
  background: #e81123 !important;
  color: #fff;
  opacity: 1;
}
</style>
