/**
 * TrackInfo ↔ ListenTogetherTrack 双向映射
 * stableKey 生成规则与安卓端一致
 */
import type { TrackInfo } from '@/stores/player'
import { LtChannels, type ListenTogetherTrack } from './protocol'
import { setLtQueueReference } from './queue'

type PayloadRecord = Record<string, unknown>

// 可分享队列上限（对齐 Android LISTEN_TOGETHER_MAX_SHAREABLE_QUEUE_SIZE）
const MAX_SHAREABLE_QUEUE_SIZE = 2000

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
 * 出站 mediaUri 清洗：剔除会泄漏本地文件系统路径的值
 * 绝对路径（/、C:\、file://、asset://、convertFileSrc 产物）一律丢弃，
 * 只放行平台自定义 scheme（ytmusic:// 等）与 http(s)
 */
function sanitizeOutboundMediaUri(mediaUri?: string): string | undefined {
  if (!mediaUri) return undefined
  const v = mediaUri.trim()
  if (!v) return undefined
  const lower = v.toLowerCase()
  if (
    v.startsWith('/')
    || /^[a-z]:[\\/]/i.test(v)
    || lower.startsWith('file:')
    || lower.startsWith('asset:')
    || lower.startsWith('http://asset.localhost')
    || lower.startsWith('https://asset.localhost')
    || lower.includes('tauri.localhost')
  ) {
    return undefined
  }
  return v
}

