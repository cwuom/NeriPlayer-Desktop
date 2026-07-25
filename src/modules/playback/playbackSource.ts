import { invoke } from '@tauri-apps/api/core'
import type { TrackInfo } from '@/stores/player'
import { createLogger } from '@/utils/logger'

const log = createLogger('playback-source')

export type PlaybackSourceKind = 'netease' | 'qq' | 'bilibili' | 'youtube'
export type PlaybackAudioSource = PlaybackSourceKind | 'local'

export interface PlaybackSourceSettings {
  neteaseQuality: string
  qqMusicQuality: string
  biliQuality: string
  youtubeQuality: string
  neteaseAutoSourceSwitch: boolean
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
  durationMs?: number
  mimeType?: string
  expectedContentLength?: number
  isPreview?: boolean
  audioInfo?: PlaybackAudioInfo
  cacheKeyOverride?: string
  cacheKey: string
  source: PlaybackSourceKind
  qualityKey: string
  fallbackSources?: ResolvedPlaybackSource[]

  // 兼容现有播放状态和设置页展示字段
  bitrate?: number
  codec?: string
  format?: string
}

interface BilibiliFallbackSearchResult {
  id: string
  title: string
  artist: string
  duration_ms: number
  source: string
}

interface BilibiliVideoPage {
  cid: number
  title: string
  duration_seconds: number
}

type BilibiliAutoSourceFailureStage = 'search' | 'page' | 'stream'

type BilibiliAutoSourceAttemptResult =
  | { resolved: ResolvedPlaybackSource; failure: null }
  | {
      resolved: null
      failure: {
        stage: BilibiliAutoSourceFailureStage
        reason: string
      }
    }

export type PlaybackResolution =
  | ResolvedPlaybackSource
  | { type: 'waiting_for_authoritative_stream' }
  | { type: 'requires_login'; message?: string }
  | { type: 'failure'; message: string; retryable: boolean }

export interface PlaybackResolveOptions {
  forceRefresh?: boolean
  qualityOverride?: string
  requestGeneration?: number
}

export interface PlaybackCacheWriteOptions {
  cacheKey?: string
  expectedContentLength?: number
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
  const adapter = getPlaybackSourceAdapter(track)
  if (!adapter) return []
  const configuredQuality = adapter.qualityKey(settings).trim().toLowerCase()
  const preferred = configuredQuality || (adapter.kind === 'netease' ? 'exhigh' : 'default')
  const qualities = adapter.kind === 'netease'
    ? neteaseQualityFallbacks(preferred)
    : [preferred]
  const candidates = qualities.map(qualityKey => ({
    cacheKey: stablePlaybackCacheKey(track, adapter.kind, qualityKey),
    source: adapter.kind,
    qualityKey,
  }))
  if (adapter.kind === 'netease' && settings.neteaseAutoSourceSwitch !== false) {
    const biliQuality = settings.biliQuality.trim().toLowerCase() || 'high'
    candidates.push({
      cacheKey: neteaseAutoSourceCacheKey(track, biliQuality),
      source: 'bilibili',
      qualityKey: biliQuality,
    })
  }
  return candidates
}

export function playbackPrefetchCacheId(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
): string {
  const sourceCacheId = playbackQualityCachePrefix(track, settings) ?? track.id
  if (getPlaybackSourceKind(track) !== 'netease') return sourceCacheId
  const autoSourceSwitch = settings.neteaseAutoSourceSwitch !== false ? 'on' : 'off'
  const biliQuality = settings.biliQuality.trim().toLowerCase() || 'high'
  return `${sourceCacheId}|auto-source:${autoSourceSwitch}|bili:${biliQuality}`
}

export class PlaybackUrlResolver {
  private readonly cache = new Map<string, {
    result: ResolvedPlaybackSource
    expiresAt: number
  }>()
  private readonly inFlight = new Map<string, Promise<PlaybackResolution>>()

