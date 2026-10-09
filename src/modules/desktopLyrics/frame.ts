import type { LyricLine } from '@/stores/player'
import { normalizeDesktopLyricsStyle, type DesktopLyricsStyle } from './style'
import { activeLineIndex } from './timeline'

export interface DesktopLyricsFrameWord {
  startMs: number
  durationMs: number
  text: string
}

export interface DesktopLyricsFrameLine {
  startMs: number
  durationMs: number
  text: string
  translation: string
  roman: string
  words: DesktopLyricsFrameWord[]
}

/**
 * 主窗口发给歌词窗口的一帧：当前行附近的几行（带逐字时间轴）+ 播放时钟锚点 + 外观
 * 歌词窗口按锚点每帧本地插值，不必每 150 ms 收一次文本
 */
export interface DesktopLyricsFrame {
  trackId: string
  title: string
  artist: string
  isPlaying: boolean
  /** 发帧时的播放位置（未加歌词偏移） */
  positionMs: number
  /** 发帧时的 Date.now() */
  anchorAt: number
  rate: number
  offsetMs: number
  /** lines[0] 在整首歌词里的下标，用来区分奇偶行（双行布局交替） */
  firstIndex: number
  lines: DesktopLyricsFrameLine[]
  /** 应用主题色 #rrggbb，「跟随主题色」用；拿不到时为空串 */
  accent: string
  style: DesktopLyricsStyle
}

export interface DesktopLyricsTrack {
  id: string
  title: string
  artist: string
}

/** 每帧最多带的行：上一行 + 当前行 + 后三行 */
export const FRAME_LINES_BEFORE = 1
export const FRAME_LINES_AFTER = 3
export const FRAME_MAX_WORDS = 128
/** 与后端 DesktopLyricsFrame::validate 的上限一致 */
export const FRAME_TEXT_BYTES = 1024
export const FRAME_WORD_BYTES = 64

export function emptyDesktopLyricsFrame(style?: DesktopLyricsStyle): DesktopLyricsFrame {
  return {
    trackId: '', title: '', artist: '', isPlaying: false,
    positionMs: 0, anchorAt: 0, rate: 1, offsetMs: 0,
    firstIndex: 0, lines: [], accent: '', style: style ?? normalizeDesktopLyricsStyle(null),
  }
}

/** 按 UTF-8 字节和 JSON 转义后的长度截断，不切开代理对 */
export function boundedText(value: string, bytes: number): string {
  let result = ''
  let length = 0
  let encodedLength = 0
  for (const character of value) {
    const code = character.codePointAt(0) || 0
    const size = code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4
    const encodedSize = code < 0x20 ? 6 : character === '"' || character === '\\' ? 2 : size
    if (length + size > bytes || encodedLength + encodedSize > bytes) break
    result += character
    length += size
    encodedLength += encodedSize
  }
  return result
}

function finite(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function frameLine(line: LyricLine): DesktopLyricsFrameLine {
  const words = (line.words || []).slice(0, FRAME_MAX_WORDS).map(word => ({
    startMs: Math.max(0, finite(word.startMs)),
    durationMs: Math.max(0, finite(word.durationMs)),
    text: boundedText(word.text || '', FRAME_WORD_BYTES),
  }))
  return {
    startMs: Math.max(0, finite(line.startMs)),
    durationMs: Math.max(0, finite(line.durationMs)),
    text: boundedText(line.text || words.map(word => word.text).join(''), FRAME_TEXT_BYTES),
    translation: boundedText(line.translation || '', FRAME_TEXT_BYTES),
    roman: boundedText(line.roman || '', FRAME_TEXT_BYTES),
    words,
  }
}

/** 主窗口按当前位置算出的歌词行下标（加了偏移），用来判断要不要换一批行 */
export function desktopLyricsLineIndex(lines: readonly LyricLine[], positionMs: number, offsetMs: number): number {
  return activeLineIndex(sortedLines(lines), finite(positionMs) + finite(offsetMs))
}

function sortedLines(lines: readonly LyricLine[]): LyricLine[] {
  return lines.filter(line => Number.isFinite(line.startMs) && line.startMs >= 0)
    .sort((left, right) => left.startMs - right.startMs)
}

export function buildDesktopLyricsFrame(input: {
  track: DesktopLyricsTrack | null
  lines: readonly LyricLine[]
  positionMs: number
  lyricOffsetMs: number
  isPlaying: boolean
  rate: number
  now: number
  accent: string | null
  style: DesktopLyricsStyle
}): DesktopLyricsFrame {
  const style = normalizeDesktopLyricsStyle(input.style)
  if (!input.track) return emptyDesktopLyricsFrame(style)
  const position = finite(input.positionMs)
  const offset = finite(input.lyricOffsetMs)
  const lines = sortedLines(input.lines)
  const index = activeLineIndex(lines, position + offset)
  const first = Math.max(0, index - FRAME_LINES_BEFORE)
  const rate = finite(input.rate, 1)
  return {
    trackId: boundedText(input.track.id, 512),
    title: boundedText(input.track.title, FRAME_TEXT_BYTES),
    artist: boundedText(input.track.artist, FRAME_TEXT_BYTES),
    isPlaying: input.isPlaying,
    positionMs: position,
    anchorAt: finite(input.now),
    rate: rate > 0 ? rate : 1,
    offsetMs: offset,
    firstIndex: first,
    lines: lines.slice(first, Math.max(index, 0) + FRAME_LINES_AFTER + 1).map(frameLine),
    accent: input.accent && /^#[0-9a-f]{6}$/i.test(input.accent) ? input.accent.toLowerCase() : '',
    style,
  }
}
