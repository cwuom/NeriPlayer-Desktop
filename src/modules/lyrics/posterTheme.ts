// 海报配色：数值取自安卓端 LyricShareSheet.kt（drawLyricCardBackground / drawSongInfoBar / drawLyricCardBrand），
// 原样保留它的对角渐变与叠白，只在"白字对比度不达标"时补一层压暗
// 门槛：歌词正文 ≥7:1（AAA）、底栏歌名歌手与品牌文字 ≥4.5:1（AA）

export type Rgb = readonly [number, number, number]

/** WCAG 相对亮度 */
export function relativeLuminance(color: Rgb): number {
  const channel = (value: number) => {
    const linear = value / 255
    return linear <= 0.04045 ? linear / 12.92 : ((linear + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(color[0]) + 0.7152 * channel(color[1]) + 0.0722 * channel(color[2])
}

export function contrastRatio(a: Rgb, b: Rgb): number {
  const light = Math.max(relativeLuminance(a), relativeLuminance(b))
  const dark = Math.min(relativeLuminance(a), relativeLuminance(b))
  return (light + 0.05) / (dark + 0.05)
}

export function toCss(color: Rgb, alpha = 1): string {
  const [red, green, blue] = color.map(value => Math.round(Math.min(255, Math.max(0, value))))
  return alpha >= 1 ? `rgb(${red}, ${green}, ${blue})` : `rgba(${red}, ${green}, ${blue}, ${alpha})`
}

export function mixRgb(from: Rgb, to: Rgb, amount: number): Rgb {
  return [
    from[0] + (to[0] - from[0]) * amount,
    from[1] + (to[1] - from[1]) * amount,
    from[2] + (to[2] - from[2]) * amount,
  ]
}

export const WHITE: Rgb = [255, 255, 255]
export const BLACK: Rgb = [0, 0, 0]

/** 安卓 argb(86,255,255,255)@0 → argb(64,0,0,0)@0.48 → argb(118,0,0,0)@1，左上到右下对角 */
export const CARD_SCRIM_STOPS = [
  { at: 0, color: WHITE, alpha: 86 / 255 },
  { at: 0.48, color: BLACK, alpha: 64 / 255 },
  { at: 1, color: BLACK, alpha: 118 / 255 },
] as const

/** 底栏叠白 argb(70,255,255,255)；品牌文字 argb(142,…)、图标 alpha 150；歌手 argb(224,…) */
export const BAR_OVERLAY_ALPHA = 70 / 255
export const BRAND_TEXT_ALPHA = 142 / 255
export const BRAND_ICON_ALPHA = 150 / 255
export const ARTIST_ALPHA = 224 / 255

/** 无封面时安卓的种子色 rgb(156,148,80) 与三段渐变 */
export const NO_COVER_SEED: Rgb = [156, 148, 80]

export function noCoverStops(seed: Rgb): { at: number, color: Rgb }[] {
  return [
    { at: 0, color: mixRgb(seed, WHITE, 0.18) },
    { at: 0.62, color: seed },
    { at: 1, color: mixRgb(seed, BLACK, 0.28) },
  ]
}

/**
 * 补暗上限压到 0.35：安卓只有固定对角蒙层，我们再深就会明显比它暗一档。
 * 判据也用的是歌词区的亮度分位数而不是最亮单点，避免一个高光点就把整张压黑
 */
export const EXTRA_SCRIM_RANGE = { max: 0.35, step: 0.02 } as const

/**
 * 白字压在给定底色上不达标时，求还要补多深的黑。
 * 调用方传"渐变合成后的实测最亮底色"，所以这里只做一维搜索，不再猜封面平均色
 */
export function extraScrimFor(background: Rgb, target: number): number {
  if (contrastRatio(WHITE, background) >= target) return 0
  for (let alpha = EXTRA_SCRIM_RANGE.step; alpha <= EXTRA_SCRIM_RANGE.max + 1e-9; alpha += EXTRA_SCRIM_RANGE.step) {
    if (contrastRatio(WHITE, mixRgb(background, BLACK, alpha)) >= target) {
      return Math.round(alpha * 100) / 100
    }
  }
  return EXTRA_SCRIM_RANGE.max
}
