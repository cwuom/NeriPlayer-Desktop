import { ref, watch, type WatchStopHandle } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } from '@tauri-apps/api/window'
import { normalizeUiScale, UI_SCALE_DEFAULT, UI_SCALE_MIN, UI_SCALE_MAX, UI_SCALE_STEP } from '@/stores/settings'
import { isMacPlatform } from '@/modules/shortcuts/platform'
import { createLogger } from '@/utils/logger'

const log = createLogger('ui-scale')
export const uiScaleSupported = ref(false)
export const uiScaleError = ref(false)
export const uiScaleMax = ref(UI_SCALE_DEFAULT)

export const UI_LAYOUT_MIN_WIDTH = 960
export const UI_LAYOUT_MIN_HEIGHT = 720
const MAC_TITLEBAR_HEIGHT = 52

export function uiScaleLimitForSize(width: number, height: number, fixedTitlebarHeight = 0): number {
  const heightCapacity = height < UI_LAYOUT_MIN_HEIGHT
    ? (height - fixedTitlebarHeight) / (UI_LAYOUT_MIN_HEIGHT - fixedTitlebarHeight)
    : height / UI_LAYOUT_MIN_HEIGHT
  const capacity = Math.min(width / UI_LAYOUT_MIN_WIDTH, heightCapacity) * 100
  if (!Number.isFinite(capacity)) return UI_SCALE_MIN
  return Math.max(UI_SCALE_MIN, Math.min(UI_SCALE_MAX, Math.floor(capacity / UI_SCALE_STEP) * UI_SCALE_STEP))
}

export async function installUiScale(settings: { uiScale: number }): Promise<WatchStopHandle> {
  if (!isTauri()) return () => {}
  try {
    // 旧版 macOS 没有页面缩放 API，调用之前先由原生层检查
    uiScaleSupported.value = await invoke<boolean>('ui_scale_supported')
  } catch (error) {
    uiScaleError.value = true
    log.warn('Could not check UI scaling support:', error)
    return () => {}
  }
  if (!uiScaleSupported.value) return () => {}

  let active = true
  let appliedScale = UI_SCALE_DEFAULT
  let hasAppliedScale = false
  let pending = Promise.resolve()
  let minimumScale = 0
  let minimumDpi = 0
  let minimumInset = -1
  let titlebarInset = 0
  let refreshTimer: ReturnType<typeof setTimeout> | undefined
  const appWindow = getCurrentWindow()
  const unlisten: Array<() => void> = []

  function apply(value: number): Promise<void> {
    pending = pending.then(async () => {
      if (!active) return
      const [inner, outer, dpi, monitor, maximized, fullscreen] = await Promise.all([
        appWindow.innerSize(), appWindow.outerSize(), appWindow.scaleFactor(), currentMonitor(), appWindow.isMaximized(), appWindow.isFullscreen(),
      ])
      if (!active || inner.width <= 0 || inner.height <= 0 || dpi <= 0) return
      const frameWidth = Math.max(0, outer.width - inner.width)
      const frameHeight = Math.max(0, outer.height - inner.height)
      const availableWidth = monitor ? (monitor.workArea.size.width - frameWidth) / dpi : inner.width / dpi
      const availableHeight = monitor ? (monitor.workArea.size.height - frameHeight) / dpi : inner.height / dpi
      titlebarInset = isMacPlatform && !fullscreen ? MAC_TITLEBAR_HEIGHT : 0
      uiScaleMax.value = uiScaleLimitForSize(availableWidth, availableHeight, titlebarInset)
      const percent = Math.min(normalizeUiScale(value), uiScaleMax.value)
      if (settings.uiScale === value && value !== percent) settings.uiScale = percent
      const factor = percent / 100
      const minWidth = UI_LAYOUT_MIN_WIDTH * factor
      const minHeight = UI_LAYOUT_MIN_HEIGHT * factor + Math.max(0, titlebarInset * (1 - factor))
      // 缩放后仍保留完整布局，禁止把原生窗口拖小到控件之外
      if (minimumScale !== percent || minimumDpi !== dpi || minimumInset !== titlebarInset) {
        await appWindow.setMinSize(new LogicalSize(minWidth, minHeight))
        if (!active) return
        minimumScale = percent
        minimumDpi = dpi
        minimumInset = titlebarInset
      }
      const width = Math.min(availableWidth, Math.max(inner.width / dpi, minWidth))
      const height = Math.min(availableHeight, Math.max(inner.height / dpi, minHeight))
      if (!maximized && !fullscreen && (Math.abs(width - inner.width / dpi) >= 1 || Math.abs(height - inner.height / dpi) >= 1)) {
        await appWindow.setSize(new LogicalSize(width, height))
        if (!active) return
        if (monitor) {
          const position = await appWindow.outerPosition()
          if (!active) return
          const left = monitor.workArea.position.x
          const top = monitor.workArea.position.y
          const x = Math.max(left, Math.min(position.x, left + monitor.workArea.size.width - width * dpi - frameWidth))
          const y = Math.max(top, Math.min(position.y, top + monitor.workArea.size.height - height * dpi - frameHeight))
          if (x !== position.x || y !== position.y) await appWindow.setPosition(new PhysicalPosition(x, y))
          if (!active) return
        }
      }
      if (hasAppliedScale && percent === appliedScale) return
      await getCurrentWebview().setZoom(factor)
      if (!active) return
      appliedScale = percent
      hasAppliedScale = true
      document.documentElement.style.setProperty('--app-ui-scale', String(factor))
      uiScaleError.value = false
    }).catch(async error => {
      if (!active) return
      uiScaleError.value = true
      log.warn('Could not apply UI scale:', error)
      try {
        const factor = appliedScale / 100
        await appWindow.setMinSize(new LogicalSize(UI_LAYOUT_MIN_WIDTH * factor, UI_LAYOUT_MIN_HEIGHT * factor + Math.max(0, titlebarInset * (1 - factor))))
        if (!active) return
        minimumScale = appliedScale
        minimumDpi = 0
        minimumInset = -1
      } catch (restoreError) {
        log.warn('Could not restore UI scaling constraints:', restoreError)
      }
      if (!active) return
      if (settings.uiScale === value) settings.uiScale = appliedScale
    })
    return pending
  }

  function scheduleRefresh() {
    if (refreshTimer) clearTimeout(refreshTimer)
    refreshTimer = setTimeout(() => {
      refreshTimer = undefined
      void apply(settings.uiScale)
    }, 100)
  }

  await apply(settings.uiScale)
  const stop = watch(() => settings.uiScale, value => { void apply(value) })
  try {
    unlisten.push(await appWindow.onResized(scheduleRefresh))
    unlisten.push(await appWindow.onMoved(scheduleRefresh))
    unlisten.push(await appWindow.onScaleChanged(scheduleRefresh))
  } catch (error) {
    uiScaleError.value = true
    log.warn('Could not watch UI scaling constraints:', error)
  }
  return () => {
    active = false
    if (refreshTimer) clearTimeout(refreshTimer)
    for (const release of unlisten) release()
    stop()
  }
}
