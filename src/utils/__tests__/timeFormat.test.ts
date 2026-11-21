import { describe, expect, it } from 'vitest'
import { formatTimeMs, formatTrackDuration } from '@/utils/timeFormat'

describe('formatTimeMs', () => {
  it('formats zero as 0:00', () => {
    expect(formatTimeMs(0)).toBe('0:00')
  })

  it('floors partial seconds instead of rounding up', () => {
    expect(formatTimeMs(999)).toBe('0:00')
    expect(formatTimeMs(59_999)).toBe('0:59')
  })

  it('pads seconds to two digits and rolls over into minutes', () => {
    expect(formatTimeMs(5_000)).toBe('0:05')
    expect(formatTimeMs(60_000)).toBe('1:00')
    expect(formatTimeMs(185_400)).toBe('3:05')
  })

  it('keeps counting minutes past one hour without an hour field', () => {
    expect(formatTimeMs(3_600_000)).toBe('60:00')
    expect(formatTimeMs(7_325_000)).toBe('122:05')
  })

  it('clamps negative positions to 0:00', () => {
    expect(formatTimeMs(-1)).toBe('0:00')
    expect(formatTimeMs(-90_000)).toBe('0:00')
  })
})

describe('formatTrackDuration', () => {
  it('shows a placeholder when the duration is missing or not positive', () => {
    expect(formatTrackDuration()).toBe('--:--')
    expect(formatTrackDuration(null)).toBe('--:--')
    expect(formatTrackDuration(0)).toBe('--:--')
    expect(formatTrackDuration(-5_000)).toBe('--:--')
  })

  it('shows a placeholder for non-finite durations', () => {
    expect(formatTrackDuration(Number.NaN)).toBe('--:--')
    expect(formatTrackDuration(Number.POSITIVE_INFINITY)).toBe('--:--')
  })

  it('formats positive durations like formatTimeMs', () => {
    expect(formatTrackDuration(245_000)).toBe('4:05')
    expect(formatTrackDuration(1)).toBe('0:00')
  })
})
