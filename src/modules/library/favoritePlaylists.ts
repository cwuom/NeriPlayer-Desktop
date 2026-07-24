export interface FavoritePlaylist {
  id: string
  name: string
  coverUrl: string
  trackCount: number
  source: string
  browseId: string
  playlistId: string
  subtitle: string
  songs: unknown[]
  addedTime: number
  modifiedAt: number
  isDeleted: boolean
}

type FavoritePlaylistDto = Record<string, unknown>

export type FavoritePlaylistLocation =
  | { name: 'netease-playlist'; params: { id: string } }
  | { name: 'bili-playlist'; params: { mediaId: string } }
  | { name: 'youtube-playlist'; params: { browseId: string } }

function numberValue(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function stringValue(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  return ''
}

export function normalizeFavoritePlaylist(dto: FavoritePlaylistDto): FavoritePlaylist {
  const songs = Array.isArray(dto.songs) ? dto.songs : []
  return {
    id: stringValue(dto.id),
    name: stringValue(dto.name),
    coverUrl: stringValue(dto.coverUrl ?? dto.cover_url),
    trackCount: numberValue(dto.trackCount ?? dto.track_count, songs.length),
    source: stringValue(dto.source),
    browseId: stringValue(dto.browseId ?? dto.browse_id),
    playlistId: stringValue(dto.playlistId ?? dto.playlist_id),
    subtitle: stringValue(dto.subtitle),
    songs,
    addedTime: numberValue(dto.addedTime ?? dto.added_time),
    modifiedAt: numberValue(dto.modifiedAt ?? dto.modified_at),
    isDeleted: Boolean(dto.isDeleted ?? dto.is_deleted),
  }
}

export function favoritePlaylistKey(favorite: FavoritePlaylist): string {
  return `${favorite.source.trim().toLowerCase()}:${favorite.id}`
}

function youtubeBrowseId(favorite: FavoritePlaylist): string {
  const browseId = favorite.browseId.trim()
  if (browseId) return browseId

  const playlistId = favorite.playlistId.trim()
  if (!playlistId) return ''
  if (/^(VL|MP|FE)/.test(playlistId)) return playlistId
  return `VL${playlistId}`
}

export function favoritePlaylistLocation(
  favorite: FavoritePlaylist,
): FavoritePlaylistLocation | null {
  const id = favorite.id.trim()
  const source = favorite.source.trim().toLowerCase()

  if (source === 'netease' && id) {
    return { name: 'netease-playlist', params: { id } }
  }
  if ((source === 'bili' || source === 'bilibili') && id) {
    return { name: 'bili-playlist', params: { mediaId: id } }
  }
  if (source === 'youtube' || source === 'youtubemusic') {
    const browseId = youtubeBrowseId(favorite)
    return browseId ? { name: 'youtube-playlist', params: { browseId } } : null
  }
  return null
}
