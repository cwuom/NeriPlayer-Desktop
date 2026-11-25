import { isEditableTarget, isMacPlatform } from '@/modules/shortcuts/platform'

// 桌面端习惯的前进/后退：鼠标侧键（button 3/4）、Alt+←/→，macOS 另支持 ⌘[ / ⌘]
export interface HistoryNavigationActions {
  back: () => void
  forward: () => void
}

export function historyDirectionFromKey(event: KeyboardEvent, mac = isMacPlatform): -1 | 1 | 0 {
  if (event.isComposing || event.repeat) return 0
  const altOnly = event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey
  if (altOnly && event.key === 'ArrowLeft') return -1
  if (altOnly && event.key === 'ArrowRight') return 1
  const cmdOnly = mac && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
  if (cmdOnly && event.key === '[') return -1
  if (cmdOnly && event.key === ']') return 1
  return 0
}

export function installHistoryNavigation(actions: HistoryNavigationActions, target: Window = window): () => void {
  const onMouseUp = (event: MouseEvent) => {
    if (event.button !== 3 && event.button !== 4) return
    event.preventDefault()
    if (event.button === 3) actions.back()
    else actions.forward()
  }
  // 侧键在部分 WebView 上还会触发自带的历史导航，按下时就拦掉
  const onMouseDown = (event: MouseEvent) => {
    if (event.button === 3 || event.button === 4) event.preventDefault()
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || isEditableTarget(event.target)) return
    const direction = historyDirectionFromKey(event)
    if (!direction) return
    event.preventDefault()
    if (direction < 0) actions.back()
    else actions.forward()
  }
  target.addEventListener('mousedown', onMouseDown)
  target.addEventListener('mouseup', onMouseUp)
  target.addEventListener('keydown', onKeyDown)
  return () => {
    target.removeEventListener('mousedown', onMouseDown)
    target.removeEventListener('mouseup', onMouseUp)
    target.removeEventListener('keydown', onKeyDown)
  }
}
