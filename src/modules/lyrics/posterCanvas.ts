// 海报绘制：Canvas 2D 自绘（不走 DOM→图片那类库，见方案说明）。
// 模糊、渐变、底栏参数照安卓 LyricShareSheet.kt 移植：缩到 1/8 → 可分离盒式模糊 r7×3 → 放大，
// 底栏把已画好的条带再缩 1/8 糊 r6×2 后叠白 alpha 70；canvas 没有 backdrop-filter，只能这样画
import { Image as TauriImage } from '@tauri-apps/api/image'
import { writeImage } from '@tauri-apps/plugin-clipboard-manager'
import {
  ARTIST_ALPHA,
  BAR_OVERLAY_ALPHA,
  BLACK,
  BRAND_ICON_ALPHA,
  BRAND_TEXT_ALPHA,
  CARD_SCRIM_STOPS,
  NO_COVER_SEED,
  WHITE,
  extraScrimFor,
  noCoverStops,
  relativeLuminance,
  toCss,
  type Rgb,
} from './posterTheme'
import {
  planPoster,
  POSTER_LAYOUT,
  type Measure,
  type PosterPlan,
  type PosterRect,
  type PosterText,
} from './posterLayout'

const FONT_FAMILY = "'MiSans', 'Inter', sans-serif"
/** 安卓 buildBlurredCardBackground：targetSize / 8 */
const BLUR_DIVISOR = 8
const BACKGROUND_BLUR = { radius: 7, iterations: 3 } as const
const BAR_BLUR = { radius: 6, iterations: 2 } as const
/** 无封面占位块：安卓 drawCover 的两段色 */
const PLACEHOLDER_FROM: Rgb = [229, 88, 112]
const PLACEHOLDER_TO: Rgb = [90, 138, 168]
/** 取平均色的采样密度：安卓 averageColor 走 48×48 网格 */
const AVERAGE_SAMPLES = 48

export interface PosterSource {
  /** 封面地址；取不到或加载失败时降级成无封面模板 */
  coverUrl: string | null
  lines: string[]
  title: string
  artist: string
}

export interface RenderedPoster {
  canvas: HTMLCanvasElement
  plan: PosterPlan
  /** 为达标额外补的压暗层，回归与排查用 */
  extraScrim: number
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    // 只有非 data:/blob: 才需要 crossOrigin，和正在播放页取色一致：data URL 设了反而出错
    if (!src.startsWith('data:') && !src.startsWith('blob:')) image.crossOrigin = 'anonymous'
    image.referrerPolicy = 'no-referrer'
    image.onload = () => resolve(image)
    image.onerror = () => reject(new Error(`image load failed: ${src.slice(0, 64)}`))
    image.src = src
  })
}

/** readPixels 仅给真要读写像素的画布开：开了会禁掉 GPU 光栅，别贴到主画布上 */
function makeCanvas(width: number, height: number, readPixels = false): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d', readPixels ? { willReadFrequently: true } : undefined)
  if (!ctx) throw new Error('Canvas 2D context is unavailable')
  return [canvas, ctx]
}

function roundRectPath(ctx: CanvasRenderingContext2D, rect: PosterRect, width = rect.size, height = width): void {
  const radius = Math.min(rect.radius, width / 2, height / 2)
  ctx.beginPath()
  ctx.moveTo(rect.x + radius, rect.y)
  ctx.arcTo(rect.x + width, rect.y, rect.x + width, rect.y + height, radius)
  ctx.arcTo(rect.x + width, rect.y + height, rect.x, rect.y + height, radius)
  ctx.arcTo(rect.x, rect.y + height, rect.x, rect.y, radius)
  ctx.arcTo(rect.x, rect.y, rect.x + width, rect.y, radius)
  ctx.closePath()
}

/** 封面按短边裁切铺满 x,y 起始的 width×height 区域 */
function drawCoverFill(ctx: CanvasRenderingContext2D, cover: HTMLImageElement, x: number, y: number, width: number, height: number): void {
  const scale = Math.max(width / cover.width, height / cover.height)
  const drawWidth = cover.width * scale
  const drawHeight = cover.height * scale
  ctx.drawImage(cover, x + (width - drawWidth) / 2, y + (height - drawHeight) / 2, drawWidth, drawHeight)
}

