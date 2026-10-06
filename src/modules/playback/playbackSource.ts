import { invoke } from '@tauri-apps/api/core'
import type { TrackInfo } from '@/stores/player'
import { trustedInboundStreamUrls } from '@/stores/listenTogether/mapper'

export type PlaybackSourceKind = 'netease' | 'qq' | 'bilibili' | 'youtube'
export type PlaybackAudioSource = PlaybackSourceKind | 'local'

export interface PlaybackSourceSettings {
  neteaseQuality: string
  qqMusicQuality: string
  biliQuality: string
  youtubeQuality: string
  youtubePlaybackSource?: string
  neteaseAutoSourceSwitch?: boolean
  neteaseLocalSourceFallback?: boolean
}

export interface PlaybackQualityOption {
  key: string
  label: string
}

export interface PlaybackAudioInfo {
  source: PlaybackAudioSource
  qualityKey?: string
  qualityLabel?: string
  qualityOptions?: PlaybackQualityOption[]
  codecLabel?: string
  mimeType?: string
  bitrateKbps?: number
  sampleRateHz?: number
  bitDepth?: number
  channelCount?: number
  specLabel?: string
}

export interface ResolvedPlaybackSource {
  type: 'success'
  url: string
  candidateUrls: string[]
  candidateDetails?: PlaybackCandidateDetails[]
  streamType?: 'direct' | 'hls'
  durationMs?: number
  mimeType?: string
  expectedContentLength?: number
  expectedContentMd5?: string
  isPreview?: boolean
  audioInfo?: PlaybackAudioInfo
  cacheKeyOverride?: string
  cacheKey: string
  source: PlaybackAudioSource
  qualityKey: string

  // 兼容现有播放状态和设置页展示字段
  bitrate?: number
  codec?: string
  format?: string
}

export type PlaybackResolution =
  | ResolvedPlaybackSource
  | { type: 'waiting_for_authoritative_stream' }
  | { type: 'requires_login'; message?: string }
  | { type: 'failure'; message: string; retryable: boolean }

export interface PlaybackResolveOptions {
  forceRefresh?: boolean
  avoidDirect?: boolean
  qualityOverride?: string
  requestGeneration?: number
  allowFallback?: boolean
}

export interface PlaybackCacheWriteOptions {
  cacheKey?: string
  expectedContentLength?: number
  expectedContentMd5?: string
}

export interface PlaybackCacheReadCandidate {
  cacheKey: string
  source: PlaybackSourceKind
  qualityKey: string
}

export interface PlaybackSourceAdapter {
  kind: PlaybackSourceKind
  matches(track: TrackInfo): boolean
  qualityKey(settings: PlaybackSourceSettings): string
  resolve(
    track: TrackInfo,
    settings: PlaybackSourceSettings,
    options: PlaybackResolveOptions,
  ): Promise<ResolvedPlaybackSource | null>
}

const REMOTE_SOURCE_KINDS: PlaybackSourceKind[] = [
  'netease',
  'qq',
  'bilibili',
  'youtube',
]

const NETEASE_QUALITY_FALLBACK_ORDER = [
  'jymaster',
  'sky',
  'jyeffect',
  'hires',
  'lossless',
  'exhigh',
  'higher',
  'standard',
]

const NETEASE_QUALITY_OPTIONS = NETEASE_QUALITY_FALLBACK_ORDER.map(key => ({
  key,
  label: key,
}))

const YOUTUBE_QUALITY_OPTIONS = ['low', 'medium', 'high', 'very_high']
  .map(key => ({ key, label: key }))

const RESOLUTION_TTL_MS = 90_000
const YOUTUBE_RESOLUTION_TTL_MS = 8 * 60_000
const SIGNED_URL_EXPIRY_MARGIN_MS = 90_000
const MAX_RESOLUTION_CACHE_ENTRIES = 64

export function playbackResolutionExpiresAt(result: ResolvedPlaybackSource, cachedAt = Date.now(), ttlMs?: number): number {
  const ttl = ttlMs ?? (result.source === 'youtube' ? YOUTUBE_RESOLUTION_TTL_MS : RESOLUTION_TTL_MS)
  let expiresAt = cachedAt + Math.max(0, ttl)
  if (result.source !== 'youtube') return expiresAt
  for (const url of [result.url, ...result.candidateUrls]) {
    try {
      const parsed = new URL(url)
      const pathExpiry = parsed.pathname.match(/\/expire\/(\d+)(?:\/|$)/)?.[1]
      for (const value of [parsed.searchParams.get('expire'), pathExpiry]) {
        const seconds = Number(value)
        if (Number.isFinite(seconds) && seconds > 0 && Number.isSafeInteger(seconds * 1000)) {
          expiresAt = Math.min(expiresAt, Math.max(cachedAt, seconds * 1000 - SIGNED_URL_EXPIRY_MARGIN_MS))
        }
      }
    } catch { /* 过期判断失败时沿用有界 TTL */ }
  }
  return expiresAt
}

export interface PlaybackCandidateDetails {
  url: string
  qualityKey: string
  cacheKey: string
  audioInfo: PlaybackAudioInfo
  mimeType?: string
  bitrate?: number
  codec?: string
  format?: string
  expectedContentLength?: number
  streamType?: 'direct' | 'hls'
  durationMs?: number
}

