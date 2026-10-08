import type { LyricLine } from '@/stores/player'

export interface DesktopLyricsFrame {
  trackId: string
  title: string
  artist: string
  previous: string
  current: string
  next: string
  translation: string
  isPlaying: boolean
}

export interface DesktopLyricsTrack {
  id: string
  title: string
  artist: string
}

function boundedText(value: string, bytes: number): string {
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

function text(line: LyricLine | undefined): string {
  return boundedText(line?.text || line?.words.map(word => word.text).join('') || '', 4096)
}

export function buildDesktopLyricsFrame(input: {
  track: DesktopLyricsTrack | null
  lines: readonly LyricLine[]
  positionMs: number
  lyricOffsetMs: number
  isPlaying: boolean
}): DesktopLyricsFrame {
  if (!input.track) {
    return { trackId: '', title: '', artist: '', previous: '', current: '', next: '', translation: '', isPlaying: false }
  }
  const position = Number.isFinite(input.positionMs) ? input.positionMs : 0
  const offset = Number.isFinite(input.lyricOffsetMs) ? input.lyricOffsetMs : 0
  const lines = input.lines.filter(line => Number.isFinite(line.startMs) && line.startMs >= 0)
    .sort((left, right) => left.startMs - right.startMs)
  let index = -1
  for (let i = 0; i < lines.length && lines[i].startMs <= position + offset; i++) index = i
  return {
    trackId: boundedText(input.track.id, 512),
    title: boundedText(input.track.title, 1024),
    artist: boundedText(input.track.artist, 1024),
    previous: text(lines[index - 1]),
    current: text(lines[index]),
    next: text(lines[index + 1]),
    translation: boundedText(lines[index]?.translation || '', 4096),
    isPlaying: input.isPlaying,
  }
}
