import { watch, type WatchStopHandle } from 'vue'
import { emitTo, listen, type UnlistenFn } from '@tauri-apps/api/event'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { LyricLine, TrackInfo, usePlayerStore } from '@/stores/player'
import type { useSettingsStore } from '@/stores/settings'
import type { useLyricOffsetStore } from '@/stores/lyricOffset'
import { offsetBucketForSource } from '@/modules/lyrics/lyricOffset'
import { loadTrackLyrics } from '@/modules/lyrics/loadTrackLyrics'

export const DESKTOP_LYRICS_WINDOW_LABEL = 'desktop-lyrics'
// The legacy state event remains the full hydration payload; playback ticks are lightweight.
export const DESKTOP_LYRICS_STATE_EVENT = 'desktop-lyrics:state'
export const DESKTOP_LYRICS_PLAYBACK_EVENT = 'desktop-lyrics:playback'
export const DESKTOP_LYRICS_READY_EVENT = 'desktop-lyrics:ready'
export const DESKTOP_LYRICS_HIDDEN_EVENT = 'desktop-lyrics:hidden'
export const DESKTOP_LYRICS_CONTROL_EVENT = 'desktop-lyrics:control'

const MAIN_WINDOW_LABEL = 'main'
const PLAYBACK_THROTTLE_MS = 80

export interface DesktopLyricsSnapshot {
  track: Pick<TrackInfo, 'id' | 'title' | 'artist'> | null
  lyrics: LyricLine[]
  positionMs: number
  durationMs: number
  lyricOffsetMs: number
  isPlaying: boolean
  isLoadingAudio: boolean
  isLoadingLyrics: boolean
  showTranslation: boolean
}

export interface DesktopLyricsPlaybackState {
  trackId: string | null
  positionMs: number
  durationMs: number
  isPlaying: boolean
  isLoadingAudio: boolean
}

export type DesktopLyricsControl = 'previous' | 'toggle' | 'next'

type PlayerStore = ReturnType<typeof usePlayerStore>
type SettingsStore = ReturnType<typeof useSettingsStore>
type LyricOffsetStore = ReturnType<typeof useLyricOffsetStore>

function trackSource(track: TrackInfo | null): string {
  if (!track) return 'local'
  const separator = track.id.indexOf(':')
  return separator > 0 ? track.id.slice(0, separator) : (track.source || 'local')
}

export async function openDesktopLyricsWindow(): Promise<void> {
  const existing = await WebviewWindow.getByLabel(DESKTOP_LYRICS_WINDOW_LABEL)
  if (existing) {
    await existing.show()
    await existing.setAlwaysOnTop(true)
    await existing.setFocus()
    await emitTo(MAIN_WINDOW_LABEL, DESKTOP_LYRICS_READY_EVENT)
    return
  }

  await new Promise<void>((resolve, reject) => {
    const desktopLyricsWindow = new WebviewWindow(DESKTOP_LYRICS_WINDOW_LABEL, {
      url: 'index.html?window=desktop-lyrics',
      title: 'NeriPlayer Desktop Lyrics',
      width: 720,
      height: 156,
      minWidth: 420,
      minHeight: 112,
      center: true,
      resizable: true,
      focus: true,
      visible: false,
      transparent: true,
      decorations: false,
      alwaysOnTop: true,
      skipTaskbar: true,
      shadow: false,
    })
    void desktopLyricsWindow.once('tauri://created', () => resolve())
    void desktopLyricsWindow.once('tauri://error', event => reject(new Error(String(event.payload))))
  })
}

