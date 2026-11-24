// 快捷键绑定：动作清单、默认键位、组合键字符串与按键事件的互转
//
// 组合键统一写成 "Mod+Shift+ArrowRight" 这类字符串：Mod 是主修饰键（macOS ⌘，其余 Ctrl），
// 存储只记与默认不同的项，空字符串表示用户主动清除
//
// 鼠标键复用同一套字符串语法，写成 "MouseBack" / "Mod+MouseBack"：
// 一个动作在每个作用域只对应一个键位，绑鼠标就顶替原来的键盘键位。
// 鼠标键不进系统快捷键加速器（toAccelerator 返回 null），全局鼠标走原生钩子。

import { isMacPlatform } from './platform'

export type ShortcutScope = 'local' | 'global'

export type ShortcutActionId =
  | 'toggle_play'
  | 'next'
  | 'previous'
  | 'seek_forward'
  | 'seek_back'
  | 'seek_forward_large'
  | 'seek_back_large'
  | 'volume_up'
  | 'volume_down'
  | 'mute'
  | 'shuffle'
  | 'repeat'
  | 'like'
  | 'now_playing'
  | 'desktop_lyrics'
  | 'show_window'
  | 'search'

export interface ShortcutActionMeta {
  id: ShortcutActionId
  icon: string
  /** 能否绑成系统级快捷键：搜索这类依赖窗口焦点的动作只在应用内有意义 */
  global: boolean
  /** 只在全局有意义（应用内按键时窗口本来就在前台） */
  globalOnly?: boolean
}

export const SHORTCUT_ACTIONS: ShortcutActionMeta[] = [
  { id: 'toggle_play', icon: 'play_pause', global: true },
  { id: 'previous', icon: 'skip_previous', global: true },
  { id: 'next', icon: 'skip_next', global: true },
  { id: 'seek_back', icon: 'replay_5', global: true },
  { id: 'seek_forward', icon: 'forward_5', global: true },
  { id: 'seek_back_large', icon: 'replay_30', global: true },
  { id: 'seek_forward_large', icon: 'forward_30', global: true },
  { id: 'volume_up', icon: 'volume_up', global: true },
  { id: 'volume_down', icon: 'volume_down', global: true },
  { id: 'mute', icon: 'volume_off', global: true },
  { id: 'shuffle', icon: 'shuffle', global: true },
  { id: 'repeat', icon: 'repeat', global: true },
  { id: 'like', icon: 'favorite', global: true },
  { id: 'desktop_lyrics', icon: 'subtitles', global: true },
  { id: 'now_playing', icon: 'expand_less', global: false },
  { id: 'search', icon: 'search', global: false },
  { id: 'show_window', icon: 'open_in_new', global: true, globalOnly: true },
]

export type ShortcutMap = Partial<Record<ShortcutActionId, string>>

/// 鼠标键：只用中键与两个侧键
///
/// 左右键留给正常界面操作（点击、拖拽、右键菜单），不做成快捷键，
/// 否则应用内的普通点击会全部变成命令。
export const MOUSE_KEYS = ['MouseMiddle', 'MouseBack', 'MouseForward'] as const
export type MouseKey = (typeof MOUSE_KEYS)[number]

const MOUSE_KEY_SET = new Set<string>(MOUSE_KEYS)

/// DOM MouseEvent.button：1 中键，3 后退侧键，4 前进侧键
const MOUSE_BUTTON_KEYS: Record<number, MouseKey> = {
  1: 'MouseMiddle',
  3: 'MouseBack',
  4: 'MouseForward',
}

export const DEFAULT_LOCAL_SHORTCUTS: ShortcutMap = {
  toggle_play: 'Space',
  next: 'Mod+ArrowRight',
  previous: 'Mod+ArrowLeft',
  seek_forward: 'ArrowRight',
  seek_back: 'ArrowLeft',
  seek_forward_large: 'Shift+ArrowRight',
  seek_back_large: 'Shift+ArrowLeft',
  volume_up: 'ArrowUp',
  volume_down: 'ArrowDown',
  mute: 'M',
  shuffle: 'S',
  repeat: 'R',
  like: 'L',
  desktop_lyrics: 'D',
  now_playing: 'Mod+P',
  search: 'Mod+F',
}

/// 全局默认参照主流桌面播放器的 Ctrl+Alt 组合，冲突概率低
export const DEFAULT_GLOBAL_SHORTCUTS: ShortcutMap = {
  toggle_play: 'Mod+Alt+P',
  previous: 'Mod+Alt+ArrowLeft',
  next: 'Mod+Alt+ArrowRight',
  volume_up: 'Mod+Alt+ArrowUp',
  volume_down: 'Mod+Alt+ArrowDown',
  like: 'Mod+Alt+L',
  desktop_lyrics: 'Mod+Alt+D',
  show_window: 'Mod+Alt+N',
}

export interface ShortcutBindings {
  local: ShortcutMap
  global: ShortcutMap
}

