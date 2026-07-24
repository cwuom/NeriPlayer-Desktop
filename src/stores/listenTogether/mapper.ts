/**
 * TrackInfo ↔ ListenTogetherTrack 双向映射
 * stableKey 生成规则与安卓端一致
 */
import type { TrackInfo } from '@/stores/player'
import { LtChannels, type ListenTogetherTrack } from './protocol'

type PayloadRecord = Record<string, unknown>

function readPayloadString(
  payload: PayloadRecord | null | undefined,
  ...keys: string[]
): string | undefined {
  if (!payload) return undefined
  for (const key of keys) {
    const value = payload[key]
    if (typeof value === 'string' && value.trim()) return value.trim()
    if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  }
  return undefined
}

function extractYouTubeVideoId(mediaUri?: string | null): string | undefined {
  if (!mediaUri) return undefined
  const match = mediaUri.match(/^ytmusic:\/\/video\/([^?#]+)/i)
  if (!match?.[1]) return undefined
  try {
    const decoded = decodeURIComponent(match[1]).trim()
    return decoded || undefined
  } catch {
    return match[1].trim() || undefined
  }
}

function extractYouTubePlaylistContext(mediaUri?: string | null): string | undefined {
  if (!mediaUri) return undefined
  const queryIndex = mediaUri.indexOf('?')
  if (queryIndex < 0) return undefined
  const query = mediaUri.slice(queryIndex + 1).split('#')[0] || ''
  const params = new URLSearchParams(query)
  const playlistId = params.get('playlistId')?.trim()
  return playlistId || undefined
}

function buildYouTubeMediaUri(audioId: string, playlistContextId?: string): string {
  const base = `ytmusic://video/${encodeURIComponent(audioId)}`
  if (!playlistContextId) return base
  return `${base}?playlistId=${encodeURIComponent(playlistContextId)}`
}

/**
 * TrackInfo -> ListenTogetherTrack
 * 解析 track.id / syncPayload -> 填充 channelId/audioId/subAudioId/playlistContextId
 */
export function trackInfoToLtTrack(
  track: TrackInfo,
  streamUrl?: string,
): ListenTogetherTrack {
  const payload = (track.syncPayload || null) as PayloadRecord | null
  const payloadChannel = readPayloadString(payload, 'channelId', 'channel_id')
  const payloadAudioId = readPayloadString(payload, 'audioId', 'audio_id')
  const payloadSubAudioId = readPayloadString(payload, 'subAudioId', 'sub_audio_id')
  const payloadPlaylistContext = readPayloadString(
    payload,
    'playlistContextId',
    'playlist_context_id',
  )
  const payloadMediaUri = readPayloadString(payload, 'mediaUri', 'media_uri')

  let channelId: string
  let audioId: string
  let subAudioId: string | undefined
  let playlistContextId: string | undefined
  let mediaUri: string | undefined

  if (track.id.startsWith('netease:') || payloadChannel === LtChannels.NETEASE) {
    channelId = LtChannels.NETEASE
    audioId = payloadAudioId || track.id.replace(/^netease:/i, '')
  } else if (track.id.startsWith('qq:') || payloadChannel === LtChannels.QQ_MUSIC) {
    // Desktop 可映射 QQ, 但 Android 当前无 qqMusic channel, 跨端互通仍有限
    channelId = LtChannels.QQ_MUSIC
    audioId = payloadAudioId || track.id.replace(/^qq:/i, '')
  } else if (track.id.startsWith('bilibili:') || payloadChannel === LtChannels.BILIBILI) {
    channelId = LtChannels.BILIBILI
    audioId = payloadAudioId || track.id.replace(/^bilibili:/i, '')
    // 优先 syncPayload.subAudioId, 否则从 album 字段提取 cid
    subAudioId = payloadSubAudioId
    if (!subAudioId) {
      const cidMatch = track.album?.match(/^Bilibili\|(.+)$/i)
      if (cidMatch?.[1]) subAudioId = cidMatch[1]
    }
  } else if (
    track.id.startsWith('youtube:')
    || payloadChannel === LtChannels.YOUTUBE_MUSIC
    || payloadChannel === 'youtube_music'
    || payloadChannel === 'youtube'
    || !!extractYouTubeVideoId(payloadMediaUri)
  ) {
    channelId = LtChannels.YOUTUBE_MUSIC
    audioId =
      payloadAudioId
      || extractYouTubeVideoId(payloadMediaUri)
      || track.id.replace(/^youtube:/i, '')
    playlistContextId =
      payloadPlaylistContext
      || extractYouTubePlaylistContext(payloadMediaUri)
    mediaUri = payloadMediaUri || buildYouTubeMediaUri(audioId, playlistContextId)
  } else if (track.id.startsWith('local:') || payloadChannel === LtChannels.LOCAL) {
    channelId = LtChannels.LOCAL
    audioId = payloadAudioId || track.id.replace(/^local:/i, '') || track.id
    mediaUri = payloadMediaUri || track.audioUrl || undefined
  } else {
    channelId = LtChannels.LOCAL
    audioId = payloadAudioId || track.id
    mediaUri = payloadMediaUri || track.audioUrl || undefined
  }

  if (!mediaUri && payloadMediaUri) {
    mediaUri = payloadMediaUri
  }

  const stableKey = buildStableKey(channelId, audioId, subAudioId, playlistContextId)

  return {
    stableKey,
    channelId,
    audioId,
    subAudioId,
    playlistContextId,
    mediaUri,
    streamUrl,
    name: track.title,
    artist: track.artist,
    album: track.album || undefined,
    durationMs: track.durationMs,
    coverUrl: track.coverUrl || undefined,
  }
}

/**
 * ListenTogetherTrack -> TrackInfo
 * 按 channelId 构造 Desktop 的 id 格式, 并回填 syncPayload 供播放/同步复用
 */
export function ltTrackToTrackInfo(lt: ListenTogetherTrack): TrackInfo {
  let id: string
  let album = lt.album || ''
  let source: string
  let mediaUri = lt.mediaUri || undefined
  const playlistContextId = lt.playlistContextId || undefined

  switch (lt.channelId) {
    case LtChannels.NETEASE:
      id = `netease:${lt.audioId}`
      source = 'netease'
      break
    case LtChannels.QQ_MUSIC:
      id = `qq:${lt.audioId}`
      source = 'qq'
      break
    case LtChannels.BILIBILI:
      id = `bilibili:${lt.audioId}`
      source = 'bilibili'
      // 用 subAudioId 重建 album 中的 cid 信息
      if (lt.subAudioId) {
        album = `Bilibili|${lt.subAudioId}`
      }
      break
    case LtChannels.YOUTUBE_MUSIC:
      id = `youtube:${lt.audioId}`
      source = 'youtube'
      mediaUri = mediaUri || buildYouTubeMediaUri(lt.audioId, playlistContextId)
      break
    default:
      id = lt.audioId.startsWith('local:') ? lt.audioId : `local:${lt.audioId}`
      source = 'local'
      break
  }

  const syncPayload: PayloadRecord = {
    channelId: lt.channelId,
    audioId: lt.audioId,
  }
  if (lt.subAudioId) syncPayload.subAudioId = lt.subAudioId
  if (playlistContextId) syncPayload.playlistContextId = playlistContextId
  if (mediaUri) syncPayload.mediaUri = mediaUri

  return {
    id,
    title: lt.name,
    artist: lt.artist,
    album,
    durationMs: lt.durationMs,
    coverUrl: lt.coverUrl || '',
    // 播放优先可信 streamUrl; mediaUri 仅作身份/回落
    audioUrl: lt.streamUrl || mediaUri || '',
    source,
    syncPayload,
  }
}

/**
 * stableKey 生成规则（与安卓 buildStableTrackKey 对齐）
 * - bilibili: channel:audioId[:subAudioId]
 * - youtubeMusic: channel:audioId[:playlistContextId]
 * - 其他: channel:audioId
 */
export function buildStableKey(
  channelId: string,
  audioId: string,
  subAudioId?: string,
  playlistContextId?: string,
): string {
  if (channelId === LtChannels.BILIBILI) {
    return [channelId, audioId, subAudioId]
      .filter((part): part is string => !!part && !!part.trim())
      .join(':')
  }
  if (channelId === LtChannels.YOUTUBE_MUSIC) {
    return [channelId, audioId, playlistContextId]
      .filter((part): part is string => !!part && !!part.trim())
      .join(':')
  }
  return `${channelId}:${audioId}`
}

/**
 * 将当前播放队列映射为可分享的 ListenTogetherTrack 数组
 * 默认排除 local 曲目, 对齐 Android includeLocal=false
 */
export function toShareableQueueSnapshot(
  queue: TrackInfo[],
  currentIndex: number,
  shareAudioLinks: boolean = true,
  currentStreamUrl?: string,
  includeLocal: boolean = false,
): { queue: ListenTogetherTrack[]; resolvedIndex: number } {
  const result: ListenTogetherTrack[] = []
  let resolvedIndex = 0
  for (let i = 0; i < queue.length; i++) {
    const track = queue[i]
    const isCurrentTrack = i === currentIndex
    const ltTrack = trackInfoToLtTrack(
      track,
      isCurrentTrack && shareAudioLinks ? currentStreamUrl : undefined,
    )
    if (!includeLocal && ltTrack.channelId === LtChannels.LOCAL) {
      continue
    }
    if (i === currentIndex) {
      resolvedIndex = result.length
    }
    result.push(ltTrack)
  }

  // Map the raw current index as filtered tracks are appended.
  return { queue: result, resolvedIndex }
}