/**
 * 一次可分离盒式模糊，边缘钳位（越界重复取边像素），与安卓 boxBlurPass 同算法。
 * 直接在小尺寸像素上算：135×180 约 2.4 万像素，r7 三趟也就几百万次加法
 */
function boxBlurOnce(pixels: Uint8ClampedArray, temp: Uint8ClampedArray, width: number, height: number, radius: number): void {
  const span = radius * 2 + 1
  for (let axis = 0; axis < 2; axis++) {
    const source = axis === 0 ? pixels : temp
    const target = axis === 0 ? temp : pixels
    const lines = axis === 0 ? height : width
    const along = axis === 0 ? width : height
    for (let line = 0; line < lines; line++) {
      for (let position = 0; position < along; position++) {
        let red = 0
        let green = 0
        let blue = 0
        for (let offset = -radius; offset <= radius; offset++) {
          const sample = Math.min(along - 1, Math.max(0, position + offset))
          const index = (axis === 0 ? line * width + sample : sample * width + line) * 4
          red += source[index]
          green += source[index + 1]
          blue += source[index + 2]
        }
        const targetIndex = (axis === 0 ? line * width + position : position * width + line) * 4
        target[targetIndex] = red / span
        target[targetIndex + 1] = green / span
        target[targetIndex + 2] = blue / span
      }
    }
  }
}

function blurCanvas(canvas: HTMLCanvasElement, radius: number, iterations: number): void {
  const ctx = canvas.getContext('2d', { willReadFrequently: true })
  if (!ctx) throw new Error('Canvas 2D context is unavailable')
  const { width, height } = canvas
  const image = ctx.getImageData(0, 0, width, height)
  const pixels = new Uint8ClampedArray(image.data)
  const temp = new Uint8ClampedArray(pixels.length)
  for (let pass = 0; pass < Math.max(1, iterations); pass++) {
    boxBlurOnce(pixels, temp, width, height, Math.max(1, radius))
  }
  image.data.set(pixels)
  ctx.putImageData(image, 0, 0)
}

/** 把 source 的 (x,y,w,h) 区域逐级缩小、糊完再放大画回同一位置 */
function blurStripInto(
  target: CanvasRenderingContext2D,
  source: HTMLCanvasElement,
  x: number,
  y: number,
  width: number,
  height: number,
  blur: { radius: number, iterations: number },
): void {
  const [strip, stripCtx] = makeCanvas(width, height)
  stripCtx.drawImage(source, x, y, width, height, 0, 0, width, height)
  const small = downscaleTo(strip, Math.max(1, Math.round(width / BLUR_DIVISOR)), Math.max(1, Math.round(height / BLUR_DIVISOR)))
  blurCanvas(small, blur.radius, blur.iterations)
  target.imageSmoothingEnabled = true
  target.imageSmoothingQuality = 'high'
  target.drawImage(small, 0, 0, small.width, small.height, x, y, width, height)
}

/**
 * 先按目标比例中心裁切，再逐级折半缩小到目标尺寸。
 * 一步从 600px 缩到 135px 用的是双线性采样，封面里的细条纹会混叠成残影；
 * 每次只折半就等价于做了面积平均，模糊后才干净。先裁比例是为了折半时不被拉伸。
 * 第一步不分配整张裁切面：原图可能是 3000×3000 的封面原图，那样仅为了拿 135px 的模糊底
 * 就要先复制一张约 36MB 的画布，所以首步直接缩到裁切尺寸的一半（不小于目标尺寸）
 */
