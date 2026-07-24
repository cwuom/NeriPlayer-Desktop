<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { emitTo, listen, type UnlistenFn } from '@tauri-apps/api/event'
import { availableMonitors, getCurrentWindow, PhysicalPosition } from '@tauri-apps/api/window'
import { useI18n } from 'vue-i18n'
import {
  resolveDesktopLyricLine,
  resolveNextDesktopLyricLine,
} from '@/modules/desktopLyrics/activeLine'
import { clampWindowPositionToWorkAreas } from '@/modules/desktopLyrics/windowPosition'
import {
  DESKTOP_LYRICS_HIDDEN_EVENT,
  DESKTOP_LYRICS_READY_EVENT,
  DESKTOP_LYRICS_STATE_EVENT,
  type DesktopLyricsSnapshot,
} from '@/modules/desktopLyrics/bridge'

const { t } = useI18n()
const snapshot = ref<DesktopLyricsSnapshot | null>(null)
const POSITION_STORAGE_KEY = 'neri:desktop-lyrics:position'
const LOCK_STORAGE_KEY = 'neri:desktop-lyrics:locked'

function readLocked(): boolean {
  try {
    return localStorage.getItem(LOCK_STORAGE_KEY) === 'true'
  } catch {
    return false
  }
}

function readStoredPosition(): { x: number; y: number } | null {
  try {
    const value = JSON.parse(localStorage.getItem(POSITION_STORAGE_KEY) || 'null')
    if (!value || !Number.isFinite(value.x) || !Number.isFinite(value.y)) return null
    return { x: Number(value.x), y: Number(value.y) }
  } catch {
    return null
  }
}

async function restoreWindowPosition(appWindow: ReturnType<typeof getCurrentWindow>) {
  const storedPosition = readStoredPosition()
  if (storedPosition) {
    try {
      const [windowSize, monitors] = await Promise.all([
        appWindow.outerSize(),
        availableMonitors(),
      ])
      const restoredPosition = clampWindowPositionToWorkAreas(storedPosition, windowSize, monitors)
      if (restoredPosition) {
        await appWindow.setPosition(new PhysicalPosition(restoredPosition.x, restoredPosition.y))
        return
      }
    } catch {}
  }

  try {
    await appWindow.center()
  } catch {}
}

const locked = ref(readLocked())
let unlistenState: UnlistenFn | null = null
let unlistenMoved: UnlistenFn | null = null
let unlistenCloseRequested: UnlistenFn | null = null

const currentLine = computed(() => {
  const state = snapshot.value
  if (!state) return null
  return resolveDesktopLyricLine(state.lyrics, state.positionMs, state.lyricOffsetMs)
})

const currentText = computed(() => {
  const line = currentLine.value
  if (!line) return ''
  return line.text || line.words.map(word => word.text).join('')
})

const currentTranslation = computed(() => {
  if (!snapshot.value?.showTranslation) return ''
  return currentLine.value?.translation || ''
})

const nextLine = computed(() => {
  const state = snapshot.value
  if (!state) return null
  return resolveNextDesktopLyricLine(state.lyrics, currentLine.value)
})

const nextText = computed(() => {
  const line = nextLine.value
  if (!line) return ''
  return line.text || line.words.map(word => word.text).join('')
})

const emptyText = computed(() => {
  if (!snapshot.value?.track) return t('player.not_playing')
  if (snapshot.value.isLoadingLyrics) return t('player.loading')
  return t('player.no_lyrics')
})

function toggleLocked() {
  locked.value = !locked.value
  try {
    localStorage.setItem(LOCK_STORAGE_KEY, String(locked.value))
  } catch {}
}

async function beginDrag() {
  if (locked.value) return
  try {
    await getCurrentWindow().startDragging()
  } catch {}
}

async function hideWindow() {
  try {
    await emitTo('main', DESKTOP_LYRICS_HIDDEN_EVENT)
  } catch {}
  try {
    await getCurrentWindow().hide()
  } catch {}
}

onMounted(async () => {
  document.documentElement.classList.add('desktop-lyrics-window')
  try {
    unlistenState = await listen<DesktopLyricsSnapshot>(DESKTOP_LYRICS_STATE_EVENT, event => {
      snapshot.value = event.payload
    })
  } catch {}

  let appWindow: ReturnType<typeof getCurrentWindow> | null = null
  try {
    appWindow = getCurrentWindow()
  } catch {}
  if (appWindow) {
    await restoreWindowPosition(appWindow)
    try {
      unlistenMoved = await appWindow.onMoved(event => {
        try {
          localStorage.setItem(POSITION_STORAGE_KEY, JSON.stringify({
            x: event.payload.x,
            y: event.payload.y,
          }))
        } catch {}
      })
    } catch {}
    try {
      unlistenCloseRequested = await appWindow.onCloseRequested(() => {
        void emitTo('main', DESKTOP_LYRICS_HIDDEN_EVENT).catch(() => {})
      })
    } catch {}
  }

  try {
    await emitTo('main', DESKTOP_LYRICS_READY_EVENT)
  } catch {}
  try {
    await appWindow?.show()
  } catch {}
})

onUnmounted(() => {
  unlistenState?.()
  unlistenMoved?.()
  unlistenCloseRequested?.()
  document.documentElement.classList.remove('desktop-lyrics-window')
})
</script>

