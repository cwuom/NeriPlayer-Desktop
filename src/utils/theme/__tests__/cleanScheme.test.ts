import { describe, expect, it } from 'vitest'
import {
  accentFromSeed,
  buildCleanScheme,
  clampTintStrength,
  contrastRatio,
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

  it('emits an identical key set for every input', () => {
    const keys = Object.keys(buildCleanScheme(ACCENT, false, { tint: 'accent', strength: 12 })).sort()
    expect(Object.keys(buildCleanScheme([0, 0, 0], true, { tint: '#123456', strength: 0 })).sort()).toEqual(keys)
  })
})

describe('accentFromSeed', () => {
  it('reaches at least 3:1 against the clean background in both modes', () => {
    for (const seed of [[250, 250, 120], [20, 20, 60], [200, 40, 40], [120, 200, 255]] as RGB[]) {
      for (const dark of [false, true]) {
        const background = token(buildCleanScheme(ACCENT, dark, { tint: 'neutral', strength: 0 }), '--md-background')
        expect(contrastRatio(accentFromSeed(seed, dark), background)).toBeGreaterThanOrEqual(3)
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
