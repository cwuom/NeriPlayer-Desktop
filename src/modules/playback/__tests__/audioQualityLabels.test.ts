import { describe, expect, it } from 'vitest'
import {
  qualityLabelFromKey,
  qualityOptionsFromSource,
  resolvedPlaybackAudioInfo,
} from '@/modules/playback/audioQualityLabels'
import type { ResolvedPlaybackSource } from '@/modules/playback/playbackSource'

function resolved(overrides: Partial<ResolvedPlaybackSource> = {}): ResolvedPlaybackSource {
  return {
    type: 'success',
    url: 'https://audio.example/track.flac',
    candidateUrls: [],
    cacheKey: 'netease-1-hires',
    source: 'netease',
    qualityKey: 'hires',
    ...overrides,
  }
}

describe('qualityLabelFromKey', () => {
  it('maps platform quality keys to their display names regardless of case and padding', () => {
    expect(qualityLabelFromKey('netease', 'exhigh')).toBe('极高')
    expect(qualityLabelFromKey('qq', ' Lossless ')).toBe('无损')
    expect(qualityLabelFromKey('youtube', 'very_high')).toBe('最高')
    expect(qualityLabelFromKey('bilibili', 'dolby')).toBe('多声道（E-AC-3）')
  })

  it('keeps unknown keys and sources as the raw key and ignores empty keys', () => {
    expect(qualityLabelFromKey('netease', 'custom')).toBe('custom')
    expect(qualityLabelFromKey('local', 'hires')).toBe('hires')
    expect(qualityLabelFromKey('netease', '  ')).toBeUndefined()
    expect(qualityLabelFromKey('netease', null)).toBeUndefined()
  })
})

describe('qualityOptionsFromSource', () => {
  it('lists the options of a known platform in display order', () => {
    expect(qualityOptionsFromSource('qq')).toEqual([
      { key: 'standard', label: '标准' },
      { key: 'high', label: '高' },
      { key: 'lossless', label: '无损' },
    ])
    expect(qualityOptionsFromSource('netease')?.map(option => option.key)).toEqual([
      'standard', 'higher', 'exhigh', 'lossless', 'hires', 'jyeffect', 'sky', 'jymaster',
    ])
  })

  it('has no options for sources without quality selection', () => {
    expect(qualityOptionsFromSource('local')).toBeUndefined()
    expect(qualityOptionsFromSource(undefined)).toBeUndefined()
  })
})

describe('resolvedPlaybackAudioInfo', () => {
  it('falls back to the platform name and options when the resolver reports only raw values', () => {
    expect(resolvedPlaybackAudioInfo(resolved({ bitrate: 2_964_000, codec: 'FLAC', format: 'flac' }))).toEqual({
      bitrate: 2964,
      codec: 'FLAC',
      format: 'flac',
      source: 'netease',
      qualityKey: 'hires',
      qualityLabel: 'Hi-Res',
      qualityOptions: qualityOptionsFromSource('netease'),
      mimeType: undefined,
      sampleRateHz: undefined,
      bitDepth: undefined,
      channelCount: undefined,
      specLabel: undefined,
    })
  })

  it('prefers the decoded audio info and keeps only paper specifications', () => {
    const info = resolvedPlaybackAudioInfo(resolved({
      audioInfo: {
        source: 'netease',
        qualityKey: 'lossless',
        qualityLabel: '无损音质',
        qualityOptions: [{ key: 'lossless', label: '无损' }],
        codecLabel: 'ALAC',
        mimeType: 'audio/mp4',
        bitrateKbps: 1411,
        sampleRateHz: 44_100,
        bitDepth: 16,
        channelCount: 2,
        specLabel: '44.1 kHz | 1411 kbps | 16 bit',
      },
    }))
    expect(info).toMatchObject({
      bitrate: 1411,
      codec: 'ALAC',
      format: 'audio/mp4',
      qualityKey: 'lossless',
      qualityLabel: '无损音质',
      qualityOptions: [{ key: 'lossless', label: '无损' }],
      specLabel: '44.1 kHz | 16 bit',
    })
  })

  it('replaces a bitrate-only or raw-key label with the platform name', () => {
    const kbpsLabel = resolvedPlaybackAudioInfo(resolved({
      audioInfo: { source: 'netease', qualityKey: 'exhigh', qualityLabel: '320 kbps', specLabel: '320 kbps' },
    }))
    expect(kbpsLabel.qualityLabel).toBe('极高')
    expect(kbpsLabel.specLabel).toBeUndefined()
    const rawKeyLabel = resolvedPlaybackAudioInfo(resolved({
      audioInfo: { source: 'netease', qualityKey: 'exhigh', qualityLabel: 'exhigh' },
    }))
    expect(rawKeyLabel.qualityLabel).toBe('极高')
  })
})
