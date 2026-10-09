import { biliPlaylistRoute } from './biliPlaylistReference'

export interface ContinuePlaylist {
  id: string
  key: string
  source: string
  subtype: string
  name: string
  coverUrl: string
  trackCount: number
  lastOpenedAt: number
  openCount: number
  browseId: string
  playlistId: string
  mid: string
}

type PlaylistLocation = Pick<ContinuePlaylist, 'id' | 'source' | 'name'> & Partial<ContinuePlaylist>

export function continuePlaylistRoute(playlist: PlaylistLocation) {
  const id = String(playlist.id)
  const source = playlist.source.toLowerCase()
  if (source === 'youtubemusic') {
    const playlistId = playlist.playlistId?.trim() || ''
    const browseId = playlist.browseId?.trim() || (playlistId ? (playlistId.startsWith('VL') ? playlistId : `VL${playlistId}`) : '')
    return browseId ? { name: 'youtube-playlist', params: { browseId } } : null
  }
  if (source === 'localartist') {
    return playlist.name.trim() ? { name: 'local-artist', params: { name: playlist.name } } : null
  }
  if (!/^-?\d+$/.test(id) || BigInt(id) === 0n) return null
  switch (source) {
    case 'local': return { name: 'local-playlist', params: { id } }
    case 'netease': return { name: 'netease-playlist', params: { id } }
    case 'neteasealbum': return { name: 'netease-album', params: { id } }
    case 'bili':
      return biliPlaylistRoute({
        id, kind: playlist.subtype || null, mid: playlist.mid,
        name: playlist.name, coverUrl: playlist.coverUrl, trackCount: playlist.trackCount,
      })
  }
  return null
}

export function normalizeContinuePlaylists(raw: unknown, localPlaylists: unknown = []): ContinuePlaylist[] {
  if (!Array.isArray(raw)) return []
  const localById = new Map<string, Record<string, unknown>>()
  if (Array.isArray(localPlaylists)) {
    for (const playlist of localPlaylists) {
      if (playlist && typeof playlist === 'object') localById.set(String(playlist.id), playlist)
    }
  }
  const byKey = new Map<string, ContinuePlaylist>()
  for (const value of raw) {
    if (!value || typeof value !== 'object') continue
    const id = String(value.id ?? '')
    if (!/^-?\d+$/.test(id)) continue
    const source = String(value.source ?? '')
    const subtype = String(value.subtype ?? '').trim()
    const local = source === 'local' ? localById.get(id) : undefined
    // 本地歌单以当前内容为准，删除或清空后不再显示同步快照
    if (source === 'local' && !local) continue
    const playlist: ContinuePlaylist = {
      id, source, subtype, key: `${source}:${id}${subtype ? `:${subtype}` : ''}`,
      name: String(local?.name ?? value.name ?? ''),
      coverUrl: String(local?.cover_url || value.coverUrl || ''),
      trackCount: Number(local?.track_count ?? value.trackCount ?? 0),
      lastOpenedAt: Number(value.lastOpenedAt ?? 0),
      openCount: Number(value.openCount ?? 0),
      browseId: String(value.browseId ?? ''),
      playlistId: String(value.playlistId ?? ''),
      mid: String(value.mid ?? ''),
    }
    if (!(playlist.trackCount > 0) || !continuePlaylistRoute(playlist)) continue
    const previous = byKey.get(playlist.key)
    if (!previous || compareUsage(playlist, previous) < 0) byKey.set(playlist.key, playlist)
  }
  return [...byKey.values()].sort(compareUsage)
}

function compareUsage(left: ContinuePlaylist, right: ContinuePlaylist): number {
  const order = right.lastOpenedAt - left.lastOpenedAt || right.openCount - left.openCount
  if (order) return order
  // Android Long 哈希可能超过 JS 安全整数范围
  const leftId = BigInt(left.id)
  const rightId = BigInt(right.id)
  return leftId < rightId ? -1 : leftId > rightId ? 1 : 0
}
