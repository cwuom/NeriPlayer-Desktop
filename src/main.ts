import { createApp } from 'vue'
import { createLogger } from '@/utils/logger'
import { createPinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '@tauri-apps/api/core'
import i18n from './i18n'
import { initTheme } from './utils/theme'
import { preloadUserData } from './modules/persistence/userData'
import { removeLegacyCacheBuckets } from './utils/persistentCache'
import './styles/global.scss'

// 在 DOM 挂载前应用主题（class 已在 index.html 内联脚本中预设）
initTheme()

const windowKind = new URLSearchParams(window.location.search).get('window')

async function mountWindow() {
  if (windowKind === 'desktop-lyrics') {
    const { default: DesktopLyricsView } = await import('./views/DesktopLyricsView.vue')
    createApp(DesktopLyricsView).use(i18n).mount('#app')
    return
  }
  if (windowKind === 'tray-popup') {
    const { default: TrayPopupView } = await import('./views/TrayPopupView.vue')
    createApp(TrayPopupView).use(i18n).mount('#app')
    return
  }

  const { default: App } = await import('./App.vue')
  const router = createRouter({
    history: createWebHistory(),
    routes: [
      { path: '/', name: 'home', meta: { section: 'home' }, component: () => import('./views/HomeView.vue') },
      { path: '/explore', name: 'explore', meta: { section: 'explore' }, component: () => import('./views/ExploreView.vue') },
      { path: '/library', name: 'library', meta: { section: 'library' }, component: () => import('./views/LibraryView.vue') },
      { path: '/settings', name: 'settings', meta: { section: 'settings' }, component: () => import('./views/SettingsView.vue') },
      { path: '/downloads', name: 'downloads', meta: { section: 'library' }, component: () => import('./views/DownloadsView.vue') },
      { path: '/recent', name: 'recent', meta: { section: 'library' }, component: () => import('./views/RecentView.vue') },
      { path: '/stats', name: 'playback-stats', meta: { section: 'library' }, component: () => import('./views/PlaybackStatsView.vue') },
      { path: '/playlist/netease/:id', name: 'netease-playlist', meta: { section: 'library' }, component: () => import('./views/NeteasePlaylistView.vue') },
      { path: '/album/netease/:id', name: 'netease-album', meta: { section: 'library' }, component: () => import('./views/NeteasePlaylistView.vue'), props: { isAlbum: true } },
      { path: '/artist/netease/:id', name: 'netease-artist', meta: { section: 'library' }, component: () => import('./views/NeteaseArtistView.vue') },
      { path: '/artist/bilibili/:mid', name: 'bili-artist', meta: { section: 'library' }, component: () => import('./views/BiliArtistView.vue') },
      { path: '/artist/youtube/:browseId', name: 'youtube-artist', meta: { section: 'library' }, component: () => import('./views/YouTubeArtistView.vue') },
      { path: '/playlist/bilibili/:mediaId', name: 'bili-playlist', meta: { section: 'library' }, component: () => import('./views/BiliPlaylistView.vue') },
      { path: '/playlist/youtube/:browseId', name: 'youtube-playlist', meta: { section: 'library' }, component: () => import('./views/YouTubePlaylistView.vue') },
      { path: '/playlist/local/:id', name: 'local-playlist', meta: { section: 'library' }, component: () => import('./views/LocalPlaylistView.vue') },
      { path: '/library/local-artist/:name', name: 'local-artist', meta: { section: 'library' }, component: () => import('./views/LocalArtistView.vue') },
      { path: '/library/favorite/:id', name: 'favorite-playlist', meta: { section: 'library' }, component: () => import('./views/FavoritePlaylistView.vue') },
      { path: '/debug', name: 'debug', meta: { section: 'debug' }, component: () => import('./views/DebugView.vue') },
    ],
  })

  // 未捕获异常与未处理 rejection 全部进统一日志（含后端环形缓冲），
  // 崩溃收集才有前端现场可看
  const crashLog = createLogger('frontend-crash')
  window.addEventListener('error', (event) => {
    // ResizeObserver 回调内改动被观察元素尺寸时，浏览器必然报一次
    // "loop completed with undelivered notifications"（规范行为，下一帧自愈，
    // 无实际危害）。歌词字号是视口相对值，窗口/面板尺寸变化时极易触发，
    // 误报成崩溃只会污染日志，此处过滤
    if (event.message === 'ResizeObserver loop completed with undelivered notifications.') return
    crashLog.error('uncaught error:', event.message, event.filename, `${event.lineno}:${event.colno}`)
  })
  window.addEventListener('unhandledrejection', (event) => {
    crashLog.error('unhandled rejection:', event.reason instanceof Error ? `${event.reason.message}\n${event.reason.stack ?? ''}` : String(event.reason))
  })

  // 播放队列、历史与歌词偏移在 store 创建前取回，恢复仍是同步的，不会与首个操作竞争
  if (await preloadUserData()) removeLegacyCacheBuckets()

  const app = createApp(App)
  app.use(createPinia())
  app.use(router)
  app.use(i18n)
  app.mount('#app')

  // Vue 挂载完成后显示窗口，避免闪烁
  if (isTauri()) void getCurrentWindow().show().catch(() => {})
}

void mountWindow()
