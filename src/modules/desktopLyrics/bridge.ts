import { ref, watch, type WatchStopHandle } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { usePlayerStore, type LyricLine, type TrackInfo } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import { offsetBucketForSource } from '@/modules/lyrics/lyricOffset'
import { getCachedLyrics, saveCachedLyrics } from '@/modules/lyrics/lyricsCache'
import { loadLyricsSingleFlight, hasWordTimedLyrics } from '@/modules/lyrics/lyricsRequest'
import { mapBackendLyrics, mergeParsedLyricsWithTranslations, mergeWordTimedLyricsWithBaseline, resolveStoredLyricStateFromPayload, resolveStoredTranslatedLyricStateFromPayload } from '@/modules/lyrics/lyricsFormat'
import { buildDesktopLyricsFrame, type DesktopLyricsFrame } from './frame'
import { createDesktopLyricsLoader } from './loader'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('desktop-lyrics')
let installed: { open: () => Promise<void>; dispose: () => void } | null = null

async function materialize(track: TrackInfo): Promise<LyricLine[] | null> {
  const stored = resolveStoredLyricStateFromPayload(track.syncPayload)
  if (stored.kind === 'absent') return null
  if (stored.kind === 'cleared') return []
  const parsed = mapBackendLyrics(await invoke<any[]>('parse_lrc_content', { content: stored.text }))
  const translation = resolveStoredTranslatedLyricStateFromPayload(track.syncPayload)
  if (translation.kind !== 'present' || !translation.text.trim()) return parsed
  try {
    const translated = mapBackendLyrics(await invoke<any[]>('parse_lrc_content', { content: translation.text }))
    return mergeParsedLyricsWithTranslations(parsed, translated)
  } catch {
    return parsed
  }
}

function source(track: TrackInfo | null): string {
  const prefix = track?.id.split(':', 1)[0]
  return prefix === 'netease' || prefix === 'qq' || prefix === 'bilibili' || prefix === 'youtube' ? prefix : 'local'
}

export async function openDesktopLyricsWindow(): Promise<void> {
  if (!installed) throw new Error('Desktop lyrics bridge is not ready')
  await installed.open()
}

export function installDesktopLyricsBridge(): () => void {
  if (installed) return () => {}
  const player = usePlayerStore()
  const settings = useSettingsStore()
  const offsets = useLyricOffsetStore()
  const lines = ref<LyricLine[]>([])
  let active = false
  let disposed = false
  let timer: ReturnType<typeof setInterval> | null = null
  let stopTrack: WatchStopHandle | null = null
  let loader: ReturnType<typeof createDesktopLyricsLoader> | null = null
  let pending: { sessionId: string; frame: DesktopLyricsFrame } | null = null
  let sessionId = ''
  let windowReady = false
  let openTask: Promise<void> | null = null
  let publishing = false
  let lastSent = ''
  let loggedFailure = false
  let unlisten: UnlistenFn | null = null

  async function drain() {
    if (publishing) return
    publishing = true
    try {
      while (active && pending) {
        const message = pending
        pending = null
        const key = JSON.stringify(message)
        if (key === lastSent) continue
        try {
          await invoke('publish_desktop_lyrics', message)
          lastSent = key
        } catch (error) {
          if (!loggedFailure) log.warn('frame publication failed:', summarizeLogError(error))
          loggedFailure = true
        }
      }
    } finally {
      publishing = false
    }
  }

  function publish() {
    if (!active || !windowReady) return
    const track = player.currentTrack
    pending = { sessionId, frame: buildDesktopLyricsFrame({
      track,
      lines: lines.value,
      positionMs: player.interpolatedPositionMs,
      lyricOffsetMs: offsets.effectiveOffsetMs(track, offsetBucketForSource(source(track))),
      isPlaying: player.isPlaying,
    }) }
    void drain()
  }

  function deactivate() {
    active = false
    windowReady = false
    pending = null
    stopTrack?.()
    stopTrack = null
    if (timer) clearInterval(timer)
    timer = null
    loader?.dispose()
    loader = null
    lines.value = []
    lastSent = ''
  }

  const listenersReady = listen<{ sessionId: string }>('desktop-lyrics:closed', event => {
    if (event.payload.sessionId === sessionId) deactivate()
  }).then(release => {
    if (disposed) release()
    else unlisten = release
  }).catch(error => {
    log.warn('window listener unavailable:', summarizeLogError(error))
    throw error
  })
  // 安装失败由打开操作返回，避免应用启动时出现未处理的 rejection
  void listenersReady.catch(() => {})

  function activate() {
    if (active) return
    active = true
    sessionId = crypto.randomUUID()
    loggedFailure = false
    loader = createDesktopLyricsLoader({
      materialize,
      mergeUpgrade: mergeWordTimedLyricsWithBaseline,
      cached: getCachedLyrics,
      cache: saveCachedLyrics,
      onChange: value => { lines.value = value; publish() },
      fetch: track => loadLyricsSingleFlight(track, async () => mapBackendLyrics(await invoke<any[]>('fetch_lyrics', {
        title: track.title, artist: track.artist,
        durationSecs: Math.floor((track.durationMs || 0) / 1000), audioPath: track.audioUrl || null,
        neteaseId: source(track) === 'netease' ? Number(track.id.slice(8)) || null : null,
        qqSongMid: source(track) === 'qq' ? track.id.slice(3) : null,
        youtubeVideoId: source(track) === 'youtube' ? track.id.slice(8) : null,
      }))),
      canUpgrade: (track, baseline) => settings.advancedLyrics && source(track) !== 'local'
        && !hasWordTimedLyrics(baseline) && resolveStoredLyricStateFromPayload(track.syncPayload).kind === 'absent',
      upgrade: track => loadLyricsSingleFlight(track, async () => {
        const result = mapBackendLyrics(await invoke<any[]>('fetch_word_timed_lyrics', {
          title: track.title, artist: track.artist, durationMs: track.durationMs || 0,
        }))
        return hasWordTimedLyrics(result) ? result : []
      }, 'word-timed'),
    })
    stopTrack = watch(
      [() => player.currentTrack, () => player.lyrics, () => settings.advancedLyrics],
      () => { void loader?.load(player.currentTrack, player.lyrics) },
      { deep: true, immediate: true },
    )
    timer = setInterval(publish, 150)
    publish()
  }

  const instance = {
    open(): Promise<void> {
      if (openTask) return openTask
      const task = (async () => {
        await listenersReady
        if (disposed) return
        activate()
        const openingSession = sessionId
        try {
          await invoke('open_desktop_lyrics', { sessionId: openingSession })
          if (disposed || !active || sessionId !== openingSession) {
            await invoke('close_desktop_lyrics', { sessionId: openingSession })
            return
          }
          windowReady = true
          publish()
        } catch (error) {
          deactivate()
          throw error
        }
      })()
      openTask = task
      const release = () => { if (openTask === task) openTask = null }
      task.then(release, release)
      return task
    },
    dispose() {
      const closingSession = active ? sessionId : ''
      disposed = true
      deactivate()
      unlisten?.()
      unlisten = null
      if (installed === instance) installed = null
      if (closingSession && !openTask) {
        void invoke('close_desktop_lyrics', { sessionId: closingSession }).catch(error => {
          log.warn('window cleanup failed:', summarizeLogError(error))
        })
      }
    },
  }
  installed = instance
  return instance.dispose
}
