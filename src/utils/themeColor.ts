// 主题色管理：点击色块实时切换全局 CSS 变量
// 色值参考 Material Theme Builder (material-foundation/material-color-utilities) 生成
import { detectPlatform } from '@/modules/shortcuts/platform'

// WebKitGTK 的 View Transition 会整页快照合成，在 Linux 上曾导致
// GTK 主循环内 WebKit 回调 SEGV 崩溃；Linux 走平滑过渡降级路径
const VIEW_TRANSITION_SUPPORTED = detectPlatform() !== 'linux'

export interface ThemeColorScheme {
  key: string
  seed: string
  dark: Record<string, string>
  light: Record<string, string>
}

// 预设主题色：light palette 使用 M3 标准淡色调
export const THEME_COLORS: ThemeColorScheme[] = [
  {
    key: 'purple',
    seed: '103, 80, 164',
    dark: {
      '--md-primary': 'rgb(208, 188, 255)',
      '--md-on-primary': 'rgb(56, 30, 114)',
      '--md-primary-container': 'rgb(79, 55, 139)',
      '--md-on-primary-container': 'rgb(234, 221, 255)',
      '--md-secondary': 'rgb(204, 194, 220)',
      '--md-secondary-container': 'rgb(74, 68, 88)',
      '--md-tertiary': 'rgb(239, 184, 200)',
      '--md-tertiary-container': 'rgb(99, 59, 72)',
      '--md-inverse-primary': 'rgb(103, 80, 164)',
    },
    light: {
      '--md-primary': 'rgb(101, 78, 163)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(233, 221, 255)',
      '--md-on-primary-container': 'rgb(34, 0, 94)',
      '--md-secondary': 'rgb(96, 90, 113)',
      '--md-secondary-container': 'rgb(231, 222, 248)',
      '--md-tertiary': 'rgb(124, 82, 96)',
      '--md-tertiary-container': 'rgb(255, 217, 227)',
      '--md-inverse-primary': 'rgb(208, 188, 255)',
    },
  },
  {
    key: 'teal',
    seed: '0, 150, 136',
    dark: {
      '--md-primary': 'rgb(129, 212, 200)',
      '--md-on-primary': 'rgb(0, 55, 49)',
      '--md-primary-container': 'rgb(0, 80, 72)',
      '--md-on-primary-container': 'rgb(158, 241, 228)',
      '--md-secondary': 'rgb(177, 204, 198)',
      '--md-secondary-container': 'rgb(52, 75, 70)',
      '--md-tertiary': 'rgb(172, 202, 230)',
      '--md-tertiary-container': 'rgb(42, 73, 100)',
      '--md-inverse-primary': 'rgb(0, 107, 96)',
    },
    light: {
      '--md-primary': 'rgb(0, 107, 96)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(220, 245, 239)',
      '--md-on-primary-container': 'rgb(0, 32, 28)',
      '--md-secondary': 'rgb(75, 99, 94)',
      '--md-secondary-container': 'rgb(224, 241, 236)',
      '--md-tertiary': 'rgb(66, 97, 125)',
      '--md-tertiary-container': 'rgb(222, 237, 253)',
      '--md-inverse-primary': 'rgb(129, 212, 200)',
    },
  },
  {
    key: 'blue',
    seed: '25, 118, 210',
    dark: {
      '--md-primary': 'rgb(165, 200, 255)',
      '--md-on-primary': 'rgb(0, 48, 95)',
      '--md-primary-container': 'rgb(0, 70, 134)',
      '--md-on-primary-container': 'rgb(212, 228, 255)',
      '--md-secondary': 'rgb(188, 199, 220)',
      '--md-secondary-container': 'rgb(58, 70, 88)',
      '--md-tertiary': 'rgb(218, 189, 226)',
      '--md-tertiary-container': 'rgb(75, 60, 82)',
      '--md-inverse-primary': 'rgb(0, 95, 175)',
    },
    light: {
      '--md-primary': 'rgb(0, 95, 175)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(219, 232, 255)',
      '--md-on-primary-container': 'rgb(0, 28, 57)',
      '--md-secondary': 'rgb(84, 94, 113)',
      '--md-secondary-container': 'rgb(227, 234, 248)',
      '--md-tertiary': 'rgb(108, 84, 116)',
      '--md-tertiary-container': 'rgb(241, 227, 249)',
      '--md-inverse-primary': 'rgb(165, 200, 255)',
    },
  },
  {
    key: 'rose',
    seed: '194, 24, 91',
    dark: {
      '--md-primary': 'rgb(255, 177, 195)',
      '--md-on-primary': 'rgb(102, 0, 43)',
      '--md-primary-container': 'rgb(136, 14, 62)',
      '--md-on-primary-container': 'rgb(255, 217, 224)',
      '--md-secondary': 'rgb(228, 189, 197)',
      '--md-secondary-container': 'rgb(80, 60, 66)',
      '--md-tertiary': 'rgb(239, 190, 146)',
      '--md-tertiary-container': 'rgb(89, 61, 36)',
      '--md-inverse-primary': 'rgb(174, 28, 76)',
    },
    light: {
      '--md-primary': 'rgb(174, 28, 76)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(255, 226, 233)',
      '--md-on-primary-container': 'rgb(62, 0, 22)',
      '--md-secondary': 'rgb(117, 84, 93)',
      '--md-secondary-container': 'rgb(246, 226, 233)',
      '--md-tertiary': 'rgb(122, 86, 58)',
      '--md-tertiary-container': 'rgb(248, 230, 213)',
      '--md-inverse-primary': 'rgb(255, 177, 195)',
    },
  },
  {
    key: 'olive',
    seed: '128, 128, 0',
    dark: {
      '--md-primary': 'rgb(201, 204, 120)',
      '--md-on-primary': 'rgb(50, 52, 0)',
      '--md-primary-container': 'rgb(72, 75, 0)',
      '--md-on-primary-container': 'rgb(230, 233, 144)',
      '--md-secondary': 'rgb(198, 200, 167)',
      '--md-secondary-container': 'rgb(68, 70, 43)',
      '--md-tertiary': 'rgb(163, 210, 193)',
      '--md-tertiary-container': 'rgb(37, 80, 65)',
      '--md-inverse-primary': 'rgb(96, 99, 0)',
    },
    light: {
      '--md-primary': 'rgb(92, 95, 0)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(236, 239, 196)',
      '--md-on-primary-container': 'rgb(28, 29, 0)',
      '--md-secondary': 'rgb(96, 97, 66)',
      '--md-secondary-container': 'rgb(234, 236, 211)',
      '--md-tertiary': 'rgb(62, 104, 87)',
      '--md-tertiary-container': 'rgb(224, 243, 231)',
      '--md-inverse-primary': 'rgb(201, 204, 120)',
    },
  },
  {
    key: 'brown',
    seed: '141, 110, 99',
    dark: {
      '--md-primary': 'rgb(236, 189, 170)',
      '--md-on-primary': 'rgb(68, 33, 18)',
      '--md-primary-container': 'rgb(94, 55, 38)',
      '--md-on-primary-container': 'rgb(255, 220, 205)',
      '--md-secondary': 'rgb(224, 192, 178)',
      '--md-secondary-container': 'rgb(77, 59, 49)',
      '--md-tertiary': 'rgb(203, 202, 152)',
      '--md-tertiary-container': 'rgb(69, 70, 29)',
      '--md-inverse-primary': 'rgb(121, 78, 59)',
    },
    light: {
      '--md-primary': 'rgb(121, 78, 59)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(255, 233, 222)',
      '--md-on-primary-container': 'rgb(44, 14, 0)',
      '--md-secondary': 'rgb(110, 82, 71)',
      '--md-secondary-container': 'rgb(246, 228, 218)',
      '--md-tertiary': 'rgb(96, 97, 51)',
      '--md-tertiary-container': 'rgb(237, 239, 199)',
      '--md-inverse-primary': 'rgb(236, 189, 170)',
    },
  },
  {
    key: 'orange',
    seed: '230, 126, 34',
    dark: {
      '--md-primary': 'rgb(255, 184, 92)',
      '--md-on-primary': 'rgb(72, 41, 0)',
      '--md-primary-container': 'rgb(103, 60, 0)',
      '--md-on-primary-container': 'rgb(255, 222, 178)',
      '--md-secondary': 'rgb(220, 196, 163)',
      '--md-secondary-container': 'rgb(78, 63, 37)',
      '--md-tertiary': 'rgb(184, 206, 168)',
      '--md-tertiary-container': 'rgb(52, 77, 39)',
      '--md-inverse-primary': 'rgb(135, 81, 0)',
    },
    light: {
      '--md-primary': 'rgb(135, 81, 0)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(255, 233, 207)',
      '--md-on-primary-container': 'rgb(43, 22, 0)',
      '--md-secondary': 'rgb(110, 87, 60)',
      '--md-secondary-container': 'rgb(245, 230, 209)',
      '--md-tertiary': 'rgb(78, 102, 61)',
      '--md-tertiary-container': 'rgb(227, 240, 210)',
      '--md-inverse-primary': 'rgb(255, 184, 92)',
    },
  },
  {
    key: 'green',
    seed: '76, 175, 80',
    dark: {
      '--md-primary': 'rgb(130, 216, 126)',
      '--md-on-primary': 'rgb(0, 57, 10)',
      '--md-primary-container': 'rgb(0, 82, 18)',
      '--md-on-primary-container': 'rgb(157, 244, 152)',
      '--md-secondary': 'rgb(185, 203, 179)',
      '--md-secondary-container': 'rgb(55, 74, 50)',
      '--md-tertiary': 'rgb(160, 206, 211)',
      '--md-tertiary-container': 'rgb(31, 77, 81)',
      '--md-inverse-primary': 'rgb(16, 109, 32)',
    },
    light: {
      '--md-primary': 'rgb(16, 109, 32)',
      '--md-on-primary': 'rgb(255, 255, 255)',
      '--md-primary-container': 'rgb(222, 244, 216)',
      '--md-on-primary-container': 'rgb(0, 33, 4)',
      '--md-secondary': 'rgb(82, 99, 76)',
      '--md-secondary-container': 'rgb(226, 240, 219)',
      '--md-tertiary': 'rgb(56, 101, 106)',
      '--md-tertiary-container': 'rgb(221, 240, 242)',
      '--md-inverse-primary': 'rgb(130, 216, 126)',
    },
  },
]

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

