import { invoke } from '@tauri-apps/api/core'
import {
  getPlaybackSourceKind,
  isRemotePlaybackTrack,
  playbackCacheReadCandidates,
  playbackPrefetchCacheId,
  playbackUrlResolver,
  type PlaybackCacheReadCandidate,
  type PlaybackSourceSettings,
  type ResolvedPlaybackSource,
} from '@/modules/playback/playbackSource'
import { genericUrlPrefetchTtlMs, playbackPrefetchManager } from '@/modules/playback/playbackPrefetch'
import { initialPlaybackPrefetchWindow } from '@/modules/playback/playbackRequest'
import type { TrackInfo } from '@/types/track'
import type { PlayerCoreState } from './types'

/** 取流地址预热：下一首、悬停意图与列表首屏，当前播放需求始终优先 */
export function createPlaybackPrefetch(ctx: PlayerCoreState) {
  const { settings, queue, queueIndex, repeatMode, shuffleEnabled, shuffleBag, shuffleFuture, durationMs, currentTrack } = ctx

  // URL 预热
  function playbackSourceSettings(): PlaybackSourceSettings {
    return {
      neteaseQuality: settings.neteaseQuality,
      qqMusicQuality: settings.qqMusicQuality,
      biliQuality: settings.biliQuality,
      youtubeQuality: settings.youtubeQuality,
      youtubePlaybackSource: settings.youtubePlaybackSource,
      neteaseAutoSourceSwitch: settings.neteaseAutoSourceSwitch,
      neteaseLocalSourceFallback: settings.neteaseLocalSourceFallback,
    }
  }

  function playbackDemandKey(track: TrackInfo | null): string | null {
    if (!track || !isRemotePlaybackTrack(track)) return null
    return playbackPrefetchCacheId(track, playbackSourceSettings())
  }

  function replacePlaybackDemand(track: TrackInfo | null): void {
    playbackPrefetchManager.replacePlaybackDemand(playbackDemandKey(track))
  }

  function nextPrefetchTracks(): TrackInfo[] {
    // 单曲循环不会切到下一首，预取只会浪费请求（对齐 Android）
    if (!queue.value.length || repeatMode.value === 'one') return []

    const firstIndex = (() => {
      if (shuffleEnabled.value) {
        if (shuffleFuture.value.length > 0) return shuffleFuture.value[shuffleFuture.value.length - 1]
        if (shuffleBag.value.length > 0) return shuffleBag.value[0]
        return -1
      }
      const nextIndex = queueIndex.value + 1
      if (nextIndex < queue.value.length) return nextIndex
      return repeatMode.value === 'all' ? 0 : -1
    })()
    if (firstIndex < 0) return []

    const tracks: TrackInfo[] = []
    // YouTube 只预取后两首（对齐 Android），更多会在 PoToken 队列里挤占当前请求
    const maxWindow = getPlaybackSourceKindForPrefetch(queue.value[firstIndex]) === 'youtube'
      ? 2
      : 1
    if (shuffleEnabled.value) {
      return [...shuffleFuture.value].reverse().concat(shuffleBag.value)
        .slice(0, maxWindow)
        .map(index => queue.value[index])
        .filter(track => !!track && isRemotePlaybackTrack(track))
    }
    let index = firstIndex
    for (let count = 0; count < maxWindow; count += 1) {
      const track = queue.value[index]
      if (track && isRemotePlaybackTrack(track)) tracks.push(track)
      if (maxWindow === 1) break
      index += 1
      if (index >= queue.value.length) {
        if (repeatMode.value !== 'all') break
        index = 0
      }
      if (index === queueIndex.value) break
    }
    return tracks
  }

  function getPlaybackSourceKindForPrefetch(track?: TrackInfo): string | null {
    return track ? getPlaybackSourceKind(track) : null
  }

  /**
   * 预热后续曲目的解析结果，当前播放需求始终拥有优先级。
   * 预取不带播放代际：切歌不会作废它，按下一首时前台解析直接等它完成（对齐 Android join）。
   */
  function maybePrefetchNext() {
    const tracks = nextPrefetchTracks()
    if (tracks.length === 0) return
    const settings = playbackSourceSettings()
    const ttlMs = genericUrlPrefetchTtlMs(Math.max(durationMs.value, currentTrack.value?.durationMs ?? 0))
    // 已完整缓存的曲目播放时直接离线起播，不必预取地址
    void Promise.all(tracks.map(async track => (
      await hasCompleteCachedAudio(playbackCacheReadCandidates(track, settings)) ? null : track
    ))).then((pending) => {
      const uncached = pending.filter((track): track is TrackInfo => track !== null)
      if (uncached.length === 0) return
      playbackPrefetchManager.prefetchWindow(uncached, settings, playbackUrlResolver, undefined, ttlMs)
    })
  }

  // 鼠标停在哪一行、多久以前：解析回来时它还是最新的意图，才值得预开
  let intentTrackId: string | null = null
  let intentAt = 0
  const INTENT_PREWARM_WINDOW_MS = 10_000

  // 只预开紧接着的下一首和鼠标正停着的那首：首包和到 CDN 的连接提前就位，
  // 播放时不再等冷连接（经代理要 3 秒左右）
  playbackPrefetchManager.onPrefetched = (track, result) => {
    if (result.source === 'local' || result.streamType === 'hls' || !result.url) return
    const isIntent = track.id === intentTrackId && Date.now() - intentAt < INTENT_PREWARM_WINDOW_MS
    if (!isIntent && nextPrefetchTracks()[0]?.id !== track.id) return
    void invoke('prewarm_remote_audio', {
      url: result.url,
      durationHintMs: result.durationMs || track.durationMs || 0,
    }).catch(() => {})
  }

  /** 列表行悬停或聚焦：多半马上要点它，提前解析并预开 */
  function prefetchIntent(track: TrackInfo) {
    if (!isRemotePlaybackTrack(track) || track.id === currentTrack.value?.id) return
    intentTrackId = track.id
    intentAt = Date.now()
    playbackPrefetchManager.prefetchIntent(track, playbackSourceSettings(), playbackUrlResolver)
  }

  function prefetchPlaybackTracks(tracks: readonly TrackInfo[]) {
    const candidates = initialPlaybackPrefetchWindow(tracks)
      .filter(track => isRemotePlaybackTrack(track))
    if (candidates.length === 0) return
    playbackPrefetchManager.prefetchWindow(
      [...candidates],
      playbackSourceSettings(),
      playbackUrlResolver,
    )
  }

  function takePrefetchedPlaybackUrl(track: TrackInfo): ResolvedPlaybackSource | null {
    return playbackPrefetchManager.take(track, playbackSourceSettings())
  }

  /** 是否已有完整的磁盘缓存（只查标记与长度，不做完整校验） */
  async function hasCompleteCachedAudio(candidates: readonly PlaybackCacheReadCandidate[]): Promise<boolean> {
    if (candidates.length === 0) return false
    try {
      return await invoke<boolean>('has_cached_audio', { cacheKeys: candidates.map(candidate => candidate.cacheKey) })
    } catch {
      return false
    }
  }

  return {
    playbackSourceSettings, replacePlaybackDemand, nextPrefetchTracks, maybePrefetchNext,
    prefetchIntent, prefetchPlaybackTracks, takePrefetchedPlaybackUrl, hasCompleteCachedAudio,
  }
}
