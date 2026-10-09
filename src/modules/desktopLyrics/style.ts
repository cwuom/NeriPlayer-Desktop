// 桌面歌词外观：主窗口设置里编辑、持久化，随帧发给歌词窗口；两边都按这里规整，
// 歌词窗口不信任收到的任何字段

export const DESKTOP_LYRICS_LAYOUTS = ['single', 'double', 'triple'] as const
export const DESKTOP_LYRICS_ALIGNS = ['left', 'center', 'right'] as const
/** 第二行显示什么；双行（KTV）布局两行都是歌词，不显示第二行 */
export const DESKTOP_LYRICS_SECONDARY = ['translation', 'romanization', 'next', 'none'] as const
/** 逐字：有逐字时间轴时按字推进，没有时整行匀速扫过；整行：只按行扫；关闭：唱到哪行整行高亮 */
export const DESKTOP_LYRICS_KARAOKE = ['word', 'line', 'off'] as const
export const DESKTOP_LYRICS_BACKGROUNDS = ['hover', 'always', 'never'] as const
export const DESKTOP_LYRICS_WEIGHTS = [400, 500, 600, 700, 800, 900] as const

export type DesktopLyricsLayout = typeof DESKTOP_LYRICS_LAYOUTS[number]
export type DesktopLyricsAlign = typeof DESKTOP_LYRICS_ALIGNS[number]
export type DesktopLyricsSecondary = typeof DESKTOP_LYRICS_SECONDARY[number]
export type DesktopLyricsKaraoke = typeof DESKTOP_LYRICS_KARAOKE[number]
export type DesktopLyricsBackground = typeof DESKTOP_LYRICS_BACKGROUNDS[number]

export interface DesktopLyricsBounds {
  x: number
  y: number
  width: number
  height: number
}

export interface DesktopLyricsStyle {
  /** 预设主题 id；'accent' 跟随应用主题色（开了动态取色时就是封面色），'custom' 为自选颜色 */
  theme: string
  layout: DesktopLyricsLayout
  align: DesktopLyricsAlign
  secondary: DesktopLyricsSecondary
  karaoke: DesktopLyricsKaraoke
  background: DesktopLyricsBackground
  /** 空串用应用默认字体 */
  fontFamily: string
  fontSize: number
  fontWeight: number
  letterSpacing: number
  secondaryScale: number
  playedColors: [string, string]
  unplayedColors: [string, string]
  secondaryColor: string
  strokeEnabled: boolean
  strokeColor: string
  strokeWidth: number
  shadowEnabled: boolean
  shadowColor: string
  shadowBlur: number
  glowEnabled: boolean
  opacity: number
  backgroundOpacity: number
  showTrackInfo: boolean
  /** 锁定后窗口点击穿透，悬停时只有解锁按钮可点 */
  locked: boolean
  /** 上次的位置和大小（逻辑像素）；null 时放在主屏幕底部居中 */
  bounds: DesktopLyricsBounds | null
}

export interface DesktopLyricsTheme {
  id: string
  played: [string, string]
  unplayed: [string, string]
  secondary: string
  stroke: string
  shadow: string
}

export const DESKTOP_LYRICS_THEMES: readonly DesktopLyricsTheme[] = [
  { id: 'aurora', played: ['#a8f0ff', '#7c9cff'], unplayed: ['#ffffff', '#e4e9ff'], secondary: '#e8ecff', stroke: '#0b1020', shadow: '#000000' },
  { id: 'classic', played: ['#5ad1ff', '#1e88ff'], unplayed: ['#ffffff', '#ffffff'], secondary: '#ffffff', stroke: '#000000', shadow: '#000000' },
  { id: 'sakura', played: ['#ffd6ea', '#ff7eb6'], unplayed: ['#ffffff', '#ffeef6'], secondary: '#ffe3f0', stroke: '#2a0f1c', shadow: '#1a0610' },
  { id: 'sunset', played: ['#ffe29a', '#ff8a4c'], unplayed: ['#ffffff', '#fff3e4'], secondary: '#fff0dc', stroke: '#2a1408', shadow: '#140802' },
  { id: 'mint', played: ['#d0fff0', '#2fd6a3'], unplayed: ['#ffffff', '#eafff7'], secondary: '#e3fff5', stroke: '#062018', shadow: '#02100b' },
  { id: 'neon', played: ['#f7a8ff', '#7c4dff'], unplayed: ['#e3f8ff', '#b8e6fb'], secondary: '#e9ddff', stroke: '#12002b', shadow: '#0a0018' },
  { id: 'mono', played: ['#ffffff', '#ffffff'], unplayed: ['#a3a3a3', '#c2c2c2'], secondary: '#d6d6d6', stroke: '#000000', shadow: '#000000' },
]