export function getPlaybackSourceKind(track: TrackInfo): PlaybackSourceKind | null {
  const idPrefix = track.id.split(':', 1)[0]?.toLowerCase()
  if (REMOTE_SOURCE_KINDS.includes(idPrefix as PlaybackSourceKind)) {
    return idPrefix as PlaybackSourceKind
  }

  const source = track.source?.toLowerCase()
  if (source === 'youtube_music') return 'youtube'
  if (REMOTE_SOURCE_KINDS.includes(source as PlaybackSourceKind)) {
    return source as PlaybackSourceKind
  }

  const syncChannel = syncPayloadString(track, 'channelId', 'channel_id')?.toLowerCase()
  if (syncChannel === 'youtube_music' || syncChannel === 'youtubemusic') {
    return 'youtube'
  }
  if (REMOTE_SOURCE_KINDS.includes(syncChannel as PlaybackSourceKind)) {
    return syncChannel as PlaybackSourceKind
  }

  const mediaUri = syncPayloadString(track, 'mediaUri', 'media_uri')
  if (mediaUri?.toLowerCase().startsWith('ytmusic://')) return 'youtube'
  if (!track.audioUrl?.trim() && track.album?.startsWith('Bilibili')) return 'bilibili'
  return null
}

export function getPlaybackSourceAdapter(track: TrackInfo): PlaybackSourceAdapter | null {
  return PLAYBACK_SOURCE_ADAPTERS.find(adapter => adapter.matches(track)) ?? null
}

export function isRemotePlaybackTrack(track: TrackInfo): boolean {
  return getPlaybackSourceAdapter(track) !== null
}

export function canonicalizePlaybackTrack(track: TrackInfo): TrackInfo {
  const kind = getPlaybackSourceKind(track)
  if (!kind) return track
  const sourceId = trackValue(track, kind).trim()
  if (!sourceId) return track

  const cid = kind === 'bilibili' ? bilibiliCid(track) : undefined
  return {
    ...track,
    id: `${kind}:${sourceId}`,
    source: kind,
    album: cid ? `Bilibili|${cid}` : track.album,
  }
}

export function isDirectStreamUrl(url?: string | null): boolean {
  return /^https?:\/\//i.test(url?.trim() ?? '')
}

export function buildPlaybackSpecLabel(info: Pick<PlaybackAudioInfo,
  'sampleRateHz' | 'bitDepth' | 'bitrateKbps'>): string | undefined {
  const parts: string[] = []
  if (info.sampleRateHz && info.sampleRateHz > 0) {
    const khz = info.sampleRateHz / 1000
    parts.push(info.sampleRateHz % 1000 === 0
      ? `${khz.toFixed(0)} kHz`
      : `${khz.toFixed(1)} kHz`)
  }
  if (info.bitDepth && info.bitDepth > 0) parts.push(`${info.bitDepth} bit`)
  if (info.bitrateKbps && info.bitrateKbps > 0) {
    parts.push(`${info.bitrateKbps} kbps`)
  }
  return parts.length > 0 ? parts.join(' | ') : undefined
}

export function normalizeBitrateKbps(value?: number | null): number | undefined {
  if (!value || value <= 0) return undefined
  return Math.round(value > 10_000 ? value / 1000 : value)
}

export function playbackCacheKey(
  track: TrackInfo,
  parts: Array<string | number | null | undefined>,
): string {
  const normalizedParts = parts
    .map(part => String(part ?? '').trim().toLowerCase())
    .filter(Boolean)
  return ['v1', track.id, ...normalizedParts].join('|')
}

export function playbackQualityCachePrefix(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
): string | null {
  return playbackCacheReadCandidates(track, settings)[0]?.cacheKey ?? null
}

export function playbackCacheReadCandidates(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
): PlaybackCacheReadCandidate[] {
  // 直链通常来自一起听权威流或临时签名地址，实际音质不受本机设置
  // 控制；禁止读取稳定音质键，避免把其他来源写入的缓存误当成本次直链
  if (isDirectStreamUrl(track.audioUrl)) return []
  const adapter = getPlaybackSourceAdapter(track)
  if (!adapter) return []
  const configuredQuality = adapter.qualityKey(settings).trim().toLowerCase()
  const preferred = configuredQuality || (adapter.kind === 'netease' ? 'exhigh' : 'default')
  const qualities = adapter.kind === 'netease'
    ? neteaseQualityFallbacks(preferred)
    : [preferred]
  return qualities.flatMap(qualityKey => {
    const cacheKey = stablePlaybackCacheKey(track, adapter.kind, qualityKey)
    const keys = adapter.kind === 'youtube' ? [cacheKey, `${cacheKey}-hls`] : [cacheKey]
    return keys.map(key => ({ cacheKey: key, source: adapter.kind, qualityKey }))
  })
}

export function playbackPrefetchCacheId(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
): string {
  const base = playbackQualityCachePrefix(track, settings) ?? track.id
  const source = getPlaybackSourceKind(track)
  if (source === 'youtube' && settings.youtubePlaybackSource && settings.youtubePlaybackSource !== 'automatic') {
    return `${base}|youtube-source:${settings.youtubePlaybackSource}`
  }
  if (source === 'netease' && (settings.neteaseLocalSourceFallback || settings.neteaseAutoSourceSwitch)) {
    return `${base}|fallback:${Number(!!settings.neteaseLocalSourceFallback)}:${Number(!!settings.neteaseAutoSourceSwitch)}`
  }
  return base
}

export class PlaybackUrlResolver {
  private readonly cache = new Map<string, {
    result: ResolvedPlaybackSource
    expiresAt: number
  }>()
  private readonly inFlight = new Map<string, { promise: Promise<PlaybackResolution>; requestGeneration?: number }>()

