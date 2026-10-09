import { onUnmounted, watch } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { prefetchTrackLyrics } from '@/modules/lyrics/lyricsPrefetch'
import { lyricsIdentity } from '@/modules/lyrics/lyricsRequest'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('lyrics-prefetch')

// 等当前曲目起播稳定后再取，不和它的首包、歌词抢连接
const PREFETCH_DELAY_MS = 6_000
const MAX_REMEMBERED = 200

/** 当前曲目播放一会儿后预取队列里下一首的歌词；同一首在本次运行里只取一次 */
export function useUpcomingLyricsPrefetch(): void {
  const player = usePlayerStore()
  const settings = useSettingsStore()
  const attempted = new Set<string>()
  let timer: ReturnType<typeof setTimeout> | null = null

  function cancel() {
    if (timer) clearTimeout(timer)
    timer = null
  }

  async function run() {
    timer = null
    const track = player.upcomingTrack()
    if (!track || !player.isPlaying) return
    const key = `${lyricsIdentity(track)}|${settings.defaultLyricSource}|${settings.preferWordTimedLyrics}`
    if (attempted.has(key)) return
    attempted.add(key)
    if (attempted.size > MAX_REMEMBERED) {
      const oldest = attempted.values().next().value
      if (oldest !== undefined) attempted.delete(oldest)
    }
    try {
      const outcome = await prefetchTrackLyrics(track, {
        playbackSource: getPlaybackSourceKind(track),
        defaultLyricSource: settings.defaultLyricSource,
        preferWordTimed: settings.preferWordTimedLyrics,
      })
      log.info('upcoming lyrics prefetch:', { trackId: track.id, outcome })
    } catch (error) {
      // 网络失败不记住，下一次起播再试
      attempted.delete(key)
      log.warn('upcoming lyrics prefetch failed:', summarizeLogError(error))
    }
  }

  watch(
    () => [
      player.currentTrack?.id ?? '',
      player.isPlaying && !player.isLoadingAudio,
      player.upcomingTrack()?.id ?? '',
    ] as const,
    ([currentId, steady, upcomingId]) => {
      cancel()
      if (!currentId || !steady || !upcomingId) return
      timer = setTimeout(() => { void run() }, PREFETCH_DELAY_MS)
    },
    { immediate: true },
  )

  onUnmounted(cancel)
}
