// 下一首歌词预取：在当前曲目播放中把下一首的歌词提前写进本地缓存，
// 切歌时正在播放页与桌面歌词直接命中缓存，不再等一轮在线匹配
import type { TrackInfo } from '@/stores/player'
import type { DefaultLyricSource } from '@/stores/settings'
import { readCachedLyrics, saveCachedLyrics } from './lyricsCache'
import { fetchAutomaticLyrics, fetchPreferredSourceLyrics, preferredLyricMatchSource } from './lyricsFetch'
import { resolveStoredLyricStateFromPayload } from './lyricsFormat'
import { normalizeLyricSource } from './lyricOffset'
import { loadLyricsSingleFlight } from './lyricsRequest'
import { loadBiliSubtitleLyrics, biliSubtitleRevision } from './biliSubtitleLoader'
import type { BiliSubtitlePreferences } from './biliSubtitle'

export interface LyricsPrefetchOptions {
  playbackSource: string | null
  defaultLyricSource: DefaultLyricSource
  preferWordTimed: boolean
  subtitle?: BiliSubtitlePreferences
  isCurrent?: () => boolean
}

export type LyricsPrefetchOutcome = 'skipped' | 'cached' | 'fetched' | 'empty'

/**
 * 判定顺序与正在播放页一致：同步载荷 > 本地缓存（来源符合默认歌词源时）> 默认歌词源 > 自动取词。
 * 只写缓存，不碰任何界面状态；在线请求与正在播放页共用 single-flight，切歌时正在进行的预取会被直接接上
 */
export async function prefetchTrackLyrics(
  track: TrackInfo,
  options: LyricsPrefetchOptions,
): Promise<LyricsPrefetchOutcome> {
  if (track.source === 'local' || track.id.startsWith('local:')) return 'skipped'
  if (resolveStoredLyricStateFromPayload(track.syncPayload).kind !== 'absent') return 'skipped'

  const generation = biliSubtitleRevision.value
  const isCurrent = () => generation === biliSubtitleRevision.value && (options.isCurrent?.() ?? true)
  const entry = await readCachedLyrics(track)
  if (!isCurrent()) return 'skipped'
  const cached = entry?.lines
  const preferredSource = preferredLyricMatchSource(options.playbackSource, options.defaultLyricSource)
  const trySubtitle = options.playbackSource === 'bilibili' && options.subtitle?.enabled
  if (cached?.length && ((preferredSource && normalizeLyricSource(entry?.source) === preferredSource)
    || ['local', 'localedit'].includes((entry?.source ?? '').toLowerCase().replace(/_/g, ''))
    || (!preferredSource && !trySubtitle))) {
    return 'cached'
  }

  if (preferredSource) {
    const preferred = await fetchPreferredSourceLyrics(track, preferredSource, options.preferWordTimed).catch(() => null)
    if (!isCurrent()) return 'skipped'
    if (preferred?.lines.length) {
      await saveCachedLyrics(track, preferred.lines, preferred.source)
      return 'fetched'
    }
  }

  if (trySubtitle) {
    const subtitles = await loadBiliSubtitleLyrics(track, options.subtitle!).catch(() => null)
    if (!isCurrent()) return 'skipped'
    if (subtitles?.lines.length) return 'fetched'
  }
  if (cached?.length) return 'cached'

  const fetched = await loadLyricsSingleFlight(track, () => fetchAutomaticLyrics(track, options.playbackSource, options.preferWordTimed))
  if (!isCurrent()) return 'skipped'
  if (fetched.lines.length) void saveCachedLyrics(track, fetched.lines, fetched.source).catch(() => {})
  return fetched.lines.length > 0 ? 'fetched' : 'empty'
}
