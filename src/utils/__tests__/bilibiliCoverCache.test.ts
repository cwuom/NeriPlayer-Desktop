import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  BilibiliCoverCache,
  normalizeBilibiliCoverUrl,
  normalizeCoverUrlForDisplay,
  normalizeProxiedCoverUrl,
} from '@/utils/bilibiliCoverCache'

const coverUrl = (name: string): string => `https://i0.hdslb.com/bfs/archive/${name}.jpg`
const dataUrl = (value: string): string => `data:image/png;base64,${Buffer.from(value).toString('base64')}`

const noopDecode = async (): Promise<void> => {}

describe('cover URL normalization', () => {
  it('canonicalizes only allowed HTTPS cover URLs', () => {
    expect(normalizeBilibiliCoverUrl('//I0.HDSLB.COM/bfs/archive/a.jpg#ignored')).toBe(coverUrl('a'))
    expect(normalizeBilibiliCoverUrl('http://archive.biliimg.com/a.webp?size=1')).toBe(
      'https://archive.biliimg.com/a.webp?size=1',
    )
    expect(normalizeBilibiliCoverUrl('https://example.com/a.jpg')).toBe('')
    expect(normalizeBilibiliCoverUrl('https://user@i0.hdslb.com/a.jpg')).toBe('')
    expect(normalizeBilibiliCoverUrl('https://i0.hdslb.com:8443/a.jpg')).toBe('')
    expect(normalizeCoverUrlForDisplay('//y.qq.com/music/photo_new/T002R300x300M000.jpg')).toBe(
      'https://y.qq.com/music/photo_new/T002R300x300M000.jpg',
    )
    expect(normalizeCoverUrlForDisplay('https://p4.music.126.net/cover.jpg')).toBe(
      'https://p4.music.126.net/cover.jpg',
    )
    expect(normalizeProxiedCoverUrl('https://y.qq.com/music/photo_new/cover.jpg')).toBe(
      'https://y.qq.com/music/photo_new/cover.jpg',
    )
    expect(normalizeProxiedCoverUrl('https://p4.music.126.net/cover.jpg')).toBe(
      'https://p4.music.126.net/cover.jpg',
    )
    expect(normalizeProxiedCoverUrl('https://i.ytimg.com/vi/example/maxresdefault.jpg')).toBe(
      'https://i.ytimg.com/vi/example/maxresdefault.jpg',
    )
    expect(normalizeProxiedCoverUrl('https://example.com/cover.jpg')).toBe('')
  })
})

