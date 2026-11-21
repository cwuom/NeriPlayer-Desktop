import type { TrackInfo } from '@/types/track'
import { canonicalizePlaybackTrack } from './playbackSource'
import { getTrackCoverUrl } from '@/utils/trackCover'

/** 后端 snake_case 与前端 camelCase 混合的曲目结构，字段均可能缺省 */
interface RawTrack {
  id?: string
  title?: string
  artist?: string
  album?: string
  durationMs?: number
  duration_ms?: number
  coverUrl?: unknown
  cover_url?: unknown
  audioUrl?: string
  audio_url?: string
  url?: string
  source?: string
  addedAt?: number
  added_at?: number
  syncPayload?: Record<string, unknown>
  sync_payload?: Record<string, unknown>
  playlistKey?: string
  playlist_key?: string
}

/**
 * 将后端返回的 snake_case TrackInfo 映射为前端 camelCase。
 * 前端手动构造的对象已经是 camelCase，此函数同时兼容两种格式
 */
export function normalizeTrack(value: unknown): TrackInfo {
  const raw = value as RawTrack
  const syncPayload = raw.syncPayload ?? raw.sync_payload
  const coverUrl = getTrackCoverUrl({
    coverUrl: raw.coverUrl,
    cover_url: raw.cover_url,
    syncPayload,
  })
  return canonicalizePlaybackTrack({
    id: raw.id ?? '',
    title: raw.title ?? '',
    artist: raw.artist ?? '',
    album: raw.album ?? '',
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
    coverUrl,
    audioUrl: raw.audioUrl ?? raw.audio_url ?? raw.url ?? '',
    source: raw.source,
    addedAt: raw.addedAt ?? raw.added_at ?? 0,
    syncPayload,
    playlistKey: raw.playlistKey ?? raw.playlist_key,
  })
}
