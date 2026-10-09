// B 站「歌单」有四种：自建/收藏的收藏夹走收藏夹接口；UP 主的合集（UGC season）和
// 视频列表（series）走空间归档接口，要带 UP 主 mid。Android 把种类和 mid 编进收藏的
// browseId（bili-playlist/v1/{KIND}/{fid}/{mid}），同步过来时原样保留。

export const BILI_PLAYLIST_KINDS = ['CREATED_FAVORITE', 'COLLECTED_FAVORITE', 'COLLECTION', 'SERIES'] as const
export type BiliPlaylistKind = typeof BILI_PLAYLIST_KINDS[number]

/** 详情页按归档分页加载的两种，路由 query 里的 kind 取值 */
export type BiliArchiveKind = 'collection' | 'series'

export interface BiliPlaylistReference {
  kind: BiliPlaylistKind
  fid: string
  mid: string
}

const PREFIX = 'bili-playlist/v1/'
const POSITIVE_ID = /^[1-9]\d*$/

function isKind(value: string): value is BiliPlaylistKind {
  return (BILI_PLAYLIST_KINDS as readonly string[]).includes(value)
}

/** 与 Android parseBiliFavoriteReference 一致：前缀、三段、已知种类、两个整数，缺一不认 */
export function parseBiliPlaylistReference(value: unknown): BiliPlaylistReference | null {
  if (typeof value !== 'string' || !value.startsWith(PREFIX)) return null
  const segments = value.slice(PREFIX.length).split('/')
  if (segments.length !== 3) return null
  const [kind, fid, mid] = segments
  if (!isKind(kind) || !/^-?\d+$/.test(fid) || !/^-?\d+$/.test(mid)) return null
  return { kind, fid, mid }
}

export function biliArchiveKind(kind: string | null | undefined): BiliArchiveKind | null {
  if (kind === 'COLLECTION') return 'collection'
  if (kind === 'SERIES') return 'series'
  return null
}

interface BiliPlaylistLocation {
  id: string
  kind?: string | null
  mid?: string | null
  name?: string
  coverUrl?: string
  trackCount?: number
  /** UP 主名，合集和视频列表的归档条目里没有作者 */
  uploader?: string
}

/// 打开 B 站歌单的路由；合集、视频列表缺 UP 主 mid 时无法加载，返回 null
export function biliPlaylistRoute(location: BiliPlaylistLocation) {
  const id = String(location.id)
  if (!POSITIVE_ID.test(id)) return null
  const archive = biliArchiveKind(location.kind)
  if (!archive) {
    if (location.kind && !isKind(location.kind)) return null
    return { name: 'bili-playlist', params: { mediaId: id } }
  }
  const mid = String(location.mid ?? '')
  if (!POSITIVE_ID.test(mid) || !Number.isSafeInteger(Number(mid))) return null
  return {
    name: 'bili-playlist',
    params: { mediaId: id },
    query: {
      kind: archive,
      mid,
      name: location.name ?? '',
      cover: location.coverUrl ?? '',
      count: String(location.trackCount || 0),
      ...(location.uploader ? { uploader: location.uploader } : {}),
    },
  }
}
