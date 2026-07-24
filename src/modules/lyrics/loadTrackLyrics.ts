import { invoke } from '@tauri-apps/api/core'
import type { LyricLine, TrackInfo } from '@/stores/player'
import { getCachedLyrics, saveCachedLyrics } from './lyricsCache'
import { loadLyricsSingleFlight } from './lyricsRequest'
import {
  mapBackendLyrics,
  mergeParsedLyricsWithTranslations,
  resolveStoredLyricStateFromPayload,
  resolveStoredTranslatedLyricStateFromPayload,
} from './lyricsFormat'

async function materializeStoredLyrics(track: TrackInfo): Promise<LyricLine[] | null> {
  const lyricState = resolveStoredLyricStateFromPayload(track.syncPayload)
  if (lyricState.kind === 'absent') return null
  if (lyricState.kind === 'cleared') return []

  try {
    const original = mapBackendLyrics(await invoke<any[]>('parse_lrc_content', {
      content: lyricState.text,
    }))
    const translationState = resolveStoredTranslatedLyricStateFromPayload(track.syncPayload)
    if (translationState.kind !== 'present') return original

    try {
      const translated = mapBackendLyrics(await invoke<any[]>('parse_lrc_content', {
        content: translationState.text,
      }))
      return mergeParsedLyricsWithTranslations(original, translated)
    } catch {
      return original
    }
  } catch {
    // A stored override is authoritative, including malformed content.
    return []
  }
}

export async function loadTrackLyrics(
  track: TrackInfo,
  durationHintMs = 0,
): Promise<LyricLine[]> {
  const stored = await materializeStoredLyrics(track)
  if (stored !== null) {
    if (stored.length > 0) saveCachedLyrics(track, stored)
    return stored
  }

  const cached = getCachedLyrics(track)
  if (cached?.length) return cached

  return loadLyricsSingleFlight(track, async () => {
    const neteaseId = track.id.startsWith('netease:')
      ? Number(track.id.slice('netease:'.length)) || null
      : null
    const qqSongMid = track.id.startsWith('qq:')
      ? track.id.slice('qq:'.length)
      : null
    const youtubeVideoId = track.id.startsWith('youtube:')
      ? track.id.slice('youtube:'.length)
      : null
    const raw = await invoke<any[]>('fetch_lyrics', {
      title: track.title,
      artist: track.artist,
      durationSecs: Math.floor(Math.max(track.durationMs, durationHintMs, 0) / 1000),
      audioPath: track.audioUrl || null,
      neteaseId,
      qqSongMid,
      youtubeVideoId,
    })
    const lines = mapBackendLyrics(raw)
    if (lines.length > 0) saveCachedLyrics(track, lines)
    return lines
  })
}
