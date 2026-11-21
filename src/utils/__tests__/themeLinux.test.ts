// @vitest-environment happy-dom
// Linux 的 WebKitGTK 上 View Transition 会让整页快照合成崩溃，涟漪切换必须直接应用主题
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { switchThemeWithRipple } from '@/utils/theme'
import { cancelSmoothThemeTransition, switchThemeColorWithRipple } from '@/utils/themeColor'
import { installMemoryStorage, type MemoryStorage } from './memoryStorage'

vi.mock('@/utils/platform', () => ({ detectPlatform: () => 'linux' }))
vi.mock('@/utils/colorExtractor', () => ({ reapplyDynamicColorForTheme: () => false }))

let storage: MemoryStorage
const root = () => document.documentElement

beforeEach(() => {
  storage = installMemoryStorage()
  root().removeAttribute('style')
  root().className = ''
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (media: string) => ({ media, matches: false, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList,
  )
  Object.defineProperty(document, 'startViewTransition', { value: vi.fn(), configurable: true })
})

afterEach(() => {
  cancelSmoothThemeTransition()
  Reflect.deleteProperty(document, 'startViewTransition')
  vi.unstubAllGlobals()
})

describe('ripple theme switching on Linux', () => {
  it('applies the theme directly without a view transition and clears a pre-added ripple class', async () => {
    root().classList.add('theme-ripple-active')

    await switchThemeWithRipple('light', 30, 40)

    expect(document.startViewTransition).not.toHaveBeenCalled()
    expect(root().classList.contains('light-theme')).toBe(true)
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    expect(storage.getItem('theme-mode')).toBe('light')
  })

  it('applies a theme color preset directly without a view transition', async () => {
    await switchThemeColorWithRipple('blue', 100, 50)

    expect(document.startViewTransition).not.toHaveBeenCalled()
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    expect(storage.getItem('theme-color')).toBe('blue')
  })
})
