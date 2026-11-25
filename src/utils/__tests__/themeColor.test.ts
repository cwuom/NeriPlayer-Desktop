// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  THEME_COLORS,
  applyAccent,
  applyThemeColor,
  applyThemeColorVisual,
  setBackgroundTint,
  beginSmoothThemeTransition,
  cancelSmoothThemeTransition,
  clearDynamicThemeColor,
  getSavedThemeColor,
  getSwatchColor,
  switchThemeColorWithRipple,
  type ThemeColorScheme,
} from '@/utils/themeColor'
import { buildCleanScheme, contrastRatio, parseRgb, rgb } from '@/utils/theme/cleanScheme'
import { installMemoryStorage, type MemoryStorage } from './memoryStorage'

// 是否启用 View Transition 在模块加载时按平台决定，happy-dom 的 navigator 随宿主变化；
// 固定为非 Linux，Linux 降级分支由 themeLinux.test.ts 单独覆盖
vi.mock('@/utils/platform', () => ({ detectPlatform: () => 'mac' }))

let storage: MemoryStorage

function root(): HTMLElement {
  return document.documentElement
}

function getTintForTest() {
  return { tint: 'accent' as const, strength: 12 }
}

function cssVar(name: string): string {
  return root().style.getPropertyValue(name)
}

function scheme(key: string): ThemeColorScheme {
  const found = THEME_COLORS.find(c => c.key === key)
  if (!found) throw new Error(`missing scheme ${key}`)
  return found
}

/** 预设在指定模式下的强调色（即写入 --md-primary 的值） */
function primary(key: string, mode: 'light' | 'dark'): string {
  return rgb(scheme(key)[mode])
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
  it('uses unique keys and keeps legacy keys so saved settings still resolve', () => {
    const keys = THEME_COLORS.map(c => c.key)
    expect(new Set(keys).size).toBe(keys.length)
    for (const legacy of ['purple', 'teal', 'blue', 'rose', 'olive', 'brown', 'orange', 'green']) {
      expect(keys).toContain(legacy)
    }
  })

  it('keeps every accent readable as a fill against the matching background', () => {
    const light = parseRgb(buildCleanScheme([0, 0, 0], false, { tint: 'neutral', strength: 0 })['--md-background'])!
    const dark = parseRgb(buildCleanScheme([0, 0, 0], true, { tint: 'neutral', strength: 0 })['--md-background'])!
    for (const preset of THEME_COLORS) {
      expect(contrastRatio(preset.light, light), preset.key).toBeGreaterThanOrEqual(3)
      expect(contrastRatio(preset.dark, dark), preset.key).toBeGreaterThanOrEqual(3)
    }
  })
})

describe('getSwatchColor', () => {
  it('returns the accent of a known preset for the requested mode', () => {
    expect(getSwatchColor('teal')).toBe(primary('teal', 'light'))
    expect(getSwatchColor('teal', true)).toBe(primary('teal', 'dark'))
  })

  it('falls back to the purple swatch for unknown keys', () => {
    expect(getSwatchColor('does-not-exist')).toBe(primary('purple', 'light'))
  })
})

describe('applyAccent', () => {
  it('writes the full clean token set and the seed to the root inline style', () => {
    applyAccent([10, 20, 30], false)

    const expected = buildCleanScheme([10, 20, 30], false, getTintForTest())
    for (const [prop, value] of Object.entries(expected)) expect(cssVar(prop)).toBe(value)
    expect(cssVar('--md-primary')).toBe('rgb(10, 20, 30)')
    expect(cssVar('--md-seed')).toBe('10, 20, 30')
  })

  it('writes inline surfaces in both modes so stale overrides never leak', () => {
    root().style.setProperty('--md-surface', 'rgb(1, 1, 1)')
    applyAccent([200, 100, 50], false)
    expect(cssVar('--md-surface')).not.toBe('rgb(1, 1, 1)')
    expect(cssVar('--md-surface')).not.toBe('')
  })
})

