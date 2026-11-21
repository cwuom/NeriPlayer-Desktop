import { describe, expect, it } from 'vitest'
import { accentBackdropColor, buildDynamicColorVars } from '../dynamicColors'

function channels(color: string | undefined): number[] {
  const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(color ?? '')
  expect(match).not.toBeNull()
  return match!.slice(1).map(Number)
}

describe('accentBackdropColor', () => {
  it('falls back to the neutral backdrop without a palette', () => {
    expect(accentBackdropColor(null)).toBe('rgb(18, 18, 18)')
    expect(accentBackdropColor(undefined)).toBe('rgb(18, 18, 18)')
  })

  it('keeps dark colors and darkens bright ones toward the neutral backdrop', () => {
    expect(accentBackdropColor([40, 50, 60])).toBe('rgb(40, 50, 60)')
    const darkened = channels(accentBackdropColor([255, 255, 255]))
    expect(darkened).toEqual([65, 65, 65])
  })
})

describe('buildDynamicColorVars', () => {
  it('derives every theme variable and shares the primary with the waveform thumb', () => {
    const vars = buildDynamicColorVars([32, 32, 180])
    expect(Object.keys(vars).sort()).toEqual([
      '--np-on-primary',
      '--np-on-surface',
      '--np-on-surface-variant',
      '--np-primary',
      '--np-primary-container',
      '--waveform-thumb-color',
    ])
    expect(vars['--waveform-thumb-color']).toBe(vars['--np-primary'])
    expect(vars['--np-on-surface']).toBe('rgba(255,255,255,1)')
  })

  it('keeps neutral seeds neutral', () => {
    const [r, g, b] = channels(buildDynamicColorVars([128, 128, 128])['--np-primary'])
    expect(r).toBe(g)
    expect(g).toBe(b)
  })

  it('uses dark text on the light primary container', () => {
    expect(buildDynamicColorVars([180, 32, 32])['--np-on-primary']).toBe('rgb(20, 18, 24)')
  })
})
