import { describe, expect, it } from 'vitest'
import { titlebarHeightInPoints } from '@/integrations/macTrafficLights'

describe('titlebarHeightInPoints', () => {
  it('converts CSS pixels to points with the current UI zoom', () => {
    expect(titlebarHeightInPoints(52, 1)).toBe(52)
    expect(titlebarHeightInPoints(52, 1.25)).toBe(65)
    // 缩小时 CSS 侧按 52 / scale 补偿，换算回来仍是 52pt
    expect(titlebarHeightInPoints(52 / 0.8, 0.8)).toBe(52)
  })

  it('treats a missing or broken scale as 100%', () => {
    expect(titlebarHeightInPoints(52, Number.NaN)).toBe(52)
    expect(titlebarHeightInPoints(52, 0)).toBe(52)
  })
})
