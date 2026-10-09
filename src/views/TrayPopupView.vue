<script setup lang="ts">
// Windows 托盘右键面板：曲目、播放控制、常用入口。外观跟随主窗口发布的主题色与深浅色，
// 所有操作交给后端转发主窗口执行，这里不碰播放状态
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { setLocale } from '@/i18n'

interface TrayPopupState {
  locale: string
  track: { title: string; artist: string; coverUrl: string } | null
  theme: { dark: boolean; vars: Record<string, string> }
  isPlaying: boolean
  desktopLyricsOpen: boolean
}

/** 图标字体没加载完就弹出会先闪一下图标名文字 */
const FONT_READY_TIMEOUT_MS = 800

const { t } = useI18n()
const state = ref<TrayPopupState>({
  locale: '',
  track: null,
  theme: { dark: true, vars: {} },
  isPlaying: false,
  desktopLyricsOpen: false,
})
const coverFailed = ref(false)
let release: UnlistenFn | null = null
let disposed = false

const track = computed(() => state.value.track)
const coverSrc = computed(() => (coverFailed.value ? '' : track.value?.coverUrl || ''))

watch(() => track.value?.coverUrl, () => { coverFailed.value = false })

function applyState(next: TrayPopupState) {
  state.value = next
  const root = document.documentElement
  root.classList.toggle('dark-theme', next.theme.dark)
  root.classList.toggle('light-theme', !next.theme.dark)
  for (const [name, value] of Object.entries(next.theme.vars)) root.style.setProperty(name, value)
  if (next.locale) setLocale(next.locale, false)
}

function act(action: string) {
  if (!isTauri()) return
  void invoke('tray_popup_action', { action }).catch(() => {})
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') act('dismiss')
}

async function fontsReady() {
  if (!document.fonts?.ready) return
  await Promise.race([
    document.fonts.ready,
    new Promise(resolve => setTimeout(resolve, FONT_READY_TIMEOUT_MS)),
  ])
}

onMounted(async () => {
  document.documentElement.classList.add('tray-popup-page')
  window.addEventListener('keydown', onKeydown)
  if (!isTauri()) return
  const unlisten = await listen<TrayPopupState>('tray-popup:state', event => applyState(event.payload))
  if (disposed) {
    unlisten()
    return
  }
  release = unlisten
  try {
    applyState(await invoke<TrayPopupState>('get_tray_popup_state'))
  } catch {
    // 拿不到状态时按空闲态显示，后端推送到达后再更新
  }
  await nextTick()
  await fontsReady()
  if (!disposed) void invoke('tray_popup_ready').catch(() => {})
})

onUnmounted(() => {
  disposed = true
  window.removeEventListener('keydown', onKeydown)
  release?.()
  release = null
  document.documentElement.classList.remove('tray-popup-page')
})
</script>

