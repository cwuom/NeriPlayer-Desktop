import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { toBackendTrack } from '@/modules/lyrics/syncTrackPayload'
import {
  PLAYBACK_STATS_PERIODIC_FLUSH_MS,
  PlaybackStatsTracker,
  type PlaybackStatsSnapshot,
  type StatsTrackKey,
} from '@/modules/playback/playbackStatsTracker'
import { createLogger } from '@/utils/logger'

const log = createLogger('playbackStats')

export type StatsPeriod = 'day' | 'week' | 'month' | 'year' | 'all'

export const STATS_PERIODS: StatsPeriod[] = ['day', 'week', 'month', 'year', 'all']

export interface TrackStat {
  identityKey: string
  id: string
  name: string
  artist: string
  album: string
  albumId: string
  coverUrl: string | null
  mediaUri: string | null
  durationMs: number
  totalListenMs: number
  playCount: number
  lastPlayedAt: number
  firstPlayedAt: number
}

export interface StatsSummary {
  period: StatsPeriod
  totalPlayCount: number
  totalListenMs: number
  trackCount: number
  items: TrackStat[]
  /** 本期 0..23 时的收听毫秒数；旧版后端没有这个字段 */
  hourListenMs?: number[]
}

interface TrackedPlayback extends StatsTrackKey {
  track: TrackInfo
  /** 开始播放时队列来源的本地歌单；中途换了队列也算回原歌单 */
  localPlaylistId: string | null
}

function emptySummary(period: StatsPeriod): StatsSummary {
  return { period, totalPlayCount: 0, totalListenMs: 0, trackCount: 0, items: [] }
}

export const usePlaybackStatsStore = defineStore('playbackStats', () => {
  const summaries = ref<Record<StatsPeriod, StatsSummary>>({
    day: emptySummary('day'),
    week: emptySummary('week'),
    month: emptySummary('month'),
    year: emptySummary('year'),
    all: emptySummary('all'),
  })
  const loading = ref(false)
  const activePeriod = ref<StatsPeriod>('all')
  const current = computed(() => summaries.value[activePeriod.value])

  const tracker = new PlaybackStatsTracker<TrackedPlayback>()
  let attached = false
  // 每次 attach 换一个代次：listen 是异步注册的，旧代次晚到的注册必须立即注销
  let attachGeneration = 0
  let refreshSequence = 0
  let flushTimer: ReturnType<typeof setInterval> | null = null

  async function submit(snapshot: PlaybackStatsSnapshot<TrackedPlayback> | null) {
    if (!snapshot) return
    try {
      await invoke('record_playback_session', {
        session: {
          // 后端 TrackInfo 是 snake_case 且 duration_ms/url 必填，
          // 直接传前端 camelCase TrackInfo 会反序列化失败被静默吞掉，
          // 必须走 toBackendTrack 转换（与 update_playlist_track 同一约定）
          track: toBackendTrack(snapshot.track.track),
          listenedMs: snapshot.listenedMs,
          playCountIncrement: snapshot.playCountIncrement,
          localPlaylistId: snapshot.track.localPlaylistId,
        },
      })
    } catch (e) {
      // 统计失败绝不能影响播放
      log.warn('record playback session failed:', e)
    }
  }

  function toTracked(track: TrackInfo | null): TrackedPlayback | null {
    if (!track || !track.id) return null
    return {
      // identityKey 只用于本地区分「是不是同一首」，真实身份键由后端派生
      identityKey: `${track.source}:${track.id}`,
      durationMs: track.durationMs || 0,
      track,
      localPlaylistId: usePlayerStore().localPlaylistIdFor(track),
    }
  }

  // 保存所有 watch stop 句柄与 listen unlisten，detach 时全部解除，
  // 避免 attach/detach 循环重复注册监听器并双计统计（MK-05）
  const watchStops: Array<() => void> = []
  const unlistens: Array<() => void> = []

  /// 接入播放器状态，全局只需调用一次
  function attach() {
    if (attached) return
    attached = true
    const generation = ++attachGeneration
    const player = usePlayerStore()

    watchStops.push(watch(
      () => (player.currentTrack ? `${player.currentTrack.source}:${player.currentTrack.id}` : null),
      () => void submit(tracker.onTrackChanged(toTracked(player.currentTrack))),
      { immediate: true },
    ))

    watchStops.push(watch(
      () => player.isPlaying,
      (playing) => void submit(tracker.onPlayingChanged(playing)),
      { immediate: true },
    ))

    // 位置回绕 = 单曲循环走完一轮，Android 同款判定
    watchStops.push(watch(
      () => player.positionMs,
      (position) => void submit(tracker.onProgress(position)),
    ))

    watchStops.push(watch(
      () => player.lastSeekCommand.seq,
      () => tracker.onManualSeek(player.positionMs),
    ))

    watchStops.push(watch(
      () => player.nativeTrackEndCount,
      () => void submit(tracker.onTrackEnded()),
      { flush: 'sync' },
    ))

    // 周期落盘：崩溃或强杀最多只丢一个窗口的收听时长
    flushTimer = setInterval(() => {
      if (tracker.shouldFlushPeriodically()) {
        void submit(tracker.flushPeriodic())
      }
    }, PLAYBACK_STATS_PERIODIC_FLUSH_MS)

    void listen('playback-stats-changed', () => void refresh())
      .then(un => { if (attached && generation === attachGeneration) unlistens.push(un); else un() })
      .catch(error => log.warn('listen playback-stats-changed failed:', error))
  }

  function detach() {
    if (flushTimer) {
      clearInterval(flushTimer)
      flushTimer = null
    }
    watchStops.splice(0).forEach(stop => stop())
    unlistens.splice(0).forEach(un => un())
    attached = false
  }

  async function refresh() {
    // 并发刷新只采用最后一次请求的结果，较早发出的慢响应不能覆盖新数据
    const request = ++refreshSequence
    loading.value = true
    try {
      const overview = await invoke<StatsSummary[]>('get_playback_stats_overview')
      if (request !== refreshSequence) return
      const next = { ...summaries.value }
      for (const summary of overview) next[summary.period] = summary
      summaries.value = next
    } catch (e) {
      if (request === refreshSequence) log.error('load playback stats failed:', e)
    } finally {
      if (request === refreshSequence) loading.value = false
    }
  }

  /// 立刻结算当前收听并刷新，进统计页时调用，避免刚听的歌不计数
  async function flushAndRefresh() {
    await submit(tracker.flushPeriodic())
    await refresh()
  }

  /// 退出前结算，由 App 的关闭流程 await，比 beforeunload 里的 fire-and-forget 可靠
  async function flushFinal() {
    await submit(tracker.flushFinal())
  }

  async function clearAll() {
    await invoke('clear_playback_stats')
    await refresh()
  }

  async function removeTracks(identityKeys: string[]) {
    if (!identityKeys.length) return
    await invoke('remove_playback_stats', { identityKeys })
    await refresh()
  }

  return {
    summaries,
    current,
    activePeriod,
    loading,
    attach,
    detach,
    refresh,
    flushAndRefresh,
    flushFinal,
    clearAll,
    removeTracks,
  }
})
