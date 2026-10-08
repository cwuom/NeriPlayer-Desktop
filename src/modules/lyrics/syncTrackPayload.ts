/**
 * 把当前曲目的 syncPayload / 展示字段回写到本地歌单, 供下次同步上传
 * 对齐 Android: 编辑歌词 / 偏移写入 SongItem 后触发同步
 */
import { invoke } from '@tauri-apps/api/core'
import type { TrackInfo } from '@/stores/player'
import { createLogger } from '@/utils/logger'

const log = createLogger('sync-track-payload')

function inferTrackSource(trackId: string, source?: string | null): string {
  if (source) return source
  if (trackId.startsWith('netease:')) return 'netease'
  if (trackId.startsWith('qq:')) return 'qq'
  if (trackId.startsWith('bilibili:')) return 'bilibili'
  if (trackId.startsWith('youtube:')) return 'youtube'
  return 'local'
}

export function toBackendTrack(track: TrackInfo) {
  return {
    id: track.id,
    title: track.title,
    artist: track.artist,
    album: track.album || '',
    duration_ms: track.durationMs || 0,
    cover_url: track.coverUrl || null,
    url: track.audioUrl || '',
    source: inferTrackSource(track.id, track.source),
    added_at: Math.max(0, Math.round(track.addedAt || 0)),
    sync_payload: track.syncPayload ?? null,
    playlist_key: track.playlistKey ?? null,
  }
}

/**
 * 将 track 写回所有包含该曲的本地歌单 (playlistId 可选限定)
 * 返回更新条数; 0 表示歌单中没有该曲 (仍可只更新内存中的 currentTrack)
 */
/**
 * 把匹配/编辑得到的 歌曲名/歌手/封面 写入 syncPayload 的 custom* 字段
 * 首次覆盖时保留 original*, 对齐 Android SongItem customName/customArtist/customCoverUrl
 */
export function withUpdatedCustomInfoPayload(
  payload: Record<string, unknown> | null | undefined,
  updates: { title?: string; artist?: string; coverUrl?: string; matchedSongId?: string },
  original: { title: string; artist: string; coverUrl: string },
): Record<string, unknown> {
  const base: Record<string, unknown> = { ...(payload || {}) }

  if (updates.title !== undefined && updates.title.trim()) {
    if (base.originalName == null && base.original_name == null && original.title.trim()) {
      base.originalName = original.title
    }
    base.customName = updates.title
    delete base.custom_name
  }
  if (updates.artist !== undefined && updates.artist.trim()) {
    if (base.originalArtist == null && base.original_artist == null && original.artist.trim()) {
      base.originalArtist = original.artist
    }
    base.customArtist = updates.artist
    delete base.custom_artist
  }
  if (updates.coverUrl !== undefined && updates.coverUrl.trim()) {
    if (base.originalCoverUrl == null && base.original_cover_url == null && original.coverUrl.trim()) {
      base.originalCoverUrl = original.coverUrl
    }
    base.customCoverUrl = updates.coverUrl
    delete base.custom_cover_url
  }
  if (updates.matchedSongId && updates.matchedSongId.trim()) {
    base.matchedSongId = updates.matchedSongId
    delete base.matched_song_id
  }

  // 写入后标记 CURRENT, 与 Android fromSongItem 一致
  const version = Number(base.syncMetadataVersion ?? base.sync_metadata_version ?? 0)
  if (!Number.isFinite(version) || version < 1) {
    base.syncMetadataVersion = 1
    delete base.sync_metadata_version
  }
  return base
}

export async function persistTrackSyncPayload(
  track: TrackInfo | null | undefined,
  playlistId?: number | null,
): Promise<number> {
  if (!track?.id) return 0
  try {
    const updated = await invoke<number>('update_playlist_track', {
      playlistId: playlistId ?? null,
      track: toBackendTrack(track),
    })
    return Number(updated) || 0
  } catch (e) {
    log.warn('persistTrackSyncPayload failed:', e)
    return 0
  }
}

/** 歌词编辑另记一份覆盖记录：不在任何歌单里的歌也能同步出去（对齐 Android SyncLyricOverrideStore） */
export async function recordLyricOverride(track: TrackInfo | null | undefined): Promise<void> {
  if (!track?.id) return
  try {
    await invoke('record_lyric_override', { track: toBackendTrack(track) })
  } catch (e) {
    log.warn('recordLyricOverride failed:', e)
  }
}