describe('setBackgroundTint', () => {
  afterEach(() => {
    setBackgroundTint({ tint: 'accent', strength: 12 }, false)
  })

  it('re-tints surfaces of the current accent in place', () => {
    applyAccent([255, 0, 0], true)
    setBackgroundTint({ tint: 'neutral', strength: 40 }, false)
    const neutral = cssVar('--md-surface-container')
    setBackgroundTint({ tint: '#00ff00', strength: 80 }, false)
    const green = parseRgb(cssVar('--md-surface-container'))!
    expect(cssVar('--md-surface-container')).not.toBe(neutral)
    expect(green[1]).toBeGreaterThan(green[0])
    expect(cssVar('--md-primary')).toBe('rgb(255, 0, 0)')
  })

  it('normalizes bad input to safe defaults', () => {
    applyAccent([0, 120, 255], false)
    setBackgroundTint({ tint: 'not-a-color', strength: 9999 }, false)
    expect(cssVar('--md-background')).toBe(buildCleanScheme([0, 120, 255], false, { tint: 'accent', strength: 100 })['--md-background'])
  })

  it('animates the change by default', () => {
    applyAccent([0, 120, 255], false)
    setBackgroundTint({ tint: 'neutral', strength: 0 })
    expect(root().classList.contains('theme-switching')).toBe(true)
  })
})

describe('applyThemeColorVisual', () => {
  it('applies the dark accent and seed of the preset when dark', () => {
    applyThemeColorVisual('teal', true)

    expect(cssVar('--md-primary')).toBe(primary('teal', 'dark'))
    expect(cssVar('--md-seed')).toBe(scheme('teal').dark.join(', '))
    expect(cssVar('--md-surface')).not.toBe('')
  })

  it('applies the light accent on light neutral surfaces when light', () => {
    applyThemeColorVisual('rose', false)

    expect(cssVar('--md-primary')).toBe(primary('rose', 'light'))
    const surface = parseRgb(cssVar('--md-surface'))!
    expect(Math.min(...surface)).toBeGreaterThan(220)
  })

  it('reads the light-theme class when isDark is omitted', () => {
    root().classList.add('light-theme')
    applyThemeColorVisual('blue')
    expect(cssVar('--md-primary')).toBe(primary('blue', 'light'))

    root().classList.remove('light-theme')
    applyThemeColorVisual('blue')
    expect(cssVar('--md-primary')).toBe(primary('blue', 'dark'))
  })

  it('ignores unknown preset keys', () => {
    applyThemeColorVisual('green', true)
    applyThemeColorVisual('nope', true)
    expect(cssVar('--md-primary')).toBe(primary('green', 'dark'))
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

    expect(cssVar('--md-primary')).toBe(primary('orange', 'dark'))
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

  it('falls back to purple when the persisted key no longer exists', () => {
    storage.setItem('theme-color', 'removed-preset')
    expect(getSavedThemeColor()).toBe('purple')
  })

  it('clearDynamicThemeColor restores the saved preset visually without persisting', () => {
    storage.setItem('theme-color', 'brown')
    const setItem = vi.spyOn(storage, 'setItem')
    root().style.setProperty('--md-primary', 'rgb(1, 2, 3)')

    clearDynamicThemeColor(false)

    expect(cssVar('--md-primary')).toBe(primary('brown', 'light'))
    expect(setItem).not.toHaveBeenCalled()
  })
})

describe('switchThemeColorWithRipple', () => {
  it('falls back to a smooth switch when View Transitions are unsupported', async () => {
    await switchThemeColorWithRipple('green', 10, 10)

    expect(cssVar('--md-primary')).toBe(primary('green', 'dark'))
    expect(root().classList.contains('theme-switching')).toBe(true)
    expect(storage.getItem('theme-color')).toBe('green')
  })

  it('applies the preset inside the view transition and cross-fades it in', async () => {
    const { start, animate } = installViewTransition()
    root().classList.add('theme-switching')

    const done = switchThemeColorWithRipple('blue', 100, 50)
    expect(root().classList.contains('theme-ripple-active')).toBe(true)
    expect(root().classList.contains('theme-switching')).toBe(false)
    await done

    expect(start).toHaveBeenCalledTimes(1)
    expect(cssVar('--md-primary')).toBe(primary('blue', 'dark'))
    expect(animate).toHaveBeenCalledWith(
      { opacity: [0, 1] },
      expect.objectContaining({ duration: 320, fill: 'forwards', pseudoElement: '::view-transition-new(root)' }),
    )
    expect(root().classList.contains('theme-ripple-active')).toBe(false)
    expect(storage.getItem('theme-color')).toBe('blue')
  })

  it('uses the light palette when the light theme is active', async () => {
    installViewTransition()
    root().classList.add('light-theme')

    await switchThemeColorWithRipple('teal', 0, 0)

    expect(cssVar('--md-primary')).toBe(primary('teal', 'light'))
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
    expect(cssVar('--md-primary')).toBe(primary('rose', 'dark'))
  })
})
