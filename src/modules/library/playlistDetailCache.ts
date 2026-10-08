import type { TrackInfo } from '@/stores/player'
import { getCachedValue, setCachedValue } from '@/utils/persistentCache'

const PLAYLIST_DETAIL_CACHE_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000
const PLAYLIST_DETAIL_CACHE_MAX_ENTRIES = 200
const PLAYLIST_DETAIL_CACHE_MAX_BYTES = 128 * 1024 * 1024

export interface PlaylistDetailCacheValue {
  playlistName?: string
  folderName?: string
  coverUrl?: string
  trackCount?: number
  playCount?: number
  mediaCount?: number
  description?: string
  creator?: string
  subtitle?: string
  tracks: TrackInfo[]
}

function normalizeTrack(track: TrackInfo): TrackInfo {
  return {
    id: String(track.id || ''),
    title: String(track.title || ''),
    artist: String(track.artist || ''),
    album: String(track.album || ''),
    durationMs: Number(track.durationMs || 0),
    coverUrl: String(track.coverUrl || ''),
    audioUrl: String(track.audioUrl || ''),
    source: track.source,
    addedAt: Number(track.addedAt || 0),
    syncPayload: track.syncPayload,
    playlistKey: track.playlistKey,
  }
}

function normalizeDetail(value: PlaylistDetailCacheValue): PlaylistDetailCacheValue {
  return {
    ...value,
    tracks: Array.isArray(value.tracks) ? value.tracks.map(normalizeTrack) : [],
  }
}

export async function getCachedPlaylistDetail(key: string): Promise<PlaylistDetailCacheValue | null> {
  const cached = await getCachedValue<PlaylistDetailCacheValue>(
    'platform_detail',
    key,
    PLAYLIST_DETAIL_CACHE_MAX_AGE_MS,
  )
  if (!cached) return null

  const detail = normalizeDetail(cached)
  return detail.tracks.length > 0 ? detail : null
}

export async function saveCachedPlaylistDetail(key: string, value: PlaylistDetailCacheValue) {
  const detail = normalizeDetail(value)
  if (detail.tracks.length === 0) return

  await setCachedValue('platform_detail', key, detail, {
    maxAgeMs: PLAYLIST_DETAIL_CACHE_MAX_AGE_MS,
    maxEntries: PLAYLIST_DETAIL_CACHE_MAX_ENTRIES,
    maxBytes: PLAYLIST_DETAIL_CACHE_MAX_BYTES,
  })
}

// 网易云私人雷达歌单因人而异：按账号区分缓存（对齐 Android NeteaseRadarCacheContext），
// 切换账号后不会先看到上一个账号的雷达
const NETEASE_RADAR_PLAYLIST_IDS = new Set(['5320167908', '5362359247', '5300458264', '5327906368', '5341776086'])

function shortHash(value: string): string {
  let hash = 0x811c9dc5
  for (let index = 0; index < value.length; index++) {
    hash ^= value.charCodeAt(index)
    hash = Math.imul(hash, 0x01000193)
  }
  return (hash >>> 0).toString(16).padStart(8, '0')
}

export function playlistDetailCacheKey(kind: string, id: string | number, account?: string | null): string {
  const key = `${kind}:${id}`
  if (kind !== 'netease-playlist' || !NETEASE_RADAR_PLAYLIST_IDS.has(String(id))) return key
  return `${key}:${account ? `account-${shortHash(account)}` : 'public'}`
}

export async function readPlaylistDetailCache<T extends PlaylistDetailCacheValue>(key: string): Promise<T | null> {
  return await getCachedPlaylistDetail(key) as T | null
}

export function writePlaylistDetailCache<T extends PlaylistDetailCacheValue>(key: string, value: T) {
  void saveCachedPlaylistDetail(key, value)
}

export interface CachedDetailPreview {
  /** 网络数据就绪时调用，之后读到的缓存不再覆盖新数据 */
  markFresh(): void
  /** 缓存是否已用于展示（等待读取完成） */
  shown(): Promise<boolean>
}

/**
 * 首屏缓存与网络刷新并行：调用方先发出网络请求，缓存读到后只在新数据
 * 到达之前用于展示，请求顺序与没有缓存时一致
 */
export function previewCachedDetail<T extends PlaylistDetailCacheValue>(
  key: string,
  show: (cached: T) => boolean | void,
): CachedDetailPreview {
  let fresh = false
  const read = readPlaylistDetailCache<T>(key)
    .then(cached => !!cached && !fresh && show(cached) !== false)
    .catch(() => false)
  return {
    markFresh() { fresh = true },
    shown: () => read,
  }
}
