// 本地歌手聚合：对齐 Android LocalArtistSummary + sortLocalArtists
//
// 聚合键与 Android localArtistStableKey 一致（trim + lowercase），
// 保证两端对同一批本地文件分出来的歌手组完全相同。

import { invoke } from '@tauri-apps/api/core'
import { normalizeTrack, type TrackInfo } from '@/stores/player'

export type LocalArtistSortMode = 'name' | 'song_count' | 'recent'

export interface LocalArtistSummary {
  key: string
  name: string
  tracks: TrackInfo[]
  coverUrl?: string
  latestAddedAt: number
}

export function localArtistStableKey(name: string): string {
  return name.trim().toLowerCase()
}

/// 多歌手字段按常见分隔符拆开，避免「A/B」被当成一个独立歌手
const ARTIST_SEPARATORS = /[/、,;&]|\b(?:feat|ft|vs)\b\.?/i

export function splitArtistNames(raw: string): string[] {
  const seen = new Set<string>()
  const names = raw
    .split(ARTIST_SEPARATORS)
    .map((name) => name.trim())
    .filter(Boolean)
    .filter((name) => {
      const key = localArtistStableKey(name)
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
  return names
}

export function groupLocalArtists(
  tracks: readonly TrackInfo[],
  unknownLabel: string,
): LocalArtistSummary[] {
  const grouped = new Map<string, LocalArtistSummary>()

  for (const track of tracks) {
    const names = splitArtistNames(track.artist ?? '')
    const effective = names.length ? names : [unknownLabel]
    for (const name of effective) {
      const key = localArtistStableKey(name)
      let entry = grouped.get(key)
      if (!entry) {
        entry = { key, name, tracks: [], coverUrl: undefined, latestAddedAt: 0 }
        grouped.set(key, entry)
      }
      entry.tracks.push(track)
      if (!entry.coverUrl && track.coverUrl) entry.coverUrl = track.coverUrl
      entry.latestAddedAt = Math.max(entry.latestAddedAt, track.addedAt ?? 0)
    }
  }

  return [...grouped.values()]
}

export function sortLocalArtists(
  artists: readonly LocalArtistSummary[],
  mode: LocalArtistSortMode,
): LocalArtistSummary[] {
  const byName = (left: LocalArtistSummary, right: LocalArtistSummary) =>
    left.name.toLowerCase().localeCompare(right.name.toLowerCase())

  const sorted = [...artists]
  switch (mode) {
    case 'song_count':
      sorted.sort((left, right) => right.tracks.length - left.tracks.length || byName(left, right))
      break
    case 'recent':
      sorted.sort((left, right) => right.latestAddedAt - left.latestAddedAt || byName(left, right))
      break
    default:
      sorted.sort(byName)
  }
  return sorted
}

/// 搜索同时匹配歌手名与其名下曲目，对齐 Android matchesLocalArtistSearch
export function filterLocalArtists(
  artists: readonly LocalArtistSummary[],
  query: string,
): LocalArtistSummary[] {
  const trimmed = query.trim().toLowerCase()
  if (!trimmed) return [...artists]
  return artists.filter(
    (artist) =>
      artist.name.toLowerCase().includes(trimmed) ||
      artist.tracks.some(
        (track) =>
          track.title.toLowerCase().includes(trimmed) ||
          (track.album ?? '').toLowerCase().includes(trimmed),
      ),
  )
}


/// 歌手列表与歌手详情必须共用的曲目来源
///
/// 两边各写一份取数逻辑正是「列表里有这个歌手、点进去却是空的」的成因，
/// 所以只暴露这一个入口，调用方不要再自行拼 list_playlists + get_playlist_tracks。
/// 加载参与歌手聚合的全部曲目
///
/// Android 从所有歌单聚合歌手，只看「本地音乐」会漏掉用户手动整理到其它
/// 歌单的曲目。列表页与详情页必须共用这一个来源，否则两边口径一旦漂移，
/// 就会出现「列表里有这个歌手、点进去却是空的」。
export async function loadArtistSourceTracks(): Promise<TrackInfo[]> {
  const playlists = await invoke<Array<{ id: number }>>('list_playlists')
  const collected = await Promise.all(
    (playlists || []).map(async (playlist) => {
      try {
        const raw = await invoke<any[]>('get_playlist_tracks', { id: playlist.id })
        return (raw || []).map(normalizeTrack)
      } catch {
        return [] as TrackInfo[]
      }
    }),
  )
  const seen = new Set<string>()
  return collected
    .flat()
    .filter((track) => !!track.id && !seen.has(track.id) && seen.add(track.id))
}
