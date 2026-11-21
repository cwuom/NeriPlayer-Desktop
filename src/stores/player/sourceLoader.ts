import type { Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useDownloadStore } from '@/stores/download'
import { MAX_MEDIA_CACHE_SIZE_MB, MIN_MEDIA_CACHE_SIZE_MB, useSettingsStore } from '@/stores/settings'
import {
  getPlaybackSourceKind,
  isRemotePlaybackTrack,
  normalizeBitrateKbps,
  playbackCacheReadCandidates,
  playbackCacheWriteOptions,
  playbackUrlResolver,
  resolvePlaybackResult,
  selectPlaybackCandidate,
  type PlaybackCacheReadCandidate,
  type PlaybackResolution,
  type PlaybackSourceSettings,
  type ResolvedPlaybackSource,
} from '@/modules/playback/playbackSource'
import { PlaybackFailure } from '@/modules/playback/playbackFailure'
import { playbackPrefetchManager } from '@/modules/playback/playbackPrefetch'
import { recallPlayedQuality, rememberPlayedQuality } from '@/modules/playback/playedQualityMemory'
import { shouldResolvePlaybackSourceInParallel, type PlaybackLoadStart } from '@/modules/playback/playbackRequest'
import {
  qualityLabelFromKey,
  qualityOptionsFromSource,
  resolvedPlaybackAudioInfo,
} from '@/modules/playback/audioQualityLabels'
import {
  isBiliVideoSkipTrack,
  resolveBiliVideoSkipBvid,
  resolveBiliVideoSkipCid,
  resolveBiliVideoSkipTarget,
  type BiliVideoSkipTarget,
} from '@/modules/playback/biliVideoSkip'
import type { AudioInfo } from '@/modules/playback/playbackAudioInfo'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'
import { getTrackCoverUrl } from '@/utils/trackCover'
import type { TrackInfo } from '@/types/track'
import { tracePlaybackUi } from './trace'
import type { PlaybackControl, PlayFunction } from './types'

const log = createLogger('player')

function audioFilePathKey(path: string | undefined | null) {
  const key = (path || '').replace(/\\/g, '/').trim()
  return /^[a-z]:\//i.test(key) || key.startsWith('//') ? key.toLowerCase() : key
}

/** Match downloads by exact ID, or by legacy video ID only when the requested CID agrees. */
function downloadedTrackForPlayback(track: TrackInfo) {
  const downloadStore = useDownloadStore()
  const exact = downloadStore.getDownloadedTrack(track.id)
  if (exact || !isBiliVideoSkipTrack(track)) return exact
  const cid = resolveBiliVideoSkipCid(track)
  if (!cid) return undefined
  const parentId = track.id.match(/^bilibili:([^:]+):\d+$/i)?.[1]
  if (!parentId) return undefined
  const bvid = resolveBiliVideoSkipBvid(track)
  for (const sourceId of new Set([parentId, bvid].filter((value): value is string => !!value))) {
    const legacy = downloadStore.getDownloadedTrack(`bilibili:${sourceId}`)
    if (legacy && resolveBiliVideoSkipCid(legacy) === cid) return legacy
  }
  return undefined
}

export { audioFilePathKey, downloadedTrackForPlayback }

/**
 * 后端预占这次请求，作废旧请求的在途加载；不等待结果，冷启动的缓存查找和地址解析不必排在它后面。
 * keepsPreviousAudible 为 false 时一并静音上一首
 */
