import { ref, watch, type Ref, type WatchStopHandle } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { usePlayerStore, type LyricLine, type TrackInfo } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import { readSyncedLyricSource } from '@/modules/lyrics/lyricOffset'
import {
  fetchAutomaticLyrics,
  fetchNeteaseRomanization,
  fetchPreferredSourceLyrics,
  fetchWordTimedLyrics,
  preferredLyricMatchSource,
} from '@/modules/lyrics/lyricsFetch'
import { rememberLyricSource } from '@/modules/lyrics/lyricSource'
import { getCachedLyrics, saveCachedLyrics } from '@/modules/lyrics/lyricsCache'
import { loadLyricsSingleFlight, hasWordTimedLyrics } from '@/modules/lyrics/lyricsRequest'
import {
  mapBackendLyrics,
  materializeStoredLyrics,
  mergeParsedLyricsWithRomanization,
  mergeWordTimedLyricsWithBaseline,
  resolveKnownNeteaseLyricSongId,
  resolveStoredLyricStateFromPayload,
  shouldBackfillNeteaseRomanization,
} from '@/modules/lyrics/lyricsFormat'
import { buildDesktopLyricsFrame, desktopLyricsLineIndex, type DesktopLyricsFrame } from './frame'
import { createDesktopLyricsLoader } from './loader'
import {
  nextDesktopLyricsLayout,
  normalizeDesktopLyricsStyle,
  parseCssColor,
  stepDesktopLyricsFontSize,
  type DesktopLyricsStyle,
} from './style'
import { anchoredPositionMs, type PlaybackAnchor } from './timeline'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('desktop-lyrics')
let installed: { open: () => Promise<void>; close: () => Promise<void>; dispose: () => void } | null = null

/** 桌面歌词窗口是否开着；设置页和「更多」面板据此显示开关 */
export const desktopLyricsOpen: Ref<boolean> = ref(false)

/** 两次帧之间允许的估算误差；超过说明 seek 或缓冲了，马上发新锚点 */
const DRIFT_TOLERANCE_MS = 80
/** 没有变化时也定期刷新锚点，抵消两边插值的累计误差 */
const ANCHOR_REFRESH_MS = 3_000

async function materialize(track: TrackInfo): Promise<LyricLine[] | null> {
  const lines = await materializeStoredLyrics(
    track.syncPayload,
    async (content, part) => mapBackendLyrics(await invoke<any[]>('parse_lrc_content', part === 'original'
      ? { content, title: track.title, artist: track.artist }
      : { content })),
    error => log.warn('stored translation or romanization not parsed:', summarizeLogError(error)),
  )
  if (lines?.length) rememberLyricSource(track, readSyncedLyricSource(track.syncPayload))
  return lines
}

function source(track: TrackInfo | null): string {
  const prefix = track?.id.split(':', 1)[0]
  return prefix === 'netease' || prefix === 'qq' || prefix === 'bilibili' || prefix === 'youtube' ? prefix : 'local'
}

/** 应用主题色（开了动态取色时就是封面色），「跟随主题色」用 */
function readAccent(): string | null {
  if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') return null
  return parseCssColor(getComputedStyle(document.documentElement).getPropertyValue('--md-primary'))
}

export async function openDesktopLyricsWindow(): Promise<void> {
  if (!installed) throw new Error('Desktop lyrics bridge is not ready')
  await installed.open()
}

export async function closeDesktopLyricsWindow(): Promise<void> {
  await installed?.close()
}

export interface DesktopLyricsBridgeOptions {
  /** 工具栏「设置」：主窗口跳到桌面歌词设置 */
  openSettings?: () => void
}

