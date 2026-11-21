import {
  isEmptyLocalFilesPlaylist,
  isFavoritesPlaylist,
  isLocalFilesPlaylist,
  isSystemPlaylist,
  visibleSelection,
} from '@/modules/library/localPlaylists'

/** list_playlists 返回的本地歌单摘要 */
export interface LocalPlaylistSummary {
  id: number
  name: string
  track_count: number
  modified_at: number
  cover_url: string | null
}

export type PlaylistInsertPosition = 'before' | 'after'

export function isProtectedPlaylist(pl: LocalPlaylistSummary): boolean {
  return isSystemPlaylist(pl)
}

export function playlistDragKey(pl: LocalPlaylistSummary): string {
  return String(pl.id)
}

/** 「我喜欢的音乐」置顶，「本地音乐」置底，其余沿用数据库中的自定义顺序；空的本地音乐歌单不显示 */
export function orderLibraryPlaylists(raw: readonly LocalPlaylistSummary[]): LocalPlaylistSummary[] {
  const liked: LocalPlaylistSummary[] = []
  const localFiles: LocalPlaylistSummary[] = []
  const normal: LocalPlaylistSummary[] = []
  for (const pl of raw) {
    if (isEmptyLocalFilesPlaylist(pl)) continue
    if (isFavoritesPlaylist(pl)) liked.push(pl)
    else if (isLocalFilesPlaylist(pl)) localFiles.push(pl)
    else normal.push(pl)
  }
  return [...liked, ...normal, ...localFiles]
}

/** 拖拽落点后的新顺序；任一端找不到或顺序实际没变时返回 null，调用方据此跳过保存 */
export function movePlaylist(
  list: readonly LocalPlaylistSummary[],
  fromKey: string,
  toKey: string,
  position: PlaylistInsertPosition,
): LocalPlaylistSummary[] | null {
  const arr = [...list]
  const fromIndex = arr.findIndex(p => playlistDragKey(p) === fromKey)
  let toIndex = arr.findIndex(p => playlistDragKey(p) === toKey)
  if (fromIndex < 0 || toIndex < 0 || fromIndex === toIndex) return null

  const [moved] = arr.splice(fromIndex, 1)
  if (toIndex > fromIndex) toIndex -= 1
  const insertIndex = position === 'after' ? toIndex + 1 : toIndex
  arr.splice(insertIndex, 0, moved)

  if (arr.map(playlistDragKey).join('\n') === list.map(playlistDragKey).join('\n')) return null
  return arr
}

/** 批量删除前兜底再过滤一次：系统歌单与当前不可见的歌单都绝不删除 */
export function deletablePlaylistIds(
  selected: ReadonlySet<number>,
  visible: readonly LocalPlaylistSummary[],
  all: readonly LocalPlaylistSummary[],
): number[] {
  return [...visibleSelection(selected, visible)].filter((id) => {
    const pl = all.find(p => p.id === id)
    return !pl || !isProtectedPlaylist(pl)
  })
}
