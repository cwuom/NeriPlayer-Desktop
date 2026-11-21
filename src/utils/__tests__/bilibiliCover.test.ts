// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FakeImage, installFakeImage } from './fakeImage'

const invokeMock = vi.hoisted(() =>
  vi.fn<(command: string, args?: Record<string, unknown>) => Promise<string>>(),
)
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const COVER = 'https://i0.hdslb.com/bfs/archive/cover.jpg'
const DATA_URL = `data:image/jpeg;base64,${Buffer.from('jpeg-bytes').toString('base64')}`

async function loadModule(): Promise<typeof import('@/utils/bilibiliCover')> {
  // 模块内的 coverCache 是单例，每个用例重新加载以隔离缓存
  vi.resetModules()
  return import('@/utils/bilibiliCover')
}

beforeEach(() => {
  installFakeImage()
  invokeMock.mockReset().mockResolvedValue(DATA_URL)
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('resolveCoverImage', () => {
  it('fetches through the backend with the normalized URL and caches the decoded data URL', async () => {
    const { resolveCoverImage, peekCoverImage } = await loadModule()

    expect(peekCoverImage(COVER)).toBe('')
    await expect(resolveCoverImage('//i0.hdslb.com/bfs/archive/cover.jpg#frag')).resolves.toBe(DATA_URL)

    expect(invokeMock).toHaveBeenCalledWith('fetch_bilibili_cover', { url: COVER, forceRefresh: false })
    expect(peekCoverImage(COVER)).toBe(DATA_URL)
  })

  it('decodes the data URL asynchronously before resolving', async () => {
    const { resolveCoverImage } = await loadModule()

    await resolveCoverImage(COVER)

    expect(FakeImage.instances).toHaveLength(1)
    expect(FakeImage.instances[0].src).toBe(DATA_URL)
    expect(FakeImage.instances[0].decoding).toBe('async')
  })

  it('serves repeated requests from the cache without another backend call', async () => {
    const { resolveCoverImage } = await loadModule()

    await resolveCoverImage(COVER)
    await resolveCoverImage(COVER)

    expect(invokeMock).toHaveBeenCalledTimes(1)
  })

  it('passes forceRefresh through to the backend', async () => {
    const { resolveCoverImage } = await loadModule()

    await resolveCoverImage(COVER)
    await resolveCoverImage(COVER, { forceRefresh: true })

    expect(invokeMock).toHaveBeenLastCalledWith('fetch_bilibili_cover', { url: COVER, forceRefresh: true })
    expect(invokeMock).toHaveBeenCalledTimes(2)
  })

  it('rejects invalid URLs without calling the backend', async () => {
    const { resolveCoverImage } = await loadModule()

    await expect(resolveCoverImage('https://evil.example/cover.jpg')).rejects.toThrow('Cover URL is invalid')
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('rejects and does not cache when the image fails to decode', async () => {
    FakeImage.outcome = 'error'
    const { resolveCoverImage, peekCoverImage } = await loadModule()

    await expect(resolveCoverImage(COVER)).rejects.toThrow('Bilibili cover decode failed')
    expect(peekCoverImage(COVER)).toBe('')
  })

  it('rejects images that decode without dimensions', async () => {
    FakeImage.naturalSize = 0
    const { resolveCoverImage, peekCoverImage } = await loadModule()

    await expect(resolveCoverImage(COVER)).rejects.toThrow('decoded without image dimensions')
    expect(peekCoverImage(COVER)).toBe('')
  })

  it('times out a decode that never settles after 10 seconds', async () => {
    vi.useFakeTimers()
    FakeImage.outcome = 'hang'
    const { resolveCoverImage, peekCoverImage } = await loadModule()

    const result = resolveCoverImage(COVER)
    const assertion = expect(result).rejects.toThrow('Bilibili cover decode timed out')
    await vi.advanceTimersByTimeAsync(9_999)
    expect(FakeImage.instances[0].onload).not.toBeNull()
    await vi.advanceTimersByTimeAsync(1)
    await assertion

    expect(peekCoverImage(COVER)).toBe('')
  })

  it('detaches image handlers and clears the timeout once settled', async () => {
    vi.useFakeTimers()
    const { resolveCoverImage } = await loadModule()

    await resolveCoverImage(COVER)

    expect(FakeImage.instances[0].onload).toBeNull()
    expect(FakeImage.instances[0].onerror).toBeNull()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('rejects backend responses that are not image data URLs', async () => {
    invokeMock.mockResolvedValue('data:text/html;base64,PGgxPg==')
    const { resolveCoverImage } = await loadModule()

    await expect(resolveCoverImage(COVER)).rejects.toThrow('not a valid image data URL')
    expect(FakeImage.instances).toHaveLength(0)
  })

  it('exposes resolveBilibiliCover as an alias', async () => {
    const { resolveBilibiliCover, resolveCoverImage } = await loadModule()
    expect(resolveBilibiliCover).toBe(resolveCoverImage)
  })
})
