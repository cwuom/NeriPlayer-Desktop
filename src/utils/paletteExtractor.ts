/**
 * 调色板提取器：完整移植 Android HyperBackground.kt 的 5 角色取色与 softenPaletteColor
 * 确保 shader 获得色彩多样性，避免"看起来像纯色"的问题
 */

type RGB = [number, number, number] // 0-255

interface ColorBucket {
  pixels: RGB[]
  average: RGB
  population: number
}

export interface PaletteResult {
  /** 5 个 shader 角色色 (Base, Accent, Light, Dark, Bridge)，已 soften 处理 */
  shaderColors: [RGB, RGB, RGB, RGB, RGB]
  lightOffset: number
  saturateOffset: number
  /** 底色 RGB (AccentBackdrop) */
  accentBg: RGB
  /** 主色（未 soften 的 base 原色） */
  primaryColor: RGB
  // 兼容旧接口
  dominant: RGB
  lightVibrant: RGB
  muted: RGB
  darkMuted: RGB
}

// 常量（对齐 Android HyperBackground.kt）
const STRONG_HUE_CONFLICT_DEGREES = 105
const FALLBACK_COLOR: RGB = [128, 128, 128]

enum ColorRole {
  Base = 'Base',
  Accent = 'Accent',
  Light = 'Light',
  Dark = 'Dark',
  Bridge = 'Bridge',
}

// 色彩空间工具
function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const nr = r / 255, ng = g / 255, nb = b / 255
  const max = Math.max(nr, ng, nb), min = Math.min(nr, ng, nb)
  const l = (max + min) / 2
  if (max === min) return [0, 0, l]
  const d = max - min
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  let h: number
  if (max === nr) h = ((ng - nb) / d + (ng < nb ? 6 : 0)) / 6
  else if (max === ng) h = ((nb - nr) / d + 2) / 6
  else h = ((nr - ng) / d + 4) / 6
  // h 为 0..1，转为 0..360 度（对齐 Android ColorUtils.colorToHSL 输出）
  return [h * 360, s, l]
}

function hslToRgb(h: number, s: number, l: number): RGB {
  // h: 0..360 度
  const hNorm = ((h % 360) + 360) % 360 / 360
  if (s === 0) {
    const v = Math.round(l * 255)
    return [v, v, v]
  }
  const hue2rgb = (p: number, q: number, t: number) => {
    if (t < 0) t += 1
    if (t > 1) t -= 1
    if (t < 1 / 6) return p + (q - p) * 6 * t
    if (t < 1 / 2) return q
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6
    return p
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s
  const p = 2 * l - q
  return [
    Math.round(hue2rgb(p, q, hNorm + 1 / 3) * 255),
    Math.round(hue2rgb(p, q, hNorm) * 255),
    Math.round(hue2rgb(p, q, hNorm - 1 / 3) * 255),
  ]
}

/** BT.709 luma（对齐 Android colorLuma） */
function colorLuma(c: RGB): number {
  return 0.2126 * (c[0] / 255) + 0.7152 * (c[1] / 255) + 0.0722 * (c[2] / 255)
}

function blendRgb(c1: RGB, c2: RGB, ratio: number): RGB {
  return [
    Math.round(c1[0] + (c2[0] - c1[0]) * ratio),
    Math.round(c1[1] + (c2[1] - c1[1]) * ratio),
    Math.round(c1[2] + (c2[2] - c1[2]) * ratio),
  ]
}

function hueDistanceDegrees(h1: number, h2: number): number {
  const diff = Math.abs(h1 - h2) % 360
  return Math.min(diff, 360 - diff)
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v))
}

// Median-Cut
function channelRange(pixels: RGB[], ch: 0 | 1 | 2): number {
  let min = 255, max = 0
  for (const p of pixels) {
    if (p[ch] < min) min = p[ch]
    if (p[ch] > max) max = p[ch]
  }
  return max - min
}