/** 获取色块的展示色 */
export function getSwatchColor(key: string): string {
  const scheme = THEME_COLORS.find(c => c.key === key)
  return scheme?.dark['--md-primary'] || 'rgb(208, 188, 255)'
}

// 深色模式下会通过行内样式覆盖的 surface 变量列表
const SURFACE_PROPS = [
  '--md-surface', '--md-background', '--md-surface-dim',
  '--md-surface-container', '--md-surface-container-low',
  '--md-surface-container-high', '--md-surface-container-highest',
  '--md-surface-container-lowest', '--md-surface-bright',
] as const

/**
 * 应用一套颜色令牌到 :root 行内样式（含深色 surface 微混）。
 * 由预设主题与动态取色共用，避免重复实现
 */
export function applyThemeVars(vars: Record<string, string>, seed: string, dark: boolean) {
  const root = document.documentElement.style

  // 先清除所有 surface 行内样式，确保 CSS class 的值能生效
  for (const prop of SURFACE_PROPS) {
    root.removeProperty(prop)
  }

  for (const [prop, value] of Object.entries(vars)) {
    root.setProperty(prop, value)
  }
  root.setProperty('--md-seed', seed)

  // 深色模式下给 surface 微混主题色 ~1.5%（浅色不动，保持中性白底）
  if (dark) {
    const primaryRgb = vars['--md-primary']
    const match = primaryRgb?.match(/rgb\((\d+),\s*(\d+),\s*(\d+)\)/)
    if (match) {
      const [, r, g, b] = match.map(Number)
      const mix = 0.015
      root.setProperty('--md-surface', `rgb(${Math.round(20 + r * mix)}, ${Math.round(18 + g * mix)}, ${Math.round(24 + b * mix)})`)
      root.setProperty('--md-background', `rgb(${Math.round(20 + r * mix)}, ${Math.round(18 + g * mix)}, ${Math.round(24 + b * mix)})`)
      root.setProperty('--md-surface-container', `rgb(${Math.round(33 + r * 0.02)}, ${Math.round(31 + g * 0.02)}, ${Math.round(38 + b * 0.02)})`)
      root.setProperty('--md-surface-container-low', `rgb(${Math.round(29 + r * 0.015)}, ${Math.round(27 + g * 0.015)}, ${Math.round(32 + b * 0.015)})`)
      root.setProperty('--md-surface-container-high', `rgb(${Math.round(43 + r * 0.02)}, ${Math.round(41 + g * 0.02)}, ${Math.round(48 + b * 0.02)})`)
      root.setProperty('--md-surface-container-highest', `rgb(${Math.round(54 + r * 0.02)}, ${Math.round(52 + g * 0.02)}, ${Math.round(59 + b * 0.02)})`)
    }
  }
}