const ACTION_IDS = new Set<string>(SHORTCUT_ACTIONS.map(action => action.id))
const MODIFIER_ORDER = ['Mod', 'Ctrl', 'Alt', 'Shift', 'Meta'] as const
const MODIFIER_KEYS = new Set(['Control', 'Shift', 'Alt', 'Meta', 'OS', 'AltGraph', 'CapsLock', 'Fn'])
/** 不允许改绑：Escape 永远用来关弹层 / 退出录制，Tab 留给焦点导航 */
const RESERVED_KEYS = new Set(['Escape', 'Tab'])

/// 存储值 -> 规整后的覆盖表；未知动作、非法组合一律丢弃
export function normalizeShortcutBindings(raw: unknown): ShortcutBindings {
  const source = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  const pick = (value: unknown, scope: ShortcutScope): ShortcutMap => {
    const result: ShortcutMap = {}
    if (!value || typeof value !== 'object') return result
    for (const [id, combo] of Object.entries(value as Record<string, unknown>)) {
      if (!ACTION_IDS.has(id) || typeof combo !== 'string') continue
      if (scope === 'global' && !SHORTCUT_ACTIONS.find(action => action.id === id)?.global) continue
      if (scope === 'local' && SHORTCUT_ACTIONS.find(action => action.id === id)?.globalOnly) continue
      const normalized = combo === '' ? '' : normalizeCombo(combo)
      if (normalized !== null) result[id as ShortcutActionId] = normalized
    }
    return result
  }
  return { local: pick(source.local, 'local'), global: pick(source.global, 'global') }
}

/// 覆盖表叠加默认值得到实际生效的键位；空字符串表示未绑定
export function effectiveShortcuts(bindings: ShortcutBindings, scope: ShortcutScope): Record<ShortcutActionId, string> {
  const defaults = scope === 'local' ? DEFAULT_LOCAL_SHORTCUTS : DEFAULT_GLOBAL_SHORTCUTS
  const overrides = bindings[scope]
  const result = {} as Record<ShortcutActionId, string>
  for (const action of SHORTCUT_ACTIONS) {
    const allowed = scope === 'local' ? !action.globalOnly : action.global
    result[action.id] = allowed ? overrides[action.id] ?? defaults[action.id] ?? '' : ''
  }
  return result
}

/// 写回覆盖表：与默认相同就删掉该项，保持存储精简
export function withShortcut(bindings: ShortcutBindings, scope: ShortcutScope, id: ShortcutActionId, combo: string): ShortcutBindings {
  const defaults = scope === 'local' ? DEFAULT_LOCAL_SHORTCUTS : DEFAULT_GLOBAL_SHORTCUTS
  const next = { ...bindings[scope] }
  if ((defaults[id] ?? '') === combo) delete next[id]
  else next[id] = combo
  return { ...bindings, [scope]: next }
}

/// 同一作用域里被多个动作占用的组合键
export function conflictingActions(map: Record<ShortcutActionId, string>): Map<string, ShortcutActionId[]> {
  const byCombo = new Map<string, ShortcutActionId[]>()
  for (const [id, combo] of Object.entries(map) as [ShortcutActionId, string][]) {
    if (!combo) continue
    byCombo.set(combo, [...(byCombo.get(combo) ?? []), id])
  }
  return new Map([...byCombo].filter(([, ids]) => ids.length > 1))
}

const NAMED_KEYS = new Map(
  [
    'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Space', 'Enter', 'Backspace', 'Delete', 'Insert',
    'Home', 'End', 'PageUp', 'PageDown', 'Escape', 'Tab', 'CapsLock', 'Control', 'Shift', 'Alt', 'Meta',
    ...Array.from({ length: 24 }, (_, index) => `F${index + 1}`),
  ].map(name => [name.toLowerCase(), name]),
)
const KEY_ALIASES: Record<string, string> = {
  ' ': 'Space', spacebar: 'Space', left: 'ArrowLeft', right: 'ArrowRight', up: 'ArrowUp', down: 'ArrowDown',
  esc: 'Escape', del: 'Delete',
}

function normalizeKeyName(key: string): string | null {
  if (!key) return null
  const alias = KEY_ALIASES[key.toLowerCase()] ?? KEY_ALIASES[key]
  if (alias) return alias
  if (key.length === 1) return key.toUpperCase()
  return NAMED_KEYS.get(key.toLowerCase()) ?? key
}

const MODIFIER_NAMES = new Map(MODIFIER_ORDER.map(name => [name.toLowerCase(), name]))

function normalizeCombo(combo: string): string | null {
  const parts = combo.split('+').map(part => part.trim()).filter(Boolean)
  if (!parts.length) return null
  // 末尾是 "+" 本身（Mod++）时 split 会丢掉它
  if (combo.endsWith('++')) parts.push('+')
  const rawKey = parts.pop()!
  // 鼠标键直接用规范名，不走键盘的键名规整
  const mouseKey = MOUSE_KEY_SET.has(rawKey)
    ? rawKey
    : MOUSE_KEYS.find(key => key.toLowerCase() === rawKey.toLowerCase())
  const key = mouseKey ?? normalizeKeyName(rawKey)
  if (!key || MODIFIER_KEYS.has(key) || RESERVED_KEYS.has(key)) return null
  const modifiers = new Set<string>()
  for (const part of parts) {
    const name = MODIFIER_NAMES.get(part.toLowerCase())
    if (!name) return null
    modifiers.add(name)
  }
  return [...MODIFIER_ORDER.filter(modifier => modifiers.has(modifier)), key].join('+')
}

