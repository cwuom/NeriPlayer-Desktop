// 图片查看器的缩放/平移数学：以光标为锚点缩放，平移量限制在图片不离开视口的范围内
export const VIEWER_MIN_SCALE = 1
export const VIEWER_MAX_SCALE = 6

export interface ViewerTransform {
  scale: number
  x: number
  y: number
}

export const IDENTITY: ViewerTransform = { scale: 1, x: 0, y: 0 }

export function clampScale(scale: number): number {
  if (!Number.isFinite(scale)) return VIEWER_MIN_SCALE
  return Math.min(VIEWER_MAX_SCALE, Math.max(VIEWER_MIN_SCALE, scale))
}

/** 平移上限：放大后图片超出容器的一半，保证图片边缘不会被拖进视口内部 */
export function clampPan(transform: ViewerTransform, size: { width: number; height: number }): ViewerTransform {
  const limitX = Math.max(0, (size.width * transform.scale - size.width) / 2)
  const limitY = Math.max(0, (size.height * transform.scale - size.height) / 2)
  return {
    scale: transform.scale,
    x: Math.min(limitX, Math.max(-limitX, transform.x)),
    y: Math.min(limitY, Math.max(-limitY, transform.y)),
  }
}

/**
 * 以 anchor（相对图片中心的坐标）为不动点缩放到 nextScale：
 * 缩放前后 anchor 下方的图片像素保持在同一位置，符合桌面看图软件的习惯
 */
export function zoomAround(
  current: ViewerTransform,
  nextScale: number,
  anchor: { x: number; y: number },
  size: { width: number; height: number },
): ViewerTransform {
  const scale = clampScale(nextScale)
  if (scale === VIEWER_MIN_SCALE) return { ...IDENTITY }
  const ratio = scale / current.scale
  return clampPan({
    scale,
    x: anchor.x - (anchor.x - current.x) * ratio,
    y: anchor.y - (anchor.y - current.y) * ratio,
  }, size)
}

/** 滚轮增量换算成缩放倍率，触控板的细粒度增量也能平滑缩放 */
export function wheelZoomFactor(deltaY: number): number {
  return Math.exp(-Math.max(-200, Math.min(200, deltaY)) * 0.0025)
}