function widestChannel(pixels: RGB[]): 0 | 1 | 2 {
  const rr = channelRange(pixels, 0)
  const gr = channelRange(pixels, 1)
  const br = channelRange(pixels, 2)
  if (rr >= gr && rr >= br) return 0
  if (gr >= rr && gr >= br) return 1
  return 2
}

function averageColor(pixels: RGB[]): RGB {
  if (pixels.length === 0) return [128, 128, 128]
  let tr = 0, tg = 0, tb = 0
  for (const p of pixels) { tr += p[0]; tg += p[1]; tb += p[2] }
  const n = pixels.length
  return [Math.round(tr / n), Math.round(tg / n), Math.round(tb / n)]
}

function medianCut(pixels: RGB[], maxBuckets: number): ColorBucket[] {
  if (pixels.length === 0) return []
  const buckets: RGB[][] = [pixels]

  while (buckets.length < maxBuckets) {
    let maxIdx = 0, maxLen = 0
    for (let i = 0; i < buckets.length; i++) {
      if (buckets[i].length > maxLen) {
        maxLen = buckets[i].length
        maxIdx = i
      }
    }
    if (maxLen <= 1) break

    const bucket = buckets[maxIdx]
    const ch = widestChannel(bucket)
    bucket.sort((a, b) => a[ch] - b[ch])
    const mid = Math.floor(bucket.length / 2)
    buckets.splice(maxIdx, 1, bucket.slice(0, mid), bucket.slice(mid))
  }

  return buckets.map(b => ({
    pixels: b,
    average: averageColor(b),
    population: b.length,
  }))
}

// Android 对齐：角色色选取（buildDynamicBackgroundPalette）
/** 对齐 Android colorfulnessScore */
function colorfulnessScore(c: RGB): number {
  const [, s, l] = rgbToHsl(c[0], c[1], c[2])
  return s * (0.45 + Math.min(Math.abs(l - 0.5), 0.5))
}

/** 对齐 Android pickPaletteColor：取第一个非黑非零色 */
function pickColor(...candidates: (RGB | null)[]): RGB {
  for (const c of candidates) {
    if (c && (c[0] !== 0 || c[1] !== 0 || c[2] !== 0)) return c
  }
  return FALLBACK_COLOR
}

/** 对齐 Android pickAccentPaletteColor：选色彩最丰富的 */
function pickAccentColor(fallback: RGB, ...candidates: (RGB | null)[]): RGB {
  const validColors = candidates.filter(
    (c): c is RGB => c != null && (c[0] !== 0 || c[1] !== 0 || c[2] !== 0)
  )
  const best = validColors.reduce<RGB | null>((best, c) => {
    if (!best) return c
    return colorfulnessScore(c) > colorfulnessScore(best) ? c : best
  }, null)
  const selected = best && colorfulnessScore(best) >= 0.10 ? best : fallback
  return ensureAccentColor(selected, fallback)
}

/** 对齐 Android ensureAccentColor：保证 accent 有最低饱和度 */
function ensureAccentColor(color: RGB, fallback: RGB): RGB {
  const [h, s, l] = rgbToHsl(color[0], color[1], color[2])
  if (s >= 0.24) return color

  const [fh, fs] = rgbToHsl(fallback[0], fallback[1], fallback[2])
  const resolvedHue = s > 0.04 ? h : (fs > 0.04 ? fh : 320)
  return hslToRgb(resolvedHue, 0.48, clamp(l, 0.40, 0.68))
}

function darkenColor(color: RGB, targetLightness: number): RGB {
  const [h, s, l] = rgbToHsl(color[0], color[1], color[2])
  return hslToRgb(h, s, Math.min(l, targetLightness))
}

