import { createI18n } from 'vue-i18n'
import zhCN from './zh-CN.json'
import zhTW from './zh-TW.json'
import en from './en.json'
import ja from './ja.json'

// WebKitGTK 的 View Transition 实现不稳定：语言切换会整树重渲染，在 Linux
// 上曾导致 WebKit 主循环内 SEGV 崩溃（GTK 主循环 -> WebKit 回调 -> JSC）。
// Linux 走平滑降级路径（直接切 locale），Windows/macOS 保留过渡动画
import { detectPlatform } from '@/modules/shortcuts/platform'

export const VIEW_TRANSITION_SUPPORTED = detectPlatform() !== 'linux'

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
  ;(i18n.global.locale as any).value = locale
  if (persist) localStorage.setItem('locale', locale)
  document.documentElement.lang = locale
  // 同步托盘菜单文案：从中央 i18n 资源取当前语言的已翻译文案下发（浏览器开发模式静默失败）
  const t = i18n.global.t
  import('@tauri-apps/api/core')
    .then(({ invoke }) => void invoke('set_tray_texts', {
      prev: t('tray.prev'),
      toggle: t('tray.toggle'),
      next: t('tray.next'),
      nowPlaying: t('tray.now_playing'),
      nowPlayingTitle: t('tray.now_playing_title'),
      openHome: t('tray.open_home'),
      quit: t('tray.quit'),
    }).catch(() => {}))
    .catch(() => {})
}

/** 带 View Transition 动画的语言切换 */
export async function setLocaleWithTransition(locale: string, x?: number, y?: number, persist = true) {
  if (!(document as any).startViewTransition || !x || !y || !VIEW_TRANSITION_SUPPORTED) {
    setLocale(locale, persist)
    return
  }

  const maxRadius = Math.hypot(
    Math.max(x, window.innerWidth - x),
    Math.max(y, window.innerHeight - y),
  )

  const transition = (document as any).startViewTransition(() => {
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
