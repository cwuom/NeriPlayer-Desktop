import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import { getCurrentWindow } from '@tauri-apps/api/window'
import App from './App.vue'
import DesktopLyricsWindow from './components/DesktopLyricsWindow.vue'
import i18n from './i18n'
import { usePlayerStore } from './stores/player'
import { useSettingsStore } from './stores/settings'
import { useLyricOffsetStore } from './stores/lyricOffset'
import {
  DESKTOP_LYRICS_WINDOW_LABEL,
  startDesktopLyricsBridge,
} from './modules/desktopLyrics/bridge'
import { initTheme } from './utils/theme'
import './styles/global.scss'

// 在 DOM 挂载前应用主题（class 已在 index.html 内联脚本中预设）
initTheme()

function currentWindowLabel(): string {
  try {
    return getCurrentWindow().label
  } catch {
    return 'main'
  }
}

const isDesktopLyricsWindow =
  currentWindowLabel() === DESKTOP_LYRICS_WINDOW_LABEL
  || new URLSearchParams(window.location.search).get('window') === DESKTOP_LYRICS_WINDOW_LABEL

if (isDesktopLyricsWindow) {
  document.documentElement.classList.add('desktop-lyrics-window')
  const app = createApp(DesktopLyricsWindow)
  app.use(i18n)
  app.mount('#app')
} else {
  const router = createRouter({
    history: createWebHistory(),
    routes: [
      { path: '/', name: 'home', component: () => import('./views/HomeView.vue') },
      { path: '/explore', name: 'explore', component: () => import('./views/ExploreView.vue') },
      { path: '/library', name: 'library', component: () => import('./views/LibraryView.vue') },
      { path: '/settings', name: 'settings', component: () => import('./views/SettingsView.vue') },
      { path: '/downloads', name: 'downloads', component: () => import('./views/DownloadsView.vue') },
      { path: '/recent', name: 'recent', component: () => import('./views/RecentView.vue') },
      { path: '/playlist/netease/:id', name: 'netease-playlist', component: () => import('./views/NeteasePlaylistView.vue') },
      { path: '/album/netease/:id', name: 'netease-album', component: () => import('./views/NeteasePlaylistView.vue'), props: { isAlbum: true } },
      { path: '/playlist/bilibili/:mediaId', name: 'bili-playlist', component: () => import('./views/BiliPlaylistView.vue') },
      { path: '/playlist/youtube/:browseId', name: 'youtube-playlist', component: () => import('./views/YouTubePlaylistView.vue') },
      { path: '/playlist/local/:id', name: 'local-playlist', component: () => import('./views/LocalPlaylistView.vue') },
      { path: '/debug', name: 'debug', component: () => import('./views/DebugView.vue') },
    ],
  })
  const pinia = createPinia()
  const app = createApp(App)
  app.use(pinia)
  app.use(router)
  app.use(i18n)
  app.mount('#app')

  let bridgeDisposed = false
  let stopDesktopLyricsBridge: (() => void) | null = null
  void startDesktopLyricsBridge(
    usePlayerStore(pinia),
    useSettingsStore(pinia),
    useLyricOffsetStore(pinia),
  ).then(stop => {
    if (bridgeDisposed) stop()
    else stopDesktopLyricsBridge = stop
  })
  window.addEventListener('pagehide', () => {
    bridgeDisposed = true
    stopDesktopLyricsBridge?.()
  }, { once: true })
}

// Vue 挂载完成后显示窗口，避免闪烁
if (!isDesktopLyricsWindow) {
  try {
    getCurrentWindow().show().catch(() => {})
  } catch {
    // Browser preview does not expose the Tauri window runtime.
  }
}
