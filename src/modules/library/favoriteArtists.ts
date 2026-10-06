export const ARTIST_FAVORITE_SOURCES = ['neteaseArtist', 'biliArtist', 'youtubeMusicArtist'] as const
export type ArtistFavoriteSource = typeof ARTIST_FAVORITE_SOURCES[number]

export interface FavoritePlaylist {
  id: string
  name: string
  coverUrl: string
  trackCount: number
  source: string
  songs: unknown[]
  addedTime: number
  modifiedAt: number
  isDeleted: boolean
  sortOrder: number
  browseId: string
  playlistId: string
  subtitle: string
}

export function isArtistFavoriteSource(source: string): source is ArtistFavoriteSource {
  return ARTIST_FAVORITE_SOURCES.some(value => value === source)
}

export function favoriteKey(favorite: Pick<FavoritePlaylist, 'source' | 'id' | 'browseId'>): string {
  // YouTube 的 Long 哈希可能超出 JS 整数精度，页面身份优先使用原始 browseId
  return `${favorite.source}:${favorite.source === 'youtubeMusicArtist' && favorite.browseId ? favorite.browseId : favorite.id}`
}

export function parseFavoritePlaylists(raw: unknown): FavoritePlaylist[] {
  if (!Array.isArray(raw)) return []
  return raw.filter(value => value && typeof value === 'object').map(value => ({
    id: String(value.id ?? ''),
    name: String(value.name ?? ''),
    coverUrl: String(value.coverUrl ?? value.cover_url ?? ''),
    trackCount: Number(value.trackCount ?? value.track_count ?? value.songs?.length ?? 0) || 0,
    source: String(value.source ?? ''),
    songs: Array.isArray(value.songs) ? value.songs : [],
    addedTime: Number(value.addedTime ?? value.added_time ?? 0) || 0,
    modifiedAt: Number(value.modifiedAt ?? value.modified_at ?? 0) || 0,
    isDeleted: Boolean(value.isDeleted ?? value.is_deleted ?? false),
    sortOrder: Number(value.sortOrder ?? value.sort_order ?? value.addedTime ?? value.added_time ?? 0) || 0,
    browseId: String(value.browseId ?? value.browse_id ?? ''),
    playlistId: String(value.playlistId ?? value.playlist_id ?? ''),
    subtitle: String(value.subtitle ?? ''),
  })).filter(value => !value.isDeleted).sort((a, b) => b.sortOrder - a.sortOrder)
}

export function filterFavoriteArtists(favorites: FavoritePlaylist[], source: ArtistFavoriteSource, query = ''): FavoritePlaylist[] {
  const search = query.trim().toLocaleLowerCase()
  return favorites.filter(favorite => favorite.source === source && (!search ||
    [favorite.name, favorite.subtitle].some(value => value.toLocaleLowerCase().includes(search))))
}

export function favoriteArtistRoute(favorite: Pick<FavoritePlaylist, 'source' | 'id' | 'name'> & Partial<FavoritePlaylist>) {
  const query = {
    name: favorite.name,
    ...(favorite.coverUrl ? { cover: favorite.coverUrl } : {}),
    ...(favorite.subtitle ? { subtitle: favorite.subtitle } : {}),
  }
  switch (favorite.source) {
    case 'neteaseArtist':
      return /^[1-9]\d*$/.test(favorite.id) ? { name: 'netease-artist', params: { id: favorite.id }, query } : null
    case 'biliArtist':
      return /^[1-9]\d*$/.test(favorite.id) ? { name: 'bili-artist', params: { mid: favorite.id }, query } : null
    case 'youtubeMusicArtist':
      return favorite.browseId ? { name: 'youtube-artist', params: { browseId: favorite.browseId }, query } : null
    default:
      return null
  }
}
