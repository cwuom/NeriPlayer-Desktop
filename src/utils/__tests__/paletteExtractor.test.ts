import { describe, expect, it } from 'vitest'
import { extractPalette, type PaletteResult } from '@/utils/paletteExtractor'
import { hueDistance, toHsl, type RGB } from './colorMath'

type RGBA = [number, number, number, number?]

/** 构造纯数据的 ImageData（extractPalette 只读 data/width/height，不需要 canvas） */
function makeImage(pixels: RGBA[]): ImageData {
  const data = new Uint8ClampedArray(pixels.length * 4)
  pixels.forEach(([r, g, b, a = 255], i) => {
    data.set([r, g, b, a], i * 4)
  })
  return { data, width: pixels.length, height: 1, colorSpace: 'srgb' }
}

function fill(color: RGBA, count = 64): RGBA[] {
  return Array.from({ length: count }, () => color)
}

function allColors(result: PaletteResult): RGB[] {
  return [...result.shaderColors, result.accentBg, result.primaryColor]
}

const RED: RGBA = [220, 30, 30]
const BLUE: RGBA = [30, 60, 220]
const YELLOW: RGBA = [240, 220, 30]

describe('extractPalette', () => {
  it('produces integer RGB channels within 0..255 for every role', () => {
    const inputs = [fill(RED), fill([255, 255, 255]), fill([0, 0, 0, 0]), [...fill(BLUE, 40), ...fill(YELLOW, 24)]]
    for (const pixels of inputs) {
      for (const dark of [true, false]) {
        for (const color of allColors(extractPalette(makeImage(pixels), 16, dark))) {
          for (const channel of color) {
            expect(Number.isInteger(channel)).toBe(true)
            expect(channel).toBeGreaterThanOrEqual(0)
            expect(channel).toBeLessThanOrEqual(255)
          }
        }
      }
    }
  })

  it('falls back to a neutral grey primary when no pixel is opaque', () => {
    const { primaryColor } = extractPalette(makeImage(fill([255, 0, 0, 0])))
    expect(primaryColor[0]).toBe(primaryColor[1])
    expect(primaryColor[1]).toBe(primaryColor[2])
  })

  it('drops pixels with alpha below 8 but keeps alpha 8', () => {
    const transparent = extractPalette(makeImage(fill([0, 0, 0, 0])))
    expect(extractPalette(makeImage(fill([255, 0, 0, 7])))).toEqual(transparent)

    const visible = extractPalette(makeImage(fill([255, 0, 0, 8])))
    expect(toHsl(visible.primaryColor).s).toBeGreaterThan(0.5)
    expect(hueDistance(toHsl(visible.primaryColor).h, 0)).toBeLessThan(5)
  })

  it('excludes transparent pixels from the colour statistics', () => {
    const mixed = [...fill([255, 0, 0, 0], 32), ...fill(BLUE, 32)]
    expect(extractPalette(makeImage(mixed))).toEqual(extractPalette(makeImage(fill(BLUE, 32))))
  })

  it('treats a pure black cover like missing colour data', () => {
    expect(extractPalette(makeImage(fill([0, 0, 0])))).toEqual(extractPalette(makeImage(fill([0, 0, 0, 0]))))
  })

  it('keeps the hue of a solid colour cover for the primary and every shader role', () => {
    const result = extractPalette(makeImage(fill(RED)))
    expect(toHsl(result.primaryColor).s).toBeGreaterThan(0.5)
    for (const color of result.shaderColors) {
      expect(hueDistance(toHsl(color).h, 0)).toBeLessThan(5)
    }
  })

  it('mirrors the shader roles in the legacy fields', () => {
    const result = extractPalette(makeImage([...fill(BLUE, 40), ...fill(YELLOW, 24)]))
    expect(result.primaryColor).toEqual(result.shaderColors[0])
    expect(result.dominant).toEqual(result.shaderColors[0])
    expect(result.lightVibrant).toEqual(result.shaderColors[1])
    expect(result.muted).toEqual(result.shaderColors[2])
    expect(result.darkMuted).toEqual(result.shaderColors[3])
  })

  it('uses the dominant colour as primary and the most colourful minority as accent', () => {
    const result = extractPalette(makeImage([...fill(BLUE, 45), ...fill(YELLOW, 19)]))
    expect(hueDistance(toHsl(result.primaryColor).h, toHsl([30, 60, 220]).h)).toBeLessThan(10)
    expect(hueDistance(toHsl(result.shaderColors[1]).h, toHsl([240, 220, 30]).h)).toBeLessThan(15)
  })

  it('averages every pixel into a single colour when maxColors is 1', () => {
    const result = extractPalette(makeImage([...fill([255, 0, 0], 32), ...fill([0, 0, 255], 32)]), 1)
    // 红蓝平均为紫色（300°）
    expect(hueDistance(toHsl(result.primaryColor).h, 300)).toBeLessThan(5)
  })

  it('gives a fully grey cover a saturated magenta accent so the shader is not monochrome', () => {
    const result = extractPalette(makeImage(fill([128, 128, 128])))
    expect(toHsl(result.primaryColor).s).toBe(0)
    const accent = toHsl(result.shaderColors[1])
    expect(accent.s).toBeGreaterThanOrEqual(0.3)
    expect(hueDistance(accent.h, 320)).toBeLessThan(5)
  })

  it('renders a lighter primary in light mode than in dark mode', () => {
    const dark = extractPalette(makeImage(fill(RED)), 16, true)
    const light = extractPalette(makeImage(fill(RED)), 16, false)
    expect(toHsl(light.primaryColor).l).toBeGreaterThan(toHsl(dark.primaryColor).l)
  })

  it('uses a negative light offset and stronger saturate offset in dark mode', () => {
    const dark = extractPalette(makeImage(fill(RED)), 16, true)
    const light = extractPalette(makeImage(fill(RED)), 16, false)
    expect(dark.lightOffset).toBeLessThan(0)
    expect(light.lightOffset).toBeGreaterThan(0)
    expect(dark.saturateOffset).toBe(0.24)
    expect(light.saturateOffset).toBe(0.16)
  })

  it('keeps the light offset within ±0.12 for extreme covers', () => {
    for (const color of [[255, 255, 255], [0, 0, 0], [10, 10, 10], [250, 250, 0]] satisfies RGBA[]) {
      for (const dark of [true, false]) {
        const { lightOffset } = extractPalette(makeImage(fill(color)), 16, dark)
        expect(Math.abs(lightOffset)).toBeLessThanOrEqual(0.12)
      }
    }
  })

  it('clamps shader lightness into the mode-specific range', () => {
    const tolerance = 0.01
    for (const color of [[255, 255, 255], [5, 5, 30], RED] satisfies RGBA[]) {
      for (const shader of extractPalette(makeImage(fill(color)), 16, true).shaderColors) {
        const { l } = toHsl(shader)
        expect(l).toBeGreaterThanOrEqual(0.12 - tolerance)
        expect(l).toBeLessThanOrEqual(0.66 + tolerance)
      }
      for (const shader of extractPalette(makeImage(fill(color)), 16, false).shaderColors) {
        const { l } = toHsl(shader)
        expect(l).toBeGreaterThanOrEqual(0.28 - tolerance)
        expect(l).toBeLessThanOrEqual(0.86 + tolerance)
      }
    }
  })

  it('caps shader saturation so vivid covers do not oversaturate', () => {
    const tolerance = 0.02
    const vivid: RGBA = [255, 0, 0]
    const dark = extractPalette(makeImage(fill(vivid)), 16, true).shaderColors
    expect(toHsl(dark[1]).s).toBeLessThanOrEqual(0.70 + tolerance)
    for (const index of [0, 2, 3, 4]) expect(toHsl(dark[index]).s).toBeLessThanOrEqual(0.64 + tolerance)

    const light = extractPalette(makeImage(fill(vivid)), 16, false).shaderColors
    expect(toHsl(light[1]).s).toBeLessThanOrEqual(0.64 + tolerance)
    for (const index of [0, 2, 3, 4]) expect(toHsl(light[index]).s).toBeLessThanOrEqual(0.56 + tolerance)
  })

  it('derives a dark backdrop in dark mode and a near-white backdrop in light mode', () => {
    const dark = toHsl(extractPalette(makeImage(fill(BLUE)), 16, true).accentBg)
    const light = toHsl(extractPalette(makeImage(fill(BLUE)), 16, false).accentBg)
    expect(dark.l).toBeLessThanOrEqual(0.32)
    expect(light.l).toBeGreaterThanOrEqual(0.85)
  })

  it('keeps the hue of a saturated cover in the backdrop', () => {
    for (const color of [RED, BLUE, YELLOW]) {
      const rgb: RGB = [color[0], color[1], color[2]]
      const { accentBg } = extractPalette(makeImage(fill(color)))
      expect(hueDistance(toHsl(accentBg).h, toHsl(rgb).h)).toBeLessThan(10)
      expect(toHsl(accentBg).s).toBeGreaterThan(0.2)
    }
  })

  describe('themeSeed', () => {
    const WHITE: RGBA = [248, 246, 244]
    const SKIN: RGBA = [214, 160, 130]
    const chroma = ([r, g, b]: RGB) => (Math.max(r, g, b) - Math.min(r, g, b)) / 255

    it('follows the larger coloured area on a fully coloured cover', () => {
      for (const dark of [true, false]) {
        const result = extractPalette(makeImage([...fill(BLUE, 45), ...fill(YELLOW, 19)]), 16, dark)
        expect(hueDistance(toHsl(result.themeSeed).h, toHsl([30, 60, 220]).h)).toBeLessThan(10)
      }
    })

    it('takes the warm tone of a muted sepia photo with a grey sky', () => {
      const result = extractPalette(makeImage([...fill([150, 148, 146], 70), ...fill([120, 100, 80], 30)]))
      expect(hueDistance(toHsl(result.themeSeed).h, toHsl([120, 100, 80]).h)).toBeLessThan(12)
      expect(toHsl(result.themeSeed).s).toBeGreaterThan(0.08)
    })

    it('keeps the cast of a muted cover that has little saturated area', () => {
      const result = extractPalette(makeImage([...fill([90, 78, 53], 60), ...fill([40, 36, 30], 40)]))
      expect(chroma(result.themeSeed)).toBeGreaterThan(0.08)
      expect(hueDistance(toHsl(result.themeSeed).h, toHsl([90, 78, 53]).h)).toBeLessThan(15)
    })

    it('keeps the warm cast of a dark sepia cover', () => {
      // 《发如雪》这类暗调棕褐封面：平均色 ≈ (38,33,23)，色度只有 0.06，但不能退成中性灰
      const result = extractPalette(makeImage([...fill([30, 26, 18], 80), ...fill([70, 61, 42], 20)]))
      // accentFromSeed 以 HSL 饱和度 < 0.08 判定中性
      expect(toHsl(result.themeSeed).s).toBeGreaterThan(0.08)
      expect(hueDistance(toHsl(result.themeSeed).h, toHsl([38, 33, 23]).h)).toBeLessThan(15)
    })

    it('falls back to neutral grey when no pixel is opaque', () => {
      expect(chroma(extractPalette(makeImage(fill([255, 0, 0, 0]))).themeSeed)).toBe(0)
    })

    it('takes the coloured subject of a portrait on a white backdrop instead of the grey', () => {
      // 回归：《情歌》这类白底人像封面，dominant 取到灰色，全局强调色因此退成中性灰
      for (const dark of [true, false]) {
        const result = extractPalette(makeImage([...fill(WHITE, 1400), ...fill([60, 58, 56], 400), ...fill(SKIN, 300)]), 16, dark)
        expect(chroma(result.themeSeed)).toBeGreaterThan(0.12)
        expect(hueDistance(toHsl(result.themeSeed).h, toHsl([214, 160, 130]).h)).toBeLessThan(12)
      }
    })

    it('stays neutral for a black-and-white cover', () => {
      const result = extractPalette(makeImage([...fill(WHITE, 40), ...fill([20, 20, 20], 40), ...fill([128, 128, 128], 20)]))
      expect(chroma(result.themeSeed)).toBeLessThan(0.08)
    })

    it('ignores a coloured speck that covers less than 5% of the cover', () => {
      const result = extractPalette(makeImage([...fill(WHITE, 980), ...fill([220, 30, 30], 20)]))
      expect(chroma(result.themeSeed)).toBeLessThan(0.08)
    })

    it('picks the hue with the larger coloured area when the backdrop is grey', () => {
      const result = extractPalette(makeImage([...fill(WHITE, 600), ...fill(BLUE, 300), ...fill(RED, 100)]))
      expect(hueDistance(toHsl(result.themeSeed).h, toHsl([30, 60, 220]).h)).toBeLessThan(10)
    })
  })
})
