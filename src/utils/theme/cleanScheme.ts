// Clean 配色：对齐 Android 端 CleanTheme。所有表面都是略带冷调的中性灰，
// 强调色只落在可交互的角色上；背景偏色（色相来源 + 强度）由用户调节。
// 输出沿用 --md-* 变量名，现有组件无需改名即可切到新配色。

export type RGB = [number, number, number]

/** 背景偏色来源：跟随强调色、纯中性，或一个具体颜色（#rrggbb） */
export type BackgroundTint = 'accent' | 'neutral' | `#${string}`

export interface TintConfig {
  tint: BackgroundTint
  /** 0–100，0 为纯中性 */
  strength: number
}

export const BACKGROUND_TINT_DEFAULT: BackgroundTint = 'accent'
export const BACKGROUND_TINT_STRENGTH_DEFAULT = 12
export const BACKGROUND_TINT_STRENGTH_MAX = 100

/** 背景偏色预设色（暖、冷、青绿、玫瑰、沙），色块与设置页共用 */
export const BACKGROUND_TINT_PRESETS: { key: string; tint: BackgroundTint }[] = [
  { key: 'accent', tint: 'accent' },
  { key: 'neutral', tint: 'neutral' },
  { key: 'warm', tint: '#e0a060' },
  { key: 'cool', tint: '#5b8def' },
  { key: 'sage', tint: '#6fae8c' },
  { key: 'rose', tint: '#e07a9a' },
  { key: 'sand', tint: '#c8b48a' },
]

interface Neutrals {
  background: RGB
  surfaceDim: RGB
  surfaceBright: RGB
  containerLowest: RGB
  containerLow: RGB
  container: RGB
  containerHigh: RGB
  containerHighest: RGB
  onSurface: RGB
  onSurfaceVariant: RGB
  outline: RGB
  outlineVariant: RGB
  inverseSurface: RGB
  inverseOnSurface: RGB
}

const hex = (value: number): RGB => [(value >> 16) & 255, (value >> 8) & 255, value & 255]

// 色值取自 Android CleanTheme；桌面端侧栏/卡片层级更多，containerHigh 在浅色下略压一档，
// 以便悬停态与卡片底能区分开
const LIGHT: Neutrals = {
  background: hex(0xf4f4f7),
  surfaceDim: hex(0xe3e3e8),
  surfaceBright: hex(0xffffff),
  containerLowest: hex(0xffffff),
  containerLow: hex(0xffffff),
  container: hex(0xf9f9fb),
  containerHigh: hex(0xefeff3),
  containerHighest: hex(0xe7e7ec),
  onSurface: hex(0x1d1d1f),
  onSurfaceVariant: hex(0x5e5e64),
  outline: hex(0x85858c),
  outlineVariant: hex(0xe1e1e7),
  inverseSurface: hex(0x2c2c30),
  inverseOnSurface: hex(0xf2f2f6),
}

const DARK: Neutrals = {
  background: hex(0x0e0e10),
  surfaceDim: hex(0x0a0a0c),
  surfaceBright: hex(0x2b2b30),
  containerLowest: hex(0x09090b),
  containerLow: hex(0x17171a),
  container: hex(0x1b1b1e),
  containerHigh: hex(0x232327),
  containerHighest: hex(0x2c2c31),
  onSurface: hex(0xf2f2f5),
  onSurfaceVariant: hex(0xadadb5),
  outline: hex(0x8a8a93),
  outlineVariant: hex(0x2e2e34),
  inverseSurface: hex(0xe6e6eb),
  inverseOnSurface: hex(0x1c1c1f),
}

// 强度 100 时表面最多向偏色混入的比例；深色底需要更多才看得出来
const MAX_TINT_LIGHT = 0.14
const MAX_TINT_DARK = 0.2

const ERROR_LIGHT = hex(0xd93636)
const ERROR_DARK = hex(0xff6b6b)

export function clampTintStrength(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return BACKGROUND_TINT_STRENGTH_DEFAULT
  return Math.round(Math.min(BACKGROUND_TINT_STRENGTH_MAX, Math.max(0, value)))
}

export function normalizeBackgroundTint(value: unknown): BackgroundTint {
  if (value === 'accent' || value === 'neutral') return value
  if (typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value)) return value.toLowerCase() as BackgroundTint
  return BACKGROUND_TINT_DEFAULT
}