  async resolve(
    track: TrackInfo,
    settings: PlaybackSourceSettings,
    options: PlaybackResolveOptions = {},
  ): Promise<PlaybackResolution> {
    this.pruneExpiredCache()
    const adapter = getPlaybackSourceAdapter(track)
    if (!adapter) {
      return {
        type: 'failure',
        message: 'No playback source adapter',
        retryable: false,
      }
    }

    const resolvedSettings = settingsWithQualityOverride(
      settings,
      adapter.kind,
      options.qualityOverride,
    )
    const cacheKey = playbackPrefetchCacheId(track, resolvedSettings)
      + (options.avoidDirect ? '|hls' : '') + (options.allowFallback === false ? '|original' : '')
    if (options.forceRefresh) this.cache.delete(cacheKey)

    if (!options.forceRefresh && !options.avoidDirect && isDirectStreamUrl(track.audioUrl)) {
      const directUrl = track.audioUrl.trim()
      const candidates = trustedDirectStreamCandidates(track, adapter.kind, directUrl)
      const qualityKey = adapter.qualityKey(resolvedSettings)
      const audioInfo = createAudioInfo(adapter.kind, qualityKey)
      return createSuccess(track, adapter.kind, resolvedSettings, {
        url: directUrl,
        streamType: sharedStreamType(adapter.kind, directUrl),
        candidateUrls: candidates,
        candidateDetails: candidates.map(url => ({
          url, streamType: sharedStreamType(adapter.kind, url), qualityKey,
          cacheKey: `${cacheKey}|direct`, audioInfo,
        })),
        qualityKey,
        // 直链/一起听 streamUrl 的实际音质未知，不进入本地持久缓存
        cacheKey: `${cacheKey}|direct`,
        isPreview: true,
        audioInfo,
      })
    }

    if (!options.forceRefresh) {
      const cached = this.cache.get(cacheKey)
      if (cached) {
        if (cached.expiresAt > Date.now()) return cached.result
        this.cache.delete(cacheKey)
      }
      const existing = this.inFlight.get(cacheKey)
      if (existing && existing.requestGeneration === options.requestGeneration) {
        return existing.promise
      }
    }

    const pending = adapter.resolve(track, resolvedSettings, options)
      .then(result => result ?? {
        type: 'failure' as const,
        message: 'No playable stream returned',
        retryable: true,
      })
      .catch(error => classifyPlaybackError(error))
      .then(result => {
        if (result.type === 'success' && this.inFlight.get(cacheKey)?.promise === pending) {
          if (!this.cache.has(cacheKey) && this.cache.size >= MAX_RESOLUTION_CACHE_ENTRIES) {
            const oldest = this.cache.keys().next().value
            if (oldest !== undefined) this.cache.delete(oldest)
          }
          this.cache.delete(cacheKey)
          this.cache.set(cacheKey, {
            result,
            expiresAt: playbackResolutionExpiresAt(result),
          })
        }
        return result
      })
      .finally(() => {
        if (this.inFlight.get(cacheKey)?.promise === pending) this.inFlight.delete(cacheKey)
      })

    this.inFlight.set(cacheKey, { promise: pending, requestGeneration: options.requestGeneration })
    return pending
  }

  private pruneExpiredCache(now = Date.now()): void {
    for (const [key, entry] of this.cache) {
      if (entry.expiresAt <= now) this.cache.delete(key)
    }
  }

  invalidate(track: TrackInfo, settings: PlaybackSourceSettings): void {
    const key = playbackPrefetchCacheId(track, settings)
    for (const variant of [key, `${key}|hls`]) {
      this.cache.delete(variant)
      this.inFlight.delete(variant)
    }
  }

  clear(): void {
    this.cache.clear()
    this.inFlight.clear()
  }
}

function trustedDirectStreamCandidates(
  track: TrackInfo,
  kind: PlaybackSourceKind,
  directUrl: string,
): string[] {
  const raw = track.syncPayload?.streamUrls
  if (!Array.isArray(raw)) return []
  const channelId = kind === 'youtube' ? 'youtubeMusic' : kind
  const trusted = trustedInboundStreamUrls(channelId, raw)
  // 仅关联到本次主直链的候选才能进入播放，旧载荷不能附加无关资源
  return trusted.includes(directUrl) ? trusted.filter(url => url !== directUrl) : []
}

export const playbackUrlResolver = new PlaybackUrlResolver()

export async function resolvePlaybackResult(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions = {},
): Promise<PlaybackResolution> {
  return playbackUrlResolver.resolve(track, settings, options)
}

export async function resolvePlaybackSource(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
): Promise<ResolvedPlaybackSource | null> {
  const result = await resolvePlaybackResult(track, settings)
  return result.type === 'success' ? result : null
}

export async function resolveDownloadSource(track: TrackInfo, settings: PlaybackSourceSettings): Promise<ResolvedPlaybackSource> {
  const result = await resolvePlaybackResult(track, settings, { forceRefresh: true, allowFallback: false })
  if (result.type !== 'success') {
    throw new Error('message' in result ? result.message || 'No downloadable stream' : 'No downloadable stream')
  }
  if (result.isPreview) throw new Error('Preview audio cannot be saved as a full download')
  const candidates = uniqueUrls([result.url, ...result.candidateUrls])
    .map((_, index) => selectPlaybackCandidate(result, index))
  const playable = candidates.find(candidate => !unsupportedDesktopCodec(candidate.codec ?? candidate.audioInfo?.codecLabel))
  if (!playable) throw new Error('No downloadable stream has a supported audio codec')
  return playable
}

