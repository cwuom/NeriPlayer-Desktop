import { describe, expect, it, vi } from 'vitest'
import { lyricSourceForPlatform, parseLyricsFromSearchResult } from '../searchResultLyrics'

describe('lyricSourceForPlatform', () => {
  it('uses the Android platform names for NetEase and QQ', () => {
    expect(lyricSourceForPlatform('netease')).toBe('CLOUD_MUSIC')
    expect(lyricSourceForPlatform('NetEase Cloud')).toBe('CLOUD_MUSIC')
    expect(lyricSourceForPlatform('qq')).toBe('QQ_MUSIC')
  })

  it('upper-cases other platforms and marks unknown sources as local edits', () => {
    expect(lyricSourceForPlatform('lrclib')).toBe('LRCLIB')
    expect(lyricSourceForPlatform('')).toBe('LOCAL_EDIT')
    expect(lyricSourceForPlatform(null)).toBe('LOCAL_EDIT')
  })
})

describe('parseLyricsFromSearchResult', () => {
  it('parses synced lyrics and merges their translation', async () => {
    const parseLrc = vi.fn(async (content: string) => content === 'original'
      ? [{ start_ms: 1000, duration_ms: 500, text: '你好', words: [] }]
      : [{ start_ms: 1000, duration_ms: 500, text: 'hello', words: [] }])
    const parsed = await parseLyricsFromSearchResult(
      { synced_lyrics: 'original', translated_lyrics: 'translated' },
      parseLrc,
    )
    expect(parseLrc.mock.calls.map(([content]) => content)).toEqual(['original', 'translated'])
    expect(parsed?.rawLyric).toBe('original')
    expect(parsed?.rawTranslated).toBe('translated')
    expect(parsed?.lines).toHaveLength(1)
    expect(parsed?.lines[0]).toMatchObject({ startMs: 1000, text: '你好', translation: 'hello' })
  })

  it('keeps the original lyric when only the translation fails to parse', async () => {
    const parseLrc = vi.fn(async (content: string) => {
      if (content === 'broken') throw new Error('bad translation')
      return [{ start_ms: 0, duration_ms: 1000, text: 'line', words: [] }]
    })
    const parsed = await parseLyricsFromSearchResult({ synced_lyrics: 'ok', translated_lyrics: 'broken' }, parseLrc)
    expect(parsed?.lines.map(line => line.text)).toEqual(['line'])
    expect(parsed?.rawTranslated).toBe('broken')
  })

  it('spreads plain lyrics three seconds apart without calling the parser', async () => {
    const parseLrc = vi.fn(async () => [])
    const parsed = await parseLyricsFromSearchResult({ plain_lyrics: ' first \r\n\nsecond ' }, parseLrc)
    expect(parseLrc).not.toHaveBeenCalled()
    expect(parsed).toEqual({
      lines: [
        { startMs: 0, durationMs: 3000, words: [], text: 'first', translation: undefined },
        { startMs: 3000, durationMs: 3000, words: [], text: 'second', translation: undefined },
      ],
      rawLyric: ' first \r\n\nsecond ',
      rawTranslated: null,
    })
  })

  it('returns null when the result carries no lyrics', async () => {
    expect(await parseLyricsFromSearchResult({ title: 'song' }, vi.fn(async () => []))).toBeNull()
  })
})
