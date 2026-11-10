// CC subtitle selection and interval matching ported from NeriPlayer PR #575.
import type { LyricLine, TrackInfo } from '@/stores/player'
import { isBiliVideoSkipTrack, resolveBiliVideoSkipBvid, resolveBiliVideoSkipCid } from '@/modules/playback/biliVideoSkip'
import { isLyricCreditMetadataLine } from './lyricsFormat'

export interface BiliSubtitleTrack {
  id: string
  language: string
  label: string
  url: string
  isAi: boolean
}

export interface BiliSubtitlePreferences {
  enabled: boolean
  language: string
  aiFallback: boolean
}

export interface BiliSubtitleCue {
  startMs: number
  endMs: number
  text: string
}

export function biliSubtitlePreferences(settings: {
  biliCcSubtitleEnabled?: boolean
  biliCcSubtitleLanguage?: string
  biliCcSubtitleAiFallback?: boolean
}): BiliSubtitlePreferences {
  return {
    enabled: settings.biliCcSubtitleEnabled ?? true,
    language: settings.biliCcSubtitleLanguage?.trim() ?? '',
    aiFallback: settings.biliCcSubtitleAiFallback ?? true,
  }
}

export function biliSubtitleTarget(track: TrackInfo) {
  if (!isBiliVideoSkipTrack(track)) return null
  const bvid = resolveBiliVideoSkipBvid(track) ?? ''
  const raw = track.syncPayload?.audioId ?? track.syncPayload?.audio_id
    ?? track.id.replace(/^bilibili:/i, '').split(':')[0]
  const avid = /^\d+$/.test(String(raw)) ? Number(raw) : null
  if (!bvid && !(avid && Number.isSafeInteger(avid))) return null
  const idCid = Number(track.id.replace(/^bilibili:/i, '').split(':')[1])
  return { bvid, avid: bvid ? null : avid, cid: resolveBiliVideoSkipCid(track)
    ?? (Number.isSafeInteger(idCid) && idCid > 0 ? idCid : null) }
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}

export function normalizeBiliSubtitleUrl(value: unknown): string | null {
  if (typeof value !== 'string' || !value.trim()) return null
  const raw = value.trim()
  try {
    const url = new URL(raw.startsWith('//') ? `https:${raw}` : /^[a-z]+:/i.test(raw) ? raw : `https://${raw}`)
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.port) return null
    if (!['hdslb.com', 'bilibili.com'].some(host => url.hostname === host || url.hostname.endsWith(`.${host}`))) return null
    url.protocol = 'https:'
    return url.href
  } catch { return null }
}

export function parseBiliSubtitleTracks(payload: unknown): BiliSubtitleTrack[] {
  const subtitles = record(payload).subtitles
  if (!Array.isArray(subtitles)) return []
  return subtitles.flatMap((raw) => {
    const item = record(raw)
    const url = normalizeBiliSubtitleUrl(item.subtitle_url)
    const language = typeof item.lan === 'string' ? item.lan.trim() : ''
    if (!url || !language) return []
    return [{
      // id_str preserves IDs larger than JavaScript's safe integer range.
      id: String(item.id_str ?? item.id ?? language), language, url,
      label: typeof item.lan_doc === 'string' && item.lan_doc.trim() ? item.lan_doc.trim() : language,
      isAi: Number(item.type) === 1 || /^ai-/i.test(language),
    }]
  })
}

function baseLanguage(language: string): string {
  return language.trim().toLowerCase().replace(/^ai-/, '').split(/[-_]/)[0] ?? ''
}

function script(language: string): string {
  if (/(?:^|[-_])(hant|tw|hk|mo)(?:$|[-_])/.test(language)) return 'hant'
  if (/(?:^|[-_])(hans|cn|sg)(?:$|[-_])/.test(language)) return 'hans'
  return ''
}

export function matchesBiliSubtitleLanguage(language: string, preferred: string): boolean {
  const a = language.trim().toLowerCase()
  const b = preferred.trim().toLowerCase()
  if (!a || !b) return false
  if (a === b) return true
  if (a.startsWith('ai-') || b.startsWith('ai-')) return false
  if (baseLanguage(a) !== baseLanguage(b)) return false
  return !script(a) || !script(b) || script(a) === script(b)
}

