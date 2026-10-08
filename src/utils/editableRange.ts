const RANGE_EPSILON = 1e-9

export function formatEditableNumber(value: number, scale = 1): string {
  const displayedValue = value * scale
  if (!Number.isFinite(displayedValue)) return ''
  if (Object.is(displayedValue, -0)) return '0'

  return trimTrailingZeros(displayedValue.toFixed(8))
}

export function parseEditableNumber(
  input: string | number,
  min: number,
  max: number,
  scale = 1,
): number | null {
  const text = typeof input === 'number' ? String(input) : input.trim()
  if (!text || !Number.isFinite(scale) || scale <= 0) return null

  const displayedValue = Number(text)
  if (!Number.isFinite(displayedValue)) return null

  const value = displayedValue / scale
  if (!Number.isFinite(value)) return null
  if (value < min - RANGE_EPSILON || value > max + RANGE_EPSILON) return null

  return normalizeFloatingPoint(value)
}

export function normalizeFloatingPoint(value: number): number {
  return Number(value.toFixed(8))
}

/**
 * 输入框提交值按 step 的精度取整，并夹回范围内。
 * 整数步长的设置（毫秒、MB）在 Rust 端是整数字段，带小数会让整份设置保存失败。
 */
export function roundToStepPrecision(value: number, step: number, min: number, max: number): number {
  let rounded: number
  if (!Number.isFinite(step) || step <= 0 || /e/i.test(String(step))) {
    rounded = normalizeFloatingPoint(value)
  } else if (Number.isInteger(step)) {
    rounded = Math.round(value)
  } else {
    const decimals = Math.min(8, (String(step).split('.')[1] ?? '').length)
    rounded = Number(value.toFixed(decimals))
  }
  return Math.min(max, Math.max(min, rounded))
}

function trimTrailingZeros(value: string): string {
  return value.replace(/(\.\d*?[1-9])0+$|\.0+$/, '$1').replace(/\.$/, '')
}