export function playbackCacheWriteOptions(
  resolved: ResolvedPlaybackSource,
  candidateIndex: number,
): PlaybackCacheWriteOptions {
  if (resolved.isPreview || resolved.source === 'local') return {}
  const selected = selectPlaybackCandidate(resolved, candidateIndex)
  if (candidateIndex !== 0 && selected !== resolved) {
    return { cacheKey: selected.cacheKey, expectedContentLength: selected.expectedContentLength }
  }
  const primaryCacheKey = resolved.cacheKeyOverride || resolved.cacheKey
  if (candidateIndex !== 0) {
    return {
      // 候选 URL 往往含短时签名，不能把完整 URL 写进键，否则每次刷新
      // 都会生成永不复用的磁盘缓存文件（SR-08）
      cacheKey: `${primaryCacheKey}|candidate:${candidateIndex}`,
    }
  }
  return {
    cacheKey: primaryCacheKey,
    expectedContentLength: resolved.expectedContentLength,
    ...(resolved.expectedContentMd5 ? { expectedContentMd5: resolved.expectedContentMd5 } : {}),
  }
}

export function selectPlaybackCandidate(resolved: ResolvedPlaybackSource, candidateIndex: number): ResolvedPlaybackSource {
  if (candidateIndex === 0) return resolved
  const urls = resolved.source === 'local'
    ? [resolved.url, ...resolved.candidateUrls].filter((url, index, values) => values.indexOf(url) === index)
    : uniqueUrls([resolved.url, ...resolved.candidateUrls])
  const url = urls[candidateIndex]
  const candidate = resolved.candidateDetails?.find(item => item.url === url)
  if (!candidate) return resolved
  return {
    ...resolved,
    ...candidate,
    cacheKeyOverride: undefined,
    expectedContentMd5: undefined,
    candidateUrls: [],
    candidateDetails: undefined,
  }
}

function settingsWithQualityOverride(
  settings: PlaybackSourceSettings,
  kind: PlaybackSourceKind,
  qualityOverride?: string,
): PlaybackSourceSettings {
  if (!qualityOverride) return settings
  if (kind === 'netease') return { ...settings, neteaseQuality: qualityOverride }
  if (kind === 'qq') return { ...settings, qqMusicQuality: qualityOverride }
  if (kind === 'bilibili') return { ...settings, biliQuality: qualityOverride }
  return { ...settings, youtubeQuality: qualityOverride }
}

function neteaseQualityFallbacks(preferred: string): string[] {
  const preferredIndex = NETEASE_QUALITY_FALLBACK_ORDER.indexOf(preferred)
  return preferredIndex >= 0
    ? NETEASE_QUALITY_FALLBACK_ORDER.slice(preferredIndex)
    : [preferred, 'exhigh', 'standard'].filter(
      (quality, index, values) => quality && values.indexOf(quality) === index,
    )
}

function trackValue(track: TrackInfo, kind: PlaybackSourceKind): string {
  const payloadAudioId = syncPayloadString(track, 'audioId', 'audio_id')
  if (payloadAudioId) return payloadAudioId
  if (kind === 'youtube') {
    const mediaUri = syncPayloadString(track, 'mediaUri', 'media_uri')
    const videoId = mediaUri
      ?.match(/^ytmusic:\/\/video\/([^?]+)/i)?.[1]
    if (videoId) return videoId
  }
  const prefix = `${kind}:`
  if (track.id.toLowerCase().startsWith(prefix)) return track.id.slice(prefix.length)
  return track.id
}

function syncPayloadString(track: TrackInfo, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const value = track.syncPayload?.[key]
    if (typeof value === 'string' && value.trim()) return value.trim()
    if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  }
  return undefined
}

function stablePlaybackCacheKey(
  track: TrackInfo,
  kind: PlaybackSourceKind,
  quality: string,
): string {
  const normalizedQuality = quality.trim().toLowerCase() || 'default'
  if (kind === 'netease') {
    return `netease-${trackValue(track, kind)}-${normalizedQuality}`
  }
  if (kind === 'qq') {
    return `qq-${trackValue(track, kind)}-${normalizedQuality}`
  }
  if (kind === 'youtube') {
    return `ytmusic-${trackValue(track, kind)}-${normalizedQuality}`
  }

  const cid = bilibiliCid(track)
  const base = `bili-${trackValue(track, kind)}`
  return cid ? `${base}-${cid}-${normalizedQuality}` : `${base}-${normalizedQuality}`
}

function sharedStreamType(kind: PlaybackSourceKind, value: string): 'direct' | 'hls' {
  if (kind !== 'youtube') return 'direct'
  try {
    const url = new URL(value)
    const trustedHost = url.hostname === 'googlevideo.com' || url.hostname.endsWith('.googlevideo.com')
    return url.protocol === 'https:' && trustedHost && (!url.port || url.port === '443')
      && (/\/manifest\/hls(?:_|\/)/i.test(url.pathname) || /\.m3u8$/i.test(url.pathname))
      ? 'hls' : 'direct'
  } catch { return 'direct' }
}

function youtubeStreamCacheKey(track: TrackInfo, quality: string, streamType?: 'direct' | 'hls'): string {
  const key = stablePlaybackCacheKey(track, 'youtube', quality)
  return streamType === 'hls' ? `${key}-hls` : key
}

