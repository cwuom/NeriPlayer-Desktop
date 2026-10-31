// 歌词海报排版：模糊封面铺底 + 左上角品牌 + 中间歌词 + 底部通栏玻璃信息栏。
// 几何数值对齐安卓端 LyricShareSheet.kt（padding 88 / lyricsTop 246 / bottomBar 214 / 封面 132 圆角 28）。
// 纯计算、量宽函数由外部注入（浏览器用 ctx.measureText，回归脚本用假量宽），不带运行时依赖
export type Measure = (text: string, fontSize: number, weight: number) => number

/**
 * 字号阶梯。上限受"每行能放几个字"约束（904px 内容宽下 112px 只排 8 个汉字），
 * 所以下面优先挑"整句不折行"的字号；下限 44 与安卓 MinimumTextSizePx 一致
 */
const LYRIC_FONT_STEPS = [112, 96, 88, 80, 72, 66, 60, 54, 48, 44] as const
/** 一句歌词最多折成 3 个视觉行：再碎就不是歌词卡而是诗断了 */
const MAX_SUB_LINES_PER_LINE = 3
/**
 * 版面预算：海报最多容纳 24 个视觉行。
 * 限的是折行之后的行数而不是源行数——12 句 26 字的长句和 24 句 10 字的短句占用一样，
 * 按源行数限会放行前者、拦掉后者。24 行对应字号不低于 54、宽高比不超过 1:2.1
 */
export const MAX_POSTER_VISUAL_LINES = 24
const LYRIC_WEIGHT = 700
/** 歌词行距 = 字号 × 1.35，等价于安卓"字号 + 16（最小字号时 +12）"的行间累加 */
const LYRIC_LINE_HEIGHT = 1.35
/** canvas 没有 StaticLayout 的字体度量，按 MiSans 的上行比例近似 */
export const LYRIC_ASCENT_RATIO = 0.78

export const POSTER_LAYOUT = {
  width: 1080,
  padding: 88,
  /** 下限就是安卓的 1080×1080 方形：只选两三句时不会拉成长条 */
  minHeight: 1080,
  maxHeight: 2400,
  // 品牌位：安卓 drawLyricCardBrand（图标 48@y84 圆角 12，文字 42、基线 124、距图标 18）
  brandIcon: 48,
  brandIconY: 84,
  brandIconRadius: 12,
  brandTextGap: 18,
  brandFontSize: 42,
  brandWeight: 700,
  brandBaseline: 124,
  lyricsTop: 246,
  lyricsBottomGap: 48,
  // 底栏：通栏、无圆角，安卓 bottomBarHeight 214
  barHeight: 214,
  barCover: 132,
  barCoverRadius: 28,
  barCoverGap: 28,
  titleFontSize: 46,
  titleWeight: 700,
  titleBaselineOffset: 82,
  artistFontSize: 38,
  artistWeight: 400,
  artistBaselineOffset: 136,
  placeholderFontSize: 58,
  placeholderWeight: 700,
} as const

export interface PosterSource {
  /** 勾选中的歌词行原文，按歌词顺序 */
  lines: string[]
  title: string
  artist: string
  /** 无封面时底栏封面位改画安卓那套渐变占位块 + 首字母 */
  hasCover?: boolean
}

export interface PosterRect {
  x: number
  y: number
  size: number
  radius: number
}

/** y 一律是文字基线，与安卓 canvas.drawText 同一模型 */
export interface PosterText {
  text: string
  x: number
  y: number
  fontSize: number
  weight: number
}

export interface PosterPlan {
  width: number
  height: number
  /** 左上角品牌：图标 + 软件名 */
  brand: { icon: PosterRect; text: PosterText }
  lyrics: PosterText[]
  /** 底部通栏玻璃：栏底、封面位（无封面时画占位块）、歌名、歌手 */
  bar: {
    rect: PosterRect
    cover: PosterRect
    placeholderLetter: string
    title: PosterText
    artist: PosterText
  }
  lyricFontSize: number
  /** 这批歌词需要的视觉行数（按选定字号折行、丢行之前统计） */
  requiredVisualLines: number
  /** 因装不下而被丢掉的行数，弹窗要提示 */
  dropped: number
  titleTruncated: boolean
  artistTruncated: boolean
}

/** 行首禁则标点：断行后不能出现在一行之首 */
const NO_LINE_START = new Set([
  '，', '。', '、', '！', '？', '；', '：', '）', '】', '》', '」', '』', '”', '’',
  ',', '.', '!', '?', ';', ':', ')', ']', '}', '%', '‰',
])

