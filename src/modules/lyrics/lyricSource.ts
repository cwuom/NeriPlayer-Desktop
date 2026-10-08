// 各曲目当前显示的歌词来自哪里：后端报的来源、同步载荷里的 matchedLyricSource、手动编辑等。
// 正在播放页和桌面歌词都从这里取，再由偏移模型决定用哪个来源的默认偏移。
import { reactive } from 'vue'

type SourceTrack = {
  id?: string | null
  title?: string | null
  artist?: string | null
  durationMs?: number | null
} | null | undefined

// 只记最近用到的曲目，防止长时间播放后无限增长
const MAX_TRACKED = 500
const sources = reactive(new Map<string, string>())

// 与 lyricsIdentity 同一口径；字段不全的曲目也不会出错
function keyOf(track: NonNullable<SourceTrack>): string {
  if (track.id) return track.id
  return `${(track.title ?? '').trim()}::${(track.artist ?? '').trim()}::${track.durationMs || 0}`
}

/** 记下这首歌现在显示的歌词来源；source 为空表示来源未知 */
export function rememberLyricSource(track: SourceTrack, source: string | null | undefined) {
  if (!track) return
  const key = keyOf(track)
  sources.delete(key)
  if (!source) return
  sources.set(key, source)
  if (sources.size > MAX_TRACKED) {
    const oldest = sources.keys().next().value
    if (oldest !== undefined) sources.delete(oldest)
  }
}

export function lyricSourceOf(track: SourceTrack): string | null {
  return track ? sources.get(keyOf(track)) ?? null : null
}
