// 大列表分块渲染：首屏只渲染一批行，滚动接近底部再扩容。
// 歌单/队列可达上千行，一次性渲染会拖垮首帧布局与滚动
// （WebKitGTK 上尤甚）；本地歌单页已有同款模式，这里抽成共享实现
import { computed, ref, watch } from 'vue'

const RENDER_CHUNK = 100
// 距底部剩余高度小于该值时预扩一批；阈值太小时快速甩动会先看到底部空白
const SCROLL_PRELOAD_PX = 2400
// 定位当前曲目时目标行多带一段缓冲，居中后下方仍有内容
const LOCATE_BUFFER = 20

/**
 * @param getSource 完整列表的读取函数（搜索过滤后的列表或原始数组）。
 *                  列表内容变化（搜索、加载完成、清空）时窗口自动重置
 */
export function useIncrementalList<T>(getSource: () => readonly T[]) {
  const source = computed(() => getSource())
  const renderCount = ref(RENDER_CHUNK)
  const visibleItems = computed(() => source.value.slice(0, renderCount.value))
  const hasMore = computed(() => renderCount.value < source.value.length)

  // 内容整体变化（搜索过滤/替换）时重置窗口
  watch(source, (list) => {
    renderCount.value = Math.min(RENDER_CHUNK, list.length)
  })
  // 行数收缩（删除/清空）时把窗口拉回来，避免窗口悬空
  watch(
    () => source.value.length,
    (len) => {
      if (len < renderCount.value) {
        renderCount.value = Math.min(RENDER_CHUNK, len)
      }
    },
  )

  function expand(extra = RENDER_CHUNK) {
    if (!hasMore.value) return
    renderCount.value = Math.min(source.value.length, renderCount.value + extra)
  }

  /** 绑定到滚动容器：接近底部时自动扩容 */
  function onScroll(e: Event) {
    const el = e.currentTarget as HTMLElement | null
    if (!el || !hasMore.value) return
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - SCROLL_PRELOAD_PX) {
      expand()
    }
  }

  /** 确保真实索引已渲染（定位当前曲目用），返回该行是否可达 */
  function ensureIndex(index: number): boolean {
    if (index < 0 || index >= source.value.length) return false
    if (index >= renderCount.value) {
      renderCount.value = Math.min(source.value.length, index + LOCATE_BUFFER)
    }
    return true
  }

  return { visibleItems, hasMore, expand, onScroll, ensureIndex }
}