export function installDesktopLyricsBridge(options: DesktopLyricsBridgeOptions = {}): () => void {
  if (installed) return () => {}
  const player = usePlayerStore()
  const settings = useSettingsStore()
  const offsets = useLyricOffsetStore()
  const lines = ref<LyricLine[]>([])
  let active = false
  let disposed = false
  let timer: ReturnType<typeof setInterval> | null = null
  let stopTrack: WatchStopHandle | null = null
  let stopLock: WatchStopHandle | null = null
  let loader: ReturnType<typeof createDesktopLyricsLoader> | null = null
  let pending: { sessionId: string; frame: DesktopLyricsFrame } | null = null
  let sessionId = ''
  let windowReady = false
  let openTask: Promise<void> | null = null
  let publishing = false
  let lastSent = ''
  let loggedFailure = false
  let unlisten: UnlistenFn[] = []
  /** 上一帧的要点：没变化就不发 */
  let published: {
    trackId: string
    index: number
    offset: number
    styleKey: string
    accent: string | null
    anchor: PlaybackAnchor
  } | null = null

  function style(): DesktopLyricsStyle {
    return normalizeDesktopLyricsStyle(settings.desktopLyrics)
  }

  function updateStyle(change: (current: DesktopLyricsStyle) => DesktopLyricsStyle) {
    settings.desktopLyrics = normalizeDesktopLyricsStyle(change(style()))
  }

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

  // 同步载荷里的歌词没带音译时，和播放页一样去网易云补上（音译模式下桌面歌词才有第二行）
  async function backfillRomanization(track: TrackInfo, synced: LyricLine[]) {
    if (!shouldBackfillNeteaseRomanization(track.syncPayload, synced)) return
    try {
      const text = await fetchNeteaseRomanization(track, resolveKnownNeteaseLyricSongId(track))
      if (!text) return
      const roman = mapBackendLyrics(await invoke<any[]>('parse_lrc_content', { content: text }))
      const current = lines.value
      if (!active || player.currentTrack?.id !== track.id || current.length !== synced.length || current.some(line => line.roman)) return
      lines.value = mergeParsedLyricsWithRomanization(current, roman)
      publish(true)
    } catch (error) {
      log.warn('romanization backfill unavailable:', summarizeLogError(error))
    }
  }

  /**
   * 歌词窗口按锚点自己插值，这里只在需要时发帧：换行、换歌、暂停/播放、倍速、偏移、外观变化，
   * 估算位置与实际相差超过 80 ms（seek、缓冲），或距上次超过 3 秒
   */
  function publish(force = false) {
    if (!active || !windowReady) return
    const track = player.currentTrack
    const now = Date.now()
    // 主窗口最小化时 rAF 暂停，interpolatedPositionMs 会停住；按锚点现算
    const positionMs = player.livePositionMs()
    const offset = offsets.effectiveOffsetMs(track)
    const rate = typeof player.effectivePlaybackSpeed === 'function' ? player.effectivePlaybackSpeed() : 1
    const currentStyle = style()
    const styleKey = JSON.stringify(currentStyle)
    const accent = currentStyle.theme === 'accent' ? readAccent() : null
    const index = desktopLyricsLineIndex(lines.value, positionMs, offset)
    const trackId = track?.id ?? ''
    const anchor: PlaybackAnchor = { positionMs, anchorAt: now, rate, isPlaying: player.isPlaying }
    const previous = published
    const due = force
      || !previous
      || previous.trackId !== trackId
      || previous.index !== index
      || previous.offset !== offset
      || previous.styleKey !== styleKey
      || previous.accent !== accent
      || previous.anchor.isPlaying !== anchor.isPlaying
      || previous.anchor.rate !== anchor.rate
      || Math.abs(anchoredPositionMs(previous.anchor, now) - positionMs) > DRIFT_TOLERANCE_MS
      || now - previous.anchor.anchorAt > ANCHOR_REFRESH_MS
    if (!due) return
    published = { trackId, index, offset, styleKey, accent, anchor }
    pending = { sessionId, frame: buildDesktopLyricsFrame({
      track,
      lines: lines.value,
      positionMs,
      lyricOffsetMs: offset,
      isPlaying: anchor.isPlaying,
      rate,
      now,
      accent,
      style: currentStyle,
    }) }
    void drain()
  }

  function applyLock() {
    if (!active || !windowReady) return
    const locked = style().locked
    void invoke('set_desktop_lyrics_lock', { sessionId, locked }).catch(error => {
      log.warn('lock not applied:', summarizeLogError(error))
    })
  }

  function handleAction(action: string) {
    if (!active) return
    switch (action) {
      case 'toggle-play':
        void (player.isPlaying ? player.pause() : player.resume())
        break
      case 'previous':
        void player.previous()
        break
      case 'next':
        void player.next()
        break
      case 'font-smaller':
        updateStyle(current => stepDesktopLyricsFontSize(current, -1))
        break
      case 'font-larger':
        updateStyle(current => stepDesktopLyricsFontSize(current, 1))
        break
      case 'cycle-layout':
        updateStyle(current => ({ ...current, layout: nextDesktopLyricsLayout(current.layout) }))
        break
      case 'lock':
      case 'unlock':
        updateStyle(current => ({ ...current, locked: action === 'lock' }))
        break
      case 'open-settings':
        options.openSettings?.()
        break
      default:
        log.warn('unknown toolbar action:', action)
    }
  }

  function deactivate() {
    active = false
    windowReady = false
    desktopLyricsOpen.value = false
    pending = null
    published = null
    stopTrack?.()
    stopTrack = null
    stopLock?.()
    stopLock = null
    if (timer) clearInterval(timer)
    timer = null
    loader?.dispose()
    loader = null
    lines.value = []
    lastSent = ''
  }

  const listenersReady = Promise.all([
    listen<{ sessionId: string }>('desktop-lyrics:closed', event => {
      if (event.payload.sessionId === sessionId) deactivate()
    }),
    listen<{ action: string }>('desktop-lyrics:action', event => handleAction(String(event.payload?.action ?? ''))),
    listen<unknown>('desktop-lyrics:bounds', event => {
      if (!active) return
      updateStyle(current => ({ ...current, bounds: normalizeDesktopLyricsStyle({ bounds: event.payload }).bounds ?? current.bounds }))
    }),
  ]).then(releases => {
    if (disposed) releases.forEach(release => release())
    else unlisten = releases
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
      materialize: async track => {
        const synced = await materialize(track)
        if (synced?.length) void backfillRomanization(track, synced)
        return synced
      },
      mergeUpgrade: mergeWordTimedLyricsWithBaseline,
      cached: getCachedLyrics,
      cache: saveCachedLyrics,
      onChange: value => { lines.value = value; publish(true) },
      fetch: async track => {
        const preferredSource = preferredLyricMatchSource(source(track) === 'local' ? null : source(track), settings.defaultLyricSource)
        if (preferredSource) {
          const preferred = await fetchPreferredSourceLyrics(track, preferredSource, settings.preferWordTimedLyrics)
            .catch(error => { log.warn('preferred lyric source unavailable:', summarizeLogError(error)); return null })
          if (preferred) {
            rememberLyricSource(track, preferred.source)
            return preferred.lines
          }
        }
        const fetched = await loadLyricsSingleFlight(track, () => fetchAutomaticLyrics(
          track, source(track), settings.preferWordTimedLyrics,
        ))
        if (fetched.lines.length) rememberLyricSource(track, fetched.source)
        return fetched.lines
      },
      canUpgrade: (track, baseline) => settings.preferWordTimedLyrics && settings.defaultLyricSource === 'automatic'
        && source(track) !== 'local'
        && !hasWordTimedLyrics(baseline) && resolveStoredLyricStateFromPayload(track.syncPayload).kind === 'absent',
      upgrade: async track => {
        const fetched = await loadLyricsSingleFlight(track, () => fetchWordTimedLyrics({
          title: track.title, artist: track.artist, durationMs: track.durationMs || 0,
        }), 'word-timed')
        return hasWordTimedLyrics(fetched.lines) ? fetched : { source: null, lines: [] }
      },
      adoptSource: rememberLyricSource,
    })
    stopTrack = watch(
      [() => player.currentTrack, () => player.lyrics, () => settings.preferWordTimedLyrics, () => settings.defaultLyricSource],
      () => { void loader?.load(player.currentTrack, player.lyrics) },
      { deep: true, immediate: true },
    )
    stopLock = watch(() => style().locked, () => applyLock())
    timer = setInterval(() => publish(), 150)
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
          await invoke('open_desktop_lyrics', { sessionId: openingSession, bounds: style().bounds })
          if (disposed || !active || sessionId !== openingSession) {
            await invoke('close_desktop_lyrics', { sessionId: openingSession })
            return
          }
          windowReady = true
          desktopLyricsOpen.value = true
          publish(true)
          applyLock()
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
    async close(): Promise<void> {
      if (!active) return
      const closingSession = sessionId
      await invoke('close_desktop_lyrics', { sessionId: closingSession })
    },
    dispose() {
      const closingSession = active ? sessionId : ''
      disposed = true
      deactivate()
      unlisten.forEach(release => release())
      unlisten = []
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
