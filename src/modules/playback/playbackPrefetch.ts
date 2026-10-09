import type { TrackInfo } from '@/stores/player'
import {
  getPlaybackSourceKind,
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

interface QueuedPrefetch {
  cacheKey: string
  token: symbol
  run: () => Promise<void>
}

const DEFAULT_MAX_ENTRIES = 16
const GENERIC_PREFETCH_TTL_FALLBACK_MS = 90_000
const GENERIC_PREFETCH_TTL_PADDING_MS = 30_000
const GENERIC_PREFETCH_TTL_MAX_MS = 10 * 60_000

/**
 * 下一首预取结果的有效期：当前曲目时长 + 30s，最长 10 分钟（对齐 Android resolveGenericUrlPrefetchTtlMs）。
 * 固定 90s 时，任何超过 90s 的曲目播完前预取就过期了。YouTube 仍受签名 expire 限制。
 */
export function genericUrlPrefetchTtlMs(currentDurationMs: number): number {
  if (!Number.isFinite(currentDurationMs) || currentDurationMs <= 0) return GENERIC_PREFETCH_TTL_FALLBACK_MS
  return Math.max(1, Math.min(currentDurationMs + GENERIC_PREFETCH_TTL_PADDING_MS, GENERIC_PREFETCH_TTL_MAX_MS))
}

export class PlaybackPrefetchManager {
  readonly demandArbiter = new PlaybackDemandArbiter()

  private readonly entries = new Map<string, PrefetchEntry>()
  // 使用令牌而不是仅保存 Promise，清除后已在途的解析结果不能重新写回缓存
  private readonly jobs = new Map<string, { token: symbol; requestGeneration?: number }>()
  private readonly ttlMs: number | undefined
  private readonly maxEntries: number
  private currentDemandKey: string | null = null
  private intentJob: { cacheKey: string; token: symbol } | null = null
  /** 预取解析成功、结果已入缓存后调用；播放层据此预开下一首的直链 */
  onPrefetched: ((track: TrackInfo, result: ResolvedPlaybackSource) => void) | null = null
  // YouTube 预取串行执行（对齐 Android 单许可的预取闸门），避免抢在用户要听的曲目前面排队取 PoToken；
  // 被取代或清除的任务立即让出名额
  private readonly youtubeQueue: QueuedPrefetch[] = []
  private youtubeSlot: { cacheKey: string; token: symbol } | null = null

  /** ttlMs 省略时按来源的解析有效期（YouTube 8 分钟，其余 90 秒） */
  constructor(ttlMs?: number, maxEntries = DEFAULT_MAX_ENTRIES) {
    this.ttlMs = ttlMs === undefined ? undefined : Math.max(1, ttlMs)
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
    ttlMs?: number,
  ): void {
    if (!isRemotePlaybackTrack(track)) return
    const cacheKey = playbackPrefetchCacheId(track, settings)
    if (this.demandArbiter.shouldYieldPrefetch(cacheKey)) return
    if (this.hasFresh(cacheKey)) return
    const existing = this.jobs.get(cacheKey)
    if (existing && existing.requestGeneration === requestGeneration) return

    const token = Symbol(cacheKey)
    this.jobs.set(cacheKey, { token, requestGeneration })
    const run = () => resolver.resolve(track, settings, { requestGeneration }).then((resolution) => {
      if (resolution.type !== 'success') return
      if (this.demandArbiter.shouldYieldPrefetch(cacheKey)) return
      // clearForTrack/clear 可能在解析完成前删除了令牌，此时丢弃旧结果
      if (this.jobs.get(cacheKey)?.token !== token) return
      this.put(cacheKey, resolution, ttlMs)
      this.onPrefetched?.(track, resolution)
    }).catch(() => {
      // 预热失败不影响当前播放
    }).finally(() => {
      if (this.jobs.get(cacheKey)?.token === token) this.jobs.delete(cacheKey)
    })

    if (getPlaybackSourceKind(track) !== 'youtube') {
      void run()
      return
    }
    for (let index = this.youtubeQueue.length - 1; index >= 0; index--) {
      if (this.youtubeQueue[index].cacheKey === cacheKey) this.youtubeQueue.splice(index, 1)
    }
    this.youtubeQueue.push({ cacheKey, token, run })
    this.pumpYoutubeQueue()
  }

  /**
   * 悬停、聚焦等「可能马上要点」的预取：只留最新的一个。鼠标扫过一长串行时，
   * 还在排队的旧意图直接撤掉，不在 YouTube 单许可队列里堵住真正要点的那首。
   * 已经预取过的立即回报 onPrefetched，让播放层照样预开直链
   */
  prefetchIntent(
    track: TrackInfo,
    settings: PlaybackSourceSettings,
    resolver: PlaybackUrlResolver,
  ): void {
    if (!isRemotePlaybackTrack(track)) return
    const cacheKey = playbackPrefetchCacheId(track, settings)
    const previous = this.intentJob
    this.intentJob = null
    // 只撤意图自己发起、还在排队的那个任务；同一首若已被下一首预取排上，那是别人的任务
    if (previous && previous.cacheKey !== cacheKey && this.jobs.get(previous.cacheKey)?.token === previous.token) {
      const queued = this.youtubeQueue.findIndex(entry => entry.token === previous.token)
      if (queued >= 0) {
        this.youtubeQueue.splice(queued, 1)
        this.jobs.delete(previous.cacheKey)
      }
    }
    const fresh = this.hasFresh(cacheKey) ? this.entries.get(cacheKey) : undefined
    if (fresh) {
      this.onPrefetched?.(track, fresh.result)
      return
    }
    const existing = this.jobs.get(cacheKey)?.token
    this.prefetch(track, settings, resolver)
    const created = this.jobs.get(cacheKey)?.token
    if (created && created !== existing) this.intentJob = { cacheKey, token: created }
  }

  prefetchWindow(
    tracks: TrackInfo[],
    settings: PlaybackSourceSettings,
    resolver: PlaybackUrlResolver,
    requestGeneration?: number,
    ttlMs?: number,
  ): void {
    for (const track of tracks) this.prefetch(track, settings, resolver, requestGeneration, ttlMs)
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
    this.pumpYoutubeQueue()
  }

  clear(): void {
    this.entries.clear()
    this.jobs.clear()
    this.youtubeQueue.length = 0
    this.youtubeSlot = null
  }

  private youtubeSlotBusy(): boolean {
    const slot = this.youtubeSlot
    return !!slot && this.jobs.get(slot.cacheKey)?.token === slot.token
  }

  private pumpYoutubeQueue(): void {
    while (!this.youtubeSlotBusy() && this.youtubeQueue.length > 0) {
      const next = this.youtubeQueue.shift()!
      if (this.jobs.get(next.cacheKey)?.token !== next.token) continue
      // 排队期间这首已成为当前播放需求：前台解析自己处理，不再预取
      if (this.demandArbiter.shouldYieldPrefetch(next.cacheKey)) {
        this.jobs.delete(next.cacheKey)
        continue
      }
      const slot = { cacheKey: next.cacheKey, token: next.token }
      this.youtubeSlot = slot
      void next.run().finally(() => {
        if (this.youtubeSlot === slot) this.youtubeSlot = null
        this.pumpYoutubeQueue()
      })
    }
  }

  private hasFresh(cacheKey: string): boolean {
    const entry = this.entries.get(cacheKey)
    if (!entry) return false
    if (entry.expiresAt > Date.now()) return true
    this.entries.delete(cacheKey)
    return false
  }

  private put(cacheKey: string, result: ResolvedPlaybackSource, ttlMs?: number): void {
    this.removeExpired()
    if (!this.entries.has(cacheKey) && this.entries.size >= this.maxEntries) {
      // 满了淘汰最快过期的一条
      let soonestKey: string | null = null
      let soonestExpiry = Infinity
      for (const [key, entry] of this.entries) {
        if (entry.expiresAt < soonestExpiry) {
          soonestExpiry = entry.expiresAt
          soonestKey = key
        }
      }
      if (soonestKey) this.entries.delete(soonestKey)
    }
    this.entries.set(cacheKey, {
      result,
      expiresAt: playbackResolutionExpiresAt(result, Date.now(), ttlMs ?? this.ttlMs),
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