/// 按键事件 -> 组合键；纯修饰键、保留键返回 null
export function comboFromEvent(event: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(event.key)) return null
  // macOS 上 Option 会把字母变成特殊符号（Alt+P = π），字母数字改从物理键位取
  let key = event.key
  if (/^Key[A-Z]$/.test(event.code)) key = event.code.slice(3)
  else if (/^Digit\d$/.test(event.code)) key = event.code.slice(5)
  // Windows 输入法可能将 Ctrl+Space 的 key 改为 Process，空格键改从物理键位取
  else if (event.code === 'Space') key = 'Space'
  const name = normalizeKeyName(key)
  if (!name || RESERVED_KEYS.has(name)) return null
  const primary = isMacPlatform ? event.metaKey : event.ctrlKey
  const parts: string[] = []
  if (primary) parts.push('Mod')
  if (isMacPlatform && event.ctrlKey) parts.push('Ctrl')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  if (!isMacPlatform && event.metaKey) parts.push('Meta')
  return [...parts, name].join('+')
}

const KEY_LABELS: Record<string, string> = {
  ArrowLeft: '←', ArrowRight: '→', ArrowUp: '↑', ArrowDown: '↓',
  Space: 'Space', Enter: 'Enter', Backspace: '⌫', Delete: 'Del',
  PageUp: 'PgUp', PageDown: 'PgDn',
  MouseMiddle: '中键', MouseBack: '后退键', MouseForward: '前进键',
}

/// 展示用的按键分段
export function comboLabelParts(combo: string): string[] {
  if (!combo) return []
  return combo.split('+').map((part, index, parts) => {
    if (index === parts.length - 1) return KEY_LABELS[part] ?? part
    if (part === 'Mod') return isMacPlatform ? '⌘' : 'Ctrl'
    if (part === 'Alt') return isMacPlatform ? '⌥' : 'Alt'
    if (part === 'Shift') return isMacPlatform ? '⇧' : 'Shift'
    if (part === 'Ctrl') return isMacPlatform ? '⌃' : 'Ctrl'
    if (part === 'Meta') return 'Win'
    return part
  })
}

const ACCELERATOR_KEYS: Record<string, string> = {
  '=': 'Equal', '-': 'Minus', ',': 'Comma', '.': 'Period', '/': 'Slash', ';': 'Semicolon',
  "'": 'Quote', '[': 'BracketLeft', ']': 'BracketRight', '\\': 'Backslash', '`': 'Backquote',
}

/// 组合键 -> Tauri 全局快捷键加速器字符串；无法表达的键返回 null
///
/// 鼠标键无法表达成加速器（插件的语法里没有鼠标），全局鼠标走原生钩子。
export function toAccelerator(combo: string): string | null {
  if (!combo) return null
  const parts = combo.split('+')
  const key = parts.pop()!
  if (MOUSE_KEY_SET.has(key)) return null
  const mapped = ACCELERATOR_KEYS[key] ?? key
  if (!/^([A-Z0-9]|F([1-9]|1\d|2[0-4])|Arrow(Left|Right|Up|Down)|Space|Enter|Backspace|Delete|Insert|Home|End|PageUp|PageDown|Equal|Minus|Comma|Period|Slash|Semicolon|Quote|BracketLeft|BracketRight|Backslash|Backquote)$/.test(mapped)) {
    return null
  }
  const modifiers = parts.map(part => ({ Mod: 'CommandOrControl', Ctrl: 'Control', Alt: 'Alt', Shift: 'Shift', Meta: 'Super' }[part] ?? part))
  return [...modifiers, mapped].join('+')
}

/// 组合键里是否带鼠标键
export function isMouseCombo(combo: string): boolean {
  if (!combo) return false
  const parts = combo.split('+')
  return MOUSE_KEY_SET.has(parts.pop() ?? '')
}

/// 组合键的主键部分：去掉修饰键后的鼠标键名，键盘键返回 null
export function mouseKeyOfCombo(combo: string): MouseKey | null {
  if (!combo) return null
  const parts = combo.split('+')
  const key = parts.pop() ?? ''
  return MOUSE_KEY_SET.has(key) ? (key as MouseKey) : null
}

/// 鼠标事件 -> 组合键；只认中键与侧键，左右键返回 null
export function comboFromMouseEvent(event: MouseEvent): string | null {
  const key = MOUSE_BUTTON_KEYS[event.button]
  if (!key) return null
  const primary = isMacPlatform ? event.metaKey : event.ctrlKey
  const parts: string[] = []
  if (primary) parts.push('Mod')
  if (isMacPlatform && event.ctrlKey) parts.push('Ctrl')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  if (!isMacPlatform && event.metaKey) parts.push('Meta')
  return [...parts, key].join('+')
}
