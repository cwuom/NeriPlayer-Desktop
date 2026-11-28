import { describe, expect, it } from 'vitest'
import { shouldKeepCachedDetail, type CachedDetailPreview } from '@/modules/library/playlistDetailCache'

function preview(shown: boolean): CachedDetailPreview {
  return { markFresh() {}, shown: async () => shown }
}

describe('shouldKeepCachedDetail', () => {
  it('完整结果总是替换缓存，不等待缓存读取', async () => {
    const pending: CachedDetailPreview = { markFresh() {}, shown: () => new Promise(() => {}) }
    await expect(shouldKeepCachedDetail(pending, false, () => 300, 100)).resolves.toBe(false)
  })

  it('分页中断时保留曲目更多的已展示缓存（#73）', async () => {
    await expect(shouldKeepCachedDetail(preview(true), true, () => 300, 100)).resolves.toBe(true)
  })

  it('分页中断但结果不比缓存少时使用新结果', async () => {
    await expect(shouldKeepCachedDetail(preview(true), true, () => 100, 100)).resolves.toBe(false)
    await expect(shouldKeepCachedDetail(preview(true), true, () => 80, 100)).resolves.toBe(false)
  })

  it('没有展示缓存时使用不完整的新结果', async () => {
    await expect(shouldKeepCachedDetail(preview(false), true, () => 0, 100)).resolves.toBe(false)
  })
})
