// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { applyTheme, getThemeMode, initTheme, switchThemeWithRipple } from '@/utils/theme'
import { THEME_COLORS, cancelSmoothThemeTransition, type ThemeColorScheme } from '@/utils/themeColor'
import { installMemoryStorage, type MemoryStorage } from './memoryStorage'

const reapplyDynamicColorForTheme = vi.hoisted(() => vi.fn<(isDark: boolean) => boolean>())

// colorExtractor 依赖 Tauri invoke 与 canvas，这里只关心 theme 是否把深浅色交给它重算
vi.mock('@/utils/colorExtractor', () => ({ reapplyDynamicColorForTheme }))
// 是否启用 View Transition 在模块加载时按平台决定，happy-dom 的 navigator 随宿主变化；
// 固定为非 Linux，Linux 降级分支由 themeLinux.test.ts 单独覆盖
vi.mock('@/utils/platform', () => ({ detectPlatform: () => 'mac' }))

class FakeMediaQueryList extends EventTarget implements MediaQueryList {
  onchange: ((this: MediaQueryList, ev: MediaQueryListEvent) => unknown) | null = null

  constructor(readonly media: string, public matches: boolean) {
    super()
  }

  addListener(): void {}
  removeListener(): void {}
}

let storage: MemoryStorage
let systemDark: FakeMediaQueryList

function root(): HTMLElement {
  return document.documentElement
}

function cssVar(name: string): string {
  return root().style.getPropertyValue(name)
}

function scheme(key: string): ThemeColorScheme {
  const found = THEME_COLORS.find(c => c.key === key)
  if (!found) throw new Error(`missing scheme ${key}`)
  return found
}

function themeClasses(): { dark: boolean; light: boolean } {
  return {
    dark: root().classList.contains('dark-theme'),
    light: root().classList.contains('light-theme'),
  }
}

interface FakeViewTransition {
  ready: Promise<void>
  finished: Promise<void>
}

function installViewTransition(finished: Promise<void> = Promise.resolve()) {
  const start = vi.fn((update: () => void): FakeViewTransition => {
    update()
    return { ready: Promise.resolve(), finished: Promise.resolve() }
  })
  Object.defineProperty(document, 'startViewTransition', { value: start, configurable: true })
  const radiusAtAnimate: string[] = []
  const animate = vi.fn(() => {
    radiusAtAnimate.push(cssVar('--theme-ripple-r'))
    return { finished }
  })
  Object.defineProperty(root(), 'animate', { value: animate, configurable: true })
  return { start, animate, radiusAtAnimate }
}

beforeEach(() => {
  storage = installMemoryStorage()
  root().removeAttribute('style')
  root().className = ''
  reapplyDynamicColorForTheme.mockReset().mockReturnValue(false)
  systemDark = new FakeMediaQueryList('(prefers-color-scheme: dark)', false)
  vi.spyOn(window, 'matchMedia').mockImplementation(() => systemDark)
})

