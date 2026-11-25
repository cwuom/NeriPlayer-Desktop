// 主题色管理：强调色预设 + Clean 中性表面（见 cleanScheme.ts）。
// Material You 的整套色调梯度已废弃：强调色只落在交互角色上，表面统一由 buildCleanScheme 生成。
import { detectPlatform } from '@/utils/platform'
import { runThemeCrossfade } from '@/utils/theme/crossfade'
import {
  BACKGROUND_TINT_DEFAULT,
  BACKGROUND_TINT_STRENGTH_DEFAULT,
  buildCleanScheme,
  clampTintStrength,
  normalizeBackgroundTint,
  rgb,
  type RGB,
  type TintConfig,
} from '@/utils/theme/cleanScheme'

// WebKitGTK 的 View Transition 会整页快照合成，在 Linux 上曾导致
// GTK 主循环内 WebKit 回调 SEGV 崩溃；Linux 走平滑过渡降级路径
const VIEW_TRANSITION_SUPPORTED = detectPlatform() !== 'linux'

export interface ThemeColorScheme {
  key: string
  /** 浅色底上使用的强调色 */
  light: RGB
  /** 深色底上使用的强调色（更亮，保证对比度） */
  dark: RGB
}

// 键名保持不变以兼容已保存的设置；色值改为 Clean 风格的饱和强调色
export const THEME_COLORS: ThemeColorScheme[] = [
  { key: 'purple', light: [104, 84, 204], dark: [172, 150, 255] },
  { key: 'blue', light: [10, 106, 222], dark: [102, 168, 255] },
  { key: 'teal', light: [0, 128, 122], dark: [70, 204, 188] },
  { key: 'green', light: [28, 136, 64], dark: [96, 208, 128] },
  { key: 'olive', light: [104, 112, 18], dark: [196, 204, 96] },
  { key: 'orange', light: [200, 92, 0], dark: [255, 166, 82] },
  { key: 'rose', light: [204, 44, 100], dark: [255, 130, 168] },
  { key: 'brown', light: [140, 90, 60], dark: [222, 168, 132] },
  { key: 'graphite', light: [44, 44, 48], dark: [232, 232, 236] },
]

const DEFAULT_THEME_COLOR = 'purple'

let tintConfig: TintConfig = { tint: BACKGROUND_TINT_DEFAULT, strength: BACKGROUND_TINT_STRENGTH_DEFAULT }
// 最近一次应用的强调色：背景偏色变化时据此原地重算，不必知道强调色来自预设还是动态取色
let currentAccent: { accent: RGB; dark: boolean } | null = null

// 非 ripple 路径的平滑过渡计时器：连点时重置，防止过渡类被上一次的定时器提前摘除
let smoothTransitionTimer: number | undefined

/**
 * 非 ripple 路径的主题平滑过渡：在改 CSS 变量 / 深浅色类之前调用，
 * 给根元素加 theme-switching（global.scss 中让颜色类属性过渡 300ms），
 * 350ms 后移除。ripple（View Transition）期间由 theme-ripple-active
 * 统一禁用颜色过渡以防圆外提前变色，此处跳过避免冲突
 */
export function beginSmoothThemeTransition() {
  const root = document.documentElement
  if (root.classList.contains('theme-ripple-active')) return
  root.classList.add('theme-switching')
  if (smoothTransitionTimer !== undefined) window.clearTimeout(smoothTransitionTimer)
  smoothTransitionTimer = window.setTimeout(() => {
    document.documentElement.classList.remove('theme-switching')
    smoothTransitionTimer = undefined
  }, 350)
}

/** ripple 启动前取消平滑过渡，避免两套过渡机制在同一帧竞争 */
export function cancelSmoothThemeTransition() {
  if (smoothTransitionTimer !== undefined) {
    window.clearTimeout(smoothTransitionTimer)
    smoothTransitionTimer = undefined
  }
  document.documentElement.classList.remove('theme-switching')
}

