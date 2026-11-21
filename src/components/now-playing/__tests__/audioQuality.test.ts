import { describe, expect, it } from 'vitest'
import { qualityKeyForSource, qualityOptionsForSource, setQualityKeyForSource, type QualitySettings } from '../audioQuality'

function settings(): QualitySettings {
  return { neteaseQuality: 'exhigh', qqMusicQuality: 'high', biliQuality: 'high', youtubeQuality: 'very_high' }
}

describe('audio quality options', () => {
  it('lists each platform in ascending quality order and nothing for local files', () => {
    expect(qualityOptionsForSource('netease').map(option => option.key))
      .toEqual(['standard', 'higher', 'exhigh', 'lossless', 'hires', 'jyeffect', 'sky', 'jymaster'])
    expect(qualityOptionsForSource('qq').map(option => option.key)).toEqual(['standard', 'high', 'lossless'])
    expect(qualityOptionsForSource('youtube').map(option => option.key)).toEqual(['low', 'medium', 'high', 'very_high'])
    expect(qualityOptionsForSource('bilibili').map(option => option.key))
      .toEqual(['low', 'medium', 'high', 'lossless', 'hires', 'dolby'])
    expect(qualityOptionsForSource('local')).toEqual([])
  })

  it('uses i18n keys for labels', () => {
    for (const source of ['netease', 'qq', 'youtube', 'bilibili']) {
      expect(qualityOptionsForSource(source).every(option => option.label.startsWith('settings.q_'))).toBe(true)
    }
  })
})

describe('quality setting per source', () => {
  it('reads the matching setting field', () => {
    const current = settings()
    expect(qualityKeyForSource(current, 'netease')).toBe('exhigh')
    expect(qualityKeyForSource(current, 'qq')).toBe('high')
    expect(qualityKeyForSource(current, 'bilibili')).toBe('high')
    expect(qualityKeyForSource(current, 'youtube')).toBe('very_high')
    expect(qualityKeyForSource(current, 'local')).toBe('')
  })

  it('writes only the matching setting field', () => {
    const current = settings()
    setQualityKeyForSource(current, 'qq', 'lossless')
    expect(current).toEqual({ ...settings(), qqMusicQuality: 'lossless' })
    setQualityKeyForSource(current, 'local', 'lossless')
    expect(current).toEqual({ ...settings(), qqMusicQuality: 'lossless' })
  })
})
