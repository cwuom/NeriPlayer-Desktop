/** 系统歌单 ID（对齐 Android FavoritesPlaylist / LocalFilesPlaylist） */
export const SYSTEM_FAVORITES_PLAYLIST_ID = -1001
export const SYSTEM_LOCAL_FILES_PLAYLIST_ID = -1002

/** 同步数据可能带来任一语言的系统歌单名；首项是桌面端新建时使用的名称 */
export const FAVORITES_PLAYLIST_NAMES = ['我喜欢的音乐', '我喜歡的音樂', 'お気に入りの曲', 'Liked Songs', 'My Favorite Music']
const LOCAL_FILES_NAMES = new Set(['本地音乐', '本機音樂', 'ローカル音楽', 'Local Music', '本地文件', 'Local Files'])

/** 旧版只写在 localStorage 的歌单自定义顺序，迁移到数据库后删除 */
export const LEGACY_PLAYLIST_ORDER_KEY = 'neri:playlist-order'

interface PlaylistIdentity {
  id: number | string
  name: string
}

function nameIn(names: Iterable<string>, name: string): boolean {
  const wanted = name.trim().toLowerCase()
  return wanted !== '' && [...names].some(candidate => candidate.toLowerCase() === wanted)
}

/**
 * 对齐 Android FavoritesPlaylist：固定 id 一定是系统歌单，名字只对负数 id 生效，
 * 用户自建的同名歌单（正数 id）仍是普通歌单
 */
export function isFavoritesPlaylist(playlist: PlaylistIdentity): boolean {
  const id = Number(playlist.id)
  return id === SYSTEM_FAVORITES_PLAYLIST_ID || (id < 0 && nameIn(FAVORITES_PLAYLIST_NAMES, playlist.name))
}

export function isLocalFilesPlaylist(playlist: PlaylistIdentity): boolean {
  const id = Number(playlist.id)
  return id === SYSTEM_LOCAL_FILES_PLAYLIST_ID || (id < 0 && nameIn(LOCAL_FILES_NAMES, playlist.name))
}

/** 系统歌单固定首尾，不能删除、重命名或参与排序 */
export function isSystemPlaylist(playlist: PlaylistIdentity): boolean {
  return isFavoritesPlaylist(playlist) || isLocalFilesPlaylist(playlist)
}

/** 系统歌单按当前语言显示，其余保持用户起的名字 */
export function localPlaylistDisplayName(
  playlist: PlaylistIdentity,
  labels: { favorites: string; localFiles: string },
): string {
  if (isFavoritesPlaylist(playlist)) return labels.favorites
  if (isLocalFilesPlaylist(playlist)) return labels.localFiles
  return playlist.name
}

export function isEmptyLocalFilesPlaylist(playlist: { id: number; name: string; track_count: number }): boolean {
  return playlist.track_count === 0 && isLocalFilesPlaylist(playlist)
}

/** 提交给后端的自定义顺序：系统歌单固定首尾，不参与排序 */
export function playlistOrderIds<T extends { id: number | string }>(
  playlists: readonly T[],
  isProtected: (playlist: T) => boolean,
): string[] {
  return playlists.filter(playlist => !isProtected(playlist)).map(playlist => String(playlist.id))
}

/** 仍在可见列表里的选中项；搜索把某项筛掉后，它不能再被批量操作误删 */
export function visibleSelection<T extends { id: number }>(selected: ReadonlySet<number>, visible: readonly T[]): Set<number> {
  const visibleIds = new Set(visible.map(item => item.id))
  return new Set([...selected].filter(id => visibleIds.has(id)))
}

/**
 * 读取旧版本地顺序；没有旧数据返回 null，损坏的旧数据返回空数组
 * （调用方随后会删除这个键，不会每次启动重复尝试）
 */
export function readLegacyPlaylistOrder(storage: Pick<Storage, 'getItem'> | undefined): string[] | null {
  const raw = storage?.getItem(LEGACY_PLAYLIST_ORDER_KEY)
  if (raw === null || raw === undefined) return null
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.map(value => String(value)).filter(id => /^-?\d+$/.test(id))
  } catch {
    return []
  }
}
