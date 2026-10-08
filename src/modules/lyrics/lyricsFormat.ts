/**
 * 歌词文本往返工具
 * 有逐字时间轴时导出 YRC，否则 LRC
 */
import type { LyricLine, LyricWord } from '@/stores/player'

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

/** 单行导出为 LRC：`[mm:ss.xx]text` */
export function lyricLineToLrc(line: Pick<LyricLine, 'startMs'>, text: string): string {
  const startMs = Math.max(0, Number(line.startMs) || 0)
  const min = Math.floor(startMs / 60000)
  const sec = Math.floor((startMs % 60000) / 1000)
  const ms = Math.floor((startMs % 1000) / 10)
  return `[${pad2(min)}:${pad2(sec)}.${pad2(ms)}]${text || ''}`
}

/**
 * 单行导出为 YRC
 * 格式：`[startMs,durationMs](wordStartMs,wordDurationMs,0)text...`
 * word 时间使用绝对毫秒
 */
export function lyricLineToYrc(line: LyricLine): string {
  const startMs = Math.max(0, Math.floor(Number(line.startMs) || 0))
  const durationMs = Math.max(0, Math.floor(Number(line.durationMs) || 0))
  const words = Array.isArray(line.words) ? line.words : []
  let body = ''
  if (words.length > 0) {
    body = words
      .map((word: LyricWord) => {
        const ws = Math.max(0, Math.floor(Number(word.startMs) || 0))
        const wd = Math.max(0, Math.floor(Number(word.durationMs) || 0))
        return `(${ws},${wd},0)${word.text || ''}`
      })
      .join('')
  } else {
    body = line.text || ''
  }
  return `[${startMs},${durationMs}]${body}`
}

export function hasWordTimedEntries(lines: LyricLine[]): boolean {
  return lines.some(line => Array.isArray(line.words) && line.words.some(w => (w.text || '').length > 0))
}

/**
 * 导出可编辑歌词文本
 * 任一行含逐字时间轴时整段走 YRC, 无逐字行也写成 `[start,dur]text` 以便 parse_auto 往返
 * 全部无逐字时走 LRC
 */
export function toEditableLyricsText(lines: LyricLine[]): string {
  if (!lines.length) return ''
  if (hasWordTimedEntries(lines)) {
    // 混排时统一 YRC, 避免 parse_auto 进 YRC 后丢掉 LRC 行
    return lines.map(lyricLineToYrc).join('\n')
  }
  return lines.map(line => lyricLineToLrc(line, line.text || '')).join('\n')
}

/** 翻译轨始终按 LRC 时间戳导出（翻译通常无逐字） */
export function toEditableTranslationText(lines: LyricLine[]): string {
  return lines
    .filter(line => !!line.translation)
    .map(line => lyricLineToLrc(line, line.translation || ''))
    .join('\n')
}

/** 从后端 snake_case / 前端 camelCase 统一映射 LyricLine */
export function mapBackendLyrics(raw: any[]): LyricLine[] {
  if (!Array.isArray(raw)) return []
  return raw.map((l: any) => ({
    startMs: Number(l.startMs ?? l.start_ms ?? 0),
    durationMs: Number(l.durationMs ?? l.duration_ms ?? 0),
    words: Array.isArray(l.words)
      ? l.words.map((w: any) => ({
        startMs: Number(w.startMs ?? w.start_ms ?? 0),
        durationMs: Number(w.durationMs ?? w.duration_ms ?? 0),
        text: String(w.text || ''),
      }))
      : [],
    text: String(l.text || ''),
    translation: l.translation || undefined,
    roman: l.roman || undefined,
  }))
}

const LYRIC_CREDIT_KEYWORDS = [
  '作词', '作曲', '编曲', '制作', '混音', '母带', '和声', '录音',
  '出品', '监制', '配唱', '词：', '曲：',
  'Lyricist', 'Composer', 'Arranger', 'Producer', 'Mixing', 'Mastering',
]

/** 制作信息行（作词/作曲等署名），翻译匹配时跳过（对齐 Android isLyricCreditMetadataLine） */
function isLyricCreditMetadataLine(text: string): boolean {
  const hasColon = text.includes('：') || text.includes(':')
  return hasColon && LYRIC_CREDIT_KEYWORDS.some(k => text.includes(k))
}

