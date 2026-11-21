import { describe, expect, it, vi } from 'vitest'
import {
  isBilibiliCover,
  libraryCoverKey,
  toDisplayableLibraryCoverUrl,
  useLibraryCovers,
} from '@/components/library/useLibraryCovers'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
}))

describe('isBilibiliCover', () => {
  it('recognises Bilibili image hosts only', () => {
    expect(isBilibiliCover('https://i0.hdslb.com/bfs/archive/a.jpg')).toBe(true)
    expect(isBilibiliCover('https://i1.BILIIMG.com/a.png')).toBe(true)
    expect(isBilibiliCover('https://p1.music.126.net/a.jpg')).toBe(false)
    expect(isBilibiliCover('')).toBe(false)
    expect(isBilibiliCover(null)).toBe(false)
    expect(isBilibiliCover(undefined)).toBe(false)
  })
})

describe('toDisplayableLibraryCoverUrl', () => {
  it('upgrades network covers to https', () => {
    expect(toDisplayableLibraryCoverUrl('http://p1.music.126.net/a.jpg')).toBe('https://p1.music.126.net/a.jpg')
    expect(toDisplayableLibraryCoverUrl('//i0.hdslb.com/a.jpg')).toBe('https://i0.hdslb.com/a.jpg')
    expect(toDisplayableLibraryCoverUrl('https://example.com/a.jpg')).toBe('https://example.com/a.jpg')
  })

  it('keeps inline and asset urls untouched', () => {
    expect(toDisplayableLibraryCoverUrl('data:image/png;base64,AAAA')).toBe('data:image/png;base64,AAAA')
    expect(toDisplayableLibraryCoverUrl('blob:abc')).toBe('blob:abc')
    expect(toDisplayableLibraryCoverUrl('asset://localhost/x.jpg')).toBe('asset://localhost/x.jpg')
  })

  it('converts local file paths through the asset protocol', () => {
    expect(toDisplayableLibraryCoverUrl('/music/cover.jpg')).toBe(`asset://localhost/${encodeURIComponent('/music/cover.jpg')}`)
  })

  it('returns an empty string for missing covers', () => {
    expect(toDisplayableLibraryCoverUrl('')).toBe('')
    expect(toDisplayableLibraryCoverUrl(null)).toBe('')
    expect(toDisplayableLibraryCoverUrl(undefined)).toBe('')
  })
})

describe('useLibraryCovers', () => {
  it('keys failures by scope, id and url', () => {
    expect(libraryCoverKey('local', 1, 'a.jpg')).toBe('local:1:a.jpg')
    expect(libraryCoverKey('youtube', 'id', null)).toBe('youtube:id:')
  })

  it('remembers a failed cover until its url changes', () => {
    const { isLibraryCoverFailed, markLibraryCoverFailed } = useLibraryCovers()
    expect(isLibraryCoverFailed('local', 1, 'a.jpg')).toBe(false)
    markLibraryCoverFailed('local', 1, 'a.jpg')
    expect(isLibraryCoverFailed('local', 1, 'a.jpg')).toBe(true)
    expect(isLibraryCoverFailed('local', 1, 'b.jpg')).toBe(false)
    expect(isLibraryCoverFailed('favorite', 1, 'a.jpg')).toBe(false)
  })

  it('ignores missing urls', () => {
    const { isLibraryCoverFailed, markLibraryCoverFailed } = useLibraryCovers()
    markLibraryCoverFailed('local', 1, '')
    markLibraryCoverFailed('local', 1, null)
    expect(isLibraryCoverFailed('local', 1, '')).toBe(false)
    expect(isLibraryCoverFailed('local', 1, null)).toBe(false)
  })
})