  async resolve(
    track: TrackInfo,
    settings: PlaybackSourceSettings,
    options: PlaybackResolveOptions = {},
  ): Promise<PlaybackResolution> {
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
    const resolutionCacheKey = playbackPrefetchCacheId(track, resolvedSettings)
    const sourceCacheKey = playbackQualityCachePrefix(track, resolvedSettings) ?? track.id

    if (!options.forceRefresh && isDirectStreamUrl(track.audioUrl)) {
      return createSuccess(track, adapter.kind, resolvedSettings, {
        url: track.audioUrl.trim(),
        qualityKey: adapter.qualityKey(resolvedSettings),
        cacheKey: sourceCacheKey,
        audioInfo: createAudioInfo(
          adapter.kind,
          adapter.qualityKey(resolvedSettings),
          undefined,
          undefined,
          undefined,
        ),
      })
    }

    if (!options.forceRefresh) {
      const cached = this.cache.get(resolutionCacheKey)
      if (cached) {
        if (cached.expiresAt > Date.now()) return cached.result
        this.cache.delete(resolutionCacheKey)
      }
      const existing = this.inFlight.get(resolutionCacheKey)
      if (existing) return existing
    }

    const pending = adapter.resolve(track, resolvedSettings, options)
      .then(result => result ?? {
        type: 'failure' as const,
        message: 'No playable stream returned',
        retryable: true,
      })
      .catch(error => classifyPlaybackError(error))
      .then(result => {
        if (result.type === 'success') {
          this.cache.set(resolutionCacheKey, {
            result,
            expiresAt: Date.now() + RESOLUTION_TTL_MS,
          })
        }
        return result
      })
      .finally(() => {
        if (this.inFlight.get(resolutionCacheKey) === pending) {
          this.inFlight.delete(resolutionCacheKey)
        }
      })

    this.inFlight.set(resolutionCacheKey, pending)
    return pending
  }

  invalidate(track: TrackInfo, settings: PlaybackSourceSettings): void {
    this.cache.delete(playbackPrefetchCacheId(track, settings))
  }

