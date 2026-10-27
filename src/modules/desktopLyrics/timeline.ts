// 桌面歌词的时间轴：当前行、逐字/整行进度、按锚点推算播放位置
// 歌词窗口每帧本地插值，主窗口只在换行、暂停、seek、改样式时发新锚点

export interface TimedWord {
  startMs: number
  durationMs: number
  text: string
}

export interface TimedLine {
  startMs: number
  durationMs: number
  text: string
  words: TimedWord[]
}

export interface PlaybackAnchor {
  /** 发锚点时的播放位置（未加歌词偏移） */
  positionMs: number
  /** 发锚点时的墙钟（Date.now()），两个窗口在同一台机器上，时钟一致 */
  anchorAt: number
  rate: number
  isPlaying: boolean
  /** seek 等待后端确认时冻结时钟；旧帧省略时沿用播放状态 */
  clockRunning?: boolean
}

/** 没有行时长也没有下一行时，最后一行按这么长算 */
const FALLBACK_LINE_MS = 4_000

// 沿用 Android 悬浮歌词的停顿、速度和时长边界，往返加入缓入缓出
const SCROLL_DELAY_MS = 520
const SCROLL_PIXELS_PER_SECOND = 38
const SCROLL_MIN_MS = 2_200
const SCROLL_MAX_MS = 9_000

/** 长句往返的位移，使用播放时钟，让暂停时的文字也停住 */
export function longLineScrollOffset(overflow: number, elapsedMs: number): number {
  if (!Number.isFinite(overflow) || overflow <= 2 || !Number.isFinite(elapsedMs) || elapsedMs <= SCROLL_DELAY_MS) return 0
  const duration = Math.min(SCROLL_MAX_MS, Math.max(SCROLL_MIN_MS, overflow / SCROLL_PIXELS_PER_SECOND * 1_000))
  const phase = (elapsedMs - SCROLL_DELAY_MS) % (duration * 2) / duration
  const progress = phase <= 1 ? phase : 2 - phase
  return overflow * progress * progress * (3 - 2 * progress)
}

export function anchoredPositionMs(anchor: PlaybackAnchor, now: number): number {
  if (!anchor.isPlaying || anchor.clockRunning === false) return anchor.positionMs
  const rate = Number.isFinite(anchor.rate) && anchor.rate > 0 ? anchor.rate : 1
  // 时钟回拨或锚点来自未来时不倒退
  return anchor.positionMs + Math.max(0, now - anchor.anchorAt) * rate
}

/** 开始时间不晚于 timeMs 的最后一行；第一行之前为 -1 */
export function activeLineIndex(lines: readonly Pick<TimedLine, 'startMs'>[], timeMs: number): number {
  let index = -1
  for (let i = 0; i < lines.length && lines[i].startMs <= timeMs; i++) index = i
  return index
}

/** 行的结束时间：有逐字时取最后一个字的结尾，否则行时长，再否则下一行开头 */
export function lineEndMs(lines: readonly TimedLine[], index: number): number {
  const line = lines[index]
  if (!line) return 0
  const wordEnd = line.words.reduce((end, word) => Math.max(end, word.startMs + word.durationMs), 0)
  if (wordEnd > line.startMs) return wordEnd
  if (line.durationMs > 0) return line.startMs + line.durationMs
  const next = lines[index + 1]
  if (next && next.startMs > line.startMs) return next.startMs
  return line.startMs + FALLBACK_LINE_MS
}

function fraction(timeMs: number, startMs: number, durationMs: number): number {
  if (durationMs <= 0) return timeMs >= startMs ? 1 : 0
  return Math.min(1, Math.max(0, (timeMs - startMs) / durationMs))
}

/** 每个字唱了多少（0–1） */
export function wordProgress(line: TimedLine, timeMs: number): number[] {
  return line.words.map(word => fraction(timeMs, word.startMs, word.durationMs))
}

/** 整行匀速扫过的进度（0–1） */
export function lineProgress(line: TimedLine, endMs: number, timeMs: number): number {
  return fraction(timeMs, line.startMs, Math.max(0, endMs - line.startMs))
}

export function hasWordTiming(line: TimedLine | undefined): boolean {
  return !!line && line.words.some(word => word.durationMs > 0 && word.text.trim().length > 0)
}

/**
 * 高亮推进到的像素位置
 *
 * wordOffsets 是每个字在行内的 [left, width]（与 words 一一对应），由界面测量；
 * 有逐字时按字插值，否则按整行宽度匀速扫。
 */
export function highlightWidth(
  line: TimedLine,
  endMs: number,
  timeMs: number,
  totalWidth: number,
  wordOffsets: readonly (readonly [number, number])[] | null,
  byWord: boolean,
): number {
  if (timeMs < line.startMs) return 0
  if (byWord && wordOffsets && wordOffsets.length === line.words.length && hasWordTiming(line)) {
    const progress = wordProgress(line, timeMs)
    let width = 0
    for (let i = 0; i < progress.length; i++) {
      if (progress[i] <= 0) break
      const [left, size] = wordOffsets[i]
      width = left + size * progress[i]
      if (progress[i] < 1) break
    }
    return Math.min(totalWidth, Math.max(0, width))
  }
  return totalWidth * lineProgress(line, endMs, timeMs)
}
