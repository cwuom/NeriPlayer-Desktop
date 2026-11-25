// 横向货架的翻页按钮状态：能否向左 / 向右翻。
// 行的盒子尺寸不变、只是卡片变多时 scrollWidth 会变而 ResizeObserver 不触发，
// 所以同时监听行内子节点增删，否则后来才溢出的卡片翻页按钮一直是灰的

export interface ShelfEdges {
  canPrev: boolean
  canNext: boolean
}

export interface ScrollRow {
  scrollLeft: number
  clientWidth: number
  scrollWidth: number
}

/** 留 4px 余量，避免亚像素取整让按钮在边缘反复闪烁 */
const EDGE_SLACK_PX = 4

export function shelfEdges(row: ScrollRow): ShelfEdges {
  return {
    canPrev: row.scrollLeft > EDGE_SLACK_PX,
    canNext: row.scrollLeft + row.clientWidth < row.scrollWidth - EDGE_SLACK_PX,
  }
}

interface Observer { observe(target: unknown, options?: unknown): void; disconnect(): void }

export interface ShelfObservers {
  resize: (callback: () => void) => Observer
  mutation: (callback: () => void) => Observer
}

const browserObservers: ShelfObservers = {
  resize: callback => new ResizeObserver(callback),
  mutation: callback => new MutationObserver(callback),
}

/** 行尺寸变化或卡片增删时回调 update；返回卸载函数 */
export function watchShelfRow(row: object, update: () => void, observers: ShelfObservers = browserObservers): () => void {
  const resize = observers.resize(update)
  const mutation = observers.mutation(update)
  resize.observe(row)
  mutation.observe(row, { childList: true, subtree: true })
  return () => {
    resize.disconnect()
    mutation.disconnect()
  }
}