function bilibiliCid(track: TrackInfo): string | undefined {
  const payloadCid = syncPayloadString(track, 'subAudioId', 'sub_audio_id')
  if (payloadCid) return payloadCid
  return track.album?.match(/^Bilibili\|(\d+)/i)?.[1]
}

function createSuccess(
  track: TrackInfo,
  source: PlaybackSourceKind,
  settings: PlaybackSourceSettings,
  values: {
    url: string
    candidateUrls?: string[]
    candidateDetails?: PlaybackCandidateDetails[]
    streamType?: 'direct' | 'hls'
    durationMs?: number
    mimeType?: string
    expectedContentLength?: number
    expectedContentMd5?: string
    isPreview?: boolean
    audioInfo?: PlaybackAudioInfo
    qualityKey?: string
    cacheKey?: string
    cacheKeyOverride?: string
    bitrate?: number
    codec?: string
    format?: string
  },
): ResolvedPlaybackSource {
  const qualityKey = values.qualityKey ?? qualityForSource(source, settings)
  const cacheKey = values.cacheKey ?? stablePlaybackCacheKey(track, source, qualityKey)
  const audioInfo = values.audioInfo ?? createAudioInfo(
    source,
    qualityKey,
    values.codec,
    values.mimeType,
    values.bitrate,
  )
  if (audioInfo && !audioInfo.specLabel) {
    audioInfo.specLabel = buildPlaybackSpecLabel(audioInfo)
  }
  return {
    type: 'success',
    url: values.url,
    candidateUrls: uniqueUrls(values.candidateUrls),
    candidateDetails: values.candidateDetails,
    streamType: values.streamType,
    durationMs: values.durationMs,
    mimeType: values.mimeType,
    expectedContentLength: values.expectedContentLength,
    expectedContentMd5: values.expectedContentMd5,
    isPreview: values.isPreview,
    audioInfo,
    cacheKeyOverride: values.cacheKeyOverride,
    cacheKey,
    source,
    qualityKey,
    bitrate: values.bitrate,
    codec: values.codec ?? audioInfo?.codecLabel,
    format: values.format,
  }
}

function qualityForSource(
  source: PlaybackSourceKind,
  settings: PlaybackSourceSettings,
): string {
  if (source === 'netease') return settings.neteaseQuality
  if (source === 'qq') return settings.qqMusicQuality
  if (source === 'bilibili') return settings.biliQuality
  return settings.youtubeQuality
}

function createAudioInfo(
  source: PlaybackAudioSource,
  qualityKey?: string,
  codec?: string,
  mimeType?: string,
  bitrate?: number,
): PlaybackAudioInfo {
  const bitrateKbps = normalizeBitrateKbps(bitrate)
  return {
    source,
    qualityKey,
    qualityLabel: qualityKey,
    codecLabel: codec,
    mimeType,
    bitrateKbps,
    specLabel: buildPlaybackSpecLabel({ bitrateKbps }),
  }
}

function resolveNetease(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions,
): Promise<ResolvedPlaybackSource | null> {
  const songId = Number.parseInt(trackValue(track, 'netease'), 10)
  const preferred = settings.neteaseQuality.trim().toLowerCase() || 'exhigh'
  const qualities = neteaseQualityFallbacks(preferred)
  let lastError: unknown = null
  let previewFallback: ResolvedPlaybackSource | null = null
  let unavailable = false
  let requiresLogin = false
  const requestGeneration = options.requestGeneration

  return (async () => {
    for (const quality of qualities) {
      try {
        const result = await invoke<{
          url: string | null
          bitrate: number
          format: string
          expected_content_length?: number | null
          expected_content_md5?: string | null
          duration_ms?: number | null
          level?: string | null
          song_id?: number | null
          is_preview?: boolean
          unavailable_reason?: 'requires_login' | 'no_permission' | 'no_play_url' | 'unknown' | null
        }>(
          'get_netease_song_url',
          { songId, quality, requestGeneration },
        )
        if (result.unavailable_reason === 'requires_login') {
          requiresLogin = true
          continue
        }
        if (result.unavailable_reason === 'unknown') {
          unavailable = false
          break
        }
        if (!result.url) {
          unavailable = result.unavailable_reason === 'no_permission'
          continue
        }
        if (result.song_id != null && result.song_id !== songId) throw new Error('Playback source song identity mismatch')
        const actualQuality = result.level?.trim().toLowerCase() || quality
        const mimeType = normalizeMimeType(result.format)
        const codec = deriveCodecLabel(mimeType) ?? normalizeCodecName(result.format)
        const audioInfo = createAudioInfo(
          'netease',
          actualQuality,
          codec,
          mimeType,
          result.bitrate,
        )
        audioInfo.qualityOptions = NETEASE_QUALITY_OPTIONS
        const resolved = createSuccess(track, 'netease', settings, {
          url: result.url,
          bitrate: result.bitrate,
          codec,
          format: result.format,
          mimeType,
          expectedContentLength: result.expected_content_length ?? undefined,
          expectedContentMd5: result.expected_content_md5 ?? undefined,
          durationMs: result.duration_ms ?? undefined,
          isPreview: result.is_preview === true,
          qualityKey: actualQuality,
          cacheKey: stablePlaybackCacheKey(track, 'netease', actualQuality),
          audioInfo,
        })
        if (resolved.isPreview) {
          previewFallback = resolved
          continue
        }
        return resolved
      } catch (error) {
        if (/requires login|song identity mismatch/i.test(error instanceof Error ? error.message : String(error))) {
          throw error
        }
        lastError = error
      }
    }
    if (options.allowFallback !== false && (previewFallback || unavailable)) {
      const fallback = await resolveNeteaseFallback(track, settings, options)
      if (fallback) return fallback
    }
    if (previewFallback) return previewFallback
    if (requiresLogin) throw new Error('Playback requires login')
    if (lastError) throw lastError
    return null
  })()
}