// 入站 streamUrl 白名单：除 http(s) 外还要匹配曲目来源的 CDN 域名，
// 否则任意房间成员都能把第三方 URL 伪装成音频直链投递到播放器
export function isTrustedInboundStreamUrl(url?: string, channelId?: string): boolean {
  if (!url) return false
  const v = url.trim()
  if (!v) return false
  let parsed: URL
  try {
    parsed = new URL(v)
  } catch {
    return false
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return false
  const host = parsed.hostname.toLowerCase()
  if (!host) return false
  switch (channelId) {
    case LtChannels.NETEASE:
      return host === 'music.126.net' || host.endsWith('.music.126.net')
    case LtChannels.BILIBILI:
      return host === 'bilivideo.com'
        || host.endsWith('.bilivideo.com')
        || host === 'bilivideo.cn'
        || host.endsWith('.bilivideo.cn')
        || host === 'hdslb.com'
        || host.endsWith('.hdslb.com')
        || host === 'mountaintoys.cn'
        || host.endsWith('.mountaintoys.cn')
    case LtChannels.YOUTUBE_MUSIC:
      return host === 'googlevideo.com'
        || host.endsWith('.googlevideo.com')
        || host === 'youtube.com'
        || host.endsWith('.youtube.com')
        || host === 'youtube-nocookie.com'
        || host.endsWith('.youtube-nocookie.com')
    default:
      return false
  }
}

export function trustedInboundStreamUrls(
  channelId: string,
  streamUrls?: readonly string[] | null,
  legacyStreamUrl?: string | null,
): string[] {
  const limit = channelId === LtChannels.BILIBILI ? 2
    : channelId === LtChannels.YOUTUBE_MUSIC ? 1 : 3
  const candidates = [...(streamUrls || []), legacyStreamUrl]
  const trusted = candidates
    .filter((url): url is string => typeof url === 'string')
    .map(url => url.trim())
    .filter(url => isTrustedInboundStreamUrl(url, channelId))
  return [...new Set(trusted)].slice(0, limit)
}

/**
 * TrackInfo -> ListenTogetherTrack
 * 解析 track.id / syncPayload -> 填充 channelId/audioId/subAudioId/playlistContextId
 */
export function trackInfoToLtTrack(
  track: TrackInfo,
  streamUrl?: string,
  streamUrls?: readonly string[],
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
    // 本地曲目不得把绝对文件路径 / asset URL 泄漏进房间事件（暴露用户文件系统路径）;
    // 仅保留非文件系统的 mediaUri, 本地曲目对端本就无法直接播放, audioId 已足够标识
    mediaUri = sanitizeOutboundMediaUri(payloadMediaUri)
  } else {
    channelId = LtChannels.LOCAL
    audioId = payloadAudioId || track.id
    mediaUri = sanitizeOutboundMediaUri(payloadMediaUri)
  }

  if (!mediaUri && payloadMediaUri) {
    mediaUri = sanitizeOutboundMediaUri(payloadMediaUri)
  }

  const stableKey = buildStableKey(channelId, audioId, subAudioId, playlistContextId)
  const trustedStreamUrls = trustedInboundStreamUrls(channelId, streamUrls, streamUrl)

  const ltTrack: ListenTogetherTrack = {
    stableKey,
    channelId,
    audioId,
    subAudioId,
    playlistContextId,
    mediaUri,
    streamUrl: trustedStreamUrls[0],
    streamUrls: trustedStreamUrls,
    name: track.title,
    artist: track.artist,
    album: track.album || undefined,
    durationMs: track.durationMs,
    coverUrl: track.coverUrl || undefined,
  }
  if (track.playlistKey) {
    try {
      setLtQueueReference(ltTrack, JSON.parse(track.playlistKey))
    } catch {
      // 本地歌单的旧 playlistKey 并非房间引用，保留默认匹配路径
    }
  }
  return ltTrack
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

  // 入站 streamUrl 必须过白名单再用作 audioUrl，未通过则回落到本地解析（audioUrl 置空）;
  // mediaUri 仅作身份/回落，不作为可信直链
  const trustedStreamUrls = trustedInboundStreamUrls(lt.channelId, lt.streamUrls, lt.streamUrl)
  if (trustedStreamUrls.length) syncPayload.streamUrls = trustedStreamUrls

  return {
    id,
    title: lt.name,
    artist: lt.artist,
    album,
    durationMs: lt.durationMs,
    coverUrl: lt.coverUrl || '',
    // 播放优先可信 streamUrl; 不可信则留空由播放层自行按平台重新解析
    audioUrl: trustedStreamUrls[0] || '',
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

export function hasSameLtTrackSequence(
  first: readonly ListenTogetherTrack[],
  second: readonly ListenTogetherTrack[],
): boolean {
  return first.length === second.length
    && first.every((track, index) => !!track.stableKey && track.stableKey === second[index].stableKey)
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
  currentStreamUrls?: readonly string[],
): { queue: ListenTogetherTrack[]; resolvedIndex: number } {
  const result: ListenTogetherTrack[] = []
  // 当前曲被 QQ/local 过滤时没有合法共享索引，绝不能回退到首项
  let resolvedIndex = -1

  for (let i = 0; i < queue.length; i++) {
    const track = queue[i]
    const isCurrentTrack = i === currentIndex
    const ltTrack = trackInfoToLtTrack(
      track,
      isCurrentTrack && shareAudioLinks ? currentStreamUrl : undefined,
      isCurrentTrack && shareAudioLinks ? currentStreamUrls : undefined,
    )
    // QQ Music 没有 Android 对应频道，不能让 Android 把它改写成 netease
    // 后再参与 currentIndex/stableKey 仲裁；跨端房间统一排除这类曲目
    if (
      ltTrack.channelId === LtChannels.QQ_MUSIC
      || (!includeLocal && ltTrack.channelId === LtChannels.LOCAL)
    ) {
      continue
    }
    // 当前项的原始位置能区分重复曲目，并保留过滤后的队列顺序
    if (isCurrentTrack) resolvedIndex = result.length
    result.push(ltTrack)
  }

  // 队列上限：以当前曲为中心截窗（对齐 Android boundedAroundStableKey），
  // 避免超大歌单每次心跳全量序列化上传
  if (result.length > MAX_SHAREABLE_QUEUE_SIZE) {
    const half = Math.floor(MAX_SHAREABLE_QUEUE_SIZE / 2)
    const end = Math.min(result.length, Math.max(resolvedIndex - half, 0) + MAX_SHAREABLE_QUEUE_SIZE)
    const start = Math.max(0, end - MAX_SHAREABLE_QUEUE_SIZE)
    return {
      queue: result.slice(start, end),
      resolvedIndex: resolvedIndex < 0 ? -1 : resolvedIndex - start,
    }
  }

  return { queue: result, resolvedIndex }
}