function downscaleTo(source: HTMLImageElement | HTMLCanvasElement, targetWidth: number, targetHeight: number): HTMLCanvasElement {
  const aspect = targetWidth / targetHeight
  const sourceWidth = Math.max(1, source.width)
  const sourceHeight = Math.max(1, source.height)
  let cropWidth = sourceWidth
  let cropHeight = sourceHeight
  if (cropWidth / cropHeight > aspect) cropWidth = Math.max(1, Math.round(cropHeight * aspect))
  else cropHeight = Math.max(1, Math.round(cropWidth / aspect))

  const firstScale = Math.min(1, Math.max(0.5, Math.max(targetWidth / cropWidth, targetHeight / cropHeight)))
  const firstWidth = Math.max(targetWidth, Math.round(cropWidth * firstScale))
  const firstHeight = Math.max(targetHeight, Math.round(cropHeight * firstScale))

  let [canvas, ctx] = makeCanvas(firstWidth, firstHeight)
  ctx.imageSmoothingEnabled = true
  ctx.imageSmoothingQuality = 'high'
  ctx.drawImage(source,
    (sourceWidth - cropWidth) / 2, (sourceHeight - cropHeight) / 2, cropWidth, cropHeight,
    0, 0, firstWidth, firstHeight)

  while (canvas.width > targetWidth * 2 && canvas.height > targetHeight * 2) {
    const nextWidth = Math.max(targetWidth, Math.round(canvas.width / 2))
    const nextHeight = Math.max(targetHeight, Math.round(canvas.height / 2))
    const [next, nextCtx] = makeCanvas(nextWidth, nextHeight)
    nextCtx.imageSmoothingEnabled = true
    nextCtx.imageSmoothingQuality = 'high'
    nextCtx.drawImage(canvas, 0, 0, canvas.width, canvas.height, 0, 0, nextWidth, nextHeight)
    canvas = next
    ctx = nextCtx
  }
  if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
    const [final, finalCtx] = makeCanvas(targetWidth, targetHeight)
    finalCtx.imageSmoothingEnabled = true
    finalCtx.imageSmoothingQuality = 'high'
    finalCtx.drawImage(canvas, 0, 0, canvas.width, canvas.height, 0, 0, targetWidth, targetHeight)
    return final
  }
  return canvas
}

function blurredCoverBackground(cover: HTMLImageElement, width: number, height: number): HTMLCanvasElement {
  const smallWidth = Math.max(1, Math.round(width / BLUR_DIVISOR))
  const smallHeight = Math.max(1, Math.round(height / BLUR_DIVISOR))
  const small = downscaleTo(cover, smallWidth, smallHeight)
  blurCanvas(small, BACKGROUND_BLUR.radius, BACKGROUND_BLUR.iterations)
  const [canvas, ctx] = makeCanvas(width, height)
  ctx.imageSmoothingEnabled = true
  ctx.imageSmoothingQuality = 'high'
  ctx.drawImage(small, 0, 0, small.width, small.height, 0, 0, width, height)
  return canvas
}

/** 安卓 averageColor：按 48×48 网格采样求平均，跳过近透明像素 */
function averageColor(cover: HTMLImageElement): Rgb {
  const canvas = downscaleTo(cover, AVERAGE_SAMPLES, AVERAGE_SAMPLES)
  const ctx = canvas.getContext('2d')
  if (!ctx) return NO_COVER_SEED
  try {
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data
    let red = 0
    let green = 0
    let blue = 0
    let count = 0
    for (let index = 0; index < data.length; index += 4) {
      if (data[index + 3] <= 24) continue
      red += data[index]
      green += data[index + 1]
      blue += data[index + 2]
      count++
    }
    if (!count) return NO_COVER_SEED
    return [red / count, green / count, blue / count]
  } catch {
    // 封面被跨域策略挡住时 getImageData 会抛；退回中性色，海报照样能出
    return NO_COVER_SEED
  }
}

/**
 * 歌词区的代表底色：取亮度 90 分位，而不是最亮单点。
 * 单点会被一个高光 blob 拉爆，逼出过深的补暗，整张就比安卓暗一档
 */
