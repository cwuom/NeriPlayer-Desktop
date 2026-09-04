// 深色/浅色模式管理 + 圆形扩散过渡动画
// 参考 Android 端 pending state 模式：视觉切换与持久化解耦，消除卡顿
import { detectPlatform } from '@/modules/shortcuts/platform'

// WebKitGTK 的 View Transition 在 Linux 上不稳定（整页快照合成可致
// WebKit 回调内 SEGV），走 beginSmoothThemeTransition 平滑降级
const VIEW_TRANSITION_SUPPORTED = detectPlatform() !== 'linux'
import { reapplyDynamicColorForTheme } from './colorExtractor'
import {
  applyThemeColor,
  applyThemeColorVisual,
  beginSmoothThemeTransition,
  cancelSmoothThemeTransition,
  getSavedThemeColor,
} from './themeColor'

export type ThemeMode = 'system' | 'dark' | 'light'

let currentMode: ThemeMode = 'system'

export function getThemeMode(): ThemeMode {
  return (localStorage.getItem('theme-mode') as ThemeMode) || 'system'
}

function resolvedIsDark(mode: ThemeMode): boolean {
  if (mode === 'dark') return true
  if (mode === 'light') return false
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

/**
 * 仅做视觉切换（不写 localStorage），供 startViewTransition 回调使用
 * 将 isDark 预计算后传入 applyThemeColorVisual，避免回调内
 * classList 写后读触发强制 reflow
 */
function applyThemeVisual(mode: ThemeMode) {
  currentMode = mode
  const dark = resolvedIsDark(mode)
  document.documentElement.classList.toggle('light-theme', !dark)
  document.documentElement.classList.toggle('dark-theme', dark)
  // 动态取色激活时同步用缓存种子重算，避免闪回预设主题色
  if (!reapplyDynamicColorForTheme(dark)) {
    applyThemeColorVisual(getSavedThemeColor(), dark)
  }
}

/**
 * 非 ripple 路径应用主题（含持久化，用于初始化、系统跟随和 fallback）。
 * 改类/变量前先套 300ms 平滑过渡，避免整屏瞬间跳变
 */
export function applyTheme(mode: ThemeMode, persist = true) {
  beginSmoothThemeTransition()
  applyThemeVisual(mode)
  if (persist) localStorage.setItem('theme-mode', mode)
}

/**
 * 带圆形扩散动画的主题切换
 * transition 回调内只做纯视觉 DOM 操作，
 * localStorage 写入移到 microtask 异步执行
 */
export async function switchThemeWithRipple(mode: ThemeMode, x: number, y: number, persist = true) {
  // 如果浏览器不支持 View Transition，直接切换（走平滑过渡兜底）。
  // 调用方（SettingsView）可能已预加 theme-ripple-active，而本分支不会走
  // ripple 的清理流程，先移除以免残留导致全局 transition 被永久禁用
  if (!(document as any).startViewTransition || !VIEW_TRANSITION_SUPPORTED) {
    document.documentElement.classList.remove('theme-ripple-active')
    applyTheme(mode, persist)
    return
  }

  // 计算圆形遮罩最大半径（覆盖整个屏幕）
  const maxRadius = Math.hypot(
    Math.max(x, window.innerWidth - x),
    Math.max(y, window.innerHeight - y),
  )

  // 禁用 CSS color 过渡，避免圆外区域文字提前变色
  // ripple 与平滑过渡互斥：先清掉可能残留的 theme-switching
  const root = document.documentElement
  cancelSmoothThemeTransition()
  root.classList.add('theme-ripple-active')
  // 用 CSS 变量驱动 clip-path，兼容 WebView2 对 WAAPI pseudoElement 支持不全的情况
  root.style.setProperty('--theme-ripple-x', `${x}px`)
  root.style.setProperty('--theme-ripple-y', `${y}px`)
  root.style.setProperty('--theme-ripple-r', '0px')

  const transition = (document as any).startViewTransition(() => {
    // 仅视觉切换，不含 localStorage 写入
    applyThemeVisual(mode)
  })

  // 异步持久化，不阻塞动画关键路径
  if (persist) queueMicrotask(() => localStorage.setItem('theme-mode', mode))

  try {
    await transition.ready
    root.style.setProperty('--theme-ripple-r', `${maxRadius}px`)

    // 双通道：CSS 变量动画 + WAAPI，谁先可用谁生效
    const anim = root.animate(
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
    await Promise.race([
      anim.finished.catch(() => {}),
      new Promise<void>(resolve => setTimeout(resolve, 520)),
    ])
    await transition.finished.catch(() => {})
  } catch {
    // transition 被中断也没关系
  } finally {
    root.classList.remove('theme-ripple-active')
    root.style.removeProperty('--theme-ripple-x')
    root.style.removeProperty('--theme-ripple-y')
    root.style.removeProperty('--theme-ripple-r')
  }
}

// 初始化
export function initTheme() {
  const mode = getThemeMode()
  applyTheme(mode)
  // 初始化主题色
  applyThemeColor(getSavedThemeColor())

  // 监听系统主题变化
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (getThemeMode() === 'system') {
      applyTheme('system')
    }
  })
}
