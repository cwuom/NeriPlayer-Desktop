import { describe, expect, it, vi } from 'vitest'
import { shelfEdges, watchShelfRow, type ShelfObservers } from '@/modules/layout/shelfEdges'

describe('shelfEdges', () => {
  it('enables paging only past the slack at each edge', () => {
    expect(shelfEdges({ scrollLeft: 0, clientWidth: 800, scrollWidth: 800 })).toEqual({ canPrev: false, canNext: false })
    expect(shelfEdges({ scrollLeft: 0, clientWidth: 800, scrollWidth: 1600 })).toEqual({ canPrev: false, canNext: true })
    expect(shelfEdges({ scrollLeft: 800, clientWidth: 800, scrollWidth: 1600 })).toEqual({ canPrev: true, canNext: false })
    expect(shelfEdges({ scrollLeft: 3, clientWidth: 800, scrollWidth: 803 })).toEqual({ canPrev: false, canNext: false })
  })
})

describe('watchShelfRow', () => {
  function fakeObservers() {
    const callbacks: Record<'resize' | 'mutation', () => void> = { resize: () => {}, mutation: () => {} }
    const observed: Array<[string, unknown]> = []
    const disconnected: string[] = []
    const make = (kind: 'resize' | 'mutation') => (callback: () => void) => {
      callbacks[kind] = callback
      return { observe: (_target: unknown, options?: unknown) => { observed.push([kind, options]) }, disconnect: () => { disconnected.push(kind) } }
    }
    const observers: ShelfObservers = { resize: make('resize'), mutation: make('mutation') }
    return { observers, callbacks, observed, disconnected }
  }

  it('recomputes edges when cards are added without the row resizing', () => {
    // 回归：卡片变多只改 scrollWidth，行尺寸不变，翻页按钮曾一直保持禁用
    const { observers, callbacks, observed } = fakeObservers()
    const update = vi.fn()
    watchShelfRow({}, update, observers)

    expect(observed).toContainEqual(['mutation', { childList: true, subtree: true }])
    callbacks.mutation()
    expect(update).toHaveBeenCalledTimes(1)
    callbacks.resize()
    expect(update).toHaveBeenCalledTimes(2)
  })

  it('disconnects both observers on unmount', () => {
    const { observers, disconnected } = fakeObservers()
    watchShelfRow({}, () => {}, observers)()
    expect(disconnected.sort()).toEqual(['mutation', 'resize'])
  })
})