function representativeBackground(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number): Rgb {
  const sampleWidth = Math.max(1, Math.min(64, Math.round(width)))
  const sampleHeight = Math.max(1, Math.min(64, Math.round(height)))
  const [canvas, sampleCtx] = makeCanvas(sampleWidth, sampleHeight, true)
  sampleCtx.drawImage(ctx.canvas, x, y, width, height, 0, 0, sampleWidth, sampleHeight)
  try {
    const data = sampleCtx.getImageData(0, 0, sampleWidth, sampleHeight).data
    const colors: Rgb[] = []
    const luminances: number[] = []
    for (let index = 0; index < data.length; index += 4) {
      const color: Rgb = [data[index], data[index + 1], data[index + 2]]
      colors.push(color)
      luminances.push(relativeLuminance(color))
    }
    if (!colors.length) return [0, 0, 0]
    const sorted = [...luminances].sort((left, right) => left - right)
    const pick = sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.9))]
    let bestIndex = 0
    let bestGap = Number.POSITIVE_INFINITY
    colors.forEach((color, index) => {
      const gap = Math.abs(relativeLuminance(color) - pick)
      if (gap < bestGap) {
        bestGap = gap
        bestIndex = index
      }
    })
    return colors[bestIndex]
  } catch {
    return [0, 0, 0]
  }
}

function diagonalGradient(ctx: CanvasRenderingContext2D, height: number) {
  return ctx.createLinearGradient(0, 0, POSTER_LAYOUT.width, height)
}

function canvasMeasureFactory(ctx: CanvasRenderingContext2D): Measure {
  return (text, fontSize, weight) => {
    ctx.font = `${weight} ${fontSize}px ${FONT_FAMILY}`
    return ctx.measureText(text).width
  }
}

/** 给弹窗单独用的量宽器：勾选时估算视觉行要用，和出图走同一套字体 */
export function createPosterMeasure(): Measure {
  const [, ctx] = makeCanvas(1, 1)
  return canvasMeasureFactory(ctx)
}

function drawText(ctx: CanvasRenderingContext2D, text: PosterText, fill: string): void {
  ctx.font = `${text.weight} ${text.fontSize}px ${FONT_FAMILY}`
  ctx.fillStyle = fill
  ctx.fillText(text.text, text.x, text.y)
}