export function biliSubtitleCandidates(tracks: BiliSubtitleTrack[], preferences: BiliSubtitlePreferences): BiliSubtitleTrack[] {
  const language = preferences.language.toLowerCase()
  const usable = tracks.filter(track => track.url)
  const byLanguage = (a: BiliSubtitleTrack, b: BiliSubtitleTrack) => a.language.localeCompare(b.language)
  const explicit = usable.filter(track => matchesBiliSubtitleLanguage(track.language, language)).sort((a, b) =>
    Number(a.language.toLowerCase() !== language) - Number(b.language.toLowerCase() !== language)
    || Number(a.isAi) - Number(b.isAi) || byLanguage(a, b))
  const automatic = usable.filter(track => !explicit.includes(track) && (preferences.aiFallback || !track.isAi))
    .sort((a, b) => Number(baseLanguage(a.language) !== 'zh') - Number(baseLanguage(b.language) !== 'zh')
      || Number(a.isAi) - Number(b.isAi) || byLanguage(a, b))
  return [...explicit, ...automatic]
}

export function translatedBiliSubtitleTrack(primary: BiliSubtitleTrack, tracks: BiliSubtitleTrack[], aiFallback: boolean) {
  return tracks.filter(track => track.url && baseLanguage(track.language) !== baseLanguage(primary.language)
    && (aiFallback || !track.isAi))
    .sort((a, b) => Number(a.isAi) - Number(b.isAi) || a.language.localeCompare(b.language))[0]
}

export function parseBiliSubtitleCues(payload: unknown): BiliSubtitleCue[] {
  const body = record(payload).body
  if (!Array.isArray(body)) return []
  const millis = (value: unknown): number | null => {
    if ((typeof value !== 'number' && typeof value !== 'string') || String(value).trim() === '') return null
    const time = Math.round(Number(value) * 1000)
    return Number.isSafeInteger(time) && time >= 0 && time < Number.MAX_SAFE_INTEGER ? time : null
  }
  const seen = new Set<string>()
  return body.flatMap((raw) => {
    const item = record(raw)
    const text = typeof item.content === 'string' ? item.content.trim() : ''
    const startMs = millis(item.from)
    if (!text || startMs === null) return []
    const key = JSON.stringify([startMs, text])
    if (seen.has(key)) return []
    seen.add(key)
    return [{ startMs, endMs: Math.max(startMs + 1, millis(item.to) ?? startMs), text }]
  }).sort((a, b) => a.startMs - b.startMs)
}

function displayIntervals(cues: BiliSubtitleCue[]): BiliSubtitleCue[] {
  const sorted = [...cues].sort((a, b) => a.startMs - b.startMs)
  let groupEnd = 0
  return sorted.map((cue, index) => {
    if (index === groupEnd) {
      groupEnd = index + 1
      while (sorted[groupEnd]?.startMs === cue.startMs) groupEnd++
    }
    return { ...cue, endMs: Math.min(cue.endMs, sorted[groupEnd]?.startMs ?? cue.endMs) }
  })
}

/** A long translation can accompany several original rows; small boundary overlaps are ignored. */
export function alignBiliSubtitleLyrics(original: BiliSubtitleCue[], translated: BiliSubtitleCue[]): LyricLine[] {
  const translations = displayIntervals(translated).filter(cue => !isLyricCreditMetadataLine(cue.text)
    && !/^[\s/／]+$/.test(cue.text))
  let cursor = 0
  let active: BiliSubtitleCue[] = []
  return displayIntervals(original).map(line => {
    while (cursor < translations.length && translations[cursor]!.startMs < line.endMs) active.push(translations[cursor++]!)
    active = active.filter(cue => cue.endMs > line.startMs)
    const translation = active.filter(cue => {
      const overlap = Math.min(line.endMs, cue.endMs) - Math.max(line.startMs, cue.startMs)
      return overlap > 0 && overlap >= Math.min(line.endMs - line.startMs, cue.endMs - cue.startMs) * 0.5
    }).map(cue => cue.text).join('\n')
    return { startMs: line.startMs, durationMs: line.endMs - line.startMs, text: line.text, words: [], translation: translation || undefined }
  })
}