/** 对齐 Android softenPaletteColor：色彩散布逻辑 */
function softenPaletteColor(
  rawColor: RGB,
  anchorColor: RGB,
  isDark: boolean,
  role: ColorRole
): RGB {
  const rawHsl = rgbToHsl(rawColor[0], rawColor[1], rawColor[2])
  const anchorHsl = rgbToHsl(anchorColor[0], anchorColor[1], anchorColor[2])

  // 色相冲突检测
  const hueConflict =
    hueDistanceDegrees(rawHsl[0], anchorHsl[0]) > STRONG_HUE_CONFLICT_DEGREES &&
    rawHsl[1] > 0.28 &&
    anchorHsl[1] > 0.20

  // 对齐 Android anchorBlend 配置
  const anchorBlend = (() => {
    switch (role) {
      case ColorRole.Base: return 0.08
      case ColorRole.Accent: return hueConflict ? 0.18 : 0.04
      case ColorRole.Light: return hueConflict ? 0.24 : 0.08
      case ColorRole.Dark: return hueConflict ? 0.42 : 0.22
      case ColorRole.Bridge: return 0.20
    }
  })()

  const blended = blendRgb(rawColor, anchorColor, anchorBlend)
  const hsl = rgbToHsl(blended[0], blended[1], blended[2])

  // 饱和度调整（对齐 Android）
  const saturationScale = (() => {
    switch (role) {
      case ColorRole.Base: return 0.94
      case ColorRole.Accent: return 1.14
      case ColorRole.Light: return 1.02
      case ColorRole.Dark: return 0.76
      case ColorRole.Bridge: return 0.96
    }
  })()

  const saturationMax = role === ColorRole.Accent
    ? (isDark ? 0.70 : 0.64)
    : (isDark ? 0.64 : 0.56)

  const saturationFloor = (() => {
    if (hsl[1] < 0.05) {
      return role === ColorRole.Accent ? 0.30 : 0
    }
    if (role === ColorRole.Dark) return 0.10
    if (role === ColorRole.Accent) return 0.30
    return 0.14
  })()

  hsl[1] = clamp(hsl[1] * saturationScale, saturationFloor, saturationMax)

  // 亮度调整（对齐 Android targetLightness / lightnessBlend）
  const targetLightnessMap = (() => {
    switch (role) {
      case ColorRole.Base: return isDark ? 0.34 : 0.58
      case ColorRole.Accent: return isDark ? 0.48 : 0.66
      case ColorRole.Light: return isDark ? 0.62 : 0.82
      case ColorRole.Dark: return isDark ? 0.18 : 0.34
      case ColorRole.Bridge: return isDark ? 0.46 : 0.66
    }
  })()

  const lightnessBlend = (() => {
    switch (role) {
      case ColorRole.Base: return 0.18
      case ColorRole.Accent: return 0.18
      case ColorRole.Light: return 0.24
      case ColorRole.Dark: return 0.34
      case ColorRole.Bridge: return 0.20
    }
  })()

  const minL = isDark ? 0.12 : 0.28
  const maxL = isDark ? 0.66 : 0.86
  hsl[2] = clamp(hsl[2] + (targetLightnessMap - hsl[2]) * lightnessBlend, minL, maxL)

  return hslToRgb(hsl[0], hsl[1], hsl[2])
}

// 角色色提取（从 Median-Cut buckets 推导，模拟 AndroidX Palette swatch 分类）
interface SwatchSet {
  dominant: RGB | null
  vibrant: RGB | null
  lightVibrant: RGB | null
  darkVibrant: RGB | null
  muted: RGB | null
  lightMuted: RGB | null
  darkMuted: RGB | null
}

interface ScoredSwatch {
  color: RGB
  h: number
  s: number
  l: number
  pop: number
}

interface SwatchTarget {
  minS: number
  targetS: number
  maxS: number
  minL: number
  targetL: number
  maxL: number
}

