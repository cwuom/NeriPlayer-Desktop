import { describe, expect, it } from 'vitest'
import {
  coverSourceLabel,
  formatDurationMs,
  formatFileSize,
  formatSleepRemaining,
  isRemotePlaybackSource,
  platformLabel,
  playbackSourceIconFor,
  playbackSourceLabelFor,
  playbackSourceOfTrackId,
  primaryArtistOf,
} from '../nowPlayingHelpers'

const t = (key: string) => `t:${key}`

describe('playback source', () => {
  it('maps track id prefixes and treats everything else as local', () => {
    expect(playbackSourceOfTrackId('netease:1')).toBe('netease')
    expect(playbackSourceOfTrackId('qq:mid')).toBe('qq')
    expect(playbackSourceOfTrackId('bilibili:BV1:2')).toBe('bilibili')
    expect(playbackSourceOfTrackId('youtube:abc')).toBe('youtube')
    expect(playbackSourceOfTrackId('local:/a.flac')).toBe('local')
    expect(playbackSourceOfTrackId('')).toBe('local')
  })

  it('labels the source badge, including file URLs without a local id', () => {
    expect(playbackSourceLabelFor({ id: 'netease:1', audioUrl: '' }, t)).toBe('t:player.source_netease')
    expect(playbackSourceLabelFor({ id: 'youtube:abc', audioUrl: '' }, t)).toBe('t:player.source_youtube')
    expect(playbackSourceLabelFor({ id: 'local:1', audioUrl: '' }, t)).toBe('t:player.source_local')
    expect(playbackSourceLabelFor({ id: 'custom', audioUrl: 'file:///a.mp3' }, t)).toBe('t:player.source_local')
    expect(playbackSourceLabelFor({ id: 'custom', audioUrl: 'https://a' }, t)).toBe('')
    expect(playbackSourceLabelFor(null, t)).toBe('')
  })

  it('picks the badge icon by source and falls back to a folder', () => {
    expect(playbackSourceIconFor('netease:1')).toBe('netease')
    expect(playbackSourceIconFor('qq:1')).toBe('music_note')
    expect(playbackSourceIconFor('bilibili:1')).toBe('smart_display')
    expect(playbackSourceIconFor('youtube:1')).toBe('play_circle')
    expect(playbackSourceIconFor('local:1')).toBe('folder')
  })

  it('names search platforms case-insensitively and keeps unknown names as-is', () => {
    expect(platformLabel('NetEase', t)).toBe('t:player.source_netease')
    expect(platformLabel('lrclib', t)).toBe('LRCLIB')
    expect(platformLabel('kugou', t)).toBe('kugou')
    expect(platformLabel(null, t)).toBe('')
  })

  it('treats only streaming platforms as remote', () => {
    expect(['netease', 'qq', 'bilibili', 'youtube'].every(isRemotePlaybackSource)).toBe(true)
    expect(isRemotePlaybackSource('local')).toBe(false)
  })
})

describe('display formatting', () => {
  it('takes the first of several artists', () => {
    expect(primaryArtistOf('A / B')).toBe('A')
    expect(primaryArtistOf('A、B·C')).toBe('A')
    expect(primaryArtistOf('')).toBe('')
  })

  it('logs only the cover host or size', () => {
    expect(coverSourceLabel('')).toBe('empty')
    expect(coverSourceLabel('data:image/png;base64,AAAA')).toBe('data-url(26)')
    expect(coverSourceLabel('https://p1.music.126.net/a.jpg?param=1')).toBe('p1.music.126.net')
    expect(coverSourceLabel('not a url')).toBe('invalid(9)')
  })

  it('formats the sleep countdown', () => {
    expect(formatSleepRemaining(0)).toBe('')
    expect(formatSleepRemaining(42)).toBe('42s')
    expect(formatSleepRemaining(65)).toBe('1:05')
  })

  it('formats durations and file sizes with a dash for missing values', () => {
    expect(formatDurationMs(undefined)).toBe('-')
    expect(formatDurationMs(0)).toBe('-')
    expect(formatDurationMs(61_999)).toBe('1:01')
    expect(formatFileSize(undefined)).toBe('-')
    expect(formatFileSize(512)).toBe('512 B')
    expect(formatFileSize(1536)).toBe('1.5 KB')
    expect(formatFileSize(5 * 1024 * 1024)).toBe('5.0 MB')
  })
})