<template>
  <main class="tray-popup" @contextmenu.prevent>
    <section class="tp-card" role="menu" :aria-label="t('tray.menu_label')">
      <button
        class="tp-hero"
        :class="{ 'tp-hero--idle': !track }"
        :disabled="!track"
        :title="track ? t('tray.open_now_playing') : undefined"
        @click="act('now-playing')"
      >
        <div v-if="coverSrc" class="tp-hero-backdrop" aria-hidden="true">
          <img :src="coverSrc" alt="" referrerpolicy="no-referrer" />
        </div>
        <div class="tp-cover">
          <img v-if="coverSrc" :src="coverSrc" alt="" referrerpolicy="no-referrer" @error="coverFailed = true" />
          <span v-else class="material-symbols-rounded">music_note</span>
          <span v-if="track && state.isPlaying" class="tp-eq" aria-hidden="true"><i /><i /><i /></span>
        </div>
        <div class="tp-meta">
          <div class="tp-title">{{ track?.title || t('tray.idle') }}</div>
          <div class="tp-artist">{{ track ? (track.artist || t('tray.unknown_artist')) : 'NeriPlayer' }}</div>
        </div>
        <span v-if="track" class="material-symbols-rounded tp-chevron">chevron_right</span>
      </button>

      <div class="tp-controls">
        <button class="tp-icon-btn" :title="t('tray.previous')" :aria-label="t('tray.previous')" @click="act('previous')">
          <span class="material-symbols-rounded filled">skip_previous</span>
        </button>
        <button
          class="tp-play-btn"
          :title="state.isPlaying ? t('tray.pause') : t('tray.play')"
          :aria-label="state.isPlaying ? t('tray.pause') : t('tray.play')"
          @click="act('toggle')"
        >
          <span class="material-symbols-rounded filled">{{ state.isPlaying ? 'pause' : 'play_arrow' }}</span>
        </button>
        <button class="tp-icon-btn" :title="t('tray.next')" :aria-label="t('tray.next')" @click="act('next')">
          <span class="material-symbols-rounded filled">skip_next</span>
        </button>
      </div>

      <div class="tp-divider" />

      <button class="tp-item" role="menuitem" @click="act('show-main')">
        <span class="material-symbols-rounded">open_in_new</span>
        <span class="tp-item-label">{{ t('tray.show_main') }}</span>
      </button>
      <button
        class="tp-item"
        role="menuitemcheckbox"
        :aria-checked="state.desktopLyricsOpen"
        @click="act('desktop-lyrics')"
      >
        <span class="material-symbols-rounded">lyrics</span>
        <span class="tp-item-label">{{ t('tray.desktop_lyrics') }}</span>
        <span class="tp-switch" :class="{ 'tp-switch--on': state.desktopLyricsOpen }" aria-hidden="true">
          <span class="tp-switch-thumb" />
        </span>
      </button>

      <div class="tp-divider" />

      <button class="tp-item tp-item--quit" role="menuitem" @click="act('quit')">
        <span class="material-symbols-rounded">power_settings_new</span>
        <span class="tp-item-label">{{ t('tray.quit') }}</span>
      </button>
    </section>
  </main>
</template>

<style scoped>
:global(html.tray-popup-page),
:global(html.tray-popup-page body),
:global(html.tray-popup-page #app) {
  background: transparent !important;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

/* 窗口四周 10px 透明边留给阴影，尺寸与后端 POPUP_WIDTH/POPUP_HEIGHT 对应 */
.tray-popup {
  box-sizing: border-box;
  width: 100%;
  height: 100%;
  padding: 10px;
  user-select: none;
  -webkit-user-select: none;
  color: var(--md-on-surface);
}

.tp-card {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 6px;
  border-radius: 18px;
  background: var(--md-surface-container);
  border: 1px solid color-mix(in srgb, var(--md-outline-variant) 70%, transparent);
  box-shadow: 0 4px 12px rgb(0 0 0 / 28%), 0 1px 3px rgb(0 0 0 / 18%);
  overflow: hidden;
  animation: tp-enter 160ms cubic-bezier(0.2, 0, 0, 1);
}

@keyframes tp-enter {
  from { opacity: 0; transform: translateY(6px) scale(0.98); }
  to { opacity: 1; transform: none; }
}

button {
  font: inherit;
  color: inherit;
  border: none;
  background: none;
  cursor: pointer;
  outline: none;
}

button:focus-visible {
  box-shadow: inset 0 0 0 2px var(--md-primary);
}

.tp-hero {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1 0 76px;
  min-height: 76px;
  padding: 12px;
  border-radius: 13px;
  overflow: hidden;
  text-align: left;
  isolation: isolate;
  background: var(--md-surface-container-high);
  transition: background-color 0.15s ease;
}

.tp-hero:not(:disabled):hover {
  background: var(--md-surface-container-highest);
}

.tp-hero:disabled {
  cursor: default;
}

.tp-hero-backdrop {
  position: absolute;
  inset: 0;
  z-index: -1;
  overflow: hidden;
  pointer-events: none;
}

.tp-hero-backdrop img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  transform: scale(1.6);
  filter: blur(22px) saturate(1.5);
  opacity: 0.38;
}

:global(html.light-theme) .tp-hero-backdrop img {
  opacity: 0.28;
}

.tp-cover {
  position: relative;
  flex: none;
  display: grid;
  place-items: center;
  width: 52px;
  height: 52px;
  border-radius: 10px;
  overflow: hidden;
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
  box-shadow: 0 2px 8px rgb(0 0 0 / 22%);
}

