// 歌单打开记录（对齐 Android PlaylistUsageRepository.recordOpen）：首页「继续播放」按它排序，随同步合并
import { invoke } from '@tauri-apps/api/core'
import { createLogger } from '@/utils/logger'

const log = createLogger('playlist-usage')

type Id = string | number | null | undefined

export interface PlaylistOpen {
  source: 'local' | 'localArtist' | 'netease' | 'neteaseAlbum' | 'youtubeMusic' | 'bili'
  /** 本地歌单、网易歌单/专辑、B 站收藏夹/合集的 id；YouTube Music 与本地歌手不用传 */
  id?: Id
  name: string
  coverUrl?: string
  trackCount: number
  fid?: Id
  mid?: Id
  browseId?: string
  playlistId?: string
  /** B 站：COLLECTION / SERIES；收藏夹不传时由后端按归属判断 */
  subtype?: string
  subtitle?: string
}

function idText(value: Id): string | undefined {
  return value === null || value === undefined || value === '' ? undefined : String(value)
}

/** YouTube Music 的 browseId 是 VL + playlistId；Android 按 playlistId 计算 id，这里同样推导 */
export function youtubePlaylistIdFromBrowseId(browseId: string): string | undefined {
  return browseId.startsWith('VL') && browseId.length > 2 ? browseId.slice(2) : undefined
}

/** 只在内容确实加载出来后调用：歌曲数为 0 时后端会移除这条记录（对齐 Android） */
export function recordPlaylistOpen(open: PlaylistOpen): void {
  void invoke('record_playlist_open', {
    open: {
      ...open,
      id: idText(open.id),
      fid: idText(open.fid),
      mid: idText(open.mid),
      trackCount: Math.max(0, Math.round(open.trackCount || 0)),
    },
  }).catch(error => log.warn('record playlist open failed:', error))
}
