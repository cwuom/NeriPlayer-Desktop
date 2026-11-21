// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  THEME_COLORS,
  applyThemeColor,
  applyThemeColorVisual,
  applyThemeVars,
  beginSmoothThemeTransition,
  cancelSmoothThemeTransition,
  clearDynamicThemeColor,
  getSavedThemeColor,
  getSwatchColor,
  switchThemeColorWithRipple,
  type ThemeColorScheme,
} from '@/utils/themeColor'
import { installMemoryStorage, type MemoryStorage } from './memoryStorage'

const SURFACE_PROPS = [
  '--md-surface',
  '--md-background',
  '--md-surface-container',
  '--md-surface-container-low',
  '--md-surface-container-high',
  '--md-surface-container-highest',
] as const

let storage: MemoryStorage

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

interface FakeViewTransition {
  ready: Promise<void>
  finished: Promise<void>
}

/** happy-dom 未实现 View Transition，按需挂到 document 上，用后删除 */
function installViewTransition(transition: Partial<FakeViewTransition> = {}) {
  const start = vi.fn((update: () => void): FakeViewTransition => {
    update()
    return { ready: Promise.resolve(), finished: Promise.resolve(), ...transition }
  })
  Object.defineProperty(document, 'startViewTransition', { value: start, configurable: true })
  const animate = vi.fn(() => ({ finished: Promise.resolve() }))
  Object.defineProperty(root(), 'animate', { value: animate, configurable: true })
  return { start, animate }
}

beforeEach(() => {
  storage = installMemoryStorage()
  root().removeAttribute('style')
  root().className = ''
})

