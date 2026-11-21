import type { TrackInfo } from '@/types/track'
import type { PlaybackSource, Translate } from './types'

/** 当前曲目的播放来源：只看曲目 ID 前缀 */
export function playbackSourceOfTrackId(id: string): PlaybackSource {
  if (id.startsWith('netease:')) return 'netease'
  if (id.startsWith('qq:')) return 'qq'
  if (id.startsWith('bilibili:')) return 'bilibili'
  if (id.startsWith('youtube:')) return 'youtube'
  return 'local'
}

// 来源徽章（对齐 Android PlaybackSourceBadge）
export function playbackSourceLabelFor(track: Pick<TrackInfo, 'id' | 'audioUrl'> | null | undefined, t: Translate): string {
  const id = track?.id || ''
  if (id.startsWith('netease:')) return t('player.source_netease')
  if (id.startsWith('qq:')) return t('player.source_qq')
  if (id.startsWith('bilibili:')) return t('player.source_bilibili')
  if (id.startsWith('youtube:')) return t('player.source_youtube')
  if (id.startsWith('local:') || track?.audioUrl?.startsWith('file:')) return t('player.source_local')
  return ''
}

export function playbackSourceIconFor(id: string): string {
  if (id.startsWith('netease:')) return 'netease'
  if (id.startsWith('qq:')) return 'music_note'
  if (id.startsWith('bilibili:')) return 'smart_display'
  if (id.startsWith('youtube:')) return 'play_circle'
  return 'folder'
}

export function platformLabel(source: string | null | undefined, t: Translate): string {
  switch ((source || '').toLowerCase()) {
    case 'netease': return t('player.source_netease')
    case 'qq': return t('player.source_qq')
    case 'bilibili': return t('player.source_bilibili')
    case 'youtube': return t('player.source_youtube')
    case 'lrclib': return 'LRCLIB'
    case 'local': return t('player.source_local')
    default: return source || ''
  }
}

export function isRemotePlaybackSource(source: string) {
  return source === 'netease' || source === 'qq' || source === 'bilibili' || source === 'youtube'
}

/** 主歌手名 (展示用, 多歌手取第一个) */
export function primaryArtistOf(artist: string): string {
  return artist.split(/\s*[/,、·]\s*/)[0] || ''
}

/** 日志里只记封面来源的主机名或长度，不落完整 URL */
export function coverSourceLabel(rawUrl: string): string {
  if (!rawUrl) return 'empty'
  if (rawUrl.startsWith('data:')) return `data-url(${rawUrl.length})`
  try {
    return new URL(rawUrl).hostname || 'url'
  } catch {
    return `invalid(${rawUrl.length})`
  }
}

export function formatSleepRemaining(seconds: number): string {
  if (seconds <= 0) return ''
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return m > 0 ? `${m}:${s.toString().padStart(2, '0')}` : `${s}s`
}

export function formatDurationMs(ms?: number) {
  if (!ms || ms <= 0) return '-'
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}

export function formatFileSize(bytes?: number) {
  if (!bytes || bytes <= 0) return '-'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}
