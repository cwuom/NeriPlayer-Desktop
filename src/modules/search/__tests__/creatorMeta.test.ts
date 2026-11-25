import { describe, expect, it } from 'vitest'
import { biliCreatorStats, formatCompactCount } from '@/modules/search/creatorMeta'

describe('formatCompactCount', () => {
  it('abbreviates large numbers per locale', () => {
    expect(formatCompactCount(123_456, 'zh-CN')).toBe('12.3万')
    expect(formatCompactCount(123_456, 'en')).toBe('123.5K')
    expect(formatCompactCount(999, 'en')).toBe('999')
  })

  it('treats invalid or negative values as zero', () => {
    expect(formatCompactCount(Number.NaN, 'en')).toBe('0')
    expect(formatCompactCount(-5, 'en')).toBe('0')
  })

  it('falls back to plain digits for an unknown locale tag', () => {
    expect(formatCompactCount(42, 'not a locale!')).toBe('42')
  })
})

describe('biliCreatorStats', () => {
  it('formats follower and video counts, defaulting missing videos to zero', () => {
    expect(biliCreatorStats({ follower_count: 1_500_000, video_count: 35 }, 'en')).toEqual({ fans: '1.5M', videos: '35' })
    expect(biliCreatorStats({ follower_count: 0 }, 'en')).toEqual({ fans: '0', videos: '0' })
  })

  it('returns null without a follower count so callers can fall back', () => {
    expect(biliCreatorStats({}, 'en')).toBeNull()
    expect(biliCreatorStats({ follower_count: null, video_count: 3 }, 'en')).toBeNull()
  })
})
