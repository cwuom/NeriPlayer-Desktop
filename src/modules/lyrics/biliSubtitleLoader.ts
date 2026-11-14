import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'
import type { LyricLine, TrackInfo } from '@/stores/player'
import {
  alignBiliSubtitleLyrics, biliSubtitleCandidates, biliSubtitleTarget, matchesBiliSubtitleLanguage,
  parseBiliSubtitleCues, parseBiliSubtitleTracks, translatedBiliSubtitleTrack,
  type BiliSubtitlePreferences, type BiliSubtitleTrack,
} from './biliSubtitle'

export interface BiliSubtitleTracks {
  bvid: string
  cid: number
  tracks: BiliSubtitleTrack[]
  loginRequired: boolean
}

export interface BiliSubtitleLyrics {
  source: 'bilibili_cc'
  lines: LyricLine[]
  track?: BiliSubtitleTrack
  translatedTrack?: BiliSubtitleTrack
}

// Signed URLs and parsed captions stay in memory, scoped to this login session.
const TTL_MS = 10 * 60_000
const MAX_ENTRIES = 16
const tracksCache = new Map<string, { value: BiliSubtitleTracks; expires: number }>()
const lyricsCache = new Map<string, { value: BiliSubtitleLyrics; expires: number }>()
const pendingTracks = new Map<string, Promise<BiliSubtitleTracks>>()
const pendingLyrics = new Map<string, Promise<BiliSubtitleLyrics>>()
export const biliSubtitleRevision = ref(0)

/** Clear session data and invalidate results still pending across login or refresh changes. */
export function invalidateBiliSubtitleCache(): void {
  biliSubtitleRevision.value++
  tracksCache.clear()
  lyricsCache.clear()
  pendingTracks.clear()
  pendingLyrics.clear()
}

/** Cache a successful result with a TTL and evict the oldest entries above the size limit. */
function put<T>(cache: Map<string, { value: T; expires: number }>, key: string, value: T) {
  cache.delete(key)
  cache.set(key, { value, expires: Date.now() + TTL_MS })
  while (cache.size > MAX_ENTRIES) cache.delete(cache.keys().next().value!)
}

/** Share an in-flight request; settling an obsolete request cannot remove its replacement. */
function singleFlight<T>(pending: Map<string, Promise<T>>, key: string, loader: () => Promise<T>): Promise<T> {
  const existing = pending.get(key)
  if (existing) return existing
  const request = loader()
  pending.set(key, request)
  const release = () => { if (pending.get(key) === request) pending.delete(key) }
  request.then(release, release)
  return request
}

/** Fetch tracks for one video part, caching only nonempty results from the current session. */
export async function loadBiliSubtitleTracks(track: TrackInfo): Promise<BiliSubtitleTracks> {
  const target = biliSubtitleTarget(track)
  if (!target) return { bvid: '', cid: 0, tracks: [], loginRequired: false }
  const key = JSON.stringify(target)
  const cached = tracksCache.get(key)
  if (cached && cached.expires > Date.now()) return cached.value
  const generation = biliSubtitleRevision.value
  return singleFlight(pendingTracks, key, async () => {
    const payload = await invoke<{ bvid: string; cid: number; subtitle: unknown; loginRequired: boolean }>('get_bili_subtitle_tracks', target)
    const result = { ...payload, tracks: parseBiliSubtitleTracks(payload.subtitle) }
    if (generation === biliSubtitleRevision.value && result.tracks.length) put(tracksCache, key, result)
    return result
  })
}

/** Load and align the preferred original/translation pair, leaving incomplete results retryable. */
export async function loadBiliSubtitleLyrics(track: TrackInfo, preferences: BiliSubtitlePreferences): Promise<BiliSubtitleLyrics> {
  const empty: BiliSubtitleLyrics = { source: 'bilibili_cc', lines: [] }
  const target = biliSubtitleTarget(track)
  if (!preferences.enabled || !target) return empty
  const key = JSON.stringify([target, preferences.language, preferences.aiFallback])
  const cached = lyricsCache.get(key)
  if (cached && cached.expires > Date.now()) return cached.value
  const generation = biliSubtitleRevision.value
  return singleFlight(pendingLyrics, key, async () => {
    const available = await loadBiliSubtitleTracks(track)
    if (generation !== biliSubtitleRevision.value) return empty
    const candidates = biliSubtitleCandidates(available.tracks, preferences)
    const read = async (candidate: BiliSubtitleTrack) => parseBiliSubtitleCues(await invoke('get_bili_subtitle_payload', {
      url: candidate.url, bvid: available.bvid,
    }))
    for (const candidate of candidates) {
      const original = await read(candidate).catch(() => [])
      if (generation !== biliSubtitleRevision.value) return empty
      if (!original.length) continue
      const translatedTrack = translatedBiliSubtitleTrack(candidate, available.tracks, preferences.aiFallback)
      const translated = translatedTrack ? await read(translatedTrack).catch(() => []) : []
      if (generation !== biliSubtitleRevision.value) return empty
      const result: BiliSubtitleLyrics = {
        source: 'bilibili_cc', lines: alignBiliSubtitleLyrics(original, translated), track: candidate,
        translatedTrack: translated.length ? translatedTrack : undefined,
      }
      const matched = preferences.language ? matchesBiliSubtitleLanguage(candidate.language, preferences.language) : candidate === candidates[0]
      if (matched && (!translatedTrack || translated.length)) put(lyricsCache, key, result)
      else tracksCache.delete(JSON.stringify(target)) // Failed/expired tracks remain retryable.
      return result
    }
    if (generation === biliSubtitleRevision.value) tracksCache.delete(JSON.stringify(target))
    return empty
  })
}

// Auth changes also cover replacing cookies while remaining logged in.
if (typeof window !== 'undefined') {
  window.addEventListener('neri:auth-changed', event => {
    if ((event as CustomEvent).detail?.platform === 'bilibili') invalidateBiliSubtitleCache()
  })
}