/**
 * 合并原词与翻译 LRC/YRC 解析结果（LY-2）
 *
 * 用 450ms 容差最近匹配 + 同时间组向组尾对齐 + 跳过制作信息行 + 一行只接收一条翻译，
 * 对齐 Android matchTranslationsToLineIndices。原实现的"精确毫秒匹配 + 原文行下标兜底"
 * 在 YRC 时间戳非 10ms 整倍数或 syncPayload 往返（厘秒截断）后必然失配，导致翻译错行。
 */
export function mergeParsedLyricsWithTranslations(
  original: LyricLine[],
  translations: LyricLine[],
): LyricLine[] {
  if (!translations.length) return original
  const TOLERANCE_MS = 450
  const result = original.map(line => ({ ...line }))
  const assigned = new Array(result.length).fill(false)
  for (const tl of translations) {
    const text = tl.text || ''
    if (!text) continue
    let bestIdx = -1
    let bestDelta = Infinity
    for (let i = 0; i < result.length; i++) {
      if (assigned[i] || isLyricCreditMetadataLine(result[i].text || '')) continue
      const delta = Math.abs((result[i].startMs || 0) - (tl.startMs || 0))
      // <= 使并列最小 delta 时取更靠后的行（组尾对齐）
      if (delta < TOLERANCE_MS && delta <= bestDelta) {
        bestDelta = delta
        bestIdx = i
      }
    }
    if (bestIdx >= 0) {
      result[bestIdx].translation = text
      assigned[bestIdx] = true
    }
  }
  // 未被新翻译覆盖的行保留原 translation
  return result.map((line, i) => ({
    ...line,
    translation: line.translation || original[i].translation || undefined,
  }))
}

export function mergeWordTimedLyricsWithBaseline(
  baseline: LyricLine[],
  upgrade: LyricLine[],
): LyricLine[] {
  const translations = mergeParsedLyricsWithTranslations(upgrade, baseline.map(line => ({
    ...line, text: line.translation || '',
  })))
  const roman = mergeParsedLyricsWithTranslations(
    upgrade.map(line => ({ ...line, translation: line.roman })),
    baseline.map(line => ({ ...line, text: line.roman || '' })),
  )
  // 只补时间轴匹配的缺失字段，外源自身的翻译和音译优先
  return upgrade.map((line, index) => ({
    ...line,
    translation: line.translation || translations[index].translation,
    roman: line.roman || roman[index].translation,
  }))
}

/**
 * 本地歌词覆盖状态, 对齐 Android LocalLyricOverrideState
 * - absent: 无本地词, 允许在线拉取
 * - cleared: 用户有意清空 (matched 字段存在但为空), 禁止在线回填
 * - present: 有本地词, 直接使用
 */
export type StoredLyricState =
  | { kind: 'absent' }
  | { kind: 'cleared' }
  | { kind: 'present'; text: string }

function readPayloadString(
  payload: Record<string, unknown>,
  camel: string,
  snake: string,
): string | undefined {
  if (Object.prototype.hasOwnProperty.call(payload, camel)) {
    const value = payload[camel]
    return typeof value === 'string' ? value : value == null ? '' : String(value)
  }
  if (Object.prototype.hasOwnProperty.call(payload, snake)) {
    const value = payload[snake]
    return typeof value === 'string' ? value : value == null ? '' : String(value)
  }
  return undefined
}

function resolveStoredLyricState(
  payload: Record<string, unknown> | undefined | null,
  matchedCamel: string,
  matchedSnake: string,
  originalCamel: string,
  originalSnake: string,
): StoredLyricState {
  if (!payload || typeof payload !== 'object') return { kind: 'absent' }
  const matched = readPayloadString(payload, matchedCamel, matchedSnake)
  if (matched !== undefined) {
    // Android: currentLyric != null 时直接采用 (含空串 = CLEARED)
    return matched.trim() ? { kind: 'present', text: matched } : { kind: 'cleared' }
  }
  const original = readPayloadString(payload, originalCamel, originalSnake)
  if (original !== undefined) {
    return original.trim() ? { kind: 'present', text: original } : { kind: 'cleared' }
  }
  return { kind: 'absent' }
}

