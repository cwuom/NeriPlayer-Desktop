// 系统级全局快捷键：窗口最小化 / 在托盘里也能控制播放
//
// 键盘走 tauri-plugin-global-shortcut；鼠标键插件的加速器语法表达不了，
// 由 Windows 原生钩子上报，前端再按全局绑定表分发动作。
import { ref } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { register, unregisterAll } from '@tauri-apps/plugin-global-shortcut'
import { createLogger } from '@/utils/logger'
import {
  effectiveShortcuts,
  isMouseCombo,
  toAccelerator,
  type ShortcutActionId,
  type ShortcutBindings,
} from './bindings'
import { runShortcutAction, type ShortcutActions } from './globalShortcuts'

const log = createLogger('shortcuts')

/// 注册失败（多半被其它程序占用）的动作，设置页据此逐项提示
export const systemShortcutFailures = ref<ShortcutActionId[]>([])

let generation = 0

/// 全局鼠标键事件名，与 Rust 侧 mouse_hook 保持一致
const MOUSE_EVENT = 'shortcut:mouse-button'

/// 把全局绑定表里的鼠标组合键下发给原生钩子；没有鼠标绑定时传空列表
async function applyGlobalMouseShortcuts(bindings: ShortcutBindings, enabled: boolean): Promise<void> {
  if (!isTauri()) return
  const map = effectiveShortcuts(bindings, 'global')
  const combos: string[] = []
  for (const combo of Object.values(map)) {
    if (!combo || !isMouseCombo(combo)) continue
    // 同一个组合只下发一次
    if (!combos.includes(combo)) combos.push(combo)
  }
  try {
    // 全局鼠标开关关闭时清空绑定，钩子只上报被绑定的组合
    const applied = await invoke<string[]>('set_global_mouse_shortcuts', {
      buttons: enabled ? combos : [],
    })
    if (!enabled && applied.length) {
      log.warn('global mouse shortcuts should be cleared but the native side kept some')
    }
  } catch (error) {
    log.warn('applying global mouse shortcuts failed:', error)
  }
}

export async function applySystemShortcuts(
  bindings: ShortcutBindings,
  enabled: boolean,
  actions: ShortcutActions,
): Promise<void> {
  if (!isTauri()) return
  const current = ++generation
  try {
    await unregisterAll()
  } catch (error) {
    log.warn('unregister global shortcuts failed:', error)
  }
  if (current !== generation) return
  if (!enabled) {
    systemShortcutFailures.value = []
    await applyGlobalMouseShortcuts(bindings, false)
    return
  }
  const failed: ShortcutActionId[] = []
  const taken = new Set<string>()
  for (const [id, combo] of Object.entries(effectiveShortcuts(bindings, 'global')) as [ShortcutActionId, string][]) {
    if (!combo) continue
    // 鼠标键不走插件注册，交给原生钩子
    if (isMouseCombo(combo)) continue
    const accelerator = toAccelerator(combo)
    // 同一组合只注册给第一个动作，重复的在设置页已标成冲突
    if (!accelerator || taken.has(accelerator)) {
      if (!accelerator) failed.push(id)
      continue
    }
    taken.add(accelerator)
    try {
      await register(accelerator, (event) => {
        if (event.state === 'Pressed') runShortcutAction(id, actions)
      })
    } catch (error) {
      failed.push(id)
      log.warn(`global shortcut ${accelerator} unavailable:`, error)
    }
    if (current !== generation) return
  }
  systemShortcutFailures.value = failed
  await applyGlobalMouseShortcuts(bindings, true)
}

/// 监听原生钩子上报的鼠标组合键，按全局绑定表分发动作
export function listenGlobalMouseShortcuts(
  actions: ShortcutActions,
  bindings: () => ShortcutBindings,
): () => void {
  if (!isTauri()) return () => {}
  let disposed = false
  const unlisten = listen<string>(MOUSE_EVENT, event => {
    if (disposed) return
    const map = effectiveShortcuts(bindings(), 'global')
    // 原生侧上报的就是命中的组合键字符串，直接比对
    const id = (Object.keys(map) as ShortcutActionId[]).find(action => {
      const combo = map[action]
      return !!combo && isMouseCombo(combo) && combo === event.payload
    })
    if (!id) return
    runShortcutAction(id, actions)
  })
  return () => {
    disposed = true
    void unlisten.then(stop => stop())
  }
}

export async function clearSystemShortcuts(): Promise<void> {
  generation++
  if (!isTauri()) return
  await unregisterAll().catch(() => {})
}
