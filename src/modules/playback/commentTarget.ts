import type { TrackInfo } from '@/types/track'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'

export interface CommentTarget {
  platform: 'netease' | 'bilibili'
  target: string
}

/// 评论来源（对齐 Android resolveCommentSource）：只有网易云歌曲与 B 站视频有评论；
/// 拿不到平台 id 的曲目（YouTube、本地、未知来源）返回 null
export function resolveCommentTarget(track: TrackInfo | null | undefined): CommentTarget | null {
  if (!track) return null
  const source = getPlaybackSourceKind(track)
  const payloadId = String(track.syncPayload?.audioId ?? track.syncPayload?.audio_id ?? '')
  if (source === 'netease') {
    const id = payloadId || track.id.replace(/^netease:/i, '')
    return /^\d+$/.test(id) ? { platform: 'netease', target: id } : null
  }
  if (source === 'bilibili') {
    const haystack = `${track.id} ${payloadId}`
    const bvid = haystack.match(/BV[0-9A-Za-z]{10}/)?.[0]
    if (bvid) return { platform: 'bilibili', target: bvid }
    const avid = haystack.match(/(?:^|[^\w])(?:av)?(\d{3,})/i)?.[1]
    return avid ? { platform: 'bilibili', target: `av${avid}` } : null
  }
  return null
}
