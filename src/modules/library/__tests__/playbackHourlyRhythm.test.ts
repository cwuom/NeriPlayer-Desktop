import { describe, expect, it } from 'vitest'
import {
  buildHourlyRhythm,
  clampHour,
  hourBarRatio,
  isHourAxisTick,
} from '@/modules/library/playbackHourlyRhythm'

function hours(patch: Record<number, unknown> = {}): unknown[] {
  return Array.from({ length: 24 }, (_, hour) => patch[hour] ?? 0)
}

describe('buildHourlyRhythm', () => {
  it('picks the busiest hour as the peak', () => {
    const rhythm = buildHourlyRhythm(hours({ 6: 45_000, 19: 90_000, 22: 90_000 }))
    expect(rhythm?.peakIndex).toBe(19)
    expect(rhythm?.peakMs).toBe(90_000)
    expect(rhythm?.hours).toHaveLength(24)
  })

  it('hides the chart for missing, short, or empty data from older backends', () => {
    expect(buildHourlyRhythm(undefined)).toBeNull()
    expect(buildHourlyRhythm(null)).toBeNull()
    expect(buildHourlyRhythm([1, 2, 3])).toBeNull()
    expect(buildHourlyRhythm(hours())).toBeNull()
  })

  it('treats malformed values as zero instead of breaking the page', () => {
    const rhythm = buildHourlyRhythm(hours({ 0: -5, 1: Number.NaN, 2: '7', 3: null, 4: 10 }))
    expect(rhythm?.hours.slice(0, 5)).toEqual([0, 0, 0, 0, 10])
    expect(rhythm?.peakIndex).toBe(4)
  })
})

describe('hour chart helpers', () => {
  it('keeps small but non-zero hours visible', () => {
    expect(hourBarRatio(0, 100)).toBe(0)
    expect(hourBarRatio(1, 1_000_000)).toBe(0.04)
    expect(hourBarRatio(50, 100)).toBe(0.5)
    expect(hourBarRatio(200, 100)).toBe(1)
  })

  it('labels the same axis ticks as Android and clamps selection', () => {
    expect(Array.from({ length: 24 }, (_, hour) => hour).filter(isHourAxisTick)).toEqual([0, 6, 12, 18, 23])
    expect(clampHour(-1)).toBe(0)
    expect(clampHour(24)).toBe(23)
    expect(clampHour(7.9)).toBe(7)
  })
})