afterEach(() => {
  cancelSmoothThemeTransition()
  Reflect.deleteProperty(document, 'startViewTransition')
  Reflect.deleteProperty(root(), 'animate')
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('THEME_COLORS', () => {
  it('defines the same token set for every preset in both modes', () => {
    const expectedKeys = Object.keys(THEME_COLORS[0].dark).sort()
    for (const preset of THEME_COLORS) {
      expect(Object.keys(preset.dark).sort()).toEqual(expectedKeys)
      expect(Object.keys(preset.light).sort()).toEqual(expectedKeys)
      expect(preset.seed).toMatch(/^\d{1,3}, \d{1,3}, \d{1,3}$/)
    }
    expect(new Set(THEME_COLORS.map(c => c.key)).size).toBe(THEME_COLORS.length)
  })
})

describe('getSwatchColor', () => {
  it('returns the dark primary of a known preset', () => {
    expect(getSwatchColor('teal')).toBe(scheme('teal').dark['--md-primary'])
  })

  it('falls back to the purple swatch for unknown keys', () => {
    expect(getSwatchColor('does-not-exist')).toBe(scheme('purple').dark['--md-primary'])
  })
})

describe('applyThemeVars', () => {
  it('writes every token and the seed to the root inline style', () => {
    applyThemeVars({ '--md-primary': 'rgb(1, 2, 3)', '--md-secondary': 'rgb(4, 5, 6)' }, '9, 9, 9', false)

    expect(cssVar('--md-primary')).toBe('rgb(1, 2, 3)')
    expect(cssVar('--md-secondary')).toBe('rgb(4, 5, 6)')
    expect(cssVar('--md-seed')).toBe('9, 9, 9')
  })

  it('clears inline surface overrides in light mode so stylesheet values apply', () => {
    for (const prop of SURFACE_PROPS) root().style.setProperty(prop, 'rgb(0, 0, 0)')
    root().style.setProperty('--md-surface-bright', 'rgb(0, 0, 0)')

    applyThemeVars({ '--md-primary': 'rgb(200, 100, 50)' }, '1, 1, 1', false)

    for (const prop of [...SURFACE_PROPS, '--md-surface-bright']) expect(cssVar(prop)).toBe('')
  })

  it('derives neutral dark surfaces for a black primary', () => {
    applyThemeVars({ '--md-primary': 'rgb(0, 0, 0)' }, '0, 0, 0', true)

    expect(cssVar('--md-surface')).toBe('rgb(20, 18, 24)')
    expect(cssVar('--md-background')).toBe('rgb(20, 18, 24)')
    expect(cssVar('--md-surface-container')).toBe('rgb(33, 31, 38)')
    expect(cssVar('--md-surface-container-low')).toBe('rgb(29, 27, 32)')
    expect(cssVar('--md-surface-container-high')).toBe('rgb(43, 41, 48)')
    expect(cssVar('--md-surface-container-highest')).toBe('rgb(54, 52, 59)')
  })

  it('tints dark surfaces slightly toward the primary colour', () => {
    applyThemeVars({ '--md-primary': 'rgb(255, 0, 0)' }, '255, 0, 0', true)

    expect(cssVar('--md-surface')).toBe('rgb(24, 18, 24)')
    expect(cssVar('--md-surface-container')).toBe('rgb(38, 31, 38)')
    expect(cssVar('--md-surface-container-highest')).toBe('rgb(59, 52, 59)')
  })

  it('skips the dark surface mix when the primary is not an rgb() string', () => {
    root().style.setProperty('--md-surface', 'rgb(1, 1, 1)')

    applyThemeVars({ '--md-primary': '#ff0000' }, '255, 0, 0', true)

    for (const prop of SURFACE_PROPS) expect(cssVar(prop)).toBe('')
  })
})

describe('applyThemeColorVisual', () => {
  it('applies the dark palette and seed of the preset when dark', () => {
    applyThemeColorVisual('teal', true)

    expect(cssVar('--md-primary')).toBe(scheme('teal').dark['--md-primary'])
    expect(cssVar('--md-inverse-primary')).toBe(scheme('teal').dark['--md-inverse-primary'])
    expect(cssVar('--md-seed')).toBe(scheme('teal').seed)
    expect(cssVar('--md-surface')).not.toBe('')
  })

  it('applies the light palette without surface overrides when light', () => {
    applyThemeColorVisual('rose', false)

    expect(cssVar('--md-primary')).toBe(scheme('rose').light['--md-primary'])
    expect(cssVar('--md-surface')).toBe('')
  })

  it('reads the light-theme class when isDark is omitted', () => {
    root().classList.add('light-theme')
    applyThemeColorVisual('blue')
    expect(cssVar('--md-primary')).toBe(scheme('blue').light['--md-primary'])

    root().classList.remove('light-theme')
    applyThemeColorVisual('blue')
    expect(cssVar('--md-primary')).toBe(scheme('blue').dark['--md-primary'])
  })

  it('ignores unknown preset keys', () => {
    applyThemeColorVisual('green', true)
    applyThemeColorVisual('nope', true)
    expect(cssVar('--md-primary')).toBe(scheme('green').dark['--md-primary'])
  })

  it('does not persist the selection', () => {
    applyThemeColorVisual('green', true)
    expect(storage.getItem('theme-color')).toBeNull()
  })
})

describe('smooth theme transition', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  it('adds theme-switching and removes it after 350ms', () => {
    beginSmoothThemeTransition()
    expect(root().classList.contains('theme-switching')).toBe(true)

    vi.advanceTimersByTime(349)
    expect(root().classList.contains('theme-switching')).toBe(true)
    vi.advanceTimersByTime(1)
    expect(root().classList.contains('theme-switching')).toBe(false)
  })

  it('restarts the 350ms window on rapid repeated switches', () => {
    beginSmoothThemeTransition()
    vi.advanceTimersByTime(300)
    beginSmoothThemeTransition()

    vi.advanceTimersByTime(100)
    expect(root().classList.contains('theme-switching')).toBe(true)
    vi.advanceTimersByTime(250)
    expect(root().classList.contains('theme-switching')).toBe(false)
  })

  it('stays out of the way while a ripple transition is active', () => {
    root().classList.add('theme-ripple-active')
    beginSmoothThemeTransition()
    expect(root().classList.contains('theme-switching')).toBe(false)
  })

  it('cancel removes the class immediately and drops the pending timer', () => {
    beginSmoothThemeTransition()
    cancelSmoothThemeTransition()
    expect(root().classList.contains('theme-switching')).toBe(false)
    expect(vi.getTimerCount()).toBe(0)
  })
})

describe('applyThemeColor', () => {
  it('applies the preset, starts a smooth transition, and persists the key', () => {
    applyThemeColor('orange', true)

    expect(cssVar('--md-primary')).toBe(scheme('orange').dark['--md-primary'])
    expect(root().classList.contains('theme-switching')).toBe(true)
    expect(storage.getItem('theme-color')).toBe('orange')
  })

  it('skips persistence when persist is false', () => {
    applyThemeColor('orange', true, false)
    expect(storage.getItem('theme-color')).toBeNull()
  })
})

describe('saved theme color', () => {
  it('defaults to purple when nothing is saved', () => {
    expect(getSavedThemeColor()).toBe('purple')
  })

  it('returns the persisted key', () => {
    storage.setItem('theme-color', 'olive')
    expect(getSavedThemeColor()).toBe('olive')
  })

  it('clearDynamicThemeColor restores the saved preset visually without persisting', () => {
    storage.setItem('theme-color', 'brown')
    const setItem = vi.spyOn(storage, 'setItem')
    root().style.setProperty('--md-primary', 'rgb(1, 2, 3)')

    clearDynamicThemeColor(false)

    expect(cssVar('--md-primary')).toBe(scheme('brown').light['--md-primary'])
    expect(setItem).not.toHaveBeenCalled()
  })
})

describe('switchThemeColorWithRipple', () => {
  it('falls back to a smooth switch when View Transitions are unsupported', async () => {
    await switchThemeColorWithRipple('green', 10, 10)

    expect(cssVar('--md-primary')).toBe(scheme('green').dark['--md-primary'])
    expect(root().classList.contains('theme-switching')).toBe(true)
    expect(storage.getItem('theme-color')).toBe('green')
  })

  it('applies the preset inside the view transition and animates a circle from the click point', async () => {
    const { start, animate } = installViewTransition()
    root().classList.add('theme-switching')

    const done = switchThemeColorWithRipple('blue', 100, 50)
    expect(root().classList.contains('theme-ripple-active')).toBe(true)
    expect(root().classList.contains('theme-switching')).toBe(false)
    await done

    expect(start).toHaveBeenCalledTimes(1)
    expect(cssVar('--md-primary')).toBe(scheme('blue').dark['--md-primary'])
    const radius = Math.hypot(Math.max(100, window.innerWidth - 100), Math.max(50, window.innerHeight - 50))
    expect(animate).toHaveBeenCalledWith(
      { clipPath: ['circle(0px at 100px 50px)', `circle(${radius}px at 100px 50px)`] },
      expect.objectContaining({ duration: 500, pseudoElement: '::view-transition-new(root)' }),
    )
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    expect(storage.getItem('theme-color')).toBe('blue')
  })

  it('uses the light palette when the light theme is active', async () => {
    installViewTransition()
    root().classList.add('light-theme')

    await switchThemeColorWithRipple('teal', 0, 0)

    expect(cssVar('--md-primary')).toBe(scheme('teal').light['--md-primary'])
  })

  it('does not persist when persist is false', async () => {
    installViewTransition()
    await switchThemeColorWithRipple('blue', 0, 0, false)
    expect(storage.getItem('theme-color')).toBeNull()
  })

  it('cleans up the ripple class when the transition is aborted', async () => {
    const { animate } = installViewTransition({
      ready: Promise.reject(new Error('aborted')),
    })

    await expect(switchThemeColorWithRipple('rose', 5, 5)).resolves.toBeUndefined()

    expect(animate).not.toHaveBeenCalled()
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    expect(cssVar('--md-primary')).toBe(scheme('rose').dark['--md-primary'])
  })
})
