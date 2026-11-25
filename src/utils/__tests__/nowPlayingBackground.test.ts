import { describe, expect, it } from 'vitest'
import { shouldShowDynamicBackground } from '@/utils/nowPlayingBackground'
import type { PaletteResult } from '@/utils/paletteExtractor'

type RGB = PaletteResult['accentBg']

const gray: RGB = [18, 18, 18]

const palette: PaletteResult = {
  shaderColors: [gray, gray, gray, gray, gray],
  lightOffset: 0,
  saturateOffset: 0,
  accentBg: gray,
  primaryColor: gray,
  themeSeed: gray,
  dominant: gray,
  lightVibrant: gray,
  muted: gray,
  darkMuted: gray,
}

describe('shouldShowDynamicBackground', () => {
  it('hides the background without a playback session', () => {
    expect(shouldShowDynamicBackground(false, true, palette)).toBe(false)
  })

  it('hides the background when the setting is disabled', () => {
    expect(shouldShowDynamicBackground(true, false, palette)).toBe(false)
  })

  it('hides the background until a palette is available', () => {
    expect(shouldShowDynamicBackground(true, true, null)).toBe(false)
  })

  it('shows the background when all conditions hold', () => {
    expect(shouldShowDynamicBackground(true, true, palette)).toBe(true)
  })
})
