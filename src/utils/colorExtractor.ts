// 动态取色编排器
//
// 复用已有能力，避免重复实现：
//   - 取色算法复用 paletteExtractor.extractPalette（与 HyperBackground/NowPlaying 同源）
//   - 令牌应用复用 themeColor.applyThemeVars（含深色 surface 微混）
//   - 封面代理/缓存复用 bilibiliCover 的 resolve/peek
// 本模块只负责：把封面解析成像素 -> 取主色种子 -> 生成 M3 令牌 -> 应用/还原

import { createLogger } from '@/utils/logger'
import { extractPalette } from '@/utils/paletteExtractor'
import {
  resolveCoverImage,
  peekCoverImage,
  normalizeProxiedCoverUrl,
  normalizeCoverUrlForDisplay,
} from '@/utils/bilibiliCover'
import { applyThemeVars, clearDynamicThemeColor, beginSmoothThemeTransition } from '@/utils/themeColor'

const log = createLogger('dynamic-color')

// 0-255 RGB
type RGB = [number, number, number]

// 采样尺寸对齐 Android CoverArtColorCache（96x96）
const SAMPLE_SIZE = 96

// 色彩空间工具（h 取 0..1，仅本模块令牌生成使用）
function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const nr = r / 255, ng = g / 255, nb = b / 255
  const max = Math.max(nr, ng, nb)
  const min = Math.min(nr, ng, nb)
  const l = (max + min) / 2
  if (max === min) return [0, 0, l]
  const d = max - min
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  let h = 0
  if (max === nr) h = ((ng - nb) / d + (ng < nb ? 6 : 0)) / 6
  else if (max === ng) h = ((nb - nr) / d + 2) / 6
  else h = ((nr - ng) / d + 4) / 6
  return [h, s, l]
}