export function parseHexColor(value: string): RGB | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(value.trim())
  return match ? hex(parseInt(match[1], 16)) : null
}

export function rgbToHex([r, g, b]: RGB): string {
  return `#${[r, g, b].map(v => Math.round(v).toString(16).padStart(2, '0')).join('')}`
}

export function parseRgb(value: string): RGB | null {
  const match = /rgba?\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)/.exec(value)
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null
}

export function mix(a: RGB, b: RGB, amount: number): RGB {
  const t = Math.min(1, Math.max(0, amount))
  return [
    Math.round(a[0] + (b[0] - a[0]) * t),
    Math.round(a[1] + (b[1] - a[1]) * t),
    Math.round(a[2] + (b[2] - a[2]) * t),
  ]
}

export function rgb(color: RGB): string {
  return `rgb(${color[0]}, ${color[1]}, ${color[2]})`
}

function channel(value: number): number {
  const c = value / 255
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

export function luminance([r, g, b]: RGB): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

export function contrastRatio(a: RGB, b: RGB): number {
  const la = luminance(a)
  const lb = luminance(b)
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

function rgbToHsl([r, g, b]: RGB): [number, number, number] {
  const nr = r / 255, ng = g / 255, nb = b / 255
  const max = Math.max(nr, ng, nb)
  const min = Math.min(nr, ng, nb)
  const l = (max + min) / 2
  if (max === min) return [0, 0, l]
  const d = max - min
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  let h: number
  if (max === nr) h = ((ng - nb) / d + (ng < nb ? 6 : 0)) / 6
  else if (max === ng) h = ((nb - nr) / d + 2) / 6
  else h = ((nr - ng) / d + 4) / 6
  return [h, s, l]
}

function hslToRgb(h: number, s: number, l: number): RGB {
  const hue = ((h % 1) + 1) % 1
  if (s === 0) {
    const v = Math.round(l * 255)
    return [v, v, v]
  }
  const toRgb = (p: number, q: number, t: number) => {
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
    Math.round(toRgb(p, q, hue + 1 / 3) * 255),
    Math.round(toRgb(p, q, hue) * 255),
    Math.round(toRgb(p, q, hue - 1 / 3) * 255),
  ]
}

/**
 * 把任意种子色（封面主色、系统强调色）收敛成可用的强调色：保留色相，
 * 限制饱和度并把亮度推到能与当前底色拉开对比的区间。
 */
export function accentFromSeed(seed: RGB, dark: boolean): RGB {
  const [h, s] = rgbToHsl(seed)
  // 近灰的种子（黑白封面）保持中性，避免硬染出一个随机色相
  if (s < 0.08) return dark ? hex(0xe6e6eb) : hex(0x2c2c30)
  const saturation = Math.min(0.78, Math.max(0.42, s))
  const background = dark ? DARK.background : LIGHT.background
  let lightness = dark ? 0.7 : 0.44
  let color = hslToRgb(h, saturation, lightness)
  // 逐步调亮度直到对比度达到 3:1（大号文字/图形的 WCAG 下限），最多几步即可收敛
  for (let i = 0; i < 12 && contrastRatio(color, background) < 3.2; i++) {
    lightness += dark ? 0.03 : -0.03
    color = hslToRgb(h, saturation, lightness)
  }
  return color
}

function onColor(fill: RGB): RGB {
  const light: RGB = [255, 255, 255]
  const ink: RGB = [20, 20, 22]
  return contrastRatio(fill, light) >= contrastRatio(fill, ink) ? light : ink
}

/** 保证前景色在给定底色上可读：不够时向 onSurface 回拉 */
function readableOn(color: RGB, background: RGB, fallback: RGB, minimum = 4.5): RGB {
  let result = color
  for (let step = 1; step <= 10 && contrastRatio(result, background) < minimum; step++) {
    result = mix(color, fallback, step / 10)
  }
  return result
}

export function resolveTintColor(config: TintConfig, accent: RGB): RGB | null {
  if (config.tint === 'neutral' || config.strength <= 0) return null
  if (config.tint === 'accent') return accent
  return parseHexColor(config.tint)
}

/**
 * 生成整套颜色令牌。键集固定（--md-* + 少量 Clean 专用令牌），
 * 预设色、动态取色和系统强调色都经由这里，因此任何来源的配色层级都一致。
 */
export function buildCleanScheme(accent: RGB, dark: boolean, config: TintConfig): Record<string, string> {
  const n = dark ? DARK : LIGHT
  const tintColor = resolveTintColor(config, accent)
  const tintAmount = tintColor
    ? (clampTintStrength(config.strength) / 100) * (dark ? MAX_TINT_DARK : MAX_TINT_LIGHT)
    : 0
  const neutral = (color: RGB) => (tintColor ? mix(color, tintColor, tintAmount) : color)
  // 文字也带一点偏色才不会在暖底上显得发青，但比例远小于表面，保证对比度
  const ink = (color: RGB) => (tintColor ? mix(color, tintColor, tintAmount * 0.35) : color)

  const background = neutral(n.background)
  const containerLow = neutral(n.containerLow)
  const onSurface = ink(n.onSurface)
  const onSurfaceVariant = readableOn(ink(n.onSurfaceVariant), background, onSurface)

  const primary = accent
  const onPrimary = onColor(primary)
  const primaryContainer = mix(containerLow, primary, dark ? 0.24 : 0.14)
  const onPrimaryContainer = readableOn(mix(onSurface, primary, dark ? 0.3 : 0.45), primaryContainer, onSurface)
  // 选中态容器：只靠底色的淡染区分，文字保持墨色（对齐 Android selectionContainer）
  const selection = mix(containerLow, primary, dark ? 0.2 : 0.12)
  const onSelection = mix(onSurface, primary, 0.12)
  const secondary = readableOn(mix(onSurfaceVariant, primary, 0.3), background, onSurface)
  const [h, s, l] = rgbToHsl(primary)
  const tertiary = readableOn(hslToRgb(h + 0.11, s, l), background, onSurface, 3)
  const tertiaryContainer = mix(containerLow, tertiary, dark ? 0.22 : 0.14)
  const error = dark ? ERROR_DARK : ERROR_LIGHT
  const errorContainer = mix(containerLow, error, dark ? 0.22 : 0.12)

  return {
    '--md-primary': rgb(primary),
    '--md-on-primary': rgb(onPrimary),
    '--md-primary-container': rgb(primaryContainer),
    '--md-on-primary-container': rgb(onPrimaryContainer),
    '--md-secondary': rgb(secondary),
    '--md-on-secondary': rgb(onColor(secondary)),
    '--md-secondary-container': rgb(selection),
    '--md-on-secondary-container': rgb(onSelection),
    '--md-tertiary': rgb(tertiary),
    '--md-on-tertiary': rgb(onColor(tertiary)),
    '--md-tertiary-container': rgb(tertiaryContainer),
    '--md-on-tertiary-container': rgb(readableOn(mix(onSurface, tertiary, 0.35), tertiaryContainer, onSurface)),
    '--md-error': rgb(error),
    '--md-on-error': rgb(onColor(error)),
    '--md-error-container': rgb(errorContainer),
    '--md-on-error-container': rgb(readableOn(mix(onSurface, error, 0.5), errorContainer, onSurface)),
    '--md-background': rgb(background),
    '--md-surface': rgb(background),
    '--md-on-surface': rgb(onSurface),
    '--md-surface-dim': rgb(neutral(n.surfaceDim)),
    '--md-surface-bright': rgb(neutral(n.surfaceBright)),
    '--md-surface-variant': rgb(neutral(n.containerHighest)),
    '--md-on-surface-variant': rgb(onSurfaceVariant),
    '--md-surface-container-lowest': rgb(neutral(n.containerLowest)),
    '--md-surface-container-low': rgb(containerLow),
    '--md-surface-container': rgb(neutral(n.container)),
    '--md-surface-container-high': rgb(neutral(n.containerHigh)),
    '--md-surface-container-highest': rgb(neutral(n.containerHighest)),
    '--md-outline': rgb(neutral(n.outline)),
    '--md-outline-variant': rgb(neutral(n.outlineVariant)),
    '--md-inverse-surface': rgb(n.inverseSurface),
    '--md-inverse-on-surface': rgb(n.inverseOnSurface),
    '--md-inverse-primary': rgb(accentFromSeed(primary, !dark)),
    // Clean 专用：强调色淡染填充（对话框确认按钮等）与侧栏底色
    '--clean-accent-soft': rgb(mix(neutral(n.containerHigh), primary, dark ? 0.2 : 0.14)),
    '--clean-sidebar': rgb(neutral(dark ? n.containerLowest : n.container)),
  }
}
