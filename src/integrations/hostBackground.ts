import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

/// 宿主（窗口 + WebView2）底色跟随页面背景色：全屏播放页这类大图层撤下时，
/// 渲染帧没跟上的那一两帧透出的是宿主底色，默认白底在深色主题 / 自定义背景下就是一闪白
const SETTLE_MS = 120

function parseRgb(value: string): [number, number, number] | null {
  const match = value.match(/rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/i)
  if (match) return [Number(match[1]), Number(match[2]), Number(match[3])]
  const hex = value.trim().match(/^#([0-9a-f]{6})$/i)?.[1]
  return hex ? [0, 2, 4].map(offset => parseInt(hex.slice(offset, offset + 2), 16)) as [number, number, number] : null
}

export function installHostBackgroundSync(): () => void {
  if (!isTauri()) return () => {}
  const host = getCurrentWebviewWindow()
  const root = document.documentElement
  let applied = ''
  let timer: ReturnType<typeof setTimeout> | null = null

  const sync = () => {
    timer = null
    const rgb = parseRgb(getComputedStyle(root).getPropertyValue('--md-background'))
    if (!rgb) return
    const key = rgb.join(',')
    if (key === applied) return
    applied = key
    void host.setBackgroundColor([...rgb, 255]).catch(() => {})
  }
  const schedule = () => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(sync, SETTLE_MS)
  }

  // 主题、主题色、封面取色都写在 html 的类名和行内变量上
  const observer = new MutationObserver(schedule)
  observer.observe(root, { attributes: true, attributeFilter: ['class', 'style'] })
  sync()
  return () => {
    observer.disconnect()
    if (timer) clearTimeout(timer)
  }
}