export function claimPlaybackRequest(
  track: TrackInfo,
  token: number,
  keepsPreviousAudible: boolean,
  requestStarted: number,
  isCurrent: () => boolean,
) {
  const claimStarted = performance.now()
  void invoke<void>('begin_playback_request', {
    requestGeneration: token,
    trackId: track.id,
    source: getPlaybackSourceKind(track) || track.source || 'local',
    hasCover: !!getTrackCoverUrl(track),
    hasAudioUrl: !!track.audioUrl,
    hasSyncPayload: !!track.syncPayload,
    silencePrevious: !keepsPreviousAudible,
  }).then(() => {
    if (!isCurrent()) return
    tracePlaybackUi(
      'backend_request_claimed',
      track,
      `invokeMs=${Math.round(performance.now() - claimStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
      token,
    )
  }).catch((error) => {
    if (isCurrent()) {
      log.warn('playback request preclaim failed:', error)
    }
  })
}

/** 正在释放或改写的音频文件：期间不能再打开它 */
export interface AudioFileMutation {
  trackId: string | null
  requestToken: number | null
  shouldResume: boolean
  releasesCurrentPlayback: boolean
  positionMs: number
  finished: Promise<void>
}

export interface PlaybackLoadStartPlan extends PlaybackLoadStart {
  useCrossfade: boolean
}

/** play 发起的一次加载：代际、起播位置与淡入淡出都由 play 决定 */
export interface PlaybackSourceRequest {
  track: TrackInfo
  token: number
  forceResolve: boolean
  hadPlaybackSessionBeforeRequest: boolean
  requestStarted: number
  transitionFadeOutMs: number
  transitionFadeInMs: number
  /** 这次请求仍是最新的；过期后所有结果都要丢弃 */
  isCurrent: () => boolean
  currentLoadStartPlan: () => PlaybackLoadStartPlan
  markLoadStartApplied: (start: PlaybackLoadStart) => void
}

export interface LoadedPlaybackSource {
  dur: number
  playedFromPlaybackCache: boolean
  playingPreviewClip: boolean
}

export interface PlaybackSourceLoaderContext {
  currentTrack: Ref<TrackInfo | null>
  positionMs: Ref<number>
  isPlaying: Ref<boolean>
  play: PlayFunction
  pause: PlaybackControl
  playError: Ref<string | null>
  audioInfo: Ref<AudioInfo | null>
  isPlayingFromDownload: Ref<boolean>
  isPlayingFromCache: Ref<boolean>
  currentStreamUrl: Ref<string | null>
  audioFileMutations: Map<string, AudioFileMutation>
  /** 记录本次会话实际打开的下载文件，删除或改写它时据此释放 */
  setLoadedFromDownloadPath: (path: string | null) => void
  /** 下载文件和缓存不依赖在线地址，不参与地址过期检测 */
  resetUrlResolveTime: () => void
  setResolvedStreamUrls: (urls: string[]) => void
  rememberStreamQualities: (resolved: ResolvedPlaybackSource | null) => void
  resetDecodedAudioInfo: () => void
  readLocalAudioInfo: (path: string, requestGeneration: number) => void
  applyResolvedBiliVideoSkipTarget: (track: TrackInfo, target: BiliVideoSkipTarget, requestGeneration: number) => void
  takePrefetchedPlaybackUrl: (track: TrackInfo) => ResolvedPlaybackSource | null
  playbackSourceSettings: () => PlaybackSourceSettings
  hasCompleteCachedAudio: (candidates: readonly PlaybackCacheReadCandidate[]) => Promise<boolean>
  /** 解析请求带上当前播放代际，后端据此丢弃过期请求 */
  currentRequestGeneration: () => number
}

/**
 * 一次播放请求的音源装载：已下载文件 → 完整磁盘缓存 → 在线解析（含候选地址与刷新重试）→ 本地文件。
 * 请求过期时返回 null，由 play 直接结束；失败以异常抛给 play 统一处理。换音质重播也在这里作废旧地址
 */
export function createPlaybackSourceLoader(ctx: PlaybackSourceLoaderContext) {
  const {
    currentTrack, positionMs, isPlaying, play, pause,
    playError, audioInfo, isPlayingFromDownload, isPlayingFromCache, currentStreamUrl, audioFileMutations,
    setLoadedFromDownloadPath, resetUrlResolveTime, setResolvedStreamUrls, rememberStreamQualities,
    resetDecodedAudioInfo, readLocalAudioInfo, applyResolvedBiliVideoSkipTarget, takePrefetchedPlaybackUrl,
    playbackSourceSettings, hasCompleteCachedAudio, currentRequestGeneration,
  } = ctx

  async function resolvePlaybackUrl(
    track: TrackInfo,
    forceRefresh = false,
    qualityOverride?: string,
    avoidDirect = false,
  ): Promise<PlaybackResolution> {
    return resolvePlaybackResult(track, playbackSourceSettings(), {
      forceRefresh,
      qualityOverride,
      avoidDirect,
      requestGeneration: currentRequestGeneration(),
    })
  }

  async function loadPlaybackSource(request: PlaybackSourceRequest): Promise<LoadedPlaybackSource | null> {
    const {
      track, token, forceResolve, hadPlaybackSessionBeforeRequest, requestStarted,
      transitionFadeOutMs, transitionFadeInMs, isCurrent, currentLoadStartPlan, markLoadStartApplied,
    } = request
    let dur = 0
    let playedFromDownloadedFile = false
    let playedFromPlaybackCache = false
    let playingPreviewClip = false
    playError.value = null
    audioInfo.value = null
    resetDecodedAudioInfo()
    isPlayingFromDownload.value = false
    isPlayingFromCache.value = false

    const downloaded = downloadedTrackForPlayback(track)
    if (downloaded?.filePath && !audioFileMutations.has(audioFilePathKey(downloaded.filePath)) && isRemotePlaybackTrack(track)) {
      try {
        const startPlan = currentLoadStartPlan()
        dur = await playDownloadedFile(
          downloaded.filePath,
          downloaded.durationMs || track.durationMs,
          startPlan.useCrossfade,
          transitionFadeOutMs,
          transitionFadeInMs,
          token,
          startPlan.positionMs,
        )
        if (!isCurrent()) return null
        if (isBiliVideoSkipTrack(track)) {
          const downloadedTarget = resolveBiliVideoSkipTarget({ ...track, album: downloaded.album, syncPayload: undefined })
          if (downloadedTarget) applyResolvedBiliVideoSkipTarget(track, downloadedTarget, token)
        }
        markLoadStartApplied(startPlan)
        playedFromDownloadedFile = true
        setLoadedFromDownloadPath(downloaded.filePath)
        isPlayingFromDownload.value = true
      } catch (e) {
        if (!isCurrent()) return null
        log.warn('downloaded file unavailable, falling back to online source:', e)
        setLoadedFromDownloadPath(null)
        isPlayingFromDownload.value = false
      }
    }

    if (playedFromDownloadedFile) {
      if (downloaded?.filePath) readLocalAudioInfo(downloaded.filePath, token)
      resetUrlResolveTime()
    } else if (isRemotePlaybackTrack(track)) {
      setLoadedFromDownloadPath(null)
      isPlayingFromDownload.value = false
      // 进入在线解析前默认非缓存; 命中缓存时会再置 true
      isPlayingFromCache.value = false
      // 强制重新解析（切换音质、断流恢复）不读缓存、不用预取结果，否则会重播刚要替换掉的那份
      const prefetchedResolution = forceResolve ? null : takePrefetchedPlaybackUrl(track)
      const qualityMemoryKey = playbackCacheReadCandidates(track, playbackSourceSettings())[0]?.cacheKey ?? ''
      const cacheCandidates = forceResolve ? [] : playbackCacheReadCandidates(track, playbackSourceSettings())
      const hasCompleteCache = await hasCompleteCachedAudio(cacheCandidates)
      if (!isCurrent()) return null
      const resolveInParallel = shouldResolvePlaybackSourceInParallel(
        hadPlaybackSessionBeforeRequest,
        !!prefetchedResolution,
        hasCompleteCache,
      )
      const coldResolution = resolveInParallel
        ? resolvePlaybackUrl(track, forceResolve).catch((error): PlaybackResolution => ({
            type: 'failure',
            message: error instanceof Error ? error.message : String(error),
            retryable: true,
          }))
        : null
      tracePlaybackUi(
        'remote_pipeline_start',
        track,
        `cacheCandidates=${cacheCandidates.length}, prefetched=${!!prefetchedResolution}, parallelResolve=${resolveInParallel}`,
        token,
      )
      try {
        const startPlan = currentLoadStartPlan()
        const cacheLookupStarted = performance.now()
        tracePlaybackUi(
          'cache_lookup_start',
          track,
          `candidates=${cacheCandidates.length}, startMs=${startPlan.positionMs}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
          token,
        )
        const cached = await playCachedRemoteAudioCandidates(
          cacheCandidates,
          track.durationMs,
          startPlan.useCrossfade,
          transitionFadeOutMs,
          transitionFadeInMs,
          token,
          startPlan.positionMs,
        )
        if (!isCurrent()) return null
        if (cached) {
          markLoadStartApplied(startPlan)
          dur = cached.durationMs
          playedFromPlaybackCache = true
          isPlayingFromCache.value = true
          audioInfo.value = {
            source: cached.source,
            qualityKey: cached.qualityKey,
            // 缓存命中也要展示纸面音质名 (最高/极高…), 不能只塞 raw key
            qualityLabel: qualityLabelFromKey(cached.source, cached.qualityKey),
            qualityOptions: qualityOptionsFromSource(cached.source),
          }
          // 缓存键是首选音质，实际写入的可能是降级流：换成记录下来的实际音质
          void recallPlayedQuality(qualityMemoryKey).then((played) => {
            if (!played?.qualityKey || !isCurrent() || !audioInfo.value) return
            audioInfo.value = {
              ...audioInfo.value,
              qualityKey: played.qualityKey,
              qualityLabel: qualityLabelFromKey(cached.source, played.qualityKey),
              codec: audioInfo.value.codec ?? played.codecLabel,
              bitrate: audioInfo.value.bitrate ?? played.bitrateKbps,
              mimeType: audioInfo.value.mimeType ?? played.mimeType,
            }
          })
          tracePlaybackUi(
            'cache_lookup_hit',
            track,
            `quality=${cached.qualityKey}, durationMs=${cached.durationMs}, lookupMs=${Math.round(performance.now() - cacheLookupStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
            token,
          )
        } else {
          tracePlaybackUi(
            'cache_lookup_miss',
            track,
            `lookupMs=${Math.round(performance.now() - cacheLookupStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
            token,
          )
        }
      } catch (error) {
        if (!isCurrent()) return null
        log.warn('persistent cache unavailable, resolving online source:', error)
      }

      if (!playedFromPlaybackCache) {
        let result = prefetchedResolution
        if (!result) {
          const resolveStarted = performance.now()
          tracePlaybackUi(
            'source_resolve_wait',
            track,
            `parallel=${!!coldResolution}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
            token,
          )
          const resolution = await (coldResolution ?? resolvePlaybackUrl(track, forceResolve))
          tracePlaybackUi(
            'source_resolve_returned',
            track,
            `type=${resolution.type}, resolveMs=${Math.round(performance.now() - resolveStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
            token,
          )
          if (resolution.type !== 'success') {
            if (resolution.type === 'requires_login') {
              throw new PlaybackFailure('requires_login', resolution.message || 'Playback requires login')
            }
            if (resolution.type === 'waiting_for_authoritative_stream') {
              throw new Error('Waiting for authoritative playback stream')
            }
            throw new PlaybackFailure(resolution.reason ?? 'url_error', resolution.message)
          }
          result = resolution
        }
        tracePlaybackUi(
          'source_resolved',
          track,
          `quality=${result.qualityKey}, candidates=${result.candidateUrls?.length ?? 0}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
          token,
        )
        if (!isCurrent()) return null

        const playResolvedSource = async (resolved: ResolvedPlaybackSource) => {
          let lastError: unknown = null
          const candidates = [resolved.url, ...resolved.candidateUrls]
            .filter((url, index, values) => values.indexOf(url) === index)
          for (const [candidateIndex, candidateUrl] of candidates.entries()) {
            if (!isCurrent()) return 0
            const candidateStarted = performance.now()
            try {
              const cacheWrite = playbackCacheWriteOptions(resolved, candidateIndex)
              const selected = selectPlaybackCandidate(resolved, candidateIndex)
              const startPlan = currentLoadStartPlan()
              tracePlaybackUi(
                'backend_stream_start',
                track,
                `candidate=${candidateIndex}, startMs=${startPlan.positionMs}, crossfade=${startPlan.useCrossfade}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
                token,
              )
              const duration = selected.source === 'local' ? await playDownloadedFile(
                candidateUrl,
                selected.durationMs || track.durationMs,
                startPlan.useCrossfade,
                transitionFadeOutMs,
                transitionFadeInMs,
                token,
                startPlan.positionMs,
              ) : await playRemoteUrl(
                candidateUrl,
                selected.durationMs || track.durationMs,
                startPlan.useCrossfade,
                transitionFadeOutMs,
                transitionFadeInMs,
                token,
                isCurrent,
                startPlan.positionMs,
                cacheWrite.cacheKey,
                cacheWrite.expectedContentLength,
                cacheWrite.expectedContentMd5,
                selected.streamType,
              )
              if (isCurrent()) {
                currentStreamUrl.value = resolved.source === 'local' ? null : candidateUrl
                setResolvedStreamUrls(resolved.source === 'local' ? [] : candidates.slice(candidateIndex))
                rememberStreamQualities(resolved)
                result = selectPlaybackCandidate(resolved, candidateIndex)
                if (selected.biliVideoSkipTarget) applyResolvedBiliVideoSkipTarget(track, selected.biliVideoSkipTarget, token)
              }
              markLoadStartApplied(startPlan)
              tracePlaybackUi(
                'backend_stream_ready',
                track,
                `candidate=${candidateIndex}, durationMs=${duration}, candidateMs=${Math.round(performance.now() - candidateStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}`,
                token,
              )
              return duration
            } catch (error) {
              lastError = error
              tracePlaybackUi(
                'backend_stream_failed',
                track,
                `candidate=${candidateIndex}, candidateMs=${Math.round(performance.now() - candidateStarted)}, elapsedMs=${Math.round(performance.now() - requestStarted)}, error=${summarizeLogError(error)}`,
                token,
              )
            }
          }
          throw lastError instanceof Error
            ? lastError
            : new Error(String(lastError || 'All playback candidates failed'))
        }

        try {
          dur = await playResolvedSource(result)
        } catch (firstError) {
          if (!isCurrent()) return null
          playbackUrlResolver.invalidate(track, playbackSourceSettings())
          const refreshed = await resolvePlaybackUrl(
            track,
            true,
          )
          if (refreshed.type !== 'success') throw firstError
          result = refreshed
          try {
            dur = await playResolvedSource(result)
          } catch (refreshError) {
            if (!isCurrent()) return null
            if (getPlaybackSourceKind(track) !== 'youtube' || result.streamType === 'hls') throw refreshError
            const hls = await resolvePlaybackUrl(track, true, undefined, true)
            if (hls.type !== 'success') throw refreshError
            result = hls
            dur = await playResolvedSource(result)
          }
        }
        if (!isCurrent()) return null
        // 直链（一起听）也标记 isPreview 以免进缓存，不能据此提示试听
        playingPreviewClip = result.source === 'netease' && result.isPreview === true && !result.cacheKey.endsWith('|direct')
        if (!result.isPreview && result.source !== 'local') {
          rememberPlayedQuality(qualityMemoryKey, {
            qualityKey: result.audioInfo?.qualityKey ?? result.qualityKey,
            codecLabel: result.audioInfo?.codecLabel ?? result.codec,
            bitrateKbps: result.audioInfo?.bitrateKbps ?? normalizeBitrateKbps(result.bitrate),
            mimeType: result.audioInfo?.mimeType,
          })
        }
        audioInfo.value = resolvedPlaybackAudioInfo(result)
      }
    } else {
      setLoadedFromDownloadPath(null)
      isPlayingFromDownload.value = false
      isPlayingFromCache.value = false
      // 本地文件
      const startPlan = currentLoadStartPlan()
      tracePlaybackUi(
        'backend_file_start',
        track,
        `startMs=${startPlan.positionMs}, crossfade=${startPlan.useCrossfade}`,
        token,
      )
      if (startPlan.useCrossfade) {
        dur = await invoke<number>('crossfade_file', {
          path: track.audioUrl,
          durationHintMs: track.durationMs,
          fadeOutMs: transitionFadeOutMs, fadeInMs: transitionFadeInMs,
          requestGeneration: token,
        })
      } else {
        dur = await invoke<number>('play_file', {
          path: track.audioUrl,
          durationHintMs: track.durationMs,
          startPositionMs: startPlan.positionMs,
          requestGeneration: token,
        })
      }
      if (!isCurrent()) return null
      markLoadStartApplied(startPlan)
      readLocalAudioInfo(track.audioUrl, token)
    }

    return { dur, playedFromPlaybackCache, playingPreviewClip }
  }

  // 用指定音质重新播放当前曲目（保持进度）
  async function replayWithQuality() {
    const track = currentTrack.value
    if (!track) return
    const pos = positionMs.value
    const wasPlaying = isPlaying.value
    const sourceSettings = playbackSourceSettings()
    playbackPrefetchManager.clearForTrack(track, sourceSettings)
    playbackUrlResolver.invalidate(track, sourceSettings)
    resetUrlResolveTime()
    playError.value = null
    // 换音质只影响本机，一起听里没有控制权时也允许
    await play(track, 'local_safety', pos, true)
    if (playError.value) {
      throw new Error(playError.value)
    }
    if (!wasPlaying && currentTrack.value?.id === track.id) {
      await pause('local')
    }
  }

  return { loadPlaybackSource, replayWithQuality }
}

