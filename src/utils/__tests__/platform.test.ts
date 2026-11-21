import { afterEach, describe, expect, it, vi } from 'vitest'
import { detectPlatform } from '@/utils/platform'

const WINDOWS = { platform: 'Win32', userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' }

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('detectPlatform', () => {
  it('prefers the user agent client hints platform', () => {
    vi.stubGlobal('navigator', { ...WINDOWS, userAgentData: { platform: 'macOS' } })
    expect(detectPlatform()).toBe('mac')
  })

  it('recognizes Windows and Linux from navigator.platform', () => {
    vi.stubGlobal('navigator', WINDOWS)
    expect(detectPlatform()).toBe('windows')

    vi.stubGlobal('navigator', { platform: 'Linux x86_64', userAgent: '' })
    expect(detectPlatform()).toBe('linux')
  })

  it('treats Apple mobile devices and Darwin as mac', () => {
    vi.stubGlobal('navigator', { platform: 'iPhone', userAgent: '' })
    expect(detectPlatform()).toBe('mac')

    vi.stubGlobal('navigator', { platform: '', userAgent: 'something Darwin arm64' })
    expect(detectPlatform()).toBe('mac')
  })

  it('falls back to the user agent string when the platform is empty', () => {
    vi.stubGlobal('navigator', { platform: '', userAgent: WINDOWS.userAgent, userAgentData: { platform: '' } })
    expect(detectPlatform()).toBe('windows')
  })

  it('defaults to linux when nothing identifies the platform', () => {
    vi.stubGlobal('navigator', { platform: '', userAgent: '' })
    expect(detectPlatform()).toBe('linux')
  })
})
