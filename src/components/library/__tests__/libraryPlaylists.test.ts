import { describe, expect, it } from 'vitest'
import {
  deletablePlaylistIds,
  isProtectedPlaylist,
  movePlaylist,
  orderLibraryPlaylists,
  playlistDragKey,
  type LocalPlaylistSummary,
} from '@/components/library/libraryPlaylists'
import { SYSTEM_FAVORITES_PLAYLIST_ID, SYSTEM_LOCAL_FILES_PLAYLIST_ID } from '@/modules/library/localPlaylists'

function playlist(id: number, name = `List ${id}`, trackCount = 1): LocalPlaylistSummary {
  return { id, name, track_count: trackCount, modified_at: 0, cover_url: null }
}

const liked = playlist(SYSTEM_FAVORITES_PLAYLIST_ID, '我喜欢的音乐')
const localFiles = playlist(SYSTEM_LOCAL_FILES_PLAYLIST_ID, '本地音乐', 3)
const ids = (list: readonly LocalPlaylistSummary[] | null) => list?.map(item => item.id) ?? null

describe('isProtectedPlaylist / playlistDragKey', () => {
  it('protects only the system playlists', () => {
    expect(isProtectedPlaylist(liked)).toBe(true)
    expect(isProtectedPlaylist(localFiles)).toBe(true)
    expect(isProtectedPlaylist(playlist(7, '我喜欢的音乐'))).toBe(false)
  })

  it('uses the stringified id as the drag key', () => {
    expect(playlistDragKey(playlist(42))).toBe('42')
  })
})

describe('orderLibraryPlaylists', () => {
  it('pins liked songs first and local files last while keeping the stored order in between', () => {
    expect(ids(orderLibraryPlaylists([playlist(3), localFiles, playlist(1), liked, playlist(2)])))
      .toEqual([SYSTEM_FAVORITES_PLAYLIST_ID, 3, 1, 2, SYSTEM_LOCAL_FILES_PLAYLIST_ID])
  })

  it('hides an empty local files playlist', () => {
    expect(ids(orderLibraryPlaylists([playlist(1), { ...localFiles, track_count: 0 }]))).toEqual([1])
  })

  it('does not mutate the input', () => {
    const raw = [playlist(2), liked]
    orderLibraryPlaylists(raw)
    expect(ids(raw)).toEqual([2, SYSTEM_FAVORITES_PLAYLIST_ID])
  })
})

describe('movePlaylist', () => {
  const list = [liked, playlist(1), playlist(2), playlist(3), localFiles]

  it('moves a row before or after the drop target', () => {
    expect(ids(movePlaylist(list, '3', '1', 'before'))).toEqual([SYSTEM_FAVORITES_PLAYLIST_ID, 3, 1, 2, SYSTEM_LOCAL_FILES_PLAYLIST_ID])
    expect(ids(movePlaylist(list, '1', '3', 'after'))).toEqual([SYSTEM_FAVORITES_PLAYLIST_ID, 2, 3, 1, SYSTEM_LOCAL_FILES_PLAYLIST_ID])
    expect(ids(movePlaylist(list, '1', '3', 'before'))).toEqual([SYSTEM_FAVORITES_PLAYLIST_ID, 2, 1, 3, SYSTEM_LOCAL_FILES_PLAYLIST_ID])
  })

  it('returns null when nothing would change so callers skip persisting', () => {
    expect(movePlaylist(list, '1', '2', 'before')).toBeNull()
    expect(movePlaylist(list, '2', '1', 'after')).toBeNull()
    expect(movePlaylist(list, '2', '2', 'after')).toBeNull()
  })

  it('returns null for unknown keys and never mutates the input', () => {
    expect(movePlaylist(list, '9', '1', 'before')).toBeNull()
    expect(movePlaylist(list, '1', '9', 'before')).toBeNull()
    movePlaylist(list, '3', '1', 'before')
    expect(ids(list)).toEqual([SYSTEM_FAVORITES_PLAYLIST_ID, 1, 2, 3, SYSTEM_LOCAL_FILES_PLAYLIST_ID])
  })
})

describe('deletablePlaylistIds', () => {
  const all = [liked, playlist(1), playlist(2), playlist(3), localFiles]

  it('keeps only visible, non-system selections', () => {
    const selected = new Set([SYSTEM_FAVORITES_PLAYLIST_ID, 1, 2, 3, SYSTEM_LOCAL_FILES_PLAYLIST_ID])
    expect(deletablePlaylistIds(selected, [liked, playlist(1), playlist(3), localFiles], all)).toEqual([1, 3])
  })

  it('still deletes visible ids that are no longer in the loaded list', () => {
    expect(deletablePlaylistIds(new Set([8]), [playlist(8)], all)).toEqual([8])
  })

  it('returns nothing for an empty selection', () => {
    expect(deletablePlaylistIds(new Set(), all, all)).toEqual([])
  })
})