  clear(): void {
    this.cache.clear()
  }
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

export function playbackCacheWriteOptions(
  resolved: ResolvedPlaybackSource,
  candidateIndex: number,
  candidateUrl = resolved.url,
): PlaybackCacheWriteOptions {
  if (resolved.isPreview) return {}
  const primaryCacheKey = resolved.cacheKeyOverride || resolved.cacheKey
  if (candidateIndex !== 0) {
    return {
      cacheKey: `${primaryCacheKey}|candidate:${candidateIndex}|${candidateUrl}`,
    }
  }
  return {
    cacheKey: primaryCacheKey,
    expectedContentLength: resolved.expectedContentLength,
  }
}

const MAX_PLAYBACK_SOURCE_CANDIDATES = 3

export function playbackSourceCandidates(
  resolved: ResolvedPlaybackSource,
): ResolvedPlaybackSource[] {
  const candidates = [resolved, ...(resolved.fallbackSources ?? [])]
  const seen = new Set<string>()
  return candidates
    .filter(candidate => {
      const key = candidate.cacheKey
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    .slice(0, MAX_PLAYBACK_SOURCE_CANDIDATES)
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
    durationMs?: number
    mimeType?: string
    expectedContentLength?: number
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
    durationMs: values.durationMs,
    mimeType: values.mimeType,
    expectedContentLength: values.expectedContentLength,
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

const BILIBILI_AUTO_SOURCE_SCORE_THRESHOLD = 70
const BILIBILI_AUTO_SOURCE_SEARCH_LIMIT = 6
const BILIBILI_AUTO_SOURCE_CANDIDATE_LIMIT = MAX_PLAYBACK_SOURCE_CANDIDATES

export function shouldAutoSwitchNeteaseSource(
  enabled: boolean,
  hasPreview: boolean,
  unavailableReason: 'no_permission' | 'no_play_url' | 'unknown' | null,
): boolean {
  return enabled && (
    hasPreview
    || unavailableReason === 'no_permission'
    || unavailableReason === 'no_play_url'
  )
}

function neteaseAutoSourceCacheKey(track: TrackInfo, biliQuality: string): string {
  const quality = biliQuality.trim().toLowerCase() || 'high'
  return `bili-auto-${trackValue(track, 'netease')}-${quality}`
}

function normalizeAutoSourceText(value: string): string {
  return value
    .normalize('NFKC')
    .toLowerCase()
    .replace(/<[^>]*>/g, ' ')
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim()
}

function autoSourceTokens(value: string): string[] {
  return normalizeAutoSourceText(value)
    .split(/\s+/)
    .filter(token => token.length > 0)
}

function autoSourceTitleScore(trackTitle: string, candidateTitle: string): number {
  const title = normalizeAutoSourceText(trackTitle)
  const normalizedCandidateTitle = normalizeAutoSourceText(candidateTitle)
  if (title && normalizedCandidateTitle.includes(title)) return 55

  const titleTokens = autoSourceTokens(trackTitle)
  if (titleTokens.length === 0) return 0
  const matchedTokens = titleTokens.filter(token => normalizedCandidateTitle.includes(token)).length
  if (matchedTokens === titleTokens.length) return 35
  return matchedTokens > 0 ? 18 : 0
}

export function scoreBilibiliAutoSource(
  track: Pick<TrackInfo, 'title' | 'artist' | 'durationMs'>,
  candidate: Pick<BilibiliFallbackSearchResult, 'title' | 'artist' | 'duration_ms'>,
): number {
  const candidateText = normalizeAutoSourceText(`${candidate.title} ${candidate.artist}`)
  let score = autoSourceTitleScore(track.title, candidate.title)

  if (score === 0) {
    const titleTokens = autoSourceTokens(track.title)
    const matchedTokens = titleTokens.filter(token => candidateText.includes(token)).length
    if (matchedTokens === titleTokens.length && matchedTokens > 0) score += 35
    else if (matchedTokens > 0) score += 18
  }

  const artistTokens = autoSourceTokens(track.artist)
  if (artistTokens.some(token => candidateText.includes(token))) score += 25

  const targetDuration = Math.max(0, track.durationMs || 0)
  const candidateDuration = Math.max(0, candidate.duration_ms || 0)
  if (targetDuration > 0 && candidateDuration > 0) {
    const difference = Math.abs(targetDuration - candidateDuration)
    if (difference <= 8_000) score += 30
    else if (difference <= 20_000) score += 22
    else if (difference <= 45_000) score += 12
    if (candidateDuration > targetDuration * 2) score -= 15
  }

  return score
}

function selectBilibiliAutoSourcePage(
  track: Pick<TrackInfo, 'title' | 'artist' | 'durationMs'>,
  candidate: BilibiliFallbackSearchResult,
  pages: BilibiliVideoPage[],
): { page: BilibiliVideoPage; score: number } | null {
  const candidateScore = scoreBilibiliAutoSource(track, candidate)
  let best: { page: BilibiliVideoPage; score: number } | null = null

  const validPages = pages.filter(page => Number.isSafeInteger(page.cid) && page.cid > 0)

  for (const page of validPages) {
    const durationSeconds = Number.isFinite(page.duration_seconds)
      ? Math.max(0, page.duration_seconds)
      : 0
    const pageTitle = page.title || ''
    const pageTitleScore = autoSourceTitleScore(track.title, pageTitle)
    if (validPages.length > 1 && pageTitleScore < 35) continue

    const pageScore = scoreBilibiliAutoSource(track, {
      title: pageTitle,
      artist: candidate.artist,
      duration_ms: durationSeconds * 1_000,
    })
    const score = candidateScore + pageScore + pageTitleScore
    if (!best || score > best.score) best = { page, score }
  }

  return best
}

function bilibiliAutoSourceQueries(track: TrackInfo): string[] {
  const suffix = '\u65e0\u635f'
  return [
    `${track.title} ${track.artist} ${suffix}`,
    `${track.artist} ${track.title} ${suffix}`,
    `${track.title} ${suffix}`,
  ]
    .map(query => query.replace(/\s+/g, ' ').trim())
    .filter((query, index, values) => query && values.indexOf(query) === index)
}

export function bilibiliAutoSourceDurationFilter(durationMs: number): number {
  const durationSeconds = Math.floor(Math.max(0, Number.isFinite(durationMs) ? durationMs : 0) / 1_000)
  if (durationSeconds <= 0) return 0
  if (durationSeconds < 10 * 60) return 1
  if (durationSeconds < 30 * 60) return 2
  if (durationSeconds < 60 * 60) return 3
  return 4
}

function safeAutoSourceFailureReason(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error)
  const sanitized = raw
    .replace(/\b(?:set-cookie|cookie|authorization)\s*[:=][^\r\n]*/gi, '[redacted-credential]')
    .replace(/https?:\/\/[^\s"'<>]+/gi, '[redacted-url]')
    .trim()
    .slice(0, 240)
  return sanitized || 'unknown error'
}

function failedBilibiliAutoSourceAttempt(
  track: TrackInfo,
  stage: BilibiliAutoSourceFailureStage,
  reason: string,
): BilibiliAutoSourceAttemptResult {
  log.error('NetEase Bilibili auto-source fallback failed', {
    trackId: track.id,
    stage,
    reason,
  })
  return { resolved: null, failure: { stage, reason } }
}

async function resolveNeteaseAutoBilibiliSource(
  track: TrackInfo,
  settings: PlaybackSourceSettings,
  options: PlaybackResolveOptions,
): Promise<BilibiliAutoSourceAttemptResult> {
  const queries = bilibiliAutoSourceQueries(track)
  const durationFilter = bilibiliAutoSourceDurationFilter(track.durationMs)
  log.info('NetEase Bilibili auto-source fallback started', {
    trackId: track.id,
    queryCount: queries.length,
    durationFilter,
    quality: settings.biliQuality.trim().toLowerCase() || 'high',
  })

  const ranked = new Map<string, {
    candidate: BilibiliFallbackSearchResult
    score: number
  }>()
  let searchErrors = 0
  let emptySearches = 0

  for (const [queryIndex, query] of queries.entries()) {
    let results: BilibiliFallbackSearchResult[] = []
    const filters = durationFilter > 0 ? [durationFilter, 0] : [0]
    for (const [attemptIndex, filter] of filters.entries()) {
      try {
        const response = await invoke<BilibiliFallbackSearchResult[]>('search', {
          query,
          platform: 'bilibili',
          includeLyrics: false,
          bilibiliDuration: filter,
        })
        results = Array.isArray(response) ? response : []
      } catch (error) {
        searchErrors++
        log.warn('NetEase Bilibili auto-source search failed', {
          trackId: track.id,
          queryIndex,
          attemptIndex,
          durationFilter: filter,
          reason: safeAutoSourceFailureReason(error),
        })
        continue
      }
      if (results.length > 0) break
      emptySearches++
      log.warn('NetEase Bilibili auto-source search returned no usable results', {
        trackId: track.id,
        queryIndex,
        attemptIndex,
        durationFilter: filter,
        resultCount: 0,
      })
    }

    const queryCandidates = results
      .filter(candidate => candidate.id.toLowerCase().startsWith('bilibili:'))
      .map(candidate => ({ candidate, score: scoreBilibiliAutoSource(track, candidate) }))
      .sort((left, right) => right.score - left.score)
      .slice(0, BILIBILI_AUTO_SOURCE_SEARCH_LIMIT)

    if (queryCandidates.length === 0) {
      if (results.length > 0) {
        emptySearches++
        log.warn('NetEase Bilibili auto-source search returned no usable results', {
          trackId: track.id,
          queryIndex,
          durationFilter: filters[filters.length - 1] ?? 0,
          resultCount: results.length,
        })
      }
      continue
    }

    for (const { candidate, score } of queryCandidates) {
      const current = ranked.get(candidate.id)
      if (!current || score > current.score) ranked.set(candidate.id, { candidate, score })
    }
  }

  const candidates = [...ranked.values()]
    .sort((left, right) => right.score - left.score)

  if (candidates.length === 0) {
    return failedBilibiliAutoSourceAttempt(
      track,
      'search',
      `no usable candidates (${searchErrors} errors, ${emptySearches} empty results)`,
    )
  }

  const pageMatches: Array<{
    candidate: BilibiliFallbackSearchResult
    page: BilibiliVideoPage
    score: number
  }> = []
  let pageErrors = 0
  let lowPageMatches = 0
  for (const { candidate } of candidates) {
    const bvid = candidate.id.replace(/^bilibili:/i, '')
    if (!bvid) continue
    try {
      const pages = await invoke<BilibiliVideoPage[]>('get_bili_video_pages', { bvid })
      const match = selectBilibiliAutoSourcePage(track, candidate, pages)
      if (!match || match.score < BILIBILI_AUTO_SOURCE_SCORE_THRESHOLD) {
        lowPageMatches++
        log.warn('NetEase Bilibili auto-source page match rejected', {
          trackId: track.id,
          bvid,
          pageCount: Array.isArray(pages) ? pages.length : 0,
          score: match?.score ?? null,
          threshold: BILIBILI_AUTO_SOURCE_SCORE_THRESHOLD,
        })
        continue
      }
      pageMatches.push({ candidate, ...match })
    } catch (error) {
      pageErrors++
      log.warn('NetEase Bilibili auto-source page lookup failed', {
        trackId: track.id,
        bvid,
        reason: safeAutoSourceFailureReason(error),
      })
      continue
    }
  }
  pageMatches.sort((left, right) => right.score - left.score)

  if (pageMatches.length === 0) {
    return failedBilibiliAutoSourceAttempt(
      track,
      'page',
      `no page passed matching (${pageErrors} errors, ${lowPageMatches} low matches)`,
    )
  }

  const resolvedCandidates: ResolvedPlaybackSource[] = []
  const neteaseId = trackValue(track, 'netease')
  const quality = settings.biliQuality.trim().toLowerCase() || 'high'
  let streamErrors = 0
  let emptyStreams = 0
  for (const { candidate, page } of pageMatches) {
    if (resolvedCandidates.length >= BILIBILI_AUTO_SOURCE_CANDIDATE_LIMIT) break
    const bvid = candidate.id.replace(/^bilibili:/i, '')
    const biliTrack: TrackInfo = {
      ...track,
      id: `bilibili:${bvid}`,
      source: 'bilibili',
      album: `Bilibili|${page.cid}`,
      audioUrl: '',
      syncPayload: undefined,
    }
    try {
      const resolved = await resolveBilibili(biliTrack, settings, options)
      if (!resolved) {
        emptyStreams++
        log.warn('NetEase Bilibili auto-source stream lookup returned no playable stream', {
          trackId: track.id,
          bvid,
          cid: page.cid,
        })
        continue
      }
      const cacheKey = `bili-auto-${neteaseId}-${bvid}-${page.cid}-${quality}`
      const autoSourceCacheKey = neteaseAutoSourceCacheKey(track, quality)
      resolvedCandidates.push({
        ...resolved,
        durationMs: page.duration_seconds > 0
          ? page.duration_seconds * 1_000
          : resolved.durationMs,
        cacheKey,
        cacheKeyOverride: autoSourceCacheKey,
        fallbackSources: undefined,
      })
    } catch (error) {
      streamErrors++
      log.warn('NetEase Bilibili auto-source stream lookup failed', {
        trackId: track.id,
        bvid,
        cid: page.cid,
        reason: safeAutoSourceFailureReason(error),
      })
      continue
    }
  }

  const primary = resolvedCandidates[0]
  if (!primary) {
    return failedBilibiliAutoSourceAttempt(
      track,
      'stream',
      `no playable stream (${streamErrors} errors, ${emptyStreams} empty results)`,
    )
  }
  return {
    resolved: {
      ...primary,
      fallbackSources: resolvedCandidates.slice(1),
    },
    failure: null,
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
  let lastUnavailableReason: 'no_permission' | 'no_play_url' | 'unknown' | null = null
  const requestGeneration = options.requestGeneration

  return (async () => {
    for (const quality of qualities) {
      try {
        const result = await invoke<{
          url: string | null
          bitrate: number
          format: string
          expected_content_length?: number | null
          is_preview?: boolean
          unavailable_reason?: 'requires_login' | 'no_permission' | 'no_play_url' | 'unknown' | null
        }>(
          'get_netease_song_url',
          { songId, quality, requestGeneration },
        )
        lastError = null
        lastUnavailableReason = result.unavailable_reason === 'requires_login'
          ? null
          : result.unavailable_reason ?? null
        if (result.unavailable_reason === 'requires_login') {
          throw new Error('Playback requires login')
        }
        if (result.unavailable_reason === 'unknown') break
        if (!result.url) continue
        const mimeType = normalizeMimeType(result.format)
        const codec = deriveCodecLabel(mimeType) ?? normalizeCodecName(result.format)
        const audioInfo = createAudioInfo(
          'netease',
          quality,
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
          isPreview: result.is_preview === true,
          qualityKey: quality,
          cacheKey: stablePlaybackCacheKey(track, 'netease', quality),
          audioInfo,
        })
        if (resolved.isPreview) {
          previewFallback = resolved
          continue
        }
        return resolved
      } catch (error) {
        if (/requires login/i.test(error instanceof Error ? error.message : String(error))) {
          throw error
        }
        lastUnavailableReason = null
        lastError = error
      }
    }
    const shouldTryAutoSource = shouldAutoSwitchNeteaseSource(
      settings.neteaseAutoSourceSwitch !== false,
      previewFallback !== null,
      lastUnavailableReason,
    )
    if (shouldTryAutoSource) {
      const fallbackAttempt = await resolveNeteaseAutoBilibiliSource(track, settings, options)
      if (fallbackAttempt.resolved) return fallbackAttempt.resolved
      if (!previewFallback) {
        throw new Error(
          `Bilibili fallback was attempted but failed during ${fallbackAttempt.failure.stage}: ${fallbackAttempt.failure.reason}`,
        )
      }
    }
    if (previewFallback) return previewFallback
    if (lastError) throw lastError
    return null
  })()
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
    const mimeType = mimeTypeForCodec(result.codecs)
    const codec = normalizeCodecName(result.codecs)
    const availableQualityKeys = [quality, ...(result.candidates ?? [])
      .map(candidate => inferBiliQualityKey(candidate.bandwidth, candidate.codecs))
    ].filter((key, index, values) => values.indexOf(key) === index)
    return createSuccess(track, 'bilibili', settings, {
      url: result.url,
      candidateUrls: candidates.filter(url => url !== result.url),
      bitrate: result.bandwidth,
      codec,
      mimeType,
      qualityKey: quality,
      cacheKey: stablePlaybackCacheKey(track, 'bilibili', quality),
      audioInfo: {
        source: 'bilibili',
        qualityKey: quality,
        qualityLabel: quality,
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
        candidateUrls: ordered.slice(1).map(stream => stream.url),
        bitrate: primary.bitrate,
        codec,
        format: primary.mime_type,
        mimeType,
        expectedContentLength: primary.content_length,
        qualityKey: quality,
        cacheKey: stablePlaybackCacheKey(track, 'youtube', quality),
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
  // 桌面 rodio/symphonia 当前未启用 opus, 始终优先 audio/mp4(AAC)
  // quality 只在同 mime 档内调节码率, 不能把 webm/opus 抬到 m4a 前面
  const sorted = streams
    .filter(stream => isDirectStreamUrl(stream.url))
    .sort((a, b) => {
      const scoreDiff = youtubeMimeScore(b.mime_type) - youtubeMimeScore(a.mime_type)
      if (scoreDiff !== 0) return scoreDiff
      return b.bitrate - a.bitrate
    })
  if (sorted.length <= 1) return sorted

  const preferredScore = youtubeMimeScore(sorted[0].mime_type)
  const preferred = sorted.filter(stream => youtubeMimeScore(stream.mime_type) === preferredScore)
  const rest = sorted.filter(stream => youtubeMimeScore(stream.mime_type) !== preferredScore)

  // 仅在首选 mime 档内按 quality 挑主候选
  const primaryIndex = quality === 'low'
    ? preferred.length - 1
    : quality === 'medium'
      ? Math.floor((preferred.length - 1) / 2)
      : quality === 'high'
        ? Math.min(1, preferred.length - 1)
        : 0
  const primary = preferred[primaryIndex]
  return [
    primary,
    ...preferred.filter((_, index) => index !== primaryIndex),
    ...rest,
  ]
}

function youtubeMimeScore(mimeType?: string): number {
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
  const normalized = mimeType?.split(';', 1)[0]?.trim().toLowerCase()
  if (!normalized) return undefined
  if (normalized.includes('codecs=')) {
    const raw = normalized.match(/codecs=["']?([^;"']+)/)?.[1]?.split('.', 1)[0]
    if (raw) return normalizeCodecName(raw)
  }
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