export const DESKTOP_LYRICS_FONT_SIZE = { min: 16, max: 96, step: 2 } as const
export const DESKTOP_LYRICS_LETTER_SPACING = { min: -2, max: 12, step: 0.5 } as const
export const DESKTOP_LYRICS_SECONDARY_SCALE = { min: 0.4, max: 1, step: 0.05 } as const
export const DESKTOP_LYRICS_STROKE_WIDTH = { min: 0.5, max: 4, step: 0.5 } as const
export const DESKTOP_LYRICS_SHADOW_BLUR = { min: 0, max: 24, step: 1 } as const
export const DESKTOP_LYRICS_OPACITY = { min: 0.3, max: 1, step: 0.05 } as const
export const DESKTOP_LYRICS_BACKGROUND_OPACITY = { min: 0, max: 1, step: 0.05 } as const
/** 窗口最小、最大尺寸（逻辑像素），与后端一致 */
export const DESKTOP_LYRICS_WINDOW = { minWidth: 360, minHeight: 110, maxWidth: 4096, maxHeight: 1200 } as const

/** 常用字体；也可以手填系统里装的任意字体名 */
export const DESKTOP_LYRICS_FONT_PRESETS = [
  'MiSans', 'Microsoft YaHei', 'PingFang SC', 'Noto Sans SC', 'Source Han Sans SC',
  'SimHei', 'KaiTi', 'SimSun', 'Inter', 'Segoe UI',
] as const

const DEFAULT_THEME = DESKTOP_LYRICS_THEMES[0]

export const DEFAULT_DESKTOP_LYRICS_STYLE: Readonly<DesktopLyricsStyle> = Object.freeze({
  theme: DEFAULT_THEME.id,
  layout: 'single',
  align: 'center',
  secondary: 'translation',
  karaoke: 'word',
  background: 'hover',
  fontFamily: '',
  fontSize: 36,
  fontWeight: 700,
  letterSpacing: 0,
  secondaryScale: 0.55,
  playedColors: [...DEFAULT_THEME.played] as [string, string],
  unplayedColors: [...DEFAULT_THEME.unplayed] as [string, string],
  secondaryColor: DEFAULT_THEME.secondary,
  strokeEnabled: true,
  strokeColor: DEFAULT_THEME.stroke,
  strokeWidth: 1,
  shadowEnabled: true,
  shadowColor: DEFAULT_THEME.shadow,
  shadowBlur: 8,
  glowEnabled: false,
  opacity: 1,
  backgroundOpacity: 0.55,
  showTrackInfo: true,
  locked: false,
  bounds: null,
})

const HEX_COLOR = /^#[0-9a-f]{6}$/i

function pick<T extends string | number>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.includes(value as T) ? value as T : fallback
}

function stepped(value: unknown, range: { min: number, max: number, step: number }, fallback: number): number {
  const number = typeof value === 'number' ? value : Number(value)
  if (!Number.isFinite(number)) return fallback
  const clamped = Math.min(range.max, Math.max(range.min, number))
  const aligned = Math.round((clamped - range.min) / range.step) * range.step + range.min
  return Math.round(aligned * 1000) / 1000
}

function color(value: unknown, fallback: string): string {
  return typeof value === 'string' && HEX_COLOR.test(value.trim()) ? value.trim().toLowerCase() : fallback
}

function colorPair(value: unknown, fallback: readonly [string, string]): [string, string] {
  const values = Array.isArray(value) ? value : []
  return [color(values[0], fallback[0]), color(values[1], fallback[1])]
}

/** 字体名只留字母、数字、空格和 -_.，防止拼进 CSS 时注入其它声明 */
export function sanitizeFontFamily(value: unknown): string {
  if (typeof value !== 'string') return ''
  return value.replace(/[^\p{L}\p{N} ._-]/gu, '').replace(/\s+/g, ' ').trim().slice(0, 64)
}