export function resolveStoredLyricStateFromPayload(
  payload: Record<string, unknown> | undefined | null,
): StoredLyricState {
  return resolveStoredLyricState(
    payload,
    'matchedLyric',
    'matched_lyric',
    'originalLyric',
    'original_lyric',
  )
}

export function resolveStoredTranslatedLyricStateFromPayload(
  payload: Record<string, unknown> | undefined | null,
): StoredLyricState {
  return resolveStoredLyricState(
    payload,
    'matchedTranslatedLyric',
    'matched_translated_lyric',
    'originalTranslatedLyric',
    'original_translated_lyric',
  )
}

/** 从 sync_payload 取出可解析的歌词原文; 有意清空返回空串, 缺失返回 null */
export function resolveStoredLyricText(payload: Record<string, unknown> | undefined | null): string | null {
  const state = resolveStoredLyricStateFromPayload(payload)
  if (state.kind === 'present') return state.text
  if (state.kind === 'cleared') return ''
  return null
}

export function resolveStoredTranslatedLyricText(
  payload: Record<string, unknown> | undefined | null,
): string | null {
  const state = resolveStoredTranslatedLyricStateFromPayload(payload)
  if (state.kind === 'present') return state.text
  if (state.kind === 'cleared') return ''
  return null
}

/**
 * 更新 sync_payload 歌词字段
 * 首次覆盖时保留 original*
 */
export function withUpdatedLyricsPayload(
  payload: Record<string, unknown> | undefined | null,
  nextLyric: string | null,
  nextTranslated: string | null,
  source?: string | null,
  now: number = Date.now(),
): Record<string, unknown> {
  const base: Record<string, unknown> = { ...(payload || {}) }
  const prevMatched = typeof base.matchedLyric === 'string'
    ? base.matchedLyric
    : (typeof base.matched_lyric === 'string' ? base.matched_lyric : null)
  const prevTranslated = typeof base.matchedTranslatedLyric === 'string'
    ? base.matchedTranslatedLyric
    : (typeof base.matched_translated_lyric === 'string' ? base.matched_translated_lyric : null)

  if (base.originalLyric == null && base.original_lyric == null && prevMatched) {
    base.originalLyric = prevMatched
  }
  if (
    base.originalTranslatedLyric == null
    && base.original_translated_lyric == null
    && prevTranslated
  ) {
    base.originalTranslatedLyric = prevTranslated
  }

  // 有意清空写空串 (CLEARED), 与 Android blank matchedLyric 对齐;
  // 同步序列化时 blank -> None, CURRENT 版本可阻止远端 fill-missing 回填
  if (nextLyric == null || !nextLyric.trim()) {
    base.matchedLyric = ''
    delete base.matched_lyric
  } else {
    base.matchedLyric = nextLyric
    delete base.matched_lyric
  }

  if (nextTranslated == null || !nextTranslated.trim()) {
    base.matchedTranslatedLyric = ''
    delete base.matched_translated_lyric
  } else {
    base.matchedTranslatedLyric = nextTranslated
    delete base.matched_translated_lyric
  }

  if (source && source.trim()) {
    base.matchedLyricSource = source
    delete base.matched_lyric_source
  }

  // 写入后标记 CURRENT, 与 Android fromSongItem 一致
  const version = Number(base.syncMetadataVersion ?? base.sync_metadata_version ?? 0)
  if (!Number.isFinite(version) || version < 1) {
    base.syncMetadataVersion = 1
    delete base.sync_metadata_version
  }

  // 用户编辑：标记已编辑并推进修订号，同步合并按修订号取最新（对齐 Android nextUserLyricSyncRevision）
  const previousRevision = Number(base.lyricSyncRevision ?? base.lyric_sync_revision ?? 0)
  base.lyricSyncEdited = true
  base.lyricSyncRevision = Math.max(now, (Number.isFinite(previousRevision) ? previousRevision : 0) + 1)
  delete base.lyric_sync_edited
  delete base.lyric_sync_revision

  return base
}
