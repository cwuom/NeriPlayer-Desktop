// 给正在滚动的容器打上 data-scrolled：背景图模式下吸顶栏停在顶部时透明，
// 正文滚到它下面才淡入玻璃（样式见 global.scss）。
// 用 data 属性而不是 class：Vue 在补丁动态 :class 时会整体重写 className，手动加的类会被抹掉

export const SCROLLED_ATTRIBUTE = 'data-scrolled'
/** 超过这个距离才算滚动过，避免惯性回弹的 1px 抖动让玻璃闪烁 */
export const SCROLLED_THRESHOLD_PX = 2

export interface ScrollContainer {
  scrollTop: number
  toggleAttribute(name: string, force: boolean): boolean
}

interface ScrollRoot extends ScrollContainer {
  addEventListener(type: 'scroll', listener: (event: { target: unknown }) => void, options: AddEventListenerOptions): void
  removeEventListener(type: 'scroll', listener: (event: { target: unknown }) => void, options: AddEventListenerOptions): void
}

function isScrollContainer(value: unknown): value is ScrollContainer {
  return typeof value === 'object' && value !== null
    && typeof (value as ScrollContainer).scrollTop === 'number'
    && typeof (value as ScrollContainer).toggleAttribute === 'function'
}

export function markScrolled(element: ScrollContainer): void {
  element.toggleAttribute(SCROLLED_ATTRIBUTE, element.scrollTop > SCROLLED_THRESHOLD_PX)
}

/**
 * scroll 事件不冒泡，在捕获阶段监听 root 才能同时收到 root 自己和内部滚动容器（详情页）的滚动。
 * 返回卸载函数
 */
export function installScrolledState(root: ScrollRoot): () => void {
  const options: AddEventListenerOptions = { capture: true, passive: true }
  const onScroll = (event: { target: unknown }) => {
    if (isScrollContainer(event.target)) markScrolled(event.target)
  }
  root.addEventListener('scroll', onScroll, options)
  markScrolled(root)
  return () => root.removeEventListener('scroll', onScroll, options)
}