interface FallbackTrack {
  id: string
  title: string
  artist: string
  album: string
  url: string
  duration_ms: number
}

async function resolveNeteaseFallback(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions,
): Promise<ResolvedPlaybackSource | null> {
  const identity = {
    title: syncPayloadString(track, 'originalName', 'original_name') ?? track.title,
    artist: syncPayloadString(track, 'originalArtist', 'original_artist') ?? track.artist,
    durationMs: track.durationMs || 0,
  }
  if (settings.neteaseLocalSourceFallback) {
    const local = await invoke<FallbackTrack[]>('find_netease_local_sources', {
      ...identity, songId: trackValue(track, 'netease'),
    }).catch(() => [])
    if (local.length > 0) {
      const details = local.map(candidate => {
        const format = candidate.url.split(/[\\/]/).pop()?.split('.').pop()?.toLowerCase()
        const mimeType = normalizeMimeType(format ?? '')
        return {
          url: candidate.url, durationMs: candidate.duration_ms,
          qualityKey: '', cacheKey: '', format,
          audioInfo: { source: 'local' as const, mimeType, codecLabel: deriveCodecLabel(mimeType) },
        }
      })
      const selected = details[0]!
      return {
        ...selected, type: 'success', source: 'local',
        candidateUrls: local.slice(1).map(candidate => candidate.url),
        candidateDetails: details.slice(1),
      }
    }
  }
  if (settings.neteaseAutoSourceSwitch) {
    const candidates = await invoke<FallbackTrack[]>('find_netease_bili_sources', {
      ...identity, requestGeneration: options.requestGeneration,
    }).catch(error => {
      if (/superseded/i.test(String(error))) throw error
      return []
    })
    let primary: ResolvedPlaybackSource | null = null
    const alternates: PlaybackCandidateDetails[] = []
    for (const candidate of candidates) {
      try {
        const alternate = await resolveBilibili({
          ...track, ...candidate, durationMs: candidate.duration_ms, audioUrl: '',
          source: 'bilibili', syncPayload: undefined,
        }, settings, options)
        if (!alternate) continue
        if (!primary) {
          primary = { ...alternate, durationMs: candidate.duration_ms || track.durationMs }
        } else {
          for (const index of [0, ...alternate.candidateUrls.map((_, index) => index + 1)]) {
            const selected = selectPlaybackCandidate(alternate, index)
            alternates.push({
              url: index === 0 ? alternate.url : alternate.candidateUrls[index - 1]!,
              qualityKey: selected.qualityKey, cacheKey: selected.cacheKey,
              durationMs: candidate.duration_ms || track.durationMs,
              audioInfo: selected.audioInfo ?? createAudioInfo('bilibili', selected.qualityKey),
              mimeType: selected.mimeType, bitrate: selected.bitrate, codec: selected.codec,
              expectedContentLength: selected.expectedContentLength,
            })
          }
        }
      } catch (error) {
        if (/superseded/i.test(String(error))) throw error
      }
    }
    if (primary) return {
      ...primary,
      candidateUrls: [...primary.candidateUrls, ...alternates.map(candidate => candidate.url)],
      candidateDetails: [...(primary.candidateDetails ?? []), ...alternates],
    }
  }
  return null
}

function resolveQq(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions = {},
): Promise<ResolvedPlaybackSource | null> {
  const songMid = trackValue(track, 'qq')
  const quality = settings.qqMusicQuality
  return invoke<{ url: string | null; bitrate: number; format: string }>(
    'get_qq_song_url',
    { songMid, quality, requestGeneration: options.requestGeneration },
  ).then(result => {
    if (!result.url) return null
    const mimeType = normalizeMimeType(result.format)
    const codec = deriveCodecLabel(mimeType) ?? normalizeCodecName(result.format)
    const audioInfo = createAudioInfo('qq', quality, codec, mimeType, result.bitrate)
    audioInfo.qualityOptions = [{ key: quality, label: quality }]
    return createSuccess(track, 'qq', settings, {
      url: result.url,
      bitrate: result.bitrate,
      codec,
      format: result.format,
      mimeType,
      cacheKey: stablePlaybackCacheKey(track, 'qq', quality),
      audioInfo,
    })
  })
}

interface BiliAudioCandidate {
  url: string
  bandwidth: number
  codecs: string
  quality_key?: string
  mime_type?: string
}

