import { describe, expect, it, vi } from 'vitest'
import {
  CUSTOM_FONT,
  desktopLyricsFontChoice,
  desktopLyricsPreviewLines,
  desktopLyricsThemeSwatch,
} from '@/components/settings/useLyricsSettings'
import { DESKTOP_LYRICS_FONT_PRESETS, DESKTOP_LYRICS_THEMES } from '@/modules/desktopLyrics/style'

// 纯函数不碰 store；真实 settings store 模块在导入时就会读 localStorage
vi.mock('@/stores/settings', () => ({ DEFAULT_LYRIC_SOURCES: [], useSettingsStore: vi.fn() }))

describe('desktopLyricsPreviewLines', () => {
  it('times the first line word by word and starts the next line after a pause', () => {
    const [first, next] = desktopLyricsPreviewLines('你好', 'hello', 'next line')
    expect(first.words).toEqual([
      { startMs: 400, durationMs: 1500, text: '你' },
      { startMs: 1900, durationMs: 1500, text: '好' },
    ])
    expect(first).toMatchObject({ startMs: 400, durationMs: 3000, text: '你好', translation: 'hello', roman: 'desktop lyrics preview' })
    expect(next).toEqual({ startMs: 3700, durationMs: 3000, text: 'next line', translation: '', roman: '', words: [] })
  })

  it('keeps every word at least 120ms long for long lines', () => {
    const [first] = desktopLyricsPreviewLines('x'.repeat(40), '', '')
    expect(first.words.every(word => word.durationMs === 120)).toBe(true)
    expect(first.durationMs).toBe(40 * 120)
  })
})

describe('desktopLyricsThemeSwatch', () => {
  it('derives the accent swatch from the current primary color with a fallback', () => {
    expect(desktopLyricsThemeSwatch('accent', '#123456')).toBe('linear-gradient(135deg, #ffffff, #123456)')
    expect(desktopLyricsThemeSwatch('accent', null)).toBe('linear-gradient(135deg, #ffffff, #d0bcff)')
  })

  it('uses the played gradient of preset themes', () => {
    const theme = DESKTOP_LYRICS_THEMES[0]
    expect(desktopLyricsThemeSwatch(theme.id, null)).toBe(`linear-gradient(135deg, ${theme.played[0]}, ${theme.played[1]})`)
    expect(desktopLyricsThemeSwatch('missing', null)).toBe('transparent')
  })
})

describe('desktopLyricsFontChoice', () => {
  it('selects presets and the default font directly', () => {
    expect(desktopLyricsFontChoice('', false)).toBe('')
    expect(desktopLyricsFontChoice(DESKTOP_LYRICS_FONT_PRESETS[0], false)).toBe(DESKTOP_LYRICS_FONT_PRESETS[0])
  })

  it('switches to the custom entry for unknown fonts or after the user picked custom', () => {
    expect(desktopLyricsFontChoice('My Font', false)).toBe(CUSTOM_FONT)
    expect(desktopLyricsFontChoice(DESKTOP_LYRICS_FONT_PRESETS[0], true)).toBe(CUSTOM_FONT)
  })
})