function bounds(value: unknown): DesktopLyricsBounds | null {
  if (!value || typeof value !== 'object') return null
  const raw = value as Record<string, unknown>
  const [x, y, width, height] = ['x', 'y', 'width', 'height'].map(key => Number(raw[key]))
  if (![x, y, width, height].every(Number.isFinite)) return null
  if (Math.abs(x) > 100_000 || Math.abs(y) > 100_000) return null
  return {
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(Math.min(DESKTOP_LYRICS_WINDOW.maxWidth, Math.max(DESKTOP_LYRICS_WINDOW.minWidth, width))),
    height: Math.round(Math.min(DESKTOP_LYRICS_WINDOW.maxHeight, Math.max(DESKTOP_LYRICS_WINDOW.minHeight, height))),
  }
}

export function normalizeDesktopLyricsStyle(value: unknown): DesktopLyricsStyle {
  const raw = value && typeof value === 'object' ? value as Record<string, unknown> : {}
  const defaults = DEFAULT_DESKTOP_LYRICS_STYLE
  const themeIds = [...DESKTOP_LYRICS_THEMES.map(theme => theme.id), 'accent', 'custom']
  return {
    theme: pick(raw.theme, themeIds, defaults.theme),
    layout: pick(raw.layout, DESKTOP_LYRICS_LAYOUTS, defaults.layout),
    align: pick(raw.align, DESKTOP_LYRICS_ALIGNS, defaults.align),
    secondary: pick(raw.secondary, DESKTOP_LYRICS_SECONDARY, defaults.secondary),
    karaoke: pick(raw.karaoke, DESKTOP_LYRICS_KARAOKE, defaults.karaoke),
    background: pick(raw.background, DESKTOP_LYRICS_BACKGROUNDS, defaults.background),
    fontFamily: sanitizeFontFamily(raw.fontFamily),
    fontSize: stepped(raw.fontSize, DESKTOP_LYRICS_FONT_SIZE, defaults.fontSize),
    fontWeight: pick(Number(raw.fontWeight), DESKTOP_LYRICS_WEIGHTS, defaults.fontWeight as typeof DESKTOP_LYRICS_WEIGHTS[number]),
    letterSpacing: stepped(raw.letterSpacing, DESKTOP_LYRICS_LETTER_SPACING, defaults.letterSpacing),
    secondaryScale: stepped(raw.secondaryScale, DESKTOP_LYRICS_SECONDARY_SCALE, defaults.secondaryScale),
    playedColors: colorPair(raw.playedColors, defaults.playedColors),
    unplayedColors: colorPair(raw.unplayedColors, defaults.unplayedColors),
    secondaryColor: color(raw.secondaryColor, defaults.secondaryColor),
    strokeEnabled: typeof raw.strokeEnabled === 'boolean' ? raw.strokeEnabled : defaults.strokeEnabled,
    strokeColor: color(raw.strokeColor, defaults.strokeColor),
    strokeWidth: stepped(raw.strokeWidth, DESKTOP_LYRICS_STROKE_WIDTH, defaults.strokeWidth),
    shadowEnabled: typeof raw.shadowEnabled === 'boolean' ? raw.shadowEnabled : defaults.shadowEnabled,
    shadowColor: color(raw.shadowColor, defaults.shadowColor),
    shadowBlur: stepped(raw.shadowBlur, DESKTOP_LYRICS_SHADOW_BLUR, defaults.shadowBlur),
    glowEnabled: typeof raw.glowEnabled === 'boolean' ? raw.glowEnabled : defaults.glowEnabled,
    opacity: stepped(raw.opacity, DESKTOP_LYRICS_OPACITY, defaults.opacity),
    backgroundOpacity: stepped(raw.backgroundOpacity, DESKTOP_LYRICS_BACKGROUND_OPACITY, defaults.backgroundOpacity),
    showTrackInfo: typeof raw.showTrackInfo === 'boolean' ? raw.showTrackInfo : defaults.showTrackInfo,
    locked: raw.locked === true,
    bounds: bounds(raw.bounds),
  }
}

/** 选预设主题：把它的颜色写进样式，之后改任一颜色就变成自定义 */
export function applyDesktopLyricsTheme(style: DesktopLyricsStyle, themeId: string): DesktopLyricsStyle {
  if (themeId === 'accent') return { ...style, theme: 'accent' }
  const theme = DESKTOP_LYRICS_THEMES.find(candidate => candidate.id === themeId)
  if (!theme) return style
  return {
    ...style,
    theme: theme.id,
    playedColors: [...theme.played],
    unplayedColors: [...theme.unplayed],
    secondaryColor: theme.secondary,
    strokeColor: theme.stroke,
    shadowColor: theme.shadow,
  }
}

