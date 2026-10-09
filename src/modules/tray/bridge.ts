// 主窗口侧的托盘桥：把曲目、文案、主题色发布给托盘（原生菜单 / Windows 自绘面板），
// 并执行托盘发回的动作。播放/暂停状态由后端 ticker 直接同步给托盘，这里不发
import { watch } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import i18n from '@/i18n'
import { usePlayerStore } from '@/stores/player'
import {
  closeDesktopLyricsWindow,
  desktopLyricsOpen,
  openDesktopLyricsWindow,
} from '@/modules/desktopLyrics/bridge'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { normalizeCoverUrlForDisplay, peekCoverImage } from '@/utils/bilibiliCover'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('tray')

/** 面板用到的主题令牌；读的是 html 上的实色（自定义背景图只把 #app 内的 surface 换成半透明） */
export const TRAY_THEME_VARS = [
  '--md-primary',
  '--md-on-primary',
  '--md-primary-container',
  '--md-on-primary-container',
  '--md-secondary-container',
  '--md-on-secondary-container',
  '--md-surface-container',
  '--md-surface-container-high',
  '--md-surface-container-highest',
  '--md-on-surface',
  '--md-on-surface-variant',
  '--md-outline-variant',
] as const

const MENU_TEXT_KEYS = ['previous', 'play', 'pause', 'next', 'show_main', 'desktop_lyrics', 'quit', 'idle'] as const
const PUBLISH_DEBOUNCE_MS = 120

export interface TrayBridgeOptions {
  openNowPlaying: () => void
  /** 退出前的落盘（播放状态、统计） */
  flushBeforeQuit: () => Promise<unknown>
}

function readTheme() {
  const root = document.documentElement
  const style = getComputedStyle(root)
  const vars: Record<string, string> = {}
  for (const name of TRAY_THEME_VARS) {
    const value = style.getPropertyValue(name).trim()
    if (value) vars[name] = value
  }
  return { dark: root.classList.contains('dark-theme'), vars }
}

function currentLocale(): string {
  return (i18n.global.locale as unknown as { value: string }).value
}

function displayCover(raw: string): string {
  if (!raw) return ''
  return peekCoverImage(raw) || normalizeCoverUrlForDisplay(raw)
}

/** 退出：先落盘再让后端结束进程（托盘「退出」与关闭即退出共用） */
export async function quitApp(flush: () => Promise<unknown>): Promise<void> {
  try {
    await flush()
  } catch (error) {
    log.warn('flush before quit failed:', summarizeLogError(error))
  }
  await invoke('quit_app')
}

export function installTrayBridge(options: TrayBridgeOptions): () => void {
  if (!isTauri()) return () => {}
  const player = usePlayerStore()
  let disposed = false
  let timer: ReturnType<typeof setTimeout> | null = null
  let lastSent = ''
  let releases: UnlistenFn[] = []

  function snapshot() {
    const texts: Record<string, string> = {}
    for (const key of MENU_TEXT_KEYS) texts[key] = i18n.global.t(`tray.${key}`)
    const track = player.hasPlaybackSession ? player.currentTrack : null
    return {
      locale: currentLocale(),
      texts,
      track: track
        ? { title: track.title || '', artist: track.artist || '', coverUrl: displayCover(getTrackCoverUrl(track)) }
        : null,
      theme: readTheme(),
      desktopLyricsOpen: desktopLyricsOpen.value,
    }
  }

  function publishNow() {
    timer = null
    if (disposed) return
    const next = snapshot()
    const key = JSON.stringify(next)
    if (key === lastSent) return
    lastSent = key
    void invoke('publish_tray_snapshot', { snapshot: next }).catch(error => {
      lastSent = ''
      log.warn('tray snapshot not published:', summarizeLogError(error))
    })
  }

  function schedule() {
    if (disposed || timer) return
    timer = setTimeout(publishNow, PUBLISH_DEBOUNCE_MS)
  }

  const stopWatch = watch(
    () => [
      player.hasPlaybackSession,
      player.currentTrack?.id,
      player.currentTrack?.title,
      player.currentTrack?.artist,
      getTrackCoverUrl(player.currentTrack),
      currentLocale(),
      desktopLyricsOpen.value,
    ],
    schedule,
  )
  // 深浅色切换改 html 的 class，主题色与动态取色写在 html 的行内样式上
  const observer = new MutationObserver(schedule)
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] })

  void Promise.all([
    listen('tray:open-now-playing', () => {
      if (player.hasPlaybackSession) options.openNowPlaying()
    }),
    listen('tray:toggle-desktop-lyrics', () => {
      const task = desktopLyricsOpen.value ? closeDesktopLyricsWindow() : openDesktopLyricsWindow()
      void task.catch(error => log.warn('desktop lyrics toggle failed:', summarizeLogError(error)))
    }),
    listen('tray:quit', () => {
      void quitApp(options.flushBeforeQuit).catch(error => {
        log.warn('quit failed:', summarizeLogError(error))
      })
    }),
  ]).then(list => {
    if (disposed) list.forEach(release => release())
    else releases = list
  }).catch(error => log.warn('tray listeners unavailable:', summarizeLogError(error)))

  publishNow()

  return () => {
    disposed = true
    if (timer) clearTimeout(timer)
    timer = null
    stopWatch()
    observer.disconnect()
    releases.forEach(release => release())
    releases = []
  }
}