export async function startDesktopLyricsBridge(
  player: PlayerStore,
  settings: SettingsStore,
  lyricOffsetStore: LyricOffsetStore,
): Promise<() => void> {
  let active = false
  let lyrics: LyricLine[] = []
  let isLoadingLyrics = false
  let loadGeneration = 0
  let playbackTimer: ReturnType<typeof setTimeout> | null = null
  let unlistenReady: UnlistenFn | null = null
  let unlistenHidden: UnlistenFn | null = null
  let unlistenControl: UnlistenFn | null = null
  const stopWatches: WatchStopHandle[] = []

  function playbackSnapshot(): DesktopLyricsPlaybackState {
    return {
      trackId: player.currentTrack?.id ?? null,
      positionMs: player.interpolatedPositionMs,
      durationMs: player.durationMs,
      isPlaying: player.isPlaying,
      isLoadingAudio: player.isLoadingAudio,
    }
  }

  function stateSnapshot(): DesktopLyricsSnapshot {
    const track = player.currentTrack
    const bucket = offsetBucketForSource(trackSource(track))
    return {
      track: track ? { id: track.id, title: track.title, artist: track.artist } : null,
      lyrics,
      positionMs: player.interpolatedPositionMs,
      durationMs: player.durationMs,
      lyricOffsetMs: lyricOffsetStore.effectiveOffsetMs(track, bucket),
      isPlaying: player.isPlaying,
      isLoadingAudio: player.isLoadingAudio,
      isLoadingLyrics,
      showTranslation: settings.showTranslation,
    }
  }

  function emitState() {
    if (!active) return
    void emitTo(DESKTOP_LYRICS_WINDOW_LABEL, DESKTOP_LYRICS_STATE_EVENT, stateSnapshot()).catch(() => {})
  }

  function emitPlayback() {
    if (!active) return
    void emitTo(
      DESKTOP_LYRICS_WINDOW_LABEL,
      DESKTOP_LYRICS_PLAYBACK_EVENT,
      playbackSnapshot(),
    ).catch(() => {})
  }

  function schedulePlayback() {
    if (!active || playbackTimer) return
    playbackTimer = setTimeout(() => {
      playbackTimer = null
      emitPlayback()
    }, PLAYBACK_THROTTLE_MS)
  }

  async function refreshLyrics() {
    const generation = ++loadGeneration
    const track = player.currentTrack
    lyrics = []
    isLoadingLyrics = !!track
    emitState()
    if (!track) return

    try {
      const nextLyrics = await loadTrackLyrics(track, player.durationMs)
      if (generation !== loadGeneration || player.currentTrack?.id !== track.id) return
      lyrics = nextLyrics
    } catch {
      if (generation !== loadGeneration) return
      lyrics = []
    } finally {
      if (generation === loadGeneration) {
        isLoadingLyrics = false
        emitState()
      }
    }
  }

  async function activate() {
    active = true
    await refreshLyrics()
  }

  try {
    unlistenReady = await listen(DESKTOP_LYRICS_READY_EVENT, () => {
      void activate()
    })
  } catch {}
  try {
    unlistenHidden = await listen(DESKTOP_LYRICS_HIDDEN_EVENT, () => {
      active = false
      loadGeneration++
      if (playbackTimer) clearTimeout(playbackTimer)
      playbackTimer = null
    })
  } catch {}
  try {
    unlistenControl = await listen<DesktopLyricsControl>(DESKTOP_LYRICS_CONTROL_EVENT, event => {
      switch (event.payload) {
        case 'previous':
          void player.previous()
          break
        case 'toggle':
          void player.togglePlayPause()
          break
        case 'next':
          void player.next()
          break
      }
    })
  } catch {}

  stopWatches.push(watch(
    () => [player.currentTrack?.id, player.currentTrack?.syncPayload] as const,
    () => {
      if (active) void refreshLyrics()
    },
    { deep: true },
  ))
  stopWatches.push(watch(
    () => [
      player.currentTrack?.title,
      player.currentTrack?.artist,
      settings.showTranslation,
      settings.cloudMusicOffset,
      settings.qqMusicOffset,
      lyricOffsetStore.offsets,
    ] as const,
    () => emitState(),
    { deep: true },
  ))
  stopWatches.push(watch(
    () => [
      player.interpolatedPositionMs,
      player.durationMs,
      player.isPlaying,
      player.isLoadingAudio,
    ] as const,
    () => schedulePlayback(),
  ))

  return () => {
    active = false
    loadGeneration++
    if (playbackTimer) clearTimeout(playbackTimer)
    for (const stop of stopWatches) stop()
    unlistenReady?.()
    unlistenHidden?.()
    unlistenControl?.()
  }
}