/** 切成不可再分的排布单元：拉丁词整体、CJK 单字、空格 */
export function tokenize(text: string): string[] {
  const tokens: string[] = []
  let latin = ''
  for (const character of text) {
    if (/[\u00AD\u2013\u2014]/.test(character)) {
      // 连字符/破折号后必须断开，避免拉丁词被拉长撑爆一行
      if (latin) { tokens.push(latin); latin = '' }
      tokens.push(character)
      continue
    }
    if (/\s/.test(character)) {
      if (latin) { tokens.push(latin); latin = '' }
      tokens.push(character)
      continue
    }
    if (/[\u02EA-\uFE4F\u3000-\u303F\uFF00-\uFFEF]/.test(character) || !/[A-Za-z0-9]/.test(character)) {
      if (latin) { tokens.push(latin); latin = '' }
      tokens.push(character)
    } else {
      latin += character
    }
  }
  if (latin) tokens.push(latin)
  return tokens
}

/** 按宽度折行，并把行首禁则标点推回上一行末尾 */
export function wrapText(
  text: string,
  maxWidth: number,
  fontSize: number,
  weight: number,
  measure: Measure,
): string[] {
  const tokens = tokenize(text)
  if (!tokens.length) return ['']
  const lines: string[] = ['']
  for (let index = 0; index < tokens.length; index++) {
    let token = tokens[index]
    if (!lines[lines.length - 1] && token.length > 1 && measure(token, fontSize, weight) > maxWidth) {
      // 空行也塞不下的整块（无空格的超长拉丁词）：按字符切成若干不超宽的段，
      // 否则整词占一行，超出画布的部分直接被裁掉
      const pieces: string[] = []
      let piece = ''
      for (const character of [...token]) {
        if (piece && measure(piece + character, fontSize, weight) > maxWidth) { pieces.push(piece); piece = character }
        else piece += character
      }
      if (piece) pieces.push(piece)
      tokens.splice(index, 1, ...pieces)
      token = pieces[0]
    }
    const candidate = lines[lines.length - 1] + token
    if (lines[lines.length - 1] && measure(candidate, fontSize, weight) > maxWidth) {
      if (/\s/.test(token)) {
        // 行尾不留空格
        continue
      }
      lines.push(token)
      const previous = lines[lines.length - 2]
      const head = [...token][0]
      if (head && NO_LINE_START.has(head) && previous && measure(previous + head, fontSize, weight) <= maxWidth) {
        // 推回后必须复测上一行：行首禁则不能把上一行顶出内容宽，否则宁可让标点留在行首
        lines[lines.length - 2] = previous + head
        lines[lines.length - 1] = [...token].slice(1).join('')
        if (!lines[lines.length - 1]) lines.pop()
      }
    } else {
      lines[lines.length - 1] = candidate
    }
  }
  return lines.map(line => line.replace(/\s+$/, ''))
}

/** 单行截断，超出补省略号 */
export function truncateToWidth(
  text: string,
  maxWidth: number,
  fontSize: number,
  weight: number,
  measure: Measure,
): { text: string; truncated: boolean } {
  if (!text || measure(text, fontSize, weight) <= maxWidth) return { text, truncated: false }
  const characters = [...text]
  for (let length = characters.length - 1; length > 0; length--) {
    const candidate = `${characters.slice(0, length).join('')}…`
    if (measure(candidate, fontSize, weight) <= maxWidth) return { text: candidate, truncated: true }
  }
  return { text: '…', truncated: true }
}

function blockHeight(subLineCount: number, fontSize: number): number {
  return subLineCount * fontSize * LYRIC_LINE_HEIGHT
}

/**
 * 版式方案：歌词是唯一主角（占中部整块区域），歌名歌手降为底栏注脚。
 * 先挑"整句不折行"的最大字号，其次"每句 ≤3 视觉行"，再退到"能装下的最小字号换最少行数"，
 * 最后才从末尾丢行
 */
