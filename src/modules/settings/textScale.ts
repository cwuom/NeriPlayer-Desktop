// 「文字大小」：只缩放文字（驱动 global.scss 的 --text-scale），与整体界面缩放相互独立
export const TEXT_SCALE_DEFAULT = 100
export const TEXT_SCALE_MIN = 85
export const TEXT_SCALE_MAX = 150
export const TEXT_SCALE_STEP = 5

export function normalizeTextScale(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return TEXT_SCALE_DEFAULT
  const clamped = Math.min(TEXT_SCALE_MAX, Math.max(TEXT_SCALE_MIN, value))
  return Math.round(clamped / TEXT_SCALE_STEP) * TEXT_SCALE_STEP
}

export function applyTextScale(percent: number, root: HTMLElement = document.documentElement): void {
  root.style.setProperty('--text-scale', String(normalizeTextScale(percent) / 100))
}
