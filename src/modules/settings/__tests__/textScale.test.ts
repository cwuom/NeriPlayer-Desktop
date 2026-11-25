// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { applyTextScale, normalizeTextScale } from '@/modules/settings/textScale'

describe('text scale', () => {
  it('clamps to 85–150 and snaps to 5% steps', () => {
    expect(normalizeTextScale(10)).toBe(85)
    expect(normalizeTextScale(400)).toBe(150)
    expect(normalizeTextScale(112)).toBe(110)
    expect(normalizeTextScale(113)).toBe(115)
    expect(normalizeTextScale('big')).toBe(100)
    expect(normalizeTextScale(Number.NaN)).toBe(100)
  })

  it('writes the normalized factor to --text-scale', () => {
    applyTextScale(125)
    expect(document.documentElement.style.getPropertyValue('--text-scale')).toBe('1.25')
    applyTextScale(9999)
    expect(document.documentElement.style.getPropertyValue('--text-scale')).toBe('1.5')
  })
})