export async function playDownloadedFile(
  path: string,
  durationHintMs: number,
  useCrossfade: boolean,
  fadeOutMs: number,
  fadeInMs: number,
  requestGeneration: number,
  startPositionMs = 0,
): Promise<number> {
  if (useCrossfade) {
    return invoke<number>('crossfade_file', {
      path,
      durationHintMs,
      fadeOutMs,
      fadeInMs,
      requestGeneration,
    })
  }
  return invoke<number>('play_file', {
    path,
    durationHintMs,
    startPositionMs: Math.max(0, Math.round(startPositionMs)),
    requestGeneration,
  })
}

export async function playRemoteUrl(
  url: string,
  durationHintMs: number,
  useCrossfade: boolean,
  fadeOutMs: number,
  fadeInMs: number,
  requestGeneration: number,
  isCurrentRequest: () => boolean,
  startPositionMs = 0,
  cacheKey?: string,
  expectedContentLength?: number,
  expectedContentMd5?: string,
  streamType: 'direct' | 'hls' = 'direct',
): Promise<number> {
  const safeStartMs = Math.max(0, Math.round(startPositionMs))
  const cacheLimitBytes = playbackCacheLimitBytes()
  if (useCrossfade) {
    try {
      return await invoke<number>('crossfade_url_streaming', {
        url,
        durationHintMs,
        fadeOutMs,
        fadeInMs,
        cacheKey,
        cacheLimitBytes,
        expectedContentLength,
        expectedContentMd5,
        streamType,
        requestGeneration,
      })
    } catch (streamError) {
      if (!isCurrentRequest()) throw streamError
      log.warn(
        'streaming playback failed, falling back to temp-file playback:',
        summarizeLogError(streamError),
      )
      return invoke<number>('crossfade_url_fast', {
        url,
        durationHintMs,
        fadeOutMs,
        fadeInMs,
        cacheKey,
        cacheLimitBytes,
        expectedContentLength,
        expectedContentMd5,
        streamType,
        requestGeneration,
      })
    }
  }

  try {
    return await invoke<number>('play_url_streaming', {
      url,
      durationHintMs,
      startPositionMs: safeStartMs,
      cacheKey,
      cacheLimitBytes,
      expectedContentLength,
      expectedContentMd5,
      streamType,
      requestGeneration,
    })
  } catch (streamError) {
    if (!isCurrentRequest()) throw streamError
    log.warn(
      'streaming playback failed, falling back to temp-file playback:',
      summarizeLogError(streamError),
    )
    return invoke<number>('play_url_fast', {
      url,
      durationHintMs,
      startPositionMs: safeStartMs,
      cacheKey,
      cacheLimitBytes,
      expectedContentLength,
      expectedContentMd5,
      streamType,
      requestGeneration,
    })
  }
}