.tp-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.tp-cover .material-symbols-rounded {
  font-size: 26px;
}

.tp-eq {
  position: absolute;
  right: 4px;
  bottom: 4px;
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 12px;
  padding: 2px 3px;
  border-radius: 4px;
  background: rgb(0 0 0 / 45%);
}

.tp-eq i {
  width: 2px;
  height: 100%;
  border-radius: 1px;
  background: #fff;
  transform-origin: bottom;
  animation: tp-eq 0.9s ease-in-out infinite;
}

.tp-eq i:nth-child(2) { animation-delay: -0.3s; }
.tp-eq i:nth-child(3) { animation-delay: -0.6s; }

@keyframes tp-eq {
  0%, 100% { transform: scaleY(0.35); }
  50% { transform: scaleY(1); }
}

.tp-meta {
  flex: 1;
  min-width: 0;
}

.tp-title,
.tp-artist {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.tp-title {
  font-size: 14px;
  font-weight: 600;
  line-height: 20px;
}

.tp-artist {
  margin-top: 2px;
  font-size: 12px;
  line-height: 16px;
  color: var(--md-on-surface-variant);
}

.tp-hero--idle .tp-title {
  color: var(--md-on-surface-variant);
  font-weight: 500;
}

.tp-chevron {
  flex: none;
  font-size: 20px;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
}

.tp-controls {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 20px;
  flex: none;
  height: 60px;
}

.tp-icon-btn {
  display: grid;
  place-items: center;
  width: 40px;
  height: 40px;
  border-radius: 50%;
  color: var(--md-on-surface-variant);
  transition: background-color 0.15s ease, color 0.15s ease, transform 0.1s ease;
}

.tp-icon-btn:hover {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}

.tp-icon-btn .material-symbols-rounded {
  font-size: 26px;
}

.tp-play-btn {
  display: grid;
  place-items: center;
  width: 46px;
  height: 46px;
  border-radius: 50%;
  background: var(--md-primary);
  color: var(--md-on-primary);
  box-shadow: 0 2px 8px color-mix(in srgb, var(--md-primary) 40%, transparent);
  transition: filter 0.15s ease, transform 0.1s ease;
}

.tp-play-btn:hover {
  filter: brightness(1.08);
}

.tp-play-btn .material-symbols-rounded {
  font-size: 28px;
}

.tp-icon-btn:active,
.tp-play-btn:active {
  transform: scale(0.92);
}

.tp-divider {
  flex: none;
  height: 1px;
  margin: 6px 10px;
  background: color-mix(in srgb, var(--md-outline-variant) 80%, transparent);
}

.tp-item {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: none;
  height: 40px;
  padding: 0 12px;
  border-radius: 10px;
  font-size: 13px;
  text-align: left;
  transition: background-color 0.12s ease;
}

.tp-item:hover {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}

.tp-item > .material-symbols-rounded {
  font-size: 20px;
  color: var(--md-on-surface-variant);
}

.tp-item:hover > .material-symbols-rounded {
  color: inherit;
}

.tp-item-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.tp-item--quit:hover {
  background: color-mix(in srgb, #e5484d 16%, transparent);
  color: #e5484d;
}

:global(html.dark-theme) .tp-item--quit:hover {
  color: #ff8b8f;
}

.tp-switch {
  position: relative;
  flex: none;
  width: 32px;
  height: 18px;
  border-radius: 9px;
  border: 1.5px solid var(--md-outline-variant);
  background: var(--md-surface-container-highest);
  transition: background-color 0.18s ease, border-color 0.18s ease;
}

.tp-switch-thumb {
  position: absolute;
  top: 50%;
  left: 3px;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--md-on-surface-variant);
  transform: translateY(-50%);
  transition: left 0.18s cubic-bezier(0.2, 0, 0, 1), width 0.18s ease, height 0.18s ease, background-color 0.18s ease;
}

.tp-switch--on {
  border-color: var(--md-primary);
  background: var(--md-primary);
}

.tp-switch--on .tp-switch-thumb {
  left: 18px;
  width: 11px;
  height: 11px;
  background: var(--md-on-primary);
}
</style>
