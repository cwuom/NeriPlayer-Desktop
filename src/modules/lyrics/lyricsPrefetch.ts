// 下一首歌词预取：在当前曲目播放中把下一首的歌词提前写进本地缓存，
// 切歌时正在播放页与桌面歌词直接命中缓存，不再等一轮在线匹配
import type { TrackInfo } from '@/stores/player'
import type { DefaultLyricSource } from '@/stores/settings'
import { getCachedLyrics, saveCachedLyrics } from './lyricsCache'
import { fetchAutomaticLyrics, fetchPreferredSourceLyrics, preferredLyricMatchSource } from './lyricsFetch'
import { resolveStoredLyricStateFromPayload } from './lyricsFormat'
import { normalizeLyricSource } from './lyricOffset'
import { loadLyricsSingleFlight } from './lyricsRequest'
import { lyricSourceOf, rememberLyricSource } from './lyricSource'

export interface LyricsPrefetchOptions {
  playbackSource: string | null
  defaultLyricSource: DefaultLyricSource
  preferWordTimed: boolean
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

  const cached = await getCachedLyrics(track)
  const preferredSource = preferredLyricMatchSource(options.playbackSource, options.defaultLyricSource)
  if (cached?.length && (!preferredSource || normalizeLyricSource(lyricSourceOf(track)) === preferredSource)) {
    return 'cached'
  }

  if (preferredSource) {
    const preferred = await fetchPreferredSourceLyrics(track, preferredSource, options.preferWordTimed).catch(() => null)
    if (preferred) {
      rememberLyricSource(track, preferred.source)
      await saveCachedLyrics(track, preferred.lines, preferred.source)
      return 'fetched'
    }
    if (cached?.length) return 'cached'
  }

  const fetched = await loadLyricsSingleFlight(track, async () => {
    const result = await fetchAutomaticLyrics(track, options.playbackSource, options.preferWordTimed)
    if (result.lines.length > 0) {
      rememberLyricSource(track, result.source)
      // 切歌时正在播放页可能接上这次请求，写缓存不挡它出词
      void saveCachedLyrics(track, result.lines, result.source).catch(() => {})
    }
    return result
  })
  return fetched.lines.length > 0 ? 'fetched' : 'empty'
}
