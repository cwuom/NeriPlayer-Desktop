import { createI18n } from 'vue-i18n'
import zhCN from './zh-CN.json'
import zhTW from './zh-TW.json'
import en from './en.json'
import ja from './ja.json'

// WebKitGTK 的 View Transition 实现不稳定：语言切换会整树重渲染，在 Linux
// 上曾导致 WebKit 主循环内 SEGV 崩溃（GTK 主循环 -> WebKit 回调 -> JSC）。
// Linux 走平滑降级路径（直接切 locale），Windows/macOS 保留过渡动画。
// i18n 层不依赖平台识别，由入口按平台注入；未注入时按不支持处理，宁可少一段动画
let viewTransitionSupported = false

export function setViewTransitionSupported(supported: boolean) {
  viewTransitionSupported = supported
}

export const SUPPORTED_LOCALES = [
  { code: 'zh-CN', label: '简体中文' },
  { code: 'zh-TW', label: '繁體中文' },
  { code: 'en', label: 'English' },
  { code: 'ja', label: '日本語' },
] as const

// 检测系统语言
function detectLocale(): string {
  // 优先读取存储的设置（后续接入 tauri-plugin-store）
  const stored = localStorage.getItem('locale')
  if (stored) return stored

  const nav = navigator.language || 'zh-CN'
  if (nav === 'zh-TW' || nav === 'zh-Hant') return 'zh-TW'
  if (nav.startsWith('zh')) return 'zh-CN'
  if (nav.startsWith('ja')) return 'ja'
  return 'en'
}

const i18n = createI18n({
  legacy: false,
  locale: detectLocale(),
  fallbackLocale: 'zh-CN',
  messages: {
    'zh-CN': zhCN,
    'zh-TW': zhTW,
    en,
    ja,
  },
})

export default i18n

export function setLocale(locale: string, persist = true) {
  // 设置、托盘与配置导入可能传入未内置的语言；vue-i18n 会回退到 fallbackLocale，这里只收窄类型不做校验
  i18n.global.locale.value = locale as typeof i18n.global.locale.value
  if (persist) localStorage.setItem('locale', locale)
  document.documentElement.lang = locale
}

/** 带 View Transition 动画的语言切换 */
export async function setLocaleWithTransition(locale: string, x?: number, y?: number, persist = true) {
  if (!document.startViewTransition || !x || !y || !viewTransitionSupported) {
    setLocale(locale, persist)
    return
  }

  const maxRadius = Math.hypot(
    Math.max(x, window.innerWidth - x),
    Math.max(y, window.innerHeight - y),
  )

  const transition = document.startViewTransition(() => {
    setLocale(locale, persist)
  })

  try {
    await transition.ready
    document.documentElement.animate(
      {
        clipPath: [
          `circle(0px at ${x}px ${y}px)`,
          `circle(${maxRadius}px at ${x}px ${y}px)`,
        ],
      },
      {
        duration: 700,
        easing: 'cubic-bezier(0.2, 0, 0, 1)',
        pseudoElement: '::view-transition-new(root)',
      },
    )
  } catch {
    // 中断无影响
  }
}
