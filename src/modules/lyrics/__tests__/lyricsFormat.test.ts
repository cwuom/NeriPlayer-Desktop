import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { LyricLine } from '@/types/track'
import {
  hasWordTimedEntries,
  lyricLineToLrc,
  lyricLineToYrc,
  resolveStoredLyricStateFromPayload,
  resolveStoredLyricText,
  toEditableLyricsText,
  withUpdatedLyricsPayload,
} from '@/modules/lyrics/lyricsFormat'

const wordLine: LyricLine = {
  startMs: 10000,
  durationMs: 2000,
  text: '你好',
  words: [
    { startMs: 10000, durationMs: 500, text: '你' },
    { startMs: 10500, durationMs: 500, text: '好' },
  ],
}

const plainLine: LyricLine = {
  startMs: 12000,
  durationMs: 3000,
  text: '世界',
  words: [],
}

const EXPECTED_YRC = '[10000,2000](10000,500,0)你(10500,500,0)好'

function buildMatchedPayload(): Record<string, unknown> {
  return withUpdatedLyricsPayload(
    { matchedLyric: 'old', name: 'song' },
    lyricLineToYrc(wordLine),
    null,
    'NETEASE',
  )
}

describe('lyric line export', () => {
  it('detects word-timed entries', () => {
    expect(hasWordTimedEntries([wordLine])).toBe(true)
    expect(hasWordTimedEntries([plainLine])).toBe(false)
  })

  it('exports word-timed lines as YRC and plain lines as LRC', () => {
    expect(lyricLineToYrc(wordLine)).toBe(EXPECTED_YRC)
    expect(lyricLineToLrc(plainLine, plainLine.text)).toMatch(/^\[00:12\.00\]世界$/)
  })
})

describe('toEditableLyricsText', () => {
  it('writes mixed lyrics entirely as YRC so parse_auto keeps plain lines', () => {
    const editable = toEditableLyricsText([wordLine, plainLine])
    expect(editable).toMatch(/\[10000,2000\]/)
    expect(editable).toMatch(/\[12000,3000\]世界/)
    expect(editable).not.toMatch(/\[00:12\.00\]/)
  })

  it('writes lyrics without word timing as LRC', () => {
    expect(toEditableLyricsText([plainLine])).toMatch(/^\[00:12\.00\]世界$/)
  })
})

describe('withUpdatedLyricsPayload', () => {
  it('stores the new lyric, keeps the original and records the source', () => {
    const payload = buildMatchedPayload()
    expect(payload.matchedLyric).toBe(EXPECTED_YRC)
    expect(payload.originalLyric).toBe('old')
    expect(payload.matchedLyricSource).toBe('NETEASE')
    expect(resolveStoredLyricText(payload)).toBe(EXPECTED_YRC)
  })

  it('marks the payload with the CURRENT sync metadata version', () => {
    expect(buildMatchedPayload().syncMetadataVersion).toBe(1)
  })

  it('writes an empty string when lyrics are intentionally cleared and keeps the original', () => {
    const cleared = withUpdatedLyricsPayload(buildMatchedPayload(), null, null, null)
    expect(cleared.matchedLyric).toBe('')
    expect(cleared.originalLyric).toBe('old')
    expect(resolveStoredLyricStateFromPayload(cleared).kind).toBe('cleared')
    expect(resolveStoredLyricText(cleared)).toBe('')
  })
})

describe('resolveStoredLyricStateFromPayload', () => {
  it('reports absent when neither matched nor original lyrics exist', () => {
    expect(resolveStoredLyricStateFromPayload({ name: 'x' }).kind).toBe('absent')
    expect(resolveStoredLyricText({ name: 'x' })).toBe(null)
  })

  it('falls back to the original lyric when matched is missing', () => {
    expect(resolveStoredLyricStateFromPayload({ originalLyric: '[00:01.00]a' }).kind).toBe(
      'present',
    )
  })
})

describe('NowPlaying lyric editing source contract', () => {
  const nowPlaying = readFileSync(
    new URL('../../../components/NowPlaying.vue', import.meta.url),
    'utf8',
  )

  it('uses the shared lyric format helpers', () => {
    expect(nowPlaying).toMatch(/parse_lrc_content/)
    expect(nowPlaying).toMatch(/toEditableLyricsText/)
    expect(nowPlaying).toMatch(/resolveStoredLyricStateFromPayload|resolveStoredLyricText/)
    expect(nowPlaying).toMatch(/withUpdatedLyricsPayload/)
    expect(nowPlaying).toMatch(/commitLyricsToTrack|persistTrackSyncPayload/)
  })
})