function resolveBilibili(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions = {},
): Promise<ResolvedPlaybackSource | null> {
  const biliId = trackValue(track, 'bilibili')
  const isAvid = /^\d+$/.test(biliId)
  const cid = bilibiliCid(track)
  const quality = settings.biliQuality

  return invoke<{
    url: string
    bandwidth: number
    codecs: string
    quality_key?: string
    mime_type?: string
    candidates?: BiliAudioCandidate[]
  }>('get_bili_audio_url', {
    bvid: isAvid ? '' : biliId,
    avid: isAvid ? Number.parseInt(biliId, 10) : null,
    cid: cid ? Number.parseInt(cid, 10) : null,
    quality,
    requestGeneration: options.requestGeneration,
  }).then(result => {
    if (!result.url) return null
    const candidates = (result.candidates ?? [])
      .filter(candidate => isDirectStreamUrl(candidate.url))
      .map(candidate => candidate.url)
    const actualQuality = result.quality_key || quality
    const mimeType = normalizeMimeType(result.mime_type) || mimeTypeForCodec(result.codecs)
    const codec = normalizeCodecName(result.codecs)
    const availableQualityKeys = [quality, ...(result.candidates ?? [])
      .map(candidate => candidate.quality_key || inferBiliQualityKey(candidate.bandwidth, candidate.codecs))
    ].filter((key, index, values) => values.indexOf(key) === index)
    return createSuccess(track, 'bilibili', settings, {
      url: result.url,
      candidateUrls: candidates.filter(url => url !== result.url),
      candidateDetails: (result.candidates ?? []).filter(candidate => isDirectStreamUrl(candidate.url)).map(candidate => {
        const key = candidate.quality_key || inferBiliQualityKey(candidate.bandwidth, candidate.codecs)
        const candidateMime = normalizeMimeType(candidate.mime_type) || mimeTypeForCodec(candidate.codecs)
        const candidateCodec = normalizeCodecName(candidate.codecs)
        return {
          url: candidate.url, qualityKey: key, cacheKey: stablePlaybackCacheKey(track, 'bilibili', key),
          bitrate: candidate.bandwidth, codec: candidateCodec, mimeType: candidateMime,
          audioInfo: createAudioInfo('bilibili', key, candidateCodec, candidateMime, candidate.bandwidth),
        }
      }),
      bitrate: result.bandwidth,
      codec,
      mimeType,
      qualityKey: actualQuality,
      cacheKey: stablePlaybackCacheKey(track, 'bilibili', actualQuality),
      audioInfo: {
        source: 'bilibili',
        qualityKey: actualQuality,
        qualityLabel: actualQuality,
        qualityOptions: availableQualityKeys.map(key => ({ key, label: key })),
        codecLabel: codec,
        mimeType,
        bitrateKbps: normalizeBitrateKbps(result.bandwidth),
        specLabel: buildPlaybackSpecLabel({
          bitrateKbps: normalizeBitrateKbps(result.bandwidth),
        }),
      },
    })
  })
}

interface YoutubeAudioStream {
  url: string
  bitrate: number
  mime_type: string
  content_length?: number
  stream_type?: 'direct' | 'hls'
}

function resolveYoutube(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions = {},
): Promise<ResolvedPlaybackSource | null> {
  const videoId = trackValue(track, 'youtube')
  const quality = settings.youtubeQuality
  return invoke<YoutubeAudioStream[]>('get_youtube_audio_url', {
    videoId,
    playbackSource: settings.youtubePlaybackSource ?? 'automatic',
    forceRefresh: options.forceRefresh ?? false,
    avoidDirect: options.avoidDirect ?? false,
    requestGeneration: options.requestGeneration,
  })
    .then(streams => {
      const ordered = orderYoutubeStreams(streams ?? [], quality)
      const primary = ordered[0]
      if (!primary?.url) return null
      const mimeType = normalizeMimeType(primary.mime_type)
      const codec = deriveCodecLabel(primary.mime_type)
      const bitrateKbps = normalizeBitrateKbps(primary.bitrate)
      return createSuccess(track, 'youtube', settings, {
        url: primary.url,
        streamType: primary.stream_type ?? 'direct',
        candidateUrls: ordered.slice(1).map(stream => stream.url),
        candidateDetails: ordered.slice(1).map(stream => ({
          url: stream.url, qualityKey: quality, cacheKey: youtubeStreamCacheKey(track, quality, stream.stream_type),
          streamType: stream.stream_type ?? 'direct',
          bitrate: stream.bitrate, codec: deriveCodecLabel(stream.mime_type), format: stream.mime_type,
          mimeType: normalizeMimeType(stream.mime_type), expectedContentLength: stream.content_length,
          audioInfo: createAudioInfo('youtube', quality, deriveCodecLabel(stream.mime_type), normalizeMimeType(stream.mime_type), stream.bitrate),
        })),
        bitrate: primary.bitrate,
        codec,
        format: primary.mime_type,
        mimeType,
        expectedContentLength: primary.content_length,
        qualityKey: quality,
        cacheKey: youtubeStreamCacheKey(track, quality, primary.stream_type),
        audioInfo: {
          source: 'youtube',
          qualityKey: quality,
          qualityLabel: quality,
          qualityOptions: YOUTUBE_QUALITY_OPTIONS,
          codecLabel: codec,
          mimeType,
          bitrateKbps,
          specLabel: buildPlaybackSpecLabel({ bitrateKbps }),
        },
      })
    })
}

function orderYoutubeStreams(
  streams: YoutubeAudioStream[],
  quality: string,
): YoutubeAudioStream[] {
  // 首选可解码容器，再按 Android 音质阈值和候选流类型选择
  const minimumBitrate = ({ low: 0, standard: 0, medium: 96_000, high: 128_000, higher: 128_000 } as Record<string, number>)[quality] ?? 160_000
  const sorted = streams
    .filter(stream => isDirectStreamUrl(stream.url))
    .sort((a, b) => {
      const scoreDiff = youtubeMimeScore(b.mime_type) - youtubeMimeScore(a.mime_type)
      if (scoreDiff !== 0) return scoreDiff
      const qualityDiff = Number(b.bitrate >= minimumBitrate) - Number(a.bitrate >= minimumBitrate)
      if (qualityDiff !== 0) return qualityDiff
      const directDiff = Number((b.stream_type ?? 'direct') === 'direct') - Number((a.stream_type ?? 'direct') === 'direct')
      if (directDiff !== 0) return directDiff
      return b.bitrate - a.bitrate
    })
  return sorted
}

