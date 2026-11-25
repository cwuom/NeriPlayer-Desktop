// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  applyDynamicColorFromCover,
  clearDynamicColor,
  isDynamicColorActive,
  reapplyDynamicColorForTheme,
} from '@/utils/colorExtractor'
import type { PaletteResult } from '@/utils/paletteExtractor'
import { THEME_COLORS, cancelSmoothThemeTransition } from '@/utils/themeColor'
import { accentFromSeed, contrastRatio, rgb } from '@/utils/theme/cleanScheme'
import { hueDistance, parseRgb, toHsl, type Hsl, type RGB } from './colorMath'
import { FakeImage, installFakeImage } from './fakeImage'
import { installMemoryStorage, type MemoryStorage } from './memoryStorage'

const coverMocks = vi.hoisted(() => ({
  peekCoverImage: vi.fn<(url: string) => string>(),
  resolveCoverImage: vi.fn<(url: string) => Promise<string>>(),
}))
const extractPalette = vi.hoisted(() =>
  vi.fn<(imageData: ImageData, maxColors?: number, isDark?: boolean) => PaletteResult>(),
)

// 封面代理走 Tauri invoke，这里只替换取图入口，URL 规范化保留真实实现
vi.mock('@/utils/bilibiliCover', async () => {
  const actual = await vi.importActual<typeof import('@/utils/bilibiliCoverCache')>('@/utils/bilibiliCoverCache')
  return {
    ...coverMocks,
    normalizeProxiedCoverUrl: actual.normalizeProxiedCoverUrl,
    normalizeCoverUrlForDisplay: actual.normalizeCoverUrlForDisplay,
  }
})
// 取色算法由 paletteExtractor.test.ts 覆盖，这里固定种子色以便精确断言令牌生成
vi.mock('@/utils/paletteExtractor', () => ({ extractPalette }))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))

const PROXIED_COVER = 'https://p1.music.126.net/cover.jpg'
const DATA_URL = `data:image/png;base64,${Buffer.from('png').toString('base64')}`
const SAMPLE_SIZE = 96

const sampledImageData: ImageData = {
  data: new Uint8ClampedArray(SAMPLE_SIZE * SAMPLE_SIZE * 4),
  width: SAMPLE_SIZE,
  height: SAMPLE_SIZE,
  colorSpace: 'srgb',
}

// 只提供被调用的两个 2D 方法，返回固定像素；不模拟任何绘制
const context2d = {
  drawImage: vi.fn<(image: unknown, x: number, y: number, w: number, h: number) => void>(),
  getImageData: vi.fn<(x: number, y: number, w: number, h: number) => ImageData>(),
}
const getContext = vi.fn<(type: string, options?: unknown) => typeof context2d | null>()
const originalGetContext = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, 'getContext')

let storage: MemoryStorage

function cssVar(name: string): string {
  return document.documentElement.style.getPropertyValue(name)
}

function tokenHsl(name: string): Hsl {
  return toHsl(parseRgb(cssVar(name)))
}

