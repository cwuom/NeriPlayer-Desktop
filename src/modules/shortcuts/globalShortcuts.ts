// 应用内键盘快捷键（窗口在前台时生效），键位来自设置页可改的绑定表
//
// 设计约束：
// - 输入框获得焦点时一律让位，避免打字被当成命令
// - 有原生对话框/弹层时只保留播放/暂停与 Escape，其余交给弹层自己处理
// - 设置页录制组合键期间完全让位
// - 所有处理过的按键都 preventDefault，避免页面被空格滚动
//
// 鼠标键（中键 / 侧键）走同一套绑定表：绑了鼠标键的动作在应用内用鼠标触发，
// 没有绑鼠标键的动作不受影响。左右键不做成快捷键。

import { comboFromEvent, comboFromMouseEvent, effectiveShortcuts, type ShortcutActionId, type ShortcutBindings } from './bindings'
import { hasNativeMouseHook, isEditableTarget } from './platform'
import { shortcutRecording } from './recording'

export interface ShortcutActions {
  togglePlay: () => void
  next: () => void
  previous: () => void
  seekBy: (deltaMs: number) => void
  adjustVolume: (delta: number) => void
  toggleMute: () => void
  toggleNowPlaying: () => void
  closeOverlay: () => boolean
  focusSearch: () => void
  toggleShuffle: () => void
  cycleRepeat: () => void
  toggleLike: () => void
  toggleDesktopLyrics: () => void
  showWindow: () => void
  isOverlayOpen: () => boolean
}

const SEEK_STEP_MS = 5_000
const SEEK_STEP_LARGE_MS = 30_000
const VOLUME_STEP = 0.05

/// 应用内与系统级快捷键共用的动作分发
export function runShortcutAction(id: ShortcutActionId, actions: ShortcutActions): void {
  switch (id) {
    case 'toggle_play': return actions.togglePlay()
    case 'next': return actions.next()
    case 'previous': return actions.previous()
    case 'seek_forward': return actions.seekBy(SEEK_STEP_MS)
    case 'seek_back': return actions.seekBy(-SEEK_STEP_MS)
    case 'seek_forward_large': return actions.seekBy(SEEK_STEP_LARGE_MS)
    case 'seek_back_large': return actions.seekBy(-SEEK_STEP_LARGE_MS)
    case 'volume_up': return actions.adjustVolume(VOLUME_STEP)
    case 'volume_down': return actions.adjustVolume(-VOLUME_STEP)
    case 'mute': return actions.toggleMute()
    case 'shuffle': return actions.toggleShuffle()
    case 'repeat': return actions.cycleRepeat()
    case 'like': return actions.toggleLike()
    case 'now_playing': return actions.toggleNowPlaying()
    case 'desktop_lyrics': return actions.toggleDesktopLyrics()
    case 'show_window': return actions.showWindow()
    case 'search': return actions.focusSearch()
  }
}

export function installGlobalShortcuts(
  actions: ShortcutActions,
  bindings: () => ShortcutBindings,
  isGlobalEnabled: () => boolean = () => false,
): () => void {
  function handle(event: KeyboardEvent) {
    if (event.defaultPrevented || shortcutRecording.value) return

    // Escape 是唯一在输入态也要响应的键：先让输入框失焦，再关弹层
    if (event.key === 'Escape') {
      if (event.repeat) return
      if (isEditableTarget(event.target)) {
        ;(event.target as HTMLElement).blur()
        event.preventDefault()
        return
      }
      if (actions.closeOverlay()) event.preventDefault()
      return
    }

    if (isEditableTarget(event.target)) return
    const combo = comboFromEvent(event)
    if (!combo) return
    const map = effectiveShortcuts(bindings(), 'local')
    const id = (Object.keys(map) as ShortcutActionId[]).find(action => map[action] === combo)
    if (!id) return
    // 长按只连续调音量 / 快进退，切歌、开关类动作不跟着重复触发
    if (event.repeat && !['volume_up', 'volume_down', 'seek_forward', 'seek_back'].includes(id)) {
      event.preventDefault()
      return
    }
    // 弹层打开时不劫持其它播放键，交给弹层内部交互
    if (actions.isOverlayOpen() && id !== 'toggle_play') return
    runShortcutAction(id, actions)
    event.preventDefault()
  }

  /// 鼠标键（中键 / 侧键）触发绑定了鼠标键的动作
  function handleMouse(event: MouseEvent) {
    if (event.defaultPrevented || shortcutRecording.value) return
    if (event.button === 0 || event.button === 2) return
    // 输入控件里的中键在 X11 下会粘贴剪贴板内容，别抢
    if (isEditableTarget(event.target)) return
    const combo = comboFromMouseEvent(event)
    if (!combo) return
    const map = effectiveShortcuts(bindings(), 'local')
    const id = (Object.keys(map) as ShortcutActionId[]).find(action => map[action] === combo)
    if (!id) return
    // 同一个鼠标键同时绑了应用内和全局时，全局由原生钩子负责：
    // 原生钩子在本进程窗口在前台时也会上报，这里让位避免触发两次。
    // 其它平台没有原生钩子，全局鼠标键其实只在应用内生效，不能因此把动作跳掉。
    if (isGlobalEnabled() && hasNativeMouseHook) {
      const globalMap = effectiveShortcuts(bindings(), 'global')
      const globalId = (Object.keys(globalMap) as ShortcutActionId[]).find(
        action => globalMap[action] === combo,
      )
      if (globalId) return
    }
    if (actions.isOverlayOpen() && id !== 'toggle_play') return
    runShortcutAction(id, actions)
    // 中键默认会打开浏览器后退之类的行为，拦掉
    event.preventDefault()
  }

  window.addEventListener('keydown', handle)
  window.addEventListener('mousedown', handleMouse)
  return () => {
    window.removeEventListener('keydown', handle)
    window.removeEventListener('mousedown', handleMouse)
  }
}