/**
 * 仅视觉应用主题色 CSS 变量（不持久化）
 * @param isDark 直接传入避免 classList 读取导致强制 reflow
 */
export function applyThemeColorVisual(key: string, isDark?: boolean) {
  const scheme = THEME_COLORS.find(c => c.key === key)
  if (!scheme) return

  // 直接使用传入的 isDark，避免在 startViewTransition 回调内
  // 读取 classList（写后读会触发强制 reflow，是卡顿的主因）
  const dark = isDark ?? !document.documentElement.classList.contains('light-theme')
  const vars = dark ? scheme.dark : scheme.light
  applyThemeVars(vars, scheme.seed, dark)
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
  if (!(document as any).startViewTransition || !VIEW_TRANSITION_SUPPORTED) {
    applyThemeColor(key, undefined, persist)
    return
  }

  const maxRadius = Math.hypot(
    Math.max(x, window.innerWidth - x),
    Math.max(y, window.innerHeight - y),
  )

  // 预计算当前 dark/light 状态，避免 transition 回调中读 classList
  const isDark = !document.documentElement.classList.contains('light-theme')
  // ripple 与平滑过渡互斥：清掉可能残留的 theme-switching
  cancelSmoothThemeTransition()
  document.documentElement.classList.add('theme-ripple-active')

  const transition = (document as any).startViewTransition(() => {
    // 仅做视觉切换，不含 localStorage 写入
    applyThemeColorVisual(key, isDark)
  })

  // 异步持久化，不阻塞动画关键路径
  if (persist) queueMicrotask(() => localStorage.setItem('theme-color', key))

  try {
    await transition.ready
    const anim = document.documentElement.animate(
      {
        clipPath: [
          `circle(0px at ${x}px ${y}px)`,
          `circle(${maxRadius}px at ${x}px ${y}px)`,
        ],
      },
      {
        duration: 500,
        easing: 'cubic-bezier(0.2, 0, 0, 1)',
        pseudoElement: '::view-transition-new(root)',
      },
    )
    await anim.finished.catch(() => {})
    await transition.finished.catch(() => {})
  } catch {
    // 中断无影响
  } finally {
    document.documentElement.classList.remove('theme-ripple-active')
  }
}

/** 获取已保存的主题色 */
export function getSavedThemeColor(): string {
  return localStorage.getItem('theme-color') || 'purple'
}

/** 关闭动态取色时恢复到已保存的预设主题色（仅视觉，不持久化） */
export function clearDynamicThemeColor(isDark?: boolean) {
  applyThemeColorVisual(getSavedThemeColor(), isDark)
}