function paletteWithPrimary(primary: RGB): PaletteResult {
  return {
    shaderColors: [primary, primary, primary, primary, primary],
    lightOffset: 0,
    saturateOffset: 0,
    accentBg: primary,
    primaryColor: primary,
    dominant: primary,
    lightVibrant: primary,
    muted: primary,
    darkMuted: primary,
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve: (value: T) => void = () => {}
  const promise = new Promise<T>((res) => { resolve = res })
  return { promise, resolve }
}

beforeEach(() => {
  storage = installMemoryStorage()
  installFakeImage()
  document.documentElement.removeAttribute('style')
  document.documentElement.className = ''
  coverMocks.peekCoverImage.mockReset().mockReturnValue('')
  coverMocks.resolveCoverImage.mockReset().mockResolvedValue(DATA_URL)
  extractPalette.mockReset().mockReturnValue(paletteWithPrimary([200, 40, 40]))
  context2d.drawImage.mockReset()
  context2d.getImageData.mockReset().mockReturnValue(sampledImageData)
  getContext.mockReset().mockReturnValue(context2d)
  Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', { value: getContext, configurable: true })
})

afterEach(() => {
  // 模块级状态（isActive/lastSeed）跨用例共享，逐个复位
  clearDynamicColor()
  cancelSmoothThemeTransition()
  if (originalGetContext) Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', originalGetContext)
  vi.unstubAllGlobals()
})

describe('applyDynamicColorFromCover', () => {
  it('returns false and leaves the theme alone when there is no usable URL', async () => {
    await expect(applyDynamicColorFromCover('', true)).resolves.toBe(false)
    await expect(applyDynamicColorFromCover('   ', true)).resolves.toBe(false)

    expect(isDynamicColorActive()).toBe(false)
    expect(FakeImage.instances).toHaveLength(0)
    expect(cssVar('--md-primary')).toBe('')
  })

  it('uses the synchronously cached proxied cover without a backend round-trip', async () => {
    coverMocks.peekCoverImage.mockReturnValue(DATA_URL)

    await expect(applyDynamicColorFromCover(PROXIED_COVER, true)).resolves.toBe(true)

    expect(coverMocks.peekCoverImage).toHaveBeenCalledWith(PROXIED_COVER)
    expect(coverMocks.resolveCoverImage).not.toHaveBeenCalled()
    expect(FakeImage.instances[0].src).toBe(DATA_URL)
    expect(FakeImage.instances[0].crossOrigin).toBeNull()
    expect(FakeImage.instances[0].referrerPolicy).toBe('no-referrer')
  })

  it('resolves an uncached proxied cover to a data URL through the backend', async () => {
    await expect(applyDynamicColorFromCover('//p1.music.126.net/cover.jpg#x', true)).resolves.toBe(true)

    expect(coverMocks.resolveCoverImage).toHaveBeenCalledWith(PROXIED_COVER)
    expect(FakeImage.instances[0].src).toBe(DATA_URL)
  })

  it('falls back to the display URL with anonymous CORS when the backend proxy fails', async () => {
    coverMocks.resolveCoverImage.mockRejectedValue(new Error('proxy down'))

    await expect(applyDynamicColorFromCover(PROXIED_COVER, true)).resolves.toBe(true)

    expect(FakeImage.instances[0].src).toBe(PROXIED_COVER)
    expect(FakeImage.instances[0].crossOrigin).toBe('anonymous')
  })

  it('loads covers from non-proxied hosts directly with anonymous CORS', async () => {
    await expect(applyDynamicColorFromCover('//img.example.com/a.jpg', false)).resolves.toBe(true)

    expect(coverMocks.peekCoverImage).not.toHaveBeenCalled()
    expect(coverMocks.resolveCoverImage).not.toHaveBeenCalled()
    expect(FakeImage.instances[0].src).toBe('https://img.example.com/a.jpg')
    expect(FakeImage.instances[0].crossOrigin).toBe('anonymous')
  })

  it('samples the cover at 96x96 and extracts 16 colours for the requested mode', async () => {
    await applyDynamicColorFromCover(PROXIED_COVER, false)

    expect(getContext).toHaveBeenCalledWith('2d', { willReadFrequently: true })
    expect(context2d.drawImage).toHaveBeenCalledWith(FakeImage.instances[0], 0, 0, SAMPLE_SIZE, SAMPLE_SIZE)
    expect(context2d.getImageData).toHaveBeenCalledWith(0, 0, SAMPLE_SIZE, SAMPLE_SIZE)
    expect(extractPalette).toHaveBeenCalledWith(sampledImageData, 16, false)
  })

  it('marks dynamic colour active, records the seed, and starts a smooth transition', async () => {
    await applyDynamicColorFromCover(PROXIED_COVER, true)

    expect(isDynamicColorActive()).toBe(true)
    expect(cssVar('--md-primary')).toBe(rgb(accentFromSeed([200, 40, 40], true)))
    expect(document.documentElement.classList.contains('theme-switching')).toBe(true)
  })

  it('returns false when the cover image fails to load', async () => {
    FakeImage.outcome = 'error'

    await expect(applyDynamicColorFromCover(PROXIED_COVER, true)).resolves.toBe(false)

    expect(isDynamicColorActive()).toBe(false)
    expect(extractPalette).not.toHaveBeenCalled()
  })

  it('returns false when no 2D canvas context is available', async () => {
    getContext.mockReturnValue(null)

    await expect(applyDynamicColorFromCover(PROXIED_COVER, true)).resolves.toBe(false)
    expect(extractPalette).not.toHaveBeenCalled()
  })

  it('returns false when reading pixels from a tainted canvas throws', async () => {
    context2d.getImageData.mockImplementation(() => {
      throw new DOMException('tainted', 'SecurityError')
    })

    await expect(applyDynamicColorFromCover('https://img.example.com/a.jpg', true)).resolves.toBe(false)
    expect(cssVar('--md-primary')).toBe('')
  })

  it('discards a request superseded by a newer cover while the proxy is resolving', async () => {
    const slowProxy = deferred<string>()
    coverMocks.resolveCoverImage.mockReturnValueOnce(slowProxy.promise)

    const stale = applyDynamicColorFromCover(PROXIED_COVER, true)
    extractPalette.mockReturnValue(paletteWithPrimary([40, 200, 40]))
    await expect(applyDynamicColorFromCover('https://img.example.com/new.jpg', true)).resolves.toBe(true)
    slowProxy.resolve(DATA_URL)

    await expect(stale).resolves.toBe(false)
    expect(extractPalette).toHaveBeenCalledTimes(1)
    expect(cssVar('--md-primary')).toBe(rgb(accentFromSeed([40, 200, 40], true)))
  })

  it('discards an in-flight request when dynamic colour is cleared during image load', async () => {
    const pending = applyDynamicColorFromCover('https://img.example.com/a.jpg', true)
    clearDynamicColor()

    await expect(pending).resolves.toBe(false)
    expect(isDynamicColorActive()).toBe(false)
    expect(extractPalette).not.toHaveBeenCalled()
  })
})

describe('generated dynamic tokens', () => {
  it('emits the same clean token set as the presets so both are interchangeable', async () => {
    await applyDynamicColorFromCover(PROXIED_COVER, true)

    for (const token of ['--md-primary', '--md-on-primary', '--md-primary-container', '--md-secondary-container',
      '--md-tertiary', '--md-surface', '--md-surface-container', '--md-outline-variant']) {
      expect(() => parseRgb(cssVar(token)), token).not.toThrow()
    }
  })

  it('keeps the seed hue on the accent while surfaces stay near neutral', async () => {
    await applyDynamicColorFromCover(PROXIED_COVER, true)

    expect(hueDistance(tokenHsl('--md-primary').h, toHsl([200, 40, 40]).h)).toBeLessThan(5)
    // Clean：封面颜色只落在强调色上，深色表面饱和度保持很低
    expect(tokenHsl('--md-surface').s).toBeLessThan(0.2)
    expect(tokenHsl('--md-surface').l).toBeLessThan(0.12)
  })

  it('keeps the accent readable against the background in both modes', async () => {
    for (const dark of [true, false]) {
      await applyDynamicColorFromCover(PROXIED_COVER, dark)
      const ratio = contrastRatio(parseRgb(cssVar('--md-primary')), parseRgb(cssVar('--md-background')))
      expect(ratio).toBeGreaterThanOrEqual(3)
    }
  })

  it('maps a grey seed to a neutral ink accent instead of inventing a hue', async () => {
    extractPalette.mockReturnValue(paletteWithPrimary([128, 128, 128]))
    await applyDynamicColorFromCover(PROXIED_COVER, true)

    expect(tokenHsl('--md-primary').s).toBeLessThan(0.15)
  })

  it('caps a fully saturated seed so the accent does not glare', async () => {
    extractPalette.mockReturnValue(paletteWithPrimary([255, 0, 0]))
    await applyDynamicColorFromCover(PROXIED_COVER, false)

    expect(tokenHsl('--md-primary').s).toBeLessThanOrEqual(0.79)
  })
})

describe('reapplyDynamicColorForTheme', () => {
  it('returns false while dynamic colour is inactive', () => {
    expect(reapplyDynamicColorForTheme(true)).toBe(false)
    expect(cssVar('--md-primary')).toBe('')
  })

  it('recomputes tokens for the new mode from the cached seed', async () => {
    await applyDynamicColorFromCover(PROXIED_COVER, true)
    const darkPrimary = cssVar('--md-primary')

    expect(reapplyDynamicColorForTheme(false)).toBe(true)

    expect(cssVar('--md-primary')).not.toBe(darkPrimary)
    expect(cssVar('--md-primary')).toBe(rgb(accentFromSeed([200, 40, 40], false)))
    expect(cssVar('--md-on-primary')).toBe('rgb(255, 255, 255)')
    expect(extractPalette).toHaveBeenCalledTimes(1)
  })

  it('returns false again after dynamic colour is cleared', async () => {
    await applyDynamicColorFromCover(PROXIED_COVER, true)
    clearDynamicColor(true)
    expect(reapplyDynamicColorForTheme(false)).toBe(false)
  })
})

describe('clearDynamicColor', () => {
  it('restores the saved preset theme colour with a smooth transition', async () => {
    storage.setItem('theme-color', 'teal')
    await applyDynamicColorFromCover(PROXIED_COVER, true)
    cancelSmoothThemeTransition()

    clearDynamicColor(true)

    const teal = THEME_COLORS.find(c => c.key === 'teal')!
    expect(cssVar('--md-primary')).toBe(rgb(teal.dark))
    expect(cssVar('--md-seed')).toBe(teal.dark.join(', '))
    expect(isDynamicColorActive()).toBe(false)
    expect(document.documentElement.classList.contains('theme-switching')).toBe(true)
  })

  it('does nothing visible when dynamic colour is not active', () => {
    document.documentElement.style.setProperty('--md-primary', 'rgb(1, 2, 3)')

    clearDynamicColor(true)

    expect(cssVar('--md-primary')).toBe('rgb(1, 2, 3)')
    expect(document.documentElement.classList.contains('theme-switching')).toBe(false)
  })
})
