import type { LyricLine, TrackInfo } from '@/stores/player'
import { getCachedValue, removeCachedValue, setCachedValue } from '@/utils/persistentCache'
import { lyricsIdentity } from './lyricsRequest'
import { lyricSourceOf, rememberLyricSource } from './lyricSource'

/** 缓存条目连同歌词来源一起存，读回时偏移量还能按来源选默认；旧条目是纯数组，来源未知 */
interface CachedLyricsEntry {
  source: string | null
  lines: LyricLine[]
}

// 键带版本：跨平台优先 LRCLIB+时长硬门槛后，旧的错误同名歌词缓存一律失效；
// v4：在线歌词改为去掉制作信息/标题行、补网易云音译，B 站优先逐字歌词，旧缓存缺这些
const LYRICS_CACHE_VERSION = 'v4'
const LYRICS_CACHE_MAX_AGE_MS = 30 * 24 * 60 * 60 * 1000
const LYRICS_CACHE_MAX_ENTRIES = 500
const LYRICS_CACHE_MAX_BYTES = 32 * 1024 * 1024

function cacheKey(track: TrackInfo) {
  return `${LYRICS_CACHE_VERSION}:${lyricsIdentity(track)}`
}

// 本地文件每次都重新读取 .lrc / 内嵌歌词（对齐 Android），否则编辑后的歌词最长 30 天都看不到
function bypassesCache(track: TrackInfo): boolean {
  return track.source === 'local' || track.id.startsWith('local:')
}

function normalizeLyricLine(line: LyricLine): LyricLine {
  return {
    startMs: Number(line.startMs || 0),
    durationMs: Number(line.durationMs || 0),
    words: Array.isArray(line.words)
      ? line.words.map(word => ({
        startMs: Number(word.startMs || 0),
        durationMs: Number(word.durationMs || 0),
        text: String(word.text || ''),
      }))
      : [],
    text: String(line.text || ''),
    translation: line.translation || undefined,
    roman: line.roman || undefined,
  }
}

function hasVisibleLyric(lines: LyricLine[]): boolean {
  return lines.some(line =>
    line.text.trim().length > 0
    || line.words.some(word => word.text.trim().length > 0),
  )
}

/** 读缓存的歌词；命中时顺带记下这份歌词的来源 */
export async function getCachedLyrics(track: TrackInfo): Promise<LyricLine[] | null> {
  if (bypassesCache(track)) return null
  const cached = await getCachedValue<CachedLyricsEntry | LyricLine[]>('lyrics', cacheKey(track), LYRICS_CACHE_MAX_AGE_MS)
  const entry = Array.isArray(cached)
    ? { source: null, lines: cached }
    : cached && Array.isArray(cached.lines) ? cached : null
  if (!entry) return null

  const lines = entry.lines.map(normalizeLyricLine)
  if (!hasVisibleLyric(lines)) return null
  rememberLyricSource(track, typeof entry.source === 'string' ? entry.source : null)
  return lines
}

/** 写缓存；来源默认取这首歌当前记下的歌词来源，调用方应先 rememberLyricSource 再缓存 */
export async function saveCachedLyrics(
  track: TrackInfo,
  lines: LyricLine[],
  source: string | null = lyricSourceOf(track),
) {
  if (bypassesCache(track)) return
  const normalized = lines.map(normalizeLyricLine)
  if (!hasVisibleLyric(normalized)) {
    await clearCachedLyrics(track)
    return
  }

  const entry: CachedLyricsEntry = { source, lines: normalized }
  await setCachedValue('lyrics', cacheKey(track), entry, {
    maxAgeMs: LYRICS_CACHE_MAX_AGE_MS,
    maxEntries: LYRICS_CACHE_MAX_ENTRIES,
    maxBytes: LYRICS_CACHE_MAX_BYTES,
  })
}

export async function clearCachedLyrics(track: TrackInfo) {
  await removeCachedValue('lyrics', cacheKey(track))
}
