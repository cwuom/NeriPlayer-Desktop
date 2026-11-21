import { describe, expect, it } from 'vitest'
import {
  addAudioInfoPart,
  isHiddenAudioInfoToken,
  isSameAudioInfoToken,
  normalizeAudioDisplayToken,
  paperSpecFromAudioInfo,
  type AudioInfoPart,
} from '../audioInfoDisplay'

describe('paperSpecFromAudioInfo', () => {
  it('formats measured fields and only adds channels for probed files', () => {
    expect(paperSpecFromAudioInfo({ sampleRateHz: 48000, bitDepth: 16, channelCount: 2 }, false))
      .toEqual(['48 kHz', '16 bit', '2 ch'])
    expect(paperSpecFromAudioInfo({ sampleRateHz: 44100, bitDepth: 24, channelCount: 2 }, true))
      .toEqual(['44.1 kHz', '24 bit'])
  })

  it('merges spec label parts without kbps or duplicates', () => {
    expect(paperSpecFromAudioInfo({ sampleRateHz: 48000, specLabel: '48 kHz | 320 kbps | 无损' }))
      .toEqual(['48 kHz', '无损'])
    expect(paperSpecFromAudioInfo({ specLabel: '48 kHz | 无损' }, false)).toEqual([])
  })
})

describe('audio info parts', () => {
  it('skips blank and case-insensitive duplicate tokens', () => {
    const parts: AudioInfoPart[] = []
    addAudioInfoPart(parts, ' 极高 ', true)
    addAudioInfoPart(parts, '')
    addAudioInfoPart(parts, undefined)
    addAudioInfoPart(parts, 'FLAC')
    addAudioInfoPart(parts, 'flac')
    expect(parts).toEqual([{ text: '极高', accent: true }, { text: 'FLAC', accent: false }])
  })

  it('compares tokens ignoring case and surrounding spaces', () => {
    expect(isSameAudioInfoToken(' Opus', 'opus ')).toBe(true)
    expect(isSameAudioInfoToken('AAC', 'MP3')).toBe(false)
  })

  it('hides placeholder tokens', () => {
    for (const token of [undefined, '', 'Local', 'download', 'FILE', 'offline', 'downloaded']) {
      expect(isHiddenAudioInfoToken(token)).toBe(true)
    }
    expect(isHiddenAudioInfoToken('FLAC')).toBe(false)
  })
})

describe('normalizeAudioDisplayToken', () => {
  it('maps codecs, MIME types and codec parameters', () => {
    expect(normalizeAudioDisplayToken('flac')).toBe('FLAC')
    expect(normalizeAudioDisplayToken('mpeg')).toBe('MP3')
    expect(normalizeAudioDisplayToken('audio/mp4')).toBe('MP4')
    expect(normalizeAudioDisplayToken('audio/mp4; codecs="mp4a.40.2"')).toBe('AAC')
    expect(normalizeAudioDisplayToken('audio/webm; codecs="opus"')).toBe('Opus')
    expect(normalizeAudioDisplayToken('audio/x-flac')).toBe('FLAC')
    expect(normalizeAudioDisplayToken('audio/x-unknown')).toBe('UNKNOWN')
  })

  it('keeps generic MPEG for probed local files and drops placeholders', () => {
    expect(normalizeAudioDisplayToken('mpeg', true)).toBe('MPEG')
    expect(normalizeAudioDisplayToken('local')).toBe('')
    expect(normalizeAudioDisplayToken(undefined)).toBe('')
    expect(normalizeAudioDisplayToken('Custom Codec')).toBe('Custom Codec')
  })
})
