export type RGB = [number, number, number]

export interface Hsl {
  /** 角度 0..360 */
  h: number
  s: number
  l: number
}

/** 独立实现的 RGB -> HSL，用于断言被测代码输出色的色相/饱和度/亮度 */
export function toHsl([r, g, b]: RGB): Hsl {
  const nr = r / 255
  const ng = g / 255
  const nb = b / 255
  const max = Math.max(nr, ng, nb)
  const min = Math.min(nr, ng, nb)
  const l = (max + min) / 2
  if (max === min) return { h: 0, s: 0, l }
  const d = max - min
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  let h: number
  if (max === nr) h = (ng - nb) / d + (ng < nb ? 6 : 0)
  else if (max === ng) h = (nb - nr) / d + 2
  else h = (nr - ng) / d + 4
  return { h: h * 60, s, l }
}

export function hueDistance(a: number, b: number): number {
  const diff = Math.abs(a - b) % 360
  return Math.min(diff, 360 - diff)
}

/** 解析 `rgb(r, g, b)` 字符串 */
export function parseRgb(value: string): RGB {
  const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(value)
  if (!match) throw new Error(`not an rgb() colour: "${value}"`)
  return [Number(match[1]), Number(match[2]), Number(match[3])]
}
