<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { DesktopLyricsFrame } from '@/modules/desktopLyrics/frame'

const { t } = useI18n()
const frame = ref<DesktopLyricsFrame>({ trackId: '', title: '', artist: '', previous: '', current: '', next: '', translation: '', isPlaying: false })
let release: UnlistenFn | null = null
let disposed = false
let eventVersion = 0

async function close() {
  if (!isTauri()) return
  await getCurrentWindow().close()
}

function drag(event: PointerEvent) {
  if (!isTauri()) return
  if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return
  void getCurrentWindow().startDragging().catch(() => {})
}

function keydown(event: KeyboardEvent) {
  if (event.key === 'Escape') void close().catch(() => {})
}

onMounted(async () => {
  document.documentElement.classList.add('desktop-lyrics-page')
  window.addEventListener('keydown', keydown)
  try {
    const unlisten = await listen<DesktopLyricsFrame>('desktop-lyrics:frame', event => {
      if (disposed) return
      eventVersion++
      frame.value = event.payload
    })
    if (disposed) { unlisten(); return }
    release = unlisten
    const version = eventVersion
    const snapshot = await invoke<DesktopLyricsFrame>('get_desktop_lyrics_snapshot')
    if (!disposed && version === eventVersion) frame.value = snapshot
  } catch {
    // 只读显示窗的连接失败保持空态，不初始化任何播放状态
  } finally {
    if (!disposed && isTauri()) void getCurrentWindow().show().catch(() => {})
  }
})

onUnmounted(() => {
  disposed = true
  release?.()
  window.removeEventListener('keydown', keydown)
  document.documentElement.classList.remove('desktop-lyrics-page')
})
</script>

<template>
  <main class="desktop-lyrics" @pointerdown="drag">
    <header class="desktop-lyrics-header">
      <div class="desktop-lyrics-track" :title="[frame.title, frame.artist].filter(Boolean).join(' · ')">
        <span class="material-symbols-rounded desktop-lyrics-icon">{{ frame.isPlaying ? 'graphic_eq' : 'music_note' }}</span>
        <span>{{ frame.title || 'NeriPlayer' }}<template v-if="frame.artist"> · {{ frame.artist }}</template></span>
      </div>
      <button type="button" class="desktop-lyrics-close" :aria-label="t('common.close')" :title="t('common.close')" @click="close">
        <span class="material-symbols-rounded">close</span>
      </button>
    </header>
    <section class="desktop-lyrics-lines" aria-live="off">
      <p class="desktop-lyrics-adjacent">{{ frame.previous || '\u00a0' }}</p>
      <p class="desktop-lyrics-current">{{ frame.current || (frame.next ? '\u00a0' : t('player.no_lyrics')) }}</p>
      <p class="desktop-lyrics-adjacent" :class="{ 'desktop-lyrics-translation': frame.translation }">{{ frame.translation || frame.next || '\u00a0' }}</p>
    </section>
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
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  padding: 8px 18px 16px;
  border: 1px solid rgb(255 255 255 / 16%);
  border-radius: 18px;
  color: #fff;
  background: rgb(16 15 24 / 76%);
  box-sizing: border-box;
  user-select: none;
  cursor: grab;
  font-family: var(--font-family, system-ui, sans-serif);
}

.desktop-lyrics-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex: 0 0 32px;
}

.desktop-lyrics-track {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: rgb(255 255 255 / 64%);
  font-size: 12px;
}

.desktop-lyrics-track > span:last-child {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.desktop-lyrics-icon { font-size: 17px; }

.desktop-lyrics-close {
  display: grid;
  place-items: center;
  flex: 0 0 28px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: rgb(255 255 255 / 76%);
  cursor: pointer;
}

.desktop-lyrics-close .material-symbols-rounded { font-size: 18px; }
.desktop-lyrics-close:hover, .desktop-lyrics-close:focus-visible { background: rgb(255 255 255 / 14%); color: #fff; }
.desktop-lyrics-close:focus-visible { outline: 2px solid #d0bcff; outline-offset: 1px; }

.desktop-lyrics-lines {
  display: flex;
  flex: 1;
  flex-direction: column;
  justify-content: center;
  gap: 8px;
  min-height: 0;
  text-align: center;
  text-shadow: 0 2px 8px rgb(0 0 0 / 72%);
}

.desktop-lyrics-lines p {
  margin: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.desktop-lyrics-current { font-size: clamp(21px, 4.6vw, 30px); line-height: 1.4; font-weight: 700; }
.desktop-lyrics-adjacent { color: rgb(255 255 255 / 54%); font-size: 14px; line-height: 1.4; }
.desktop-lyrics-translation { color: rgb(234 221 255 / 86%); font-size: 15px; }
</style>
