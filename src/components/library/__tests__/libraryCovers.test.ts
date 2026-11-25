import { describe, expect, it, vi } from 'vitest'
import {
  hasProxiedCoverFallback,
  libraryCoverKey,
  toDisplayableLibraryCoverUrl,
  useLibraryCovers,
} from '@/components/library/useLibraryCovers'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
}))

describe('hasProxiedCoverFallback', () => {
  it('routes YouTube covers through the proxy-capable image so the list matches the detail page', () => {
    // 回归：本地歌单首曲来自 YouTube 时，列表直接 <img> 加载失败只剩占位图，详情页却能显示
    expect(hasProxiedCoverFallback('https://yt3.googleusercontent.com/abc=w1200-h1200')).toBe(true)
    expect(hasProxiedCoverFallback('https://i.ytimg.com/vi/x/hqdefault.jpg')).toBe(true)
  })

  it('covers the other proxied image hosts and rejects local or unknown sources', () => {
    expect(hasProxiedCoverFallback('https://i0.hdslb.com/bfs/archive/a.jpg')).toBe(true)
    expect(hasProxiedCoverFallback('https://i1.BILIIMG.com/a.png')).toBe(true)
    expect(hasProxiedCoverFallback('http://p1.music.126.net/a.jpg')).toBe(true)
    expect(hasProxiedCoverFallback('https://example.com/a.jpg')).toBe(false)
    expect(hasProxiedCoverFallback('/Users/me/Music/cover.jpg')).toBe(false)
    expect(hasProxiedCoverFallback('')).toBe(false)
    expect(hasProxiedCoverFallback(null)).toBe(false)
    expect(hasProxiedCoverFallback(undefined)).toBe(false)
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