afterEach(() => {
  cancelSmoothThemeTransition()
  Reflect.deleteProperty(document, 'startViewTransition')
  Reflect.deleteProperty(root(), 'animate')
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('getThemeMode', () => {
  it('defaults to system when nothing is stored', () => {
    expect(getThemeMode()).toBe('system')
  })

  it('returns the stored mode', () => {
    storage.setItem('theme-mode', 'light')
    expect(getThemeMode()).toBe('light')
  })
})

describe('applyTheme', () => {
  it('switches to dark classes, applies the saved preset, and persists', () => {
    storage.setItem('theme-color', 'teal')

    applyTheme('dark')

    expect(themeClasses()).toEqual({ dark: true, light: false })
    expect(cssVar('--md-primary')).toBe(scheme('teal').dark['--md-primary'])
    expect(root().classList.contains('theme-switching')).toBe(true)
    expect(storage.getItem('theme-mode')).toBe('dark')
  })

  it('switches to light classes and the light palette without persisting when asked', () => {
    root().classList.add('dark-theme')

    applyTheme('light', false)

    expect(themeClasses()).toEqual({ dark: false, light: true })
    expect(cssVar('--md-primary')).toBe(scheme('purple').light['--md-primary'])
    expect(storage.getItem('theme-mode')).toBeNull()
  })

  it('follows the system preference in system mode', () => {
    systemDark.matches = true
    applyTheme('system')
    expect(themeClasses()).toEqual({ dark: true, light: false })

    systemDark.matches = false
    applyTheme('system')
    expect(themeClasses()).toEqual({ dark: false, light: true })
  })

  it('lets an active dynamic colour recompute tokens instead of the preset', () => {
    reapplyDynamicColorForTheme.mockReturnValue(true)
    root().style.setProperty('--md-primary', 'rgb(1, 2, 3)')

    applyTheme('light')

    expect(reapplyDynamicColorForTheme).toHaveBeenCalledWith(false)
    expect(cssVar('--md-primary')).toBe('rgb(1, 2, 3)')
  })

  it('passes the resolved dark flag to the dynamic colour hook', () => {
    systemDark.matches = true
    applyTheme('system')
    expect(reapplyDynamicColorForTheme).toHaveBeenLastCalledWith(true)
  })
})

describe('initTheme', () => {
  it('applies the stored mode and saved colour and persists both', () => {
    storage.setItem('theme-mode', 'light')
    storage.setItem('theme-color', 'rose')

    initTheme()

    expect(themeClasses()).toEqual({ dark: false, light: true })
    expect(cssVar('--md-primary')).toBe(scheme('rose').light['--md-primary'])
    expect(storage.getItem('theme-mode')).toBe('light')
    expect(storage.getItem('theme-color')).toBe('rose')
  })

  it('re-applies the theme when the system preference changes in system mode', () => {
    initTheme()
    expect(themeClasses()).toEqual({ dark: false, light: true })

    systemDark.matches = true
    systemDark.dispatchEvent(new Event('change'))

    expect(themeClasses()).toEqual({ dark: true, light: false })
  })

  it('ignores system preference changes when an explicit mode is stored', () => {
    storage.setItem('theme-mode', 'light')
    initTheme()

    systemDark.matches = true
    systemDark.dispatchEvent(new Event('change'))

    expect(themeClasses()).toEqual({ dark: false, light: true })
  })
})

describe('switchThemeWithRipple', () => {
  it('falls back to applyTheme and clears a pre-set ripple class without View Transitions', async () => {
    root().classList.add('theme-ripple-active')

    await switchThemeWithRipple('dark', 10, 10)

    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    expect(themeClasses()).toEqual({ dark: true, light: false })
    expect(storage.getItem('theme-mode')).toBe('dark')
  })

  it('drives the ripple through CSS variables and cleans them up afterwards', async () => {
    const { start, animate, radiusAtAnimate } = installViewTransition()
    root().classList.add('theme-switching')

    const done = switchThemeWithRipple('light', 30, 40)

    expect(root().classList.contains('theme-switching')).toBe(false)
    expect(root().classList.contains('theme-ripple-active')).toBe(true)
    expect(cssVar('--theme-ripple-x')).toBe('30px')
    expect(cssVar('--theme-ripple-y')).toBe('40px')
    expect(cssVar('--theme-ripple-r')).toBe('0px')
    await done

    const radius = Math.hypot(Math.max(30, window.innerWidth - 30), Math.max(40, window.innerHeight - 40))
    expect(start).toHaveBeenCalledTimes(1)
    expect(radiusAtAnimate).toEqual([`${radius}px`])
    expect(animate).toHaveBeenCalledWith(
      { clipPath: ['circle(0px at 30px 40px)', `circle(${radius}px at 30px 40px)`] },
      expect.objectContaining({ pseudoElement: '::view-transition-new(root)' }),
    )
    expect(themeClasses()).toEqual({ dark: false, light: true })
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    for (const prop of ['--theme-ripple-x', '--theme-ripple-y', '--theme-ripple-r']) expect(cssVar(prop)).toBe('')
    expect(storage.getItem('theme-mode')).toBe('light')
  })

  it('does not persist the mode when persist is false', async () => {
    installViewTransition()
    await switchThemeWithRipple('dark', 0, 0, false)
    expect(storage.getItem('theme-mode')).toBeNull()
    expect(themeClasses()).toEqual({ dark: true, light: false })
  })

  it('stops waiting after 520ms when the WAAPI animation never finishes', async () => {
    vi.useFakeTimers()
    installViewTransition(new Promise<void>(() => {}))
    let settled = false

    const done = switchThemeWithRipple('dark', 0, 0).then(() => { settled = true })
    await vi.advanceTimersByTimeAsync(519)
    expect(settled).toBe(false)
    expect(root().classList.contains('theme-ripple-active')).toBe(true)

    await vi.advanceTimersByTimeAsync(1)
    await done
    expect(settled).toBe(true)
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
  })
})
