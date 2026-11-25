import { ref, watch, type WatchStopHandle } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } from '@tauri-apps/api/window'
import { normalizeUiScale, UI_SCALE_DEFAULT, UI_SCALE_MIN, UI_SCALE_MAX, UI_SCALE_STEP } from './uiScaleRange'
import { isMacPlatform } from '@/modules/shortcuts/platform'
import { createLogger } from '@/utils/logger'

const log = createLogger('ui-scale')
export const uiScaleSupported = ref(false)
export const uiScaleError = ref(false)
export const uiScaleMax = ref(UI_SCALE_DEFAULT)
/** 实际生效的缩放；当前屏幕放不下用户设定值时会低于设置值，但设置本身不被改写 */
export const uiScaleEffective = ref(UI_SCALE_DEFAULT)

// 布局可用的最小逻辑尺寸：窄窗口由响应式布局接管（侧栏收成图标栏等），
// 所以这里只需保证核心控件不被裁掉
export const UI_LAYOUT_MIN_WIDTH = 720
export const UI_LAYOUT_MIN_HEIGHT = 540
const MAC_TITLEBAR_HEIGHT = 52

export function uiScaleLimitForSize(width: number, height: number, fixedTitlebarHeight = 0): number {
  const heightCapacity = height < UI_LAYOUT_MIN_HEIGHT
    ? (height - fixedTitlebarHeight) / (UI_LAYOUT_MIN_HEIGHT - fixedTitlebarHeight)
    : height / UI_LAYOUT_MIN_HEIGHT
  const capacity = Math.min(width / UI_LAYOUT_MIN_WIDTH, heightCapacity) * 100
  if (!Number.isFinite(capacity)) return UI_SCALE_MIN
  return Math.max(UI_SCALE_MIN, Math.min(UI_SCALE_MAX, Math.floor(capacity / UI_SCALE_STEP) * UI_SCALE_STEP))
}

/**
 * 主窗口界面缩放。
 * 只在用户改缩放或系统 DPI / 所在屏幕变化时调整原生窗口；普通的拖动与改尺寸不再触发
 * 自动缩放、改尺寸或挪位置，也绝不把为适配屏幕而降低的缩放写回设置
 * （此前在小屏上会把用户设置悄悄改成 80% 之类，表现为“字变小”“莫名其妙缩放了”）。
 */
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
  let minimumKey = ''
  let titlebarInset = 0
  let refreshTimer: ReturnType<typeof setTimeout> | undefined
  const appWindow = getCurrentWindow()
  const unlisten: Array<() => void> = []

  function minimumSize(factor: number, availableWidth: number, availableHeight: number) {
    // 最小尺寸不能超过屏幕可用区域，否则窗口会被系统强行顶出屏幕
    return {
      width: Math.min(availableWidth, UI_LAYOUT_MIN_WIDTH * factor),
      height: Math.min(availableHeight, UI_LAYOUT_MIN_HEIGHT * factor + Math.max(0, titlebarInset * (1 - factor))),
    }
  }

  function apply(value: number, userChanged: boolean): Promise<void> {
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
      uiScaleMax.value = Math.max(UI_SCALE_DEFAULT, uiScaleLimitForSize(availableWidth, availableHeight, titlebarInset))
      // 100% 永远可用：小屏靠响应式布局适配，不再强行缩小整个界面
      const percent = Math.min(normalizeUiScale(value), uiScaleMax.value)
      const factor = percent / 100
      const minimum = minimumSize(factor, availableWidth, availableHeight)
      const key = `${minimum.width}x${minimum.height}`
      if (key !== minimumKey) {
        await appWindow.setMinSize(new LogicalSize(minimum.width, minimum.height))
        if (!active) return
        minimumKey = key
      }
      // 只在用户主动调大缩放时把过小的窗口撑开并挪回屏幕内
      const width = Math.min(availableWidth, Math.max(inner.width / dpi, minimum.width))
      const height = Math.min(availableHeight, Math.max(inner.height / dpi, minimum.height))
      if (userChanged && !maximized && !fullscreen && (Math.abs(width - inner.width / dpi) >= 1 || Math.abs(height - inner.height / dpi) >= 1)) {
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
      uiScaleEffective.value = percent
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
        minimumKey = ''
      } catch (restoreError) {
        log.warn('Could not restore UI scaling constraints:', restoreError)
      }
      if (!active) return
      // 用户刚改的值没能生效时退回上一次成功的值，避免设置页显示一个并未生效的数字
      if (userChanged && settings.uiScale === value) settings.uiScale = appliedScale
    })
    return pending
  }

  function scheduleRefresh() {
    if (refreshTimer) clearTimeout(refreshTimer)
    refreshTimer = setTimeout(() => {
      refreshTimer = undefined
      void apply(settings.uiScale, false)
    }, 150)
  }

  await apply(settings.uiScale, false)
  const stop = watch(() => settings.uiScale, value => { void apply(value, true) })
  try {
    // 换屏 / DPI 变化时只重算最小尺寸与可用上限；userChanged=false 时绝不改窗口尺寸和位置
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
