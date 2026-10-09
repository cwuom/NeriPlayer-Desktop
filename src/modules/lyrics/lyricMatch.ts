// 歌词「匹配」：按所选平台搜候选歌词（后端 match_lyrics 打分排序，对齐 Android EditableLyricsMatcher）
import { invoke } from '@tauri-apps/api/core'
import type { LyricLine } from '@/stores/player'
import { mapBackendLyrics } from './lyricsFormat'

/** 平台按 Android 匹配面板的顺序排列（YouTube Music 暂不提供） */
export const LYRIC_MATCH_SOURCES = ['kugou', 'netease', 'qq', 'lrclib', 'amll_ttml'] as const
export type LyricMatchSource = typeof LYRIC_MATCH_SOURCES[number]
export type LyricMatchConfidence = 'high' | 'medium' | 'low'
export type LyricMatchFormat = 'lrc' | 'yrc' | 'ttml' | 'plain'

export interface LyricMatchResult {
  id: string
  source: LyricMatchSource
  title: string
  artist: string
  album: string
  durationMs: number
  format: LyricMatchFormat
  score: number
  durationDeltaMs: number | null
  confidence: LyricMatchConfidence
  wordTimed: boolean
  hasTranslation: boolean
  hasRomanization: boolean
  lines: LyricLine[]
}

export interface LyricMatchRequest {
  keyword: string
  title: string
  artist: string
  album?: string
  durationMs: number
  preferWordTimed: boolean
  sources: LyricMatchSource[]
}

/** 对齐 Android defaultEditableLyricMatchSources：YouTube 曲目默认不查 AMLL，改查 QQ 和 LRCLIB */
export function defaultLyricMatchSources(playbackSource?: string | null): LyricMatchSource[] {
  return playbackSource === 'youtube'
    ? ['kugou', 'netease', 'qq', 'lrclib']
    : ['kugou', 'netease', 'amll_ttml']
}

export function defaultLyricMatchKeyword(title?: string | null, artist?: string | null): string {
  return [title, artist].map(value => (value || '').trim()).filter(Boolean).join(' ')
}

function isSource(value: unknown): value is LyricMatchSource {
  return (LYRIC_MATCH_SOURCES as readonly unknown[]).includes(value)
}

const FORMATS: readonly LyricMatchFormat[] = ['lrc', 'yrc', 'ttml', 'plain']
const CONFIDENCES: readonly LyricMatchConfidence[] = ['high', 'medium', 'low']

/** 后端结果逐项校验；歌词行映射成前端字段，没有歌词行的丢掉 */
export function normalizeLyricMatchResults(raw: unknown): LyricMatchResult[] {
  if (!Array.isArray(raw)) return []
  const results: LyricMatchResult[] = []
  for (const value of raw) {
    if (!value || typeof value !== 'object' || !isSource(value.source)) continue
    const lines = mapBackendLyrics(value.lines).filter(line => line.text.trim() || line.words.length)
    if (!lines.length) continue
    const delta = Number(value.durationDeltaMs)
    results.push({
      id: String(value.id ?? ''),
      source: value.source,
      title: String(value.title ?? ''),
      artist: String(value.artist ?? ''),
      album: String(value.album ?? ''),
      durationMs: Math.max(0, Number(value.durationMs) || 0),
      format: FORMATS.includes(value.format) ? value.format : 'lrc',
      score: Number(value.score) || 0,
      durationDeltaMs: value.durationDeltaMs == null || !Number.isFinite(delta) ? null : delta,
      confidence: CONFIDENCES.includes(value.confidence) ? value.confidence : 'low',
      wordTimed: Boolean(value.wordTimed),
      hasTranslation: Boolean(value.hasTranslation),
      hasRomanization: Boolean(value.hasRomanization),
      lines,
    })
  }
  return results
}

export async function matchLyrics(request: LyricMatchRequest): Promise<LyricMatchResult[]> {
  return normalizeLyricMatchResults(await invoke('match_lyrics', { request }))
}

/** 写回 syncPayload 的 matchedLyricSource，与偏移来源、Android MusicPlatform 一致 */
export function lyricMatchSourceTag(source: LyricMatchSource): string {
  switch (source) {
    case 'netease': return 'CLOUD_MUSIC'
    case 'qq': return 'QQ_MUSIC'
    default: return source.toUpperCase()
  }
}

export function formatMatchDuration(ms: number): string {
  if (!(ms > 0)) return ''
  const total = Math.round(ms / 1000)
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/** 时长差：10 秒内保留一位小数 */
export function formatMatchDelta(ms: number): string {
  const seconds = Math.abs(ms) / 1000
  return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`
}