function hslToRgb(h: number, s: number, l: number): RGB {
  const hNorm = ((h % 1) + 1) % 1
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

function rgbStr(rgb: RGB): string {
  return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`
}

const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v))

/**
 * 由封面主色种子生成一套 M3 令牌，键集与 THEME_COLORS 完全一致，
 * 因此可直接交给 applyThemeVars 应用（深色再叠加 surface 微混）。
 * 只取种子的色相/饱和度，各角色亮度按 M3 预设梯度铺开，保证对比度。
 */
function buildDynamicVars(seed: RGB, dark: boolean): Record<string, string> {
  const [h, rawS] = rgbToHsl(seed[0], seed[1], seed[2])
  // 约束饱和度，避免灰图过淡或艳图过冲
  const s = clamp(rawS, 0.32, 0.72)
  // tertiary 采用 +60° 邻近色相，制造层次
  const th = h + 1 / 6

  if (dark) {
    return {
      '--md-primary': rgbStr(hslToRgb(h, s, 0.80)),
      '--md-on-primary': rgbStr(hslToRgb(h, Math.min(s, 0.6), 0.18)),
      '--md-primary-container': rgbStr(hslToRgb(h, Math.min(s, 0.55), 0.32)),
      '--md-on-primary-container': rgbStr(hslToRgb(h, Math.min(s, 0.5), 0.90)),
      '--md-secondary': rgbStr(hslToRgb(h, 0.18, 0.80)),
      '--md-secondary-container': rgbStr(hslToRgb(h, 0.16, 0.30)),
      '--md-tertiary': rgbStr(hslToRgb(th, Math.min(s, 0.5), 0.80)),
      '--md-tertiary-container': rgbStr(hslToRgb(th, Math.min(s, 0.45), 0.30)),
      '--md-inverse-primary': rgbStr(hslToRgb(h, s, 0.45)),
    }
  }
  return {
    '--md-primary': rgbStr(hslToRgb(h, s, 0.40)),
    '--md-on-primary': 'rgb(255, 255, 255)',
    '--md-primary-container': rgbStr(hslToRgb(h, Math.min(s, 0.5), 0.90)),
    '--md-on-primary-container': rgbStr(hslToRgb(h, s, 0.14)),
    '--md-secondary': rgbStr(hslToRgb(h, 0.22, 0.40)),
    '--md-secondary-container': rgbStr(hslToRgb(h, 0.20, 0.90)),
    '--md-tertiary': rgbStr(hslToRgb(th, Math.min(s, 0.5), 0.40)),
    '--md-tertiary-container': rgbStr(hslToRgb(th, Math.min(s, 0.45), 0.90)),
    '--md-inverse-primary': rgbStr(hslToRgb(h, s, 0.80)),
  }
}

/** 载入封面并采样为 ImageData；跨域用 data URL 已绕过，其余走 anonymous CORS */
function loadImageData(src: string): Promise<ImageData | null> {
  return new Promise((resolve) => {
    const img = new Image()
    if (!src.startsWith('data:') && !src.startsWith('blob:')) {
      img.crossOrigin = 'anonymous'
    }
    img.referrerPolicy = 'no-referrer'
    img.onload = () => {
      try {
        const canvas = document.createElement('canvas')
        canvas.width = SAMPLE_SIZE
        canvas.height = SAMPLE_SIZE
        const ctx = canvas.getContext('2d', { willReadFrequently: true })
        if (!ctx) throw new Error('Canvas 2D context is unavailable')
        ctx.drawImage(img, 0, 0, SAMPLE_SIZE, SAMPLE_SIZE)
        resolve(ctx.getImageData(0, 0, SAMPLE_SIZE, SAMPLE_SIZE))
      } catch (error) {
        log.warn('封面像素读取失败:', error)
        resolve(null)
      }
    }
    img.onerror = () => {
      log.warn('封面图片加载失败')
      resolve(null)
    }
    img.src = src
  })
}

// generation token 防竞态：快速切歌/切换深浅色时作废旧请求
let activeToken = 0
let isActive = false
// 缓存最近一次种子色，供深浅色瞬时切换时同步重算令牌，避免被预设主题色覆盖
let lastSeed: RGB | null = null

/** 动态取色是否正在覆盖全局主题色 */
export function isDynamicColorActive(): boolean {
  return isActive
}

/**
 * 用缓存种子按新的深浅色重算并应用令牌。
 * 供主题切换的视觉路径同步调用，避免闪回预设色。
 * @returns 是否成功重算
 */
export function reapplyDynamicColorForTheme(isDark: boolean): boolean {
  if (!isActive || !lastSeed) return false
  const seed = lastSeed
  const vars = buildDynamicVars(seed, isDark)
  applyThemeVars(vars, `${seed[0]}, ${seed[1]}, ${seed[2]}`, isDark)
  return true
}

/**
 * 探测系统强调色作为取色种子。
 * 1) 优先 CSS accent-color: accent 关键字（WebKitGTK 等映射 GTK 主题）；
 * 2) 失败则走 Rust 命令（Windows 读注册表 SystemAccentColor）。
 * 都不支持时返回 null，调用方回退到默认取色。
 */
export async function resolveSystemAccentSeed(): Promise<RGB | null> {
  try {
    const probe = document.createElement('input')
    probe.type = 'checkbox'
    probe.style.position = 'fixed'
    probe.style.opacity = '0'
    probe.style.pointerEvents = 'none'
    probe.style.accentColor = 'accent'
    document.body.appendChild(probe)
    const computed = getComputedStyle(probe).accentColor
    probe.remove()
    const match = computed?.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/)
    if (match) return [Number(match[1]), Number(match[2]), Number(match[3])]
  } catch {
    // 探测失败走平台命令兜底
  }
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    const color = await invoke<string | null>('get_system_accent_color')
    const match = color?.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/)
    if (match) return [Number(match[1]), Number(match[2]), Number(match[3])]
  } catch {
    // 非 Tauri 环境或命令不可用时忽略
  }
  return null
}

/**
 * 用指定种子色生成并应用全局动态主题色（system / cover 共用）。
 * @returns 是否成功应用
 */
export function applyDynamicColorFromSeed(seed: RGB, isDark: boolean): boolean {
  const vars = buildDynamicVars(seed, isDark)
  beginSmoothThemeTransition()
  applyThemeVars(vars, `${seed[0]}, ${seed[1]}, ${seed[2]}`, isDark)
  lastSeed = seed
  isActive = true
  log.info('动态取色已应用（种子模式）:', {
    seed,
    isDark,
    primary: vars['--md-primary'],
  })
  return true
}

/**
 * 从封面提取主色并应用为全局动态主题色。
 * @returns 是否成功应用
 */
export async function applyDynamicColorFromCover(rawUrl: string, isDark: boolean): Promise<boolean> {
  const token = ++activeToken
  const started = performance.now()

  const proxiedUrl = normalizeProxiedCoverUrl(rawUrl)
  const displayUrl = proxiedUrl || normalizeCoverUrlForDisplay(rawUrl)
  if (!displayUrl) {
    log.warn('无可用封面 URL，跳过动态取色')
    return false
  }

  // 优先命中同步缓存，未命中再走后端代理解析为 data URL（规避跨域污染 canvas）
  let dataSrc = proxiedUrl ? peekCoverImage(proxiedUrl) : ''
  if (proxiedUrl && !dataSrc) {
    try {
      dataSrc = await resolveCoverImage(proxiedUrl)
    } catch (error) {
      log.warn('代理封面解析失败，回退原始地址:', error)
      dataSrc = displayUrl
    }
  }
  if (!dataSrc) dataSrc = displayUrl
  if (token !== activeToken) return false

  const imageData = await loadImageData(dataSrc)
  if (!imageData) {
    log.warn('封面像素不可用，跳过动态取色')
    return false
  }
  if (token !== activeToken) return false

  const palette = extractPalette(imageData, 16, isDark)
  const seed = palette.primaryColor
  const vars = buildDynamicVars(seed, isDark)
  // 换封面驱动的取色变化同样要平滑过渡，不能整屏瞬间跳色
  beginSmoothThemeTransition()
  applyThemeVars(vars, `${seed[0]}, ${seed[1]}, ${seed[2]}`, isDark)
  lastSeed = seed
  isActive = true

  log.info('动态取色已应用:', {
    seed,
    isDark,
    primary: vars['--md-primary'],
    elapsedMs: Math.round(performance.now() - started),
  })
  return true
}

/** 关闭动态取色或无封面时，还原到预设主题色（仅视觉，不持久化） */
export function clearDynamicColor(isDark?: boolean): void {
  // 作废进行中的请求，防止异步回调覆盖还原结果
  activeToken++
  if (!isActive) return
  isActive = false
  lastSeed = null
  // 还原预设色也走平滑过渡
  beginSmoothThemeTransition()
  clearDynamicThemeColor(isDark)
  log.info('动态取色已清除，恢复预设主题')
}
