import { ref, type Ref } from 'vue'
import {
  getPlaybackSourceKind,
  isDirectStreamUrl,
  resolvePlaybackResult,
  type PlaybackSourceSettings,
  type ResolvedPlaybackSource,
} from '@/modules/playback/playbackSource'
import {
  biliShareQualityOrder,
  decorateLtStreamUrl,
  hasLtStreamQuality,
  LT_SHARE_LIMITS,
  ltChannelForSource,
  neteaseShareQualityGroups,
} from '@/stores/listenTogether/streamQuality'
import { createLogger } from '@/utils/logger'
import type { TrackInfo } from '@/types/track'

const log = createLogger('player')

/** 当前会话的直链，以及一起听房主分享给听众的直链 */
export function createStreamShare(
  currentTrack: Ref<TrackInfo | null>,
  playbackSourceSettings: () => PlaybackSourceSettings,
) {
  const currentStreamUrl = ref<string | null>(null)
  let currentResolvedStreamUrls: string[] = []
  // 自己解析出的直链对应的一起听频道和音质，分享给听众时据此加音质标记
  let currentStreamChannel: string | null = null
  let currentStreamQualities = new Map<string, string>()

  function setResolvedStreamUrls(urls: string[]) {
    currentResolvedStreamUrls = urls
  }

  function rememberStreamQualities(resolved: ResolvedPlaybackSource | null) {
    // 直链（包括房间给的）和试听片段的实际音质未知，不标
    currentStreamChannel = resolved && !resolved.isPreview ? ltChannelForSource(resolved.source) : null
    currentStreamQualities = new Map()
    if (!resolved || !currentStreamChannel) return
    currentStreamQualities.set(resolved.url, resolved.audioInfo?.qualityKey ?? resolved.qualityKey)
    for (const detail of resolved.candidateDetails ?? []) {
      currentStreamQualities.set(detail.url, detail.audioInfo?.qualityKey ?? detail.qualityKey)
    }
  }

  /** 分享出去的链接带音质标记（对齐 Android decorateListenTogetherStreamUrl）；已带标记的保持原样 */
  function shareableStreamUrl(url: string): string {
    if (!currentStreamChannel || hasLtStreamQuality(url)) return url
    return decorateLtStreamUrl(url, currentStreamChannel, currentStreamQualities.get(url))
  }

  function getCurrentStreamUrl(trackId?: string): string | null {
    if (!currentTrack.value || (trackId && currentTrack.value.id !== trackId)) return null
    return currentStreamUrl.value ? shareableStreamUrl(currentStreamUrl.value) : null
  }

  function getCurrentStreamUrls(trackId?: string): string[] {
    if (!currentTrack.value || (trackId && currentTrack.value.id !== trackId)) return []
    return currentResolvedStreamUrls.map(shareableStreamUrl)
  }

  /**
   * 一起听房主给听众准备可分享的直链：只解析不播放，也不看本地缓存或下载
   * （对齐 Android resolveShareableStreamUrls）。预览片段和 HLS 分享不了，返回空
   */
  async function resolveShareableStreamUrls(track: TrackInfo): Promise<string[]> {
    try {
      const settings = playbackSourceSettings()
      if (getPlaybackSourceKind(track) === 'netease') return await resolveNeteaseShareableStreamUrls(track, settings)
      const result = await resolvePlaybackResult(track, settings)
      if (result.type !== 'success' || result.isPreview || result.streamType === 'hls') return []
      const channelId = ltChannelForSource(result.source)
      if (!channelId) return []
      const candidates = [
        { url: result.url, quality: result.audioInfo?.qualityKey ?? result.qualityKey, bitrate: result.bitrate ?? 0 },
        ...(result.candidateDetails ?? []).filter(detail => detail.streamType !== 'hls').map(detail => ({
          url: detail.url, quality: detail.audioInfo?.qualityKey ?? detail.qualityKey, bitrate: detail.bitrate ?? 0,
        })),
      ].filter((candidate, index, all) =>
        isDirectStreamUrl(candidate.url) && all.findIndex(other => other.url === candidate.url) === index)
      const chosen = result.source === 'bilibili'
        ? pickBiliShareableStreams(candidates, settings.biliQuality)
        : candidates.slice(0, LT_SHARE_LIMITS[channelId] ?? 1)
      return chosen.map(candidate => decorateLtStreamUrl(candidate.url, channelId, candidate.quality))
    } catch (error) {
      log.warn('resolve shareable stream failed:', error)
      return []
    }
  }

  /**
   * 网易云按音质组各取一条（对齐 Android resolveNeteaseListenTogetherShareableStreams）：
   * 组内依次请求，实际音质已经分享过就试下一档，最多三条
   */
  async function resolveNeteaseShareableStreamUrls(track: TrackInfo, settings: PlaybackSourceSettings): Promise<string[]> {
    const urls: string[] = []
    const qualities = new Set<string>()
    for (const group of neteaseShareQualityGroups(settings.neteaseQuality)) {
      if (urls.length >= (LT_SHARE_LIMITS.netease ?? 3)) break
      for (const quality of group) {
        const result = await resolvePlaybackResult(track, settings, { qualityOverride: quality, allowFallback: false })
          .catch(() => null)
        if (!result || result.type !== 'success' || result.isPreview || result.source !== 'netease') continue
        if (!isDirectStreamUrl(result.url)) continue
        const actual = (result.audioInfo?.qualityKey ?? result.qualityKey ?? quality).toLowerCase()
        if (qualities.has(actual)) continue
        qualities.add(actual)
        urls.push(decorateLtStreamUrl(result.url, 'netease', actual))
        break
      }
    }
    return urls
  }

  /** B 站按偏好、高→中→低、无损的顺序各取码率最高的一条，最多两条（对齐 Android selectBiliListenTogetherShareableStreams） */
  function pickBiliShareableStreams<T extends { url: string; quality: string; bitrate: number }>(
    candidates: T[],
    preferredQuality: string,
  ): T[] {
    const byQuality = new Map<string, T>()
    for (const candidate of [...candidates].sort((left, right) => right.bitrate - left.bitrate)) {
      if (!byQuality.has(candidate.quality)) byQuality.set(candidate.quality, candidate)
    }
    return biliShareQualityOrder(preferredQuality, new Set(byQuality.keys()))
      .map(quality => byQuality.get(quality))
      .filter((candidate): candidate is T => !!candidate)
  }

  return {
    currentStreamUrl, setResolvedStreamUrls, rememberStreamQualities,
    getCurrentStreamUrl, getCurrentStreamUrls, resolveShareableStreamUrls,
  }
}