const TARGET_LIGHT_VIBRANT: SwatchTarget = {
  minS: 0.35, targetS: 1.0, maxS: 1.0,
  minL: 0.55, targetL: 0.74, maxL: 1.0,
}
const TARGET_VIBRANT: SwatchTarget = {
  minS: 0.35, targetS: 1.0, maxS: 1.0,
  minL: 0.30, targetL: 0.50, maxL: 0.70,
}
const TARGET_DARK_VIBRANT: SwatchTarget = {
  minS: 0.35, targetS: 1.0, maxS: 1.0,
  minL: 0.0, targetL: 0.26, maxL: 0.45,
}
const TARGET_LIGHT_MUTED: SwatchTarget = {
  minS: 0.0, targetS: 0.30, maxS: 0.40,
  minL: 0.55, targetL: 0.74, maxL: 1.0,
}
const TARGET_MUTED: SwatchTarget = {
  minS: 0.0, targetS: 0.30, maxS: 0.40,
  minL: 0.30, targetL: 0.50, maxL: 0.70,
}
const TARGET_DARK_MUTED: SwatchTarget = {
  minS: 0.0, targetS: 0.30, maxS: 0.40,
  minL: 0.0, targetL: 0.26, maxL: 0.45,
}

function classifySwatches(buckets: ColorBucket[]): SwatchSet {
  if (buckets.length === 0) {
    return { dominant: null, vibrant: null, lightVibrant: null, darkVibrant: null, muted: null, lightMuted: null, darkMuted: null }
  }

  const sorted = [...buckets].sort((a, b) => b.population - a.population)
  const dominant = sorted[0].average
  const maxPopulation = sorted[0].population

  const withHsl: ScoredSwatch[] = buckets.map(b => {
    const [h, s, l] = rgbToHsl(b.average[0], b.average[1], b.average[2])
    return { color: b.average, h, s, l, pop: b.population }
  })

  const used = new Set<string>()
  const lightVibrant = selectTargetSwatch(withHsl, TARGET_LIGHT_VIBRANT, maxPopulation, used)
  const vibrant = selectTargetSwatch(withHsl, TARGET_VIBRANT, maxPopulation, used)
  const darkVibrant = selectTargetSwatch(withHsl, TARGET_DARK_VIBRANT, maxPopulation, used)
  const lightMuted = selectTargetSwatch(withHsl, TARGET_LIGHT_MUTED, maxPopulation, used)
  const muted = selectTargetSwatch(withHsl, TARGET_MUTED, maxPopulation, used)
  const darkMuted = selectTargetSwatch(withHsl, TARGET_DARK_MUTED, maxPopulation, used)

  return { dominant, vibrant, lightVibrant, darkVibrant, muted, lightMuted, darkMuted }
}

function selectTargetSwatch(
  swatches: ScoredSwatch[],
  target: SwatchTarget,
  maxPopulation: number,
  used: Set<string>,
): RGB | null {
  const candidates = swatches
    .filter(s =>
      s.s >= target.minS && s.s <= target.maxS &&
      s.l >= target.minL && s.l <= target.maxL &&
      !used.has(colorKey(s.color))
    )
    .sort((a, b) =>
      targetScore(b, target, maxPopulation) - targetScore(a, target, maxPopulation)
    )

  const selected = candidates[0]?.color ?? null
  if (selected) used.add(colorKey(selected))
  return selected
}

function targetScore(swatch: ScoredSwatch, target: SwatchTarget, maxPopulation: number): number {
  const saturationScore = 1 - Math.abs(swatch.s - target.targetS)
  const lightnessScore = 1 - Math.abs(swatch.l - target.targetL)
  const populationScore = maxPopulation > 0 ? swatch.pop / maxPopulation : 0
  return 0.24 * saturationScore + 0.52 * lightnessScore + 0.24 * populationScore
}

function colorKey(c: RGB): string {
  return `${c[0]},${c[1]},${c[2]}`
}