describe('BilibiliCoverCache', () => {
  it('deduplicates canonical and concurrent requests', async () => {
    let fetchCount = 0
    const releaseFetch: Array<(value: string) => void> = []
    const cache = new BilibiliCoverCache(
      async () => {
        fetchCount++
        return new Promise<string>(resolve => { releaseFetch.push(resolve) })
      },
      noopDecode,
    )

    const first = cache.resolve('//i0.hdslb.com/bfs/archive/shared.jpg#one')
    const second = cache.resolve(coverUrl('shared'))
    expect(fetchCount).toBe(1)
    releaseFetch[0](dataUrl('shared'))
    expect(await first).toBe(dataUrl('shared'))
    expect(await second).toBe(dataUrl('shared'))
    expect(fetchCount).toBe(1)
  })

  it('exposes resolved covers synchronously to newly mounted consumers', async () => {
    const value = dataUrl('shared-display-cover')
    const cache = new BilibiliCoverCache(
      async () => value,
      noopDecode,
    )

    expect(cache.peek(coverUrl('shared-display'))).toBe('')
    await cache.resolve(coverUrl('shared-display'))
    expect(cache.peek(`${coverUrl('shared-display')}#ignored`)).toBe(value)
  })

  it('does not cache fetch, validation, or decode failures', async () => {
    let fetchCount = 0
    let decodeCount = 0
    const responses: Array<Error | string> = [
      new Error('network failed'),
      'data:text/html;base64,PGh0bWw+',
      dataUrl('decode-fails'),
      dataUrl('success'),
    ]
    const cache = new BilibiliCoverCache(
      async () => {
        const response = responses[fetchCount++]
        if (response instanceof Error) throw response
        return response
      },
      async value => {
        decodeCount++
        if (value === dataUrl('decode-fails')) throw new Error('decode failed')
      },
    )

    await expect(cache.resolve(coverUrl('failures'))).rejects.toThrow(/network failed/)
    await expect(cache.resolve(coverUrl('failures'))).rejects.toThrow(/valid image data URL/)
    await expect(cache.resolve(coverUrl('failures'))).rejects.toThrow(/decode failed/)
    expect(await cache.resolve(coverUrl('failures'))).toBe(dataUrl('success'))
    expect(await cache.resolve(coverUrl('failures'))).toBe(dataUrl('success'))
    expect(fetchCount).toBe(4)
    expect(decodeCount).toBe(2)
  })

  it('uses LRU eviction and keeps query variants isolated', async () => {
    const fetchCounts = new Map<string, number>()
    const cache = new BilibiliCoverCache(
      async url => {
        fetchCounts.set(url, (fetchCounts.get(url) ?? 0) + 1)
        return dataUrl(url)
      },
      noopDecode,
      { maxEntries: 2 },
    )

    await cache.resolve(coverUrl('a'))
    await cache.resolve(coverUrl('b'))
    await cache.resolve(coverUrl('a'))
    await cache.resolve(coverUrl('c'))
    await cache.resolve(coverUrl('b'))
    await cache.resolve(`${coverUrl('a')}?size=2`)

    expect(fetchCounts.get(coverUrl('a'))).toBe(1)
    expect(fetchCounts.get(coverUrl('b'))).toBe(2)
    expect(fetchCounts.get(`${coverUrl('a')}?size=2`)).toBe(1)
  })

  it('expires entries and skips entries above the data URL budget', async () => {
    let now = 1_000
    let fetchCount = 0
    const value = dataUrl('budget')
    const cache = new BilibiliCoverCache(
      async () => {
        fetchCount++
        return value
      },
      noopDecode,
      { ttlMs: 100, maxDataUrlChars: value.length - 1, now: () => now },
    )

    await cache.resolve(coverUrl('budget'))
    await cache.resolve(coverUrl('budget'))
    expect(fetchCount).toBe(2)

    const expiringCache = new BilibiliCoverCache(
      async () => {
        fetchCount++
        return dataUrl(`ttl-${fetchCount}`)
      },
      noopDecode,
      { ttlMs: 100, now: () => now },
    )
    const first = await expiringCache.resolve(coverUrl('ttl'))
    now += 99
    expect(await expiringCache.resolve(coverUrl('ttl'))).toBe(first)
    now += 1
    expect(await expiringCache.resolve(coverUrl('ttl'))).not.toBe(first)
  })

  it('prevents superseded requests from overwriting a refresh', async () => {
    const resolvers: Array<(value: string) => void> = []
    let fetchCount = 0
    const cache = new BilibiliCoverCache(
      async () => {
        fetchCount++
        return new Promise<string>(resolve => resolvers.push(resolve))
      },
      noopDecode,
    )

    const staleRequest = cache.resolve(coverUrl('race'))
    const freshRequest = cache.resolve(coverUrl('race'), { forceRefresh: true })
    resolvers[1](dataUrl('fresh'))
    expect(await freshRequest).toBe(dataUrl('fresh'))
    resolvers[0](dataUrl('stale'))
    expect(await staleRequest).toBe(dataUrl('stale'))
    expect(await cache.resolve(coverUrl('race'))).toBe(dataUrl('fresh'))
    expect(fetchCount).toBe(2)
  })

  it('forwards force refresh to the persistent backend cache', async () => {
    const refreshFlags: boolean[] = []
    const cache = new BilibiliCoverCache(
      async (_url, forceRefresh) => {
        refreshFlags.push(forceRefresh)
        return dataUrl(`refresh-${refreshFlags.length}`)
      },
      noopDecode,
    )

    await cache.resolve(coverUrl('refresh'))
    await cache.resolve(coverUrl('refresh'), { forceRefresh: true })

    expect(refreshFlags).toEqual([false, true])
  })
})

describe('BilibiliCoverImage component contract', () => {
  it('defers backend proxy work until a rendered image fails', () => {
    const componentSource = readFileSync(
      new URL('../../components/BilibiliCoverImage.vue', import.meta.url),
      'utf8',
    )
    expect(componentSource).toMatch(/resolvedSrc\.value = cachedSrc \|\| normalizedSrc/)
    expect(componentSource).not.toMatch(/void loadCover\(proxiedUrl, false\)/)
    expect(componentSource).toMatch(/const forceRefresh = renderRetryCount > 0/)
    expect(componentSource).toMatch(/void loadCover\(proxiedUrl, forceRefresh\)/)
  })
})