function youtubeMimeScore(mimeType?: string): number {
  if (unsupportedDesktopCodec(deriveCodecLabel(mimeType))) return 0
  const base = mimeType?.split(';', 1)[0]?.trim().toLowerCase() ?? ''
  if (base.startsWith('audio/mp4') || base === 'audio/m4a' || base === 'audio/aac') return 3
  if (base.startsWith('audio/')) return 2
  if (base.startsWith('video/')) return 1
  return 0
}

function inferBiliQualityKey(bandwidth: number, codecs: string): string {
  const normalized = codecs.toLowerCase()
  if (normalized === 'ec-3' || normalized.includes('e-ac-3')) return 'dolby'
  if (normalized === 'flac') return 'lossless'
  const bitrateKbps = normalizeBitrateKbps(bandwidth) ?? 0
  if (bitrateKbps >= 180) return 'high'
  if (bitrateKbps >= 120) return 'medium'
  return 'low'
}

function normalizeMimeType(value?: string): string | undefined {
  const normalized = value?.split(';', 1)[0]?.trim().toLowerCase()
  if (!normalized) return undefined
  if (normalized.includes('/')) return normalized
  const mimeByFormat: Record<string, string> = {
    flac: 'audio/flac',
    mp3: 'audio/mpeg',
    mpeg: 'audio/mpeg',
    aac: 'audio/aac',
    m4a: 'audio/mp4',
    mp4: 'audio/mp4',
    opus: 'audio/webm',
    vorbis: 'audio/ogg',
  }
  return mimeByFormat[normalized] ?? `audio/${normalized}`
}

function mimeTypeForCodec(codec?: string): string | undefined {
  const normalized = codec?.toLowerCase().trim()
  if (!normalized) return undefined
  if (normalized === 'flac') return 'audio/flac'
  if (normalized === 'ec-3' || normalized.includes('e-ac-3')) return 'audio/eac3'
  if (normalized.includes('opus')) return 'audio/webm'
  if (normalized.includes('mp4a') || normalized.includes('aac')) return 'audio/mp4'
  if (normalized.includes('mp3') || normalized === 'mpeg') return 'audio/mpeg'
  return undefined
}

function deriveCodecLabel(mimeType?: string): string | undefined {
  const declaredCodec = mimeType?.match(/codecs\s*=\s*["']?([^;"',]+)/i)?.[1]
  if (declaredCodec) return normalizeCodecName(declaredCodec)
  const normalized = mimeType?.split(';', 1)[0]?.trim().toLowerCase()
  if (!normalized) return undefined
  const codecByMime: Record<string, string> = {
    'audio/flac': 'FLAC',
    'audio/eac3': 'E-AC-3',
    'audio/e-ac-3': 'E-AC-3',
    'audio/mp4': 'AAC',
    'audio/aac': 'AAC',
    'audio/mpeg': 'MP3',
    'audio/mp3': 'MP3',
    'audio/webm': 'OPUS',
    'audio/ogg': 'Vorbis',
  }
  return codecByMime[normalized] ?? normalized.split('/').pop()?.toUpperCase()
}

function unsupportedDesktopCodec(codec?: string): boolean {
  return /^(opus|e-?ac-?3|ec-3|ac-?3|alac|dts)$/i.test(codec?.trim() ?? '')
}

function normalizeCodecName(codec?: string): string | undefined {
  if (!codec) return undefined
  const raw = codec.trim()
  const lower = raw.toLowerCase()
  const family = lower.split('.', 1)[0]
  const codecMap: Record<string, string> = {
    flac: 'FLAC',
    mp3: 'MP3',
    mpeg: 'MP3',
    aac: 'AAC',
    mp4a: 'AAC',
    opus: 'OPUS',
    vorbis: 'Vorbis',
    'ec-3': 'E-AC-3',
    ac3: 'AC-3',
  }
  return codecMap[lower] ?? codecMap[family] ?? raw
}

function uniqueUrls(urls: string[] = []): string[] {
  return urls
    .map(url => url.trim())
    .filter(isDirectStreamUrl)
    .filter((url, index, values) => values.indexOf(url) === index)
}

function classifyPlaybackError(error: unknown): PlaybackResolution {
  const message = error instanceof Error ? error.message : String(error)
  if (/login|unauthorized|登录|未登录|需要登录/i.test(message)) {
    return { type: 'requires_login', message }
  }
  return { type: 'failure', message, retryable: true }
}

const PLAYBACK_SOURCE_ADAPTERS: PlaybackSourceAdapter[] = [
  {
    kind: 'netease',
    matches: track => getPlaybackSourceKind(track) === 'netease',
    qualityKey: settings => settings.neteaseQuality,
    resolve: resolveNetease,
  },
  {
    kind: 'qq',
    matches: track => getPlaybackSourceKind(track) === 'qq',
    qualityKey: settings => settings.qqMusicQuality,
    resolve: resolveQq,
  },
  {
    kind: 'bilibili',
    matches: track => getPlaybackSourceKind(track) === 'bilibili',
    qualityKey: settings => settings.biliQuality,
    resolve: resolveBilibili,
  },
  {
    kind: 'youtube',
    matches: track => getPlaybackSourceKind(track) === 'youtube',
    qualityKey: settings => settings.youtubeQuality,
    resolve: resolveYoutube,
  },
]