/** 生成海报画布；无封面时背景走安卓那套三段渐变，封面位画渐变占位块 + 首字母 */
export async function renderPoster(source: PosterSource, logo: HTMLImageElement | null = null): Promise<RenderedPoster> {
  const [canvas, ctx] = makeCanvas(POSTER_LAYOUT.width, POSTER_LAYOUT.width)
  const cover = source.coverUrl ? await loadImage(source.coverUrl).catch(() => null) : null
  // 字体没就绪时 measureText 用的是后备字体宽度，画出来会跑位
  await document.fonts.ready

  const measure = canvasMeasureFactory(ctx)
  const plan = planPoster(
    { lines: source.lines, title: source.title, artist: source.artist, hasCover: Boolean(cover) },
    measure,
  )
  canvas.width = plan.width
  canvas.height = plan.height
  ctx.imageSmoothingEnabled = true
  ctx.imageSmoothingQuality = 'high'
  const barTop = plan.bar.rect.y

  if (cover) {
    ctx.drawImage(blurredCoverBackground(cover, plan.width, plan.height), 0, 0)
  } else {
    const paint = diagonalGradient(ctx, plan.height)
    for (const stop of noCoverStops(NO_COVER_SEED)) paint.addColorStop(stop.at, toCss(stop.color))
    ctx.fillStyle = paint
    ctx.fillRect(0, 0, plan.width, plan.height)
  }

  // 对角蒙层：安卓 argb(86,白)@0 → argb(64,黑)@0.48 → argb(118,黑)@1
  const scrim = diagonalGradient(ctx, plan.height)
  for (const stop of CARD_SCRIM_STOPS) scrim.addColorStop(stop.at, toCss(stop.color, stop.alpha))
  ctx.fillStyle = scrim
  ctx.fillRect(0, 0, plan.width, plan.height)

  // 亮封面可能让歌词区压不住白字：取代表底色，不够就整体补一层黑（上限见 EXTRA_SCRIM_RANGE）
  const background = representativeBackground(ctx, POSTER_LAYOUT.padding, POSTER_LAYOUT.lyricsTop,
    plan.width - POSTER_LAYOUT.padding * 2, barTop - POSTER_LAYOUT.lyricsTop)
  const extraScrim = extraScrimFor(background, 7)
  if (extraScrim > 0) {
    ctx.fillStyle = toCss(BLACK, extraScrim)
    ctx.fillRect(0, 0, plan.width, plan.height)
  }

  // 底栏玻璃：条带再糊一层，然后叠白 argb(70,255,255,255)
  blurStripInto(ctx, canvas, 0, barTop, plan.width, POSTER_LAYOUT.barHeight, BAR_BLUR)
  ctx.fillStyle = toCss(WHITE, BAR_OVERLAY_ALPHA)
  ctx.fillRect(0, barTop, plan.width, POSTER_LAYOUT.barHeight)

  const slot = plan.bar.cover
  if (cover) {
    ctx.save()
    roundRectPath(ctx, slot)
    ctx.clip()
    drawCoverFill(ctx, cover, slot.x, slot.y, slot.size, slot.size)
    ctx.restore()
  } else {
    const paint = ctx.createLinearGradient(slot.x, slot.y, slot.x + slot.size, slot.y + slot.size)
    paint.addColorStop(0, toCss(PLACEHOLDER_FROM))
    paint.addColorStop(1, toCss(PLACEHOLDER_TO))
    ctx.save()
    roundRectPath(ctx, slot)
    ctx.fillStyle = paint
    ctx.fill()
    ctx.clip()
    if (plan.bar.placeholderLetter) {
      ctx.font = `${POSTER_LAYOUT.placeholderWeight} ${POSTER_LAYOUT.placeholderFontSize}px ${FONT_FAMILY}`
      ctx.fillStyle = toCss(WHITE)
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillText(plan.bar.placeholderLetter, slot.x + slot.size / 2, slot.y + slot.size / 2)
      ctx.textAlign = 'left'
      ctx.textBaseline = 'alphabetic'
    }
    ctx.restore()
  }

  ctx.textAlign = 'left'
  ctx.textBaseline = 'alphabetic'
  for (const line of plan.lyrics) drawText(ctx, line, toCss(WHITE))
  drawText(ctx, plan.bar.title, toCss(WHITE))
  drawText(ctx, plan.bar.artist, toCss(WHITE, ARTIST_ALPHA))

  const icon = plan.brand.icon
  if (logo) {
    ctx.save()
    ctx.globalAlpha = BRAND_ICON_ALPHA
    roundRectPath(ctx, icon)
    ctx.clip()
    ctx.drawImage(logo, icon.x, icon.y, icon.size, icon.size)
    ctx.restore()
  } else {
    // 与安卓 drawLyricCardBrand 的无图标分支一致：画三点占位，不让品牌位只剩文字
    ctx.save()
    ctx.globalAlpha = BRAND_TEXT_ALPHA
    ctx.fillStyle = toCss(WHITE)
    const marks: [number, number, number][] = [
      [icon.x + 12, icon.y + 24, 16],
      [icon.x + 38, icon.y + 8, 9],
      [icon.x + 38, icon.y + 40, 9],
    ]
    for (const [cx, cy, radius] of marks) {
      ctx.beginPath()
      ctx.arc(cx, cy, radius, 0, Math.PI * 2)
      ctx.fill()
    }
    ctx.restore()
  }
  drawText(ctx, plan.brand.text, toCss(WHITE, BRAND_TEXT_ALPHA))

  return { canvas, plan, extraScrim }
}

/** 品牌图标：与标题栏同一张 /app-icon.png */
export async function loadPosterLogo(): Promise<HTMLImageElement | null> {
  try {
    return await loadImage('/app-icon.png')
  } catch {
    return null
  }
}

export function posterToBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error('poster encode failed'))), 'image/png')
  })
}

/**
 * 写剪贴板。发 PNG 字节、由 Rust 侧解码成位图，而不是发原始 RGBA：
 * RGBA 那一千多万字节会被 IPC 当 JSON 摊成数字数组（1080×2400 实测序列化 2 秒、产出 3600 万字符），
 * PNG 同一张只有约 0.9MB。fromBytes 依赖 Cargo 里 tauri 的 image-png feature，缺了会在运行时报错
 */
export async function writePosterToClipboard(canvas: HTMLCanvasElement): Promise<void> {
  const blob = await posterToBlob(canvas)
  const image = await TauriImage.fromBytes(new Uint8Array(await blob.arrayBuffer()))
  try {
    await writeImage(image)
  } finally {
    await image.close()
  }
}
