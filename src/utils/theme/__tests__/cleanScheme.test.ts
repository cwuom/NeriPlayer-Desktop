import { describe, expect, it } from 'vitest'
import {
  accentFromSeed,
  apcaContrast,
  buildCleanScheme,
  clampTintStrength,
  contrastRatio,
  mix,
  normalizeBackgroundTint,
  parseHexColor,
  parseRgb,
  type RGB,
} from '@/utils/theme/cleanScheme'

const ACCENT: RGB = [104, 84, 204]

function token(scheme: Record<string, string>, name: string): RGB {
  const value = parseRgb(scheme[name])
  if (!value) throw new Error(`${name} is not rgb: ${scheme[name]}`)
  return value
}

/** 通道最大差值：中性灰只允许 Clean 自带的一点冷调（≤ 6） */
function channelSpread([r, g, b]: RGB): number {
  return Math.max(r, g, b) - Math.min(r, g, b)
}

describe('buildCleanScheme', () => {
  it('keeps surfaces fully neutral when the tint is neutral', () => {
    for (const dark of [false, true]) {
      const scheme = buildCleanScheme(ACCENT, dark, { tint: 'neutral', strength: 100 })
      for (const name of ['--md-background', '--md-surface-container', '--md-surface-container-high']) {
        expect(channelSpread(token(scheme, name)), name).toBeLessThanOrEqual(6)
      }
    }
  })

  it('increases the surface cast monotonically with strength', () => {
    const casts = [0, 25, 50, 100].map(strength => {
      const [r, , b] = token(buildCleanScheme([255, 0, 0], true, { tint: 'accent', strength }), '--md-surface-container')
      return r - b
    })
    for (let index = 1; index < casts.length; index++) expect(casts[index]).toBeGreaterThan(casts[index - 1])
  })

  it('tints toward a custom hex colour independently of the accent', () => {
    const scheme = buildCleanScheme(ACCENT, false, { tint: '#00ff00', strength: 100 })
    const [r, g, b] = token(scheme, '--md-background')
    expect(g).toBeGreaterThan(r)
    expect(g).toBeGreaterThan(b)
    expect(scheme['--md-primary']).toBe('rgb(104, 84, 204)')
  })

  it('keeps body text readable on the background at maximum tint', () => {
    for (const dark of [false, true]) {
      for (const tint of ['accent', '#e0a060', '#5b8def'] as const) {
        const scheme = buildCleanScheme(ACCENT, dark, { tint, strength: 100 })
        const background = token(scheme, '--md-background')
        expect(contrastRatio(token(scheme, '--md-on-surface'), background)).toBeGreaterThanOrEqual(7)
        expect(contrastRatio(token(scheme, '--md-on-surface-variant'), background)).toBeGreaterThanOrEqual(4.5)
      }
    }
  })

  it('picks a legible on-primary for light and dark accents', () => {
    for (const accent of [[20, 20, 22], [240, 240, 240], [255, 200, 0], [10, 106, 222]] as RGB[]) {
      const scheme = buildCleanScheme(accent, false, { tint: 'neutral', strength: 0 })
      expect(contrastRatio(token(scheme, '--md-on-primary'), accent)).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('puts white text on mid-tone accents instead of muddy ink', () => {
    // 回归：封面取色的棕色 / 橙、棕预设上，WCAG 2 只差零点几就选了深色字
    for (const accent of [[159, 115, 64], [200, 92, 0], [140, 90, 60]] as RGB[]) {
      const scheme = buildCleanScheme(accent, false, { tint: 'accent', strength: 12 })
      expect(scheme['--md-on-primary'], accent.join(',')).toBe('rgb(255, 255, 255)')
    }
    expect(buildCleanScheme([255, 200, 0], false, { tint: 'neutral', strength: 0 })['--md-on-primary']).toBe('rgb(20, 20, 22)')
  })

  it('emits an identical key set for every input', () => {
    const keys = Object.keys(buildCleanScheme(ACCENT, false, { tint: 'accent', strength: 12 })).sort()
    expect(Object.keys(buildCleanScheme([0, 0, 0], true, { tint: '#123456', strength: 0 })).sort()).toEqual(keys)
  })
})

describe('accentFromSeed', () => {
  const SEEDS: RGB[] = [[250, 250, 120], [20, 20, 60], [200, 40, 40], [120, 200, 255], [159, 115, 64], [230, 150, 60]]

  it('stays readable as body text, including on the tinted now-playing row', () => {
    for (const seed of SEEDS) {
      for (const dark of [false, true]) {
        const accent = accentFromSeed(seed, dark)
        for (const strength of [0, 12]) {
          const background = token(buildCleanScheme(accent, dark, { tint: 'accent', strength }), '--md-background')
          // 对应 detail-view.scss 的 .track-item.active：10% 强调色混入表面
          const activeRow = mix(background, accent, 0.1)
          expect(contrastRatio(accent, background), `${seed} ${dark} ${strength}`).toBeGreaterThanOrEqual(4.5)
          expect(contrastRatio(accent, activeRow), `${seed} ${dark} ${strength}`).toBeGreaterThanOrEqual(4.5)
        }
      }
    }
  })

  it('pairs with an on-primary that passes both WCAG and APCA', () => {
    for (const seed of SEEDS) {
      for (const dark of [false, true]) {
        const accent = accentFromSeed(seed, dark)
        const onPrimary = token(buildCleanScheme(accent, dark, { tint: 'accent', strength: 12 }), '--md-on-primary')
        expect(contrastRatio(onPrimary, accent), `${seed} ${dark}`).toBeGreaterThanOrEqual(4.5)
        // APCA 对粗体按钮文字 / 图标的下限是 Lc 45
        expect(apcaContrast(onPrimary, accent), `${seed} ${dark}`).toBeGreaterThanOrEqual(45)
      }
    }
  })
})

describe('input normalization', () => {
  it('accepts only accent, neutral or #rrggbb tints', () => {
    expect(normalizeBackgroundTint('neutral')).toBe('neutral')
    expect(normalizeBackgroundTint('#ABCDEF')).toBe('#abcdef')
    expect(normalizeBackgroundTint('#abc')).toBe('accent')
    expect(normalizeBackgroundTint(42)).toBe('accent')
  })

  it('clamps and rounds strength, defaulting on garbage', () => {
    expect(clampTintStrength(-5)).toBe(0)
    expect(clampTintStrength(140)).toBe(100)
    expect(clampTintStrength(33.6)).toBe(34)
    expect(clampTintStrength('x')).toBe(12)
  })

  it('parses hex colours', () => {
    expect(parseHexColor('#ff8000')).toEqual([255, 128, 0])
    expect(parseHexColor('nope')).toBeNull()
  })
})
