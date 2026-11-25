import { invoke, isTauri } from '@tauri-apps/api/core'
import { isMacPlatform } from '@/modules/shortcuts/platform'
import { createLogger } from '@/utils/logger'

// macOS：把自绘顶栏的实际高度（pt）告诉原生层，红绿灯据此垂直居中。
// 顶栏高度随界面缩放（--app-ui-scale）和全屏状态变化，都写在 html 的行内样式与类名上
const log = createLogger('traffic-lights')

export function titlebarHeightInPoints(cssHeight: number, uiScale: number): number {
  const scale = Number.isFinite(uiScale) && uiScale > 0 ? uiScale : 1
  return Math.round(cssHeight * scale * 2) / 2
}

export function installMacTrafficLightAlignment(root: HTMLElement = document.documentElement): () => void {
  if (!isTauri() || !isMacPlatform) return () => {}
  let sent = 0
  let frame = 0

  const sync = () => {
    frame = 0
    // 主题扩散动画期间不做布局测量，结束时类名变化会再触发一次
    if (root.classList.contains('theme-ripple-active')) return
    const bar = document.querySelector<HTMLElement>('.title-bar')
    if (!bar) return
    const scale = Number.parseFloat(root.style.getPropertyValue('--app-ui-scale') || '1')
    const height = titlebarHeightInPoints(bar.getBoundingClientRect().height, scale)
    if (!height || height === sent) return
    sent = height
    invoke('align_traffic_lights', { titlebarHeight: height }).catch(error => log.warn('align failed:', error))
  }
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(sync)
  }

  const observer = new MutationObserver(schedule)
  observer.observe(root, { attributes: true, attributeFilter: ['class', 'style'] })
  window.addEventListener('resize', schedule)
  schedule()
  return () => {
    observer.disconnect()
    window.removeEventListener('resize', schedule)
    if (frame) cancelAnimationFrame(frame)
  }
}