<template>
  <main
    class="desktop-lyrics-shell"
    :class="{ 'is-locked': locked, 'is-paused': snapshot && !snapshot.isPlaying }"
    @mousedown.left="beginDrag"
  >
    <div class="desktop-lyrics-meta">
      <span class="material-symbols-rounded desktop-lyrics-state" aria-hidden="true">
        {{ snapshot?.isPlaying ? 'graphic_eq' : 'pause' }}
      </span>
      <span class="desktop-lyrics-track">
        <strong>{{ snapshot?.track?.title || t('player.desktop_lyrics') }}</strong>
        <span v-if="snapshot?.track?.artist">{{ snapshot.track.artist }}</span>
      </span>
    </div>

    <div class="desktop-lyrics-copy" aria-live="polite">
      <Transition name="desktop-line" mode="out-in">
        <div :key="currentLine?.startMs ?? 'empty'" class="desktop-lyrics-line-group">
          <p class="desktop-lyrics-line" :class="{ 'is-empty': !currentText }">
            {{ currentText || emptyText }}
          </p>
          <p v-if="currentTranslation" class="desktop-lyrics-translation">
            {{ currentTranslation }}
          </p>
          <p v-if="nextText" class="desktop-lyrics-next">
            {{ nextText }}
          </p>
        </div>
      </Transition>
    </div>

    <div class="desktop-lyrics-actions" @mousedown.stop>
      <button
        type="button"
        class="desktop-lyrics-action"
        :class="{ active: locked }"
        :title="t(locked ? 'player.desktop_lyrics_unlock' : 'player.desktop_lyrics_lock')"
        @click="toggleLocked"
      >
        <span class="material-symbols-rounded">{{ locked ? 'lock' : 'lock_open' }}</span>
      </button>
      <button
        type="button"
        class="desktop-lyrics-action"
        :title="t('player.desktop_lyrics_hide')"
        @click="hideWindow"
      >
        <span class="material-symbols-rounded">close</span>
      </button>
    </div>
  </main>
</template>

<style scoped lang="scss">
:global(html.desktop-lyrics-window),
:global(html.desktop-lyrics-window body),
:global(html.desktop-lyrics-window #app) {
  background: transparent !important;
}

.desktop-lyrics-shell {
  width: 100%;
  height: 100%;
  overflow: hidden;
  display: grid;
  grid-template-columns: minmax(118px, 0.3fr) minmax(0, 1fr) 76px;
  align-items: center;
  gap: 14px;
  padding-block: 14px;
  padding-inline: 18px 12px;
  border-width: 1px;
  border-style: solid;
  border-color: rgba(255, 255, 255, 0.16);
  border-radius: 8px;
  background: rgba(19, 19, 22, 0.82);
  color: rgba(255, 255, 255, 0.96);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.3);
  backdrop-filter: blur(18px) saturate(1.12);
  cursor: grab;

  &:active:not(.is-locked) {
    cursor: grabbing;
  }

  &.is-locked {
    cursor: default;
  }
}

.desktop-lyrics-meta {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  color: rgba(255, 255, 255, 0.72);
}

.desktop-lyrics-state {
  flex: 0 0 auto;
  font-size: 20px;
  color: rgb(134, 212, 184);
}

.is-paused .desktop-lyrics-state {
  color: rgba(255, 255, 255, 0.5);
}

.desktop-lyrics-track {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;

  strong,
  span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  strong {
    font-size: 12px;
    font-weight: 650;
  }

  span {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.48);
  }
}

.desktop-lyrics-copy {
  min-width: 0;
  text-align: center;
}

.desktop-lyrics-line-group {
  min-height: 58px;
  max-height: 100%;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 4px;
}

.desktop-lyrics-line,
.desktop-lyrics-translation,
.desktop-lyrics-next {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.desktop-lyrics-line {
  font-size: 23px;
  line-height: 29px;
  font-weight: 700;
  text-shadow: 0 2px 10px rgba(0, 0, 0, 0.54);

  &.is-empty {
    font-size: 15px;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.58);
  }
}

.desktop-lyrics-translation {
  font-size: 13px;
  line-height: 16px;
  color: rgba(255, 255, 255, 0.66);
}

.desktop-lyrics-next {
  font-size: 15px;
  line-height: 19px;
  color: rgba(255, 255, 255, 0.52);
}

.desktop-lyrics-actions {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
  opacity: 0;
  transition: opacity 150ms ease;
}

.desktop-lyrics-shell:hover .desktop-lyrics-actions,
.desktop-lyrics-actions:focus-within,
.is-locked .desktop-lyrics-actions {
  opacity: 1;
}

.desktop-lyrics-action {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  color: rgba(255, 255, 255, 0.68);
  transition: color 150ms ease, background 150ms ease;

  .material-symbols-rounded {
    font-size: 18px;
  }

  &:hover,
  &.active {
    color: rgba(255, 255, 255, 0.96);
    background: rgba(255, 255, 255, 0.12);
  }
}

.desktop-line-enter-active,
.desktop-line-leave-active {
  transition: opacity 150ms ease, transform 150ms ease;
}

.desktop-line-enter-from {
  opacity: 0;
  transform: translateY(5px);
}

.desktop-line-leave-to {
  opacity: 0;
  transform: translateY(-5px);
}

@media (max-width: 520px) {
  .desktop-lyrics-shell {
    grid-template-columns: minmax(0, 1fr) 76px;
  }

  .desktop-lyrics-meta {
    display: none;
  }
}
</style>
