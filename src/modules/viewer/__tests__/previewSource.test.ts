import { describe, expect, it } from 'vitest'
import { previewImageSource } from '@/modules/viewer/previewSource'

describe('previewImageSource', () => {
  it('shows the cached cover while the original loads, then swaps to the original', () => {
    expect(previewImageSource({ src: 'https://a/big.jpg', placeholder: 'asset://cover.jpg', loaded: false, failed: false })).toBe('asset://cover.jpg')
    expect(previewImageSource({ src: 'https://a/big.jpg', placeholder: 'asset://cover.jpg', loaded: true, failed: false })).toBe('https://a/big.jpg')
  })

  it('keeps the cached cover when the original request fails (offline playback)', () => {
    // 回归：原图请求失败曾把可用的缓存封面一起隐藏
    expect(previewImageSource({ src: 'https://a/big.jpg', placeholder: 'asset://cover.jpg', loaded: false, failed: true })).toBe('asset://cover.jpg')
  })

  it('shows the unavailable state only when neither image is usable', () => {
    expect(previewImageSource({ src: 'https://a/big.jpg', loaded: false, failed: true })).toBeNull()
    expect(previewImageSource({ src: '', placeholder: '', loaded: false, failed: false })).toBeNull()
    expect(previewImageSource({ src: 'https://a/big.jpg', placeholder: null, loaded: false, failed: false })).toBe('https://a/big.jpg')
  })
})