/** 'rgb(208, 188, 255)' / '#d0bcff' → '#d0bcff'；读不出来时为 null */
export function parseCssColor(value: string | null | undefined): string | null {
  const text = (value || '').trim()
  if (HEX_COLOR.test(text)) return text.toLowerCase()
  const match = /^rgba?\(\s*(\d{1,3})[\s,]+(\d{1,3})[\s,]+(\d{1,3})/i.exec(text)
  if (!match) return null
  const channels = match.slice(1, 4).map(channel => Math.min(255, Number(channel)))
  return `#${channels.map(channel => channel.toString(16).padStart(2, '0')).join('')}`
}

function mixWithWhite(hex: string, amount: number): string {
  const channels = [1, 3, 5].map(index => Number.parseInt(hex.slice(index, index + 2), 16))
  return `#${channels.map(channel => Math.round(channel + (255 - channel) * amount).toString(16).padStart(2, '0')).join('')}`
}

export interface DesktopLyricsColors {
  played: [string, string]
  unplayed: [string, string]
  secondary: string
}

/** 实际绘制用的颜色；跟随主题色时由强调色派生 */
export function resolveDesktopLyricsColors(style: DesktopLyricsStyle, accent: string | null): DesktopLyricsColors {
  if (style.theme === 'accent' && accent) {
    return {
      played: [mixWithWhite(accent, 0.55), accent],
      unplayed: ['#ffffff', mixWithWhite(accent, 0.88)],
      secondary: mixWithWhite(accent, 0.8),
    }
  }
  return { played: style.playedColors, unplayed: style.unplayedColors, secondary: style.secondaryColor }
}

function alpha(hex: string, opacity: number): string {
  return `${hex}${Math.round(Math.min(1, Math.max(0, opacity)) * 255).toString(16).padStart(2, '0')}`
}

/** 歌词窗口和设置页预览共用的 CSS 变量 */
export function desktopLyricsCssVars(style: DesktopLyricsStyle, accent: string | null): Record<string, string> {
  const colors = resolveDesktopLyricsColors(style, accent)
  const family = style.fontFamily ? `"${style.fontFamily}", ` : ''
  const shadow = style.shadowEnabled
    ? `drop-shadow(0 ${Math.max(1, Math.round(style.shadowBlur / 4))}px ${style.shadowBlur}px ${alpha(style.shadowColor, 0.85)})`
    : 'none'
  return {
    '--dl-font-family': `${family}var(--font-family, "MiSans", system-ui, sans-serif)`,
    '--dl-font-size': `${style.fontSize}px`,
    '--dl-font-weight': String(style.fontWeight),
    '--dl-letter-spacing': `${style.letterSpacing}px`,
    '--dl-played': `linear-gradient(180deg, ${colors.played[0]}, ${colors.played[1]})`,
    '--dl-unplayed': `linear-gradient(180deg, ${colors.unplayed[0]}, ${colors.unplayed[1]})`,
    '--dl-played-solid': colors.played[1],
    '--dl-secondary-color': colors.secondary,
    '--dl-secondary-size': `${Math.round(style.fontSize * style.secondaryScale)}px`,
    '--dl-stroke': style.strokeEnabled ? `${style.strokeWidth * 2}px ${style.strokeColor}` : '0 transparent',
    '--dl-shadow': shadow,
    '--dl-glow': style.glowEnabled ? `drop-shadow(0 0 ${Math.round(style.fontSize / 4)}px ${alpha(colors.played[1], 0.75)})` : 'none',
    '--dl-opacity': String(style.opacity),
    '--dl-background': `rgba(16, 15, 24, ${style.backgroundOpacity})`,
  }
}

export function nextDesktopLyricsLayout(layout: DesktopLyricsLayout): DesktopLyricsLayout {
  return DESKTOP_LYRICS_LAYOUTS[(DESKTOP_LYRICS_LAYOUTS.indexOf(layout) + 1) % DESKTOP_LYRICS_LAYOUTS.length]
}

export function stepDesktopLyricsFontSize(style: DesktopLyricsStyle, direction: 1 | -1): DesktopLyricsStyle {
  return { ...style, fontSize: stepped(style.fontSize + direction * DESKTOP_LYRICS_FONT_SIZE.step, DESKTOP_LYRICS_FONT_SIZE, style.fontSize) }
}
