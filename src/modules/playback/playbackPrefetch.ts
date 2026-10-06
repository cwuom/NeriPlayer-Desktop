import type { TrackInfo } from '@/stores/player'
import {
  isRemotePlaybackTrack,
  playbackPrefetchCacheId,
  playbackResolutionExpiresAt,
  type PlaybackSourceSettings,
  type PlaybackUrlResolver,
  type ResolvedPlaybackSource,
} from './playbackSource'
import { PlaybackDemandArbiter } from './playbackPolicy'

interface PrefetchEntry {
  result: ResolvedPlaybackSource
  expiresAt: number
}

const DEFAULT_TTL_MS = 90_000
const DEFAULT_MAX_ENTRIES = 16

export class PlaybackPrefetchManager {
  readonly demandArbiter = new PlaybackDemandArbiter()

  private readonly entries = new Map<string, PrefetchEntry>()
  // 使用令牌而不是仅保存 Promise，清除后已在途的解析结果不能重新写回缓存
  private readonly jobs = new Map<string, { token: symbol; requestGeneration?: number }>()
  private readonly ttlMs: number
  private readonly maxEntries: number
  private currentDemandKey: string | null = null

  constructor(ttlMs = DEFAULT_TTL_MS, maxEntries = DEFAULT_MAX_ENTRIES) {
    this.ttlMs = Math.max(1, ttlMs)
    this.maxEntries = Math.max(1, maxEntries)
  }

  replacePlaybackDemand(cacheKey: string | null): void {
    this.demandArbiter.replacePlaybackDemand(this.currentDemandKey, cacheKey)
    this.currentDemandKey = cacheKey
  }

  prefetch(
    track: TrackInfo,
    settings: PlaybackSourceSettings,
    resolver: PlaybackUrlResolver,
    requestGeneration?: number,
  ): void {
    if (!isRemotePlaybackTrack(track)) return
    const cacheKey = playbackPrefetchCacheId(track, settings)
    if (this.demandArbiter.shouldYieldPrefetch(cacheKey)) return
    if (this.hasFresh(cacheKey)) return
    const existing = this.jobs.get(cacheKey)
    if (existing && existing.requestGeneration === requestGeneration) return

    const token = Symbol(cacheKey)
    this.jobs.set(cacheKey, { token, requestGeneration })
    void resolver.resolve(track, settings, { requestGeneration }).then((resolution) => {
      if (resolution.type !== 'success') return
      if (this.demandArbiter.shouldYieldPrefetch(cacheKey)) return
      // clearForTrack/clear 可能在解析完成前删除了令牌，此时丢弃旧结果
      if (this.jobs.get(cacheKey)?.token !== token) return
      this.put(cacheKey, resolution)
    }).catch(() => {
      // 预热失败不影响当前播放
    }).finally(() => {
      if (this.jobs.get(cacheKey)?.token === token) this.jobs.delete(cacheKey)
    })
  }

  prefetchWindow(
    tracks: TrackInfo[],
    settings: PlaybackSourceSettings,
    resolver: PlaybackUrlResolver,
    requestGeneration?: number,
  ): void {
    for (const track of tracks) this.prefetch(track, settings, resolver, requestGeneration)
  }

  take(track: TrackInfo, settings: PlaybackSourceSettings): ResolvedPlaybackSource | null {
    const cacheKey = playbackPrefetchCacheId(track, settings)
    const entry = this.entries.get(cacheKey)
    if (!entry) return null
    this.entries.delete(cacheKey)
    return entry.expiresAt > Date.now() ? entry.result : null
  }

  clearForTrack(track: TrackInfo, settings: PlaybackSourceSettings): void {
    const cacheKey = playbackPrefetchCacheId(track, settings)
    this.entries.delete(cacheKey)
    this.jobs.delete(cacheKey)
  }

  clear(): void {
    this.entries.clear()
    this.jobs.clear()
  }

  private hasFresh(cacheKey: string): boolean {
    const entry = this.entries.get(cacheKey)
    if (!entry) return false
    if (entry.expiresAt > Date.now()) return true
    this.entries.delete(cacheKey)
    return false
  }

  private put(cacheKey: string, result: ResolvedPlaybackSource): void {
    this.removeExpired()
    if (!this.entries.has(cacheKey) && this.entries.size >= this.maxEntries) {
      const oldestKey = this.entries.keys().next().value as string | undefined
      if (oldestKey) this.entries.delete(oldestKey)
    }
    this.entries.set(cacheKey, {
      result,
      expiresAt: playbackResolutionExpiresAt(result, Date.now(), this.ttlMs),
    })
  }

  private removeExpired(): void {
    const now = Date.now()
    for (const [key, entry] of this.entries) {
      if (entry.expiresAt <= now) this.entries.delete(key)
    }
  }
}

export const playbackPrefetchManager = new PlaybackPrefetchManager()
