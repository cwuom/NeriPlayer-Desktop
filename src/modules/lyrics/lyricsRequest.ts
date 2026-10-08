import type { LyricLine, TrackInfo } from '@/stores/player'

const inFlightLyrics = new Map<string, Promise<unknown>>()

export function lyricsIdentity(track: TrackInfo): string {
  if (track.id) return track.id
  return `${track.title.trim()}::${track.artist.trim()}::${track.durationMs || 0}`
}

/// 同一首歌同时只发一个请求；正在播放页与桌面歌词共用，因此同一 mode 的 loader 必须返回同一种结果
export function loadLyricsSingleFlight<T>(
  track: TrackInfo,
  loader: () => Promise<T>,
  mode: 'baseline' | 'word-timed' = 'baseline',
): Promise<T> {
  const key = mode === 'baseline'
    ? lyricsIdentity(track)
    : JSON.stringify([mode, lyricsIdentity(track), track.title, track.artist, track.durationMs])
  const current = inFlightLyrics.get(key)
  if (current) return current as Promise<T>

  let request: Promise<T>
  try {
    request = Promise.resolve(loader())
  } catch (error) {
    request = Promise.reject(error)
  }
  inFlightLyrics.set(key, request)
  const release = () => {
    if (inFlightLyrics.get(key) === request) {
      inFlightLyrics.delete(key)
    }
  }
  request.then(release, release)
  return request
}

export function hasWordTimedLyrics(lines: readonly LyricLine[]): boolean {
  return lines.some(line => line.words.some(word => word.text.trim() && word.durationMs > 0))
}

export function hasLyricsRequestInFlight(track: TrackInfo): boolean {
  return inFlightLyrics.has(lyricsIdentity(track))
}
