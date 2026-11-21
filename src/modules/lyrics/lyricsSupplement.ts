import type { LyricLine } from '@/types/track'
import { defaultLyricMatchKeyword, matchLyrics, type LyricMatchResult } from './lyricMatch'
import { normalizeLyricSource } from './lyricOffset'
import {
  mergeWordTimedLyricsWithBaseline,
  resolveStoredRomanizedLyricStateFromPayload,
  resolveStoredTranslatedLyricStateFromPayload,
} from './lyricsFormat'

export interface SecondaryLyricNeeds {
  translation: boolean
  romanization: boolean
}

interface SupplementTrack {
  id?: string | null
  title: string
  artist: string
  album?: string | null
  durationMs?: number | null
}

function hasTranslation(lines: LyricLine[]): boolean {
  return lines.some(line => !!line.translation?.trim())
}

function hasRomanization(lines: LyricLine[]): boolean {
  return lines.some(line => !!line.roman?.trim())
}

export function kugouSecondaryLyricNeeds(
  lines: LyricLine[],
  payload: Record<string, unknown> | null | undefined,
  source: string | null | undefined,
): SecondaryLyricNeeds {
  if (!lines.length || normalizeLyricSource(source) !== 'kugou') {
    return { translation: false, romanization: false }
  }
  const edited = payload?.lyricSyncEdited === true || payload?.lyric_sync_edited === true
  return {
    translation: !hasTranslation(lines)
      && !(edited && resolveStoredTranslatedLyricStateFromPayload(payload).kind !== 'absent'),
    romanization: !hasRomanization(lines)
      && !(edited && resolveStoredRomanizedLyricStateFromPayload(payload).kind !== 'absent'),
  }
}

export function mergeKugouSecondaryLyrics(
  original: LyricLine[],
  supplement: LyricLine[],
  needs: SecondaryLyricNeeds,
): LyricLine[] {
  if (!original.length || !supplement.length || (!needs.translation && !needs.romanization)) return original
  const secondary = supplement.map(line => ({
    ...line,
    translation: needs.translation && line.translation?.trim() ? line.translation : undefined,
    roman: needs.romanization && line.roman?.trim() ? line.roman : undefined,
  }))
  // 空白副轨也视为缺失，合并后只采纳实际补上的字段
  const merged = mergeWordTimedLyricsWithBaseline(secondary, original.map(line => ({
    ...line,
    translation: line.translation?.trim() ? line.translation : undefined,
    roman: line.roman?.trim() ? line.roman : undefined,
  })))
  let changed = false
  const result = original.map((line, index) => {
    const translation = needs.translation && !line.translation?.trim() && merged[index].translation?.trim()
      ? merged[index].translation : line.translation
    const roman = needs.romanization && !line.roman?.trim() && merged[index].roman?.trim()
      ? merged[index].roman : line.roman
    if (translation === line.translation && roman === line.roman) return line
    changed = true
    return { ...line, translation, roman }
  })
  return changed ? result : original
}

const MAX_MATCH_CACHE = 200
const matchesByTrack = new Map<string, Promise<LyricMatchResult[]>>()

function cachedMatches(track: SupplementTrack, source: 'netease' | 'qq'): Promise<LyricMatchResult[]> {
  const key = JSON.stringify([
    track.id ?? '', track.title.trim(), track.artist.trim(), track.album ?? '', track.durationMs, source,
  ])
  const existing = matchesByTrack.get(key)
  if (existing) return existing
  const pending = matchLyrics({
    keyword: defaultLyricMatchKeyword(track.title, track.artist),
    title: track.title,
    artist: track.artist,
    album: track.album || '',
    durationMs: track.durationMs || 0,
    preferWordTimed: false,
    sources: [source],
  })
  matchesByTrack.set(key, pending)
  // 后端也可能把离线失败返回空结果，完成后允许重试并保护同键的新请求
  pending.then(results => {
    if (!results.length && matchesByTrack.get(key) === pending) matchesByTrack.delete(key)
  }, () => {
    if (matchesByTrack.get(key) === pending) matchesByTrack.delete(key)
  })
  if (matchesByTrack.size > MAX_MATCH_CACHE) {
    const oldest = matchesByTrack.keys().next().value
    if (oldest !== undefined) matchesByTrack.delete(oldest)
  }
  return pending
}

function needsRemain(lines: LyricLine[], needs: SecondaryLyricNeeds): boolean {
  return (needs.translation && !hasTranslation(lines)) || (needs.romanization && !hasRomanization(lines))
}

function isReliableCandidate(candidate: LyricMatchResult, source: 'netease' | 'qq', durationMs: number): boolean {
  // 高置信度仍可能缺候选时长，自动补轨沿用 Android 的可靠时长门槛
  const toleranceMs = Math.min(Math.max(7000, durationMs * 0.06), 15000)
  return candidate.source === source && candidate.confidence === 'high'
    && Number.isFinite(candidate.durationMs) && candidate.durationMs > 0
    && Math.abs(candidate.durationMs - durationMs) <= toleranceMs
}

export async function supplementKugouLyrics(
  track: SupplementTrack,
  lines: LyricLine[],
  needs: SecondaryLyricNeeds,
): Promise<LyricLine[]> {
  const durationMs = track.durationMs || 0
  if (
    !lines.length || (!needs.translation && !needs.romanization)
    || !Number.isFinite(durationMs) || durationMs <= 0
    || !track.title.trim() || !track.artist.trim()
  ) return lines

  let current = lines
  const netease = await cachedMatches(track, 'netease').catch(() => [])
  for (const candidate of netease) {
    if (!isReliableCandidate(candidate, 'netease', durationMs)) continue
    current = mergeKugouSecondaryLyrics(current, candidate.lines, needs)
    if (!needsRemain(current, needs)) return current
  }
  if (needs.translation && !hasTranslation(current)) {
    const qq = await cachedMatches(track, 'qq').catch(() => [])
    for (const candidate of qq) {
      if (!isReliableCandidate(candidate, 'qq', durationMs)) continue
      current = mergeKugouSecondaryLyrics(current, candidate.lines, { translation: true, romanization: false })
      if (hasTranslation(current)) break
    }
  }
  return current
}