export function findThemeColor(key: string): ThemeColorScheme | undefined {
  return THEME_COLORS.find(c => c.key === key)
}

/** 获取色块的展示色 */
export function getSwatchColor(key: string, dark = false): string {
  const scheme = findThemeColor(key) ?? findThemeColor(DEFAULT_THEME_COLOR)!
  return rgb(dark ? scheme.dark : scheme.light)
}

/** 把强调色与当前背景偏色展开为整套令牌写到 :root 行内样式 */
export function applyAccent(accent: RGB, dark: boolean) {
  const root = document.documentElement.style
  for (const [prop, value] of Object.entries(buildCleanScheme(accent, dark, tintConfig))) {
    root.setProperty(prop, value)
  }
  root.setProperty('--md-seed', `${accent[0]}, ${accent[1]}, ${accent[2]}`)
  currentAccent = { accent, dark }
}

export function getBackgroundTint(): TintConfig {
  return { ...tintConfig }
}

/** 更新背景偏色并用当前强调色重算表面（带平滑过渡） */
export function setBackgroundTint(config: { tint: unknown; strength: unknown }, animate = true) {
  const next: TintConfig = {
    tint: normalizeBackgroundTint(config.tint),
    strength: clampTintStrength(config.strength),
  }
  if (next.tint === tintConfig.tint && next.strength === tintConfig.strength) return
  tintConfig = next
  if (!currentAccent) return
  if (animate) beginSmoothThemeTransition()
  applyAccent(currentAccent.accent, currentAccent.dark)
}

/**
 * 仅视觉应用主题色 CSS 变量（不持久化）
 * @param isDark 直接传入避免 classList 读取导致强制 reflow
 */
export function applyThemeColorVisual(key: string, isDark?: boolean) {
  const scheme = findThemeColor(key)
  if (!scheme) return
  // 直接使用传入的 isDark，避免在 startViewTransition 回调内
  // 读取 classList（写后读会触发强制 reflow，是卡顿的主因）
  const dark = isDark ?? !document.documentElement.classList.contains('light-theme')
  applyAccent(dark ? scheme.dark : scheme.light, dark)
}

/** 应用主题色到 CSS 变量（含持久化）。非 ripple 路径，套 300ms 平滑过渡 */
export function applyThemeColor(key: string, isDark?: boolean, persist = true) {
  beginSmoothThemeTransition()
  applyThemeColorVisual(key, isDark)
  if (persist) localStorage.setItem('theme-color', key)
}

/**
 * 带圆形扩散动画的主题色切换
 * 参考 Android 端 pending state 模式：先切换视觉 -> 异步持久化
 */
export async function switchThemeColorWithRipple(key: string, x: number, y: number, persist = true) {
  if (!document.startViewTransition || !VIEW_TRANSITION_SUPPORTED) {
    applyThemeColor(key, undefined, persist)
    return
  }

  // 预计算当前 dark/light 状态，避免 transition 回调中读 classList；x / y 仅为兼容调用方保留
  const isDark = !document.documentElement.classList.contains('light-theme')
  // 交叉淡化与平滑过渡互斥：清掉可能残留的 theme-switching
  cancelSmoothThemeTransition()
  // 异步持久化，不阻塞动画关键路径
  if (persist) queueMicrotask(() => localStorage.setItem('theme-color', key))
  await runThemeCrossfade(() => applyThemeColorVisual(key, isDark))
}

/** 获取已保存的主题色 */
export function getSavedThemeColor(): string {
  const saved = localStorage.getItem('theme-color')
  return saved && findThemeColor(saved) ? saved : DEFAULT_THEME_COLOR
}

/** 关闭动态取色时恢复到已保存的预设主题色（仅视觉，不持久化） */
export function clearDynamicThemeColor(isDark?: boolean) {
  applyThemeColorVisual(getSavedThemeColor(), isDark)
}
