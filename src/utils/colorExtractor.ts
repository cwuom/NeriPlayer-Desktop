// 动态取色编排器
//
// 复用已有能力，避免重复实现：
//   - 取色算法复用 paletteExtractor.extractPalette（与 HyperBackground/NowPlaying 同源）
//   - 令牌统一由 themeColor.applyAccent → cleanScheme 生成（中性表面 + 强调色）
//   - 封面代理/缓存复用 bilibiliCover 的 resolve/peek
// 本模块只负责：把封面解析成像素 -> 取主色种子 -> 收敛成强调色 -> 应用/还原

import { createLogger } from '@/utils/logger'
import { extractPalette } from '@/utils/paletteExtractor'
import {
  resolveCoverImage,
  peekCoverImage,
  normalizeProxiedCoverUrl,
  normalizeCoverUrlForDisplay,
} from '@/utils/bilibiliCover'
import { applyAccent, clearDynamicThemeColor, beginSmoothThemeTransition } from '@/utils/themeColor'
import { accentFromSeed, type RGB } from '@/utils/theme/cleanScheme'

const log = createLogger('dynamic-color')

// 采样尺寸对齐 Android CoverArtColorCache（96x96）
const SAMPLE_SIZE = 96

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
  applyAccent(accentFromSeed(lastSeed, isDark), isDark)
  return true
}

const RGB_RE = /rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/

/**
 * 探测系统强调色作为取色种子。
 * 1) 先问后端（Windows 读注册表 SystemAccentColor，最准确）；
 * 2) 其余平台用 CSS 系统色 AccentColor（WebKit 在 macOS/GTK 上映射系统强调色）。
 *    Chromium 内核（WebView2）出于隐私只返回固定蓝色，所以排在注册表之后。
 * 都不支持时返回 null，调用方回退到默认取色。
 */
export async function resolveSystemAccentSeed(): Promise<RGB | null> {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    const match = (await invoke<string | null>('get_system_accent_color'))?.match(RGB_RE)
    if (match) return [Number(match[1]), Number(match[2]), Number(match[3])]
  } catch {
    // 非 Tauri 环境或命令不可用时走 CSS 探测
  }
  if (typeof CSS === 'undefined' || !CSS.supports('color', 'AccentColor')) return null
  const probe = document.createElement('span')
  probe.style.cssText = 'position:fixed;opacity:0;pointer-events:none;color:AccentColor'
  document.body.appendChild(probe)
  const match = getComputedStyle(probe).color.match(RGB_RE)
  probe.remove()
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null
}

/**
 * 用指定种子色生成并应用全局动态主题色（system / cover 共用）。
 * @returns 是否成功应用
 */
export function applyDynamicColorFromSeed(seed: RGB, isDark: boolean): boolean {
  const accent = accentFromSeed(seed, isDark)
  beginSmoothThemeTransition()
  applyAccent(accent, isDark)
  lastSeed = seed
  isActive = true
  log.info('动态取色已应用（种子模式）:', { seed, isDark, accent })
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
  const seed = palette.themeSeed
  const accent = accentFromSeed(seed, isDark)
  // 换封面驱动的取色变化同样要平滑过渡，不能整屏瞬间跳色
  beginSmoothThemeTransition()
  applyAccent(accent, isDark)
  lastSeed = seed
  isActive = true

  log.info('动态取色已应用:', {
    seed,
    isDark,
    accent,
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