export function planPoster(source: PosterSource, measure: Measure): PosterPlan {
  const layout = POSTER_LAYOUT
  const contentWidth = layout.width - layout.padding * 2
  const keptLines = source.lines.filter(line => line.trim().length > 0)
  const fixedHeight = layout.lyricsTop + layout.lyricsBottomGap + layout.barHeight

  let fontSize: number = LYRIC_FONT_STEPS[LYRIC_FONT_STEPS.length - 1]
  let activeWrapped: string[][] = []
  let dropped = 0
  let tidyFit: { size: number, wrapped: string[][] } | null = null
  let smallestFit: { size: number, wrapped: string[][] } | null = null
  for (const candidate of LYRIC_FONT_STEPS) {
    const wrappedAll = keptLines.map(line => wrapText(line, contentWidth, candidate, LYRIC_WEIGHT, measure))
    const subLines = wrappedAll.reduce((count, entry) => count + entry.length, 0)
    if (fixedHeight + blockHeight(subLines, candidate) > layout.maxHeight) continue
    smallestFit = { size: candidate, wrapped: wrappedAll }
    const worstLine = wrappedAll.reduce((worst, entry) => Math.max(worst, entry.length), 0)
    if (!tidyFit && worstLine <= MAX_SUB_LINES_PER_LINE) tidyFit = { size: candidate, wrapped: wrappedAll }
    if (worstLine === 1) {
      fontSize = candidate
      activeWrapped = wrappedAll
      break
    }
  }
  if (!activeWrapped.length) {
    const fallback = tidyFit ?? smallestFit
    if (fallback) {
      fontSize = fallback.size
      activeWrapped = fallback.wrapped
    } else {
      fontSize = LYRIC_FONT_STEPS[LYRIC_FONT_STEPS.length - 1]
      activeWrapped = keptLines.map(line => wrapText(line, contentWidth, fontSize, LYRIC_WEIGHT, measure))
    }
  }

  // 这批歌词"需要"多少个视觉行（丢行之前），与下面实际画出的行数分开：
  // 调用方要判断的是预算够不够，不是最终显示了几行
  const requiredVisualLines = activeWrapped.reduce((count, entry) => count + entry.length, 0)
  let keptSubLines = requiredVisualLines
  while (activeWrapped.length > 1 && fixedHeight + blockHeight(keptSubLines, fontSize) > layout.maxHeight) {
    keptSubLines -= activeWrapped[activeWrapped.length - 1]?.length ?? 0
    activeWrapped = activeWrapped.slice(0, -1)
    dropped++
  }
  const maxSubLines = Math.max(1, Math.floor((layout.maxHeight - fixedHeight) / (fontSize * LYRIC_LINE_HEIGHT)))
  const flatLines = activeWrapped.flat()
  const shownLines = flatLines.slice(0, maxSubLines)
  if (shownLines.length < flatLines.length) dropped = Math.max(1, dropped)

  const height = Math.min(layout.maxHeight,
    Math.max(layout.minHeight, Math.round(fixedHeight + blockHeight(shownLines.length, fontSize))))
  const barTop = height - layout.barHeight
  // 与安卓一致：歌词从 lyricsTop 顶对齐起排，空出来的地方落在歌词与底栏之间
  const lyricStartTop = layout.lyricsTop

  const barTextX = layout.padding + layout.barCover + layout.barCoverGap
  const barTextWidth = layout.width - barTextX - layout.padding
  const title = truncateToWidth(source.title, barTextWidth, layout.titleFontSize, layout.titleWeight, measure)
  const artist = truncateToWidth(source.artist, barTextWidth, layout.artistFontSize, layout.artistWeight, measure)

  return {
    width: layout.width,
    height,
    brand: {
      icon: { x: layout.padding, y: layout.brandIconY, size: layout.brandIcon, radius: layout.brandIconRadius },
      text: {
        text: 'NeriPlayer',
        x: layout.padding + layout.brandIcon + layout.brandTextGap,
        y: layout.brandBaseline,
        fontSize: layout.brandFontSize,
        weight: layout.brandWeight,
      },
    },
    lyrics: shownLines.map((line, index) => ({
      text: line,
      x: layout.padding,
      y: lyricStartTop + index * fontSize * LYRIC_LINE_HEIGHT + fontSize * LYRIC_ASCENT_RATIO,
      fontSize,
      weight: LYRIC_WEIGHT,
    })),
    bar: {
      rect: { x: 0, y: barTop, size: layout.barHeight, radius: 0 },
      cover: {
        x: layout.padding,
        y: barTop + (layout.barHeight - layout.barCover) / 2,
        size: layout.barCover,
        radius: layout.barCoverRadius,
      },
      placeholderLetter: source.hasCover === false ? ([...source.title.trim()][0] ?? '') : '',
      title: {
        text: title.text,
        x: barTextX,
        y: barTop + layout.titleBaselineOffset,
        fontSize: layout.titleFontSize,
        weight: layout.titleWeight,
      },
      artist: {
        text: artist.text,
        x: barTextX,
        y: barTop + layout.artistBaselineOffset,
        fontSize: layout.artistFontSize,
        weight: layout.artistWeight,
      },
    },
    lyricFontSize: fontSize,
    /** 这批歌词需要的视觉行数（丢行前）；实际画出的行数看 lyrics.length，两者在有丢行时不同 */
    requiredVisualLines,
    dropped,
    titleTruncated: title.truncated,
    artistTruncated: artist.truncated,
  }
}

/**
 * 这批歌词会占多少个视觉行（勾选上限按它算）。
 * 取丢行之前的需求行数：报丢行之后的显示行数会让调用方以为仍有余量，
 * 于是放行一批超出预算的歌词，出图时把用户勾过的整句悄悄丢掉
 */
export function posterVisualLines(lines: string[], measure: Measure): number {
  return planPoster({ lines, title: '', artist: '' }, measure).requiredVisualLines
}