interface CachedRemoteAudioResult {
  durationMs: number
  source: 'netease' | 'qq' | 'bilibili' | 'youtube'
  qualityKey: string
}

export async function playCachedRemoteAudioCandidates(
  candidates: PlaybackCacheReadCandidate[],
  durationHintMs: number,
  useCrossfade: boolean,
  fadeOutMs: number,
  fadeInMs: number,
  requestGeneration: number,
  startPositionMs = 0,
): Promise<CachedRemoteAudioResult | null> {
  if (candidates.length === 0) return null
  return invoke<CachedRemoteAudioResult | null>('play_cached_audio_candidates', {
    request: {
      candidates,
      durationHintMs,
      startPositionMs: Math.max(0, Math.round(startPositionMs)),
      useCrossfade,
      fadeOutMs,
      fadeInMs,
      cacheLimitBytes: playbackCacheLimitBytes(),
      requestGeneration,
    },
  })
}

function playbackCacheLimitBytes(): number {
  const settings = useSettingsStore()
  const cacheSizeMb = Math.min(
    MAX_MEDIA_CACHE_SIZE_MB,
    Math.max(MIN_MEDIA_CACHE_SIZE_MB, settings.maxCacheSize),
  )
  return Math.round(cacheSizeMb * 1024 * 1024)
}
