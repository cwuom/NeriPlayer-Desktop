export const UI_SCALE_DEFAULT = 100
export const UI_SCALE_MIN = 25
export const UI_SCALE_MAX = 1000
export const UI_SCALE_STEP = 5

export function normalizeUiScale(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return UI_SCALE_DEFAULT
  return Math.round(Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_MIN, value)) / UI_SCALE_STEP) * UI_SCALE_STEP
}