// 主入口（对齐 Android buildDynamicBackgroundPalette）
export function extractPalette(
  imageData: ImageData,
  maxColors = 16,
  isDark = true,
): PaletteResult {
  const { data, width, height } = imageData
  const pixels: RGB[] = []

  for (let i = 0; i < width * height; i++) {
    const idx = i * 4
    const alpha = data[idx + 3] ?? 255
    if (alpha < 8) continue
    pixels.push([data[idx], data[idx + 1], data[idx + 2]])
  }

  const buckets = medianCut(pixels, maxColors)
  const swatches = classifySwatches(buckets)

  // 主色优先 dominant/vibrant，避免线稿黄封面被 muted 粉灰抢走
  // Android 虽先 muted，但桌面端大面积纯色封面用 dominant 更稳
  const baseColor = pickColor(
    swatches.dominant,
    swatches.vibrant,
    swatches.muted,
    swatches.darkMuted,
    swatches.lightMuted,
  )
  const accentColor = pickAccentColor(
    baseColor,
    swatches.vibrant,
    swatches.lightVibrant,
    swatches.darkVibrant,
    swatches.dominant,
    swatches.muted,
  )
  const lightColor = pickColor(swatches.lightVibrant, swatches.lightMuted, accentColor, baseColor)
  const darkColor = pickColor(
    swatches.darkVibrant,
    swatches.darkMuted,
    darkenColor(accentColor, isDark ? 0.44 : 0.34),
    swatches.muted,
    baseColor,
  )
  const bridgeColor = blendRgb(blendRgb(accentColor, lightColor, 0.50), baseColor, 0.24)

  // softenPaletteColor 对每个角色做色彩散布
  const softenedBase = softenPaletteColor(baseColor, baseColor, isDark, ColorRole.Base)
  const softenedAccent = softenPaletteColor(accentColor, baseColor, isDark, ColorRole.Accent)
  const softenedLight = softenPaletteColor(lightColor, baseColor, isDark, ColorRole.Light)
  const softenedDark = softenPaletteColor(darkColor, baseColor, isDark, ColorRole.Dark)
  const softenedBridge = softenPaletteColor(bridgeColor, baseColor, isDark, ColorRole.Bridge)

  const shaderColors: [RGB, RGB, RGB, RGB, RGB] = [
    softenedBase,
    softenedAccent,
    softenedLight,
    softenedDark,
    softenedBridge,
  ]

  // 计算 shader 偏移（对齐 Android buildDynamicBackgroundShaderPalette）
  const lumaValue = colorLuma(softenedBase)
  const lightOffset = clamp(
    isDark
      ? -0.06 + 0.12 * (lumaValue - 0.5)
      : 0.08 + 0.10 * (0.5 - lumaValue),
    -0.12,
    0.12,
  )
  const saturateOffset = isDark ? 0.24 : 0.16

  const accentBg = computeAccentBg(baseColor, isDark)

  return {
    shaderColors,
    lightOffset,
    saturateOffset,
    accentBg,
    primaryColor: softenedBase,
    // 兼容旧接口
    dominant: softenedBase,
    lightVibrant: softenedAccent,
    muted: softenedLight,
    darkMuted: softenedDark,
  }
}

function computeAccentBg(dominant: RGB, isDark: boolean): RGB {
  const [h, rawS, l] = rgbToHsl(dominant[0], dominant[1], dominant[2])
  // 抑制无意义的低饱和粉偏（常见于线稿/皮肤噪点），保留高饱和主色
  const s = rawS < 0.18 && (h < 0.05 || h > 0.90 || (h > 0.85 && h < 0.98)) ? 0 : rawS
  // 提升主色饱和参与度，让黄封面更接近黄色而不是粉紫
  const targetS = isDark
    ? Math.min(Math.max(s, 0.12) * 0.55, 0.42)
    : Math.min(Math.max(s, 0.10) * 0.42, 0.32)
  const targetL = isDark ? clamp(l * 0.55 + 0.12, 0.18, 0.32) : 0.90
  const adjusted = hslToRgb(h, targetS, targetL)
  const neutral: RGB = isDark ? [18, 18, 18] : [255, 255, 255]
  const blend = isDark ? 0.18 : 0.28
  return [
    Math.round(adjusted[0] * (1 - blend) + neutral[0] * blend),
    Math.round(adjusted[1] * (1 - blend) + neutral[1] * blend),
    Math.round(adjusted[2] * (1 - blend) + neutral[2] * blend),
  ]
}
