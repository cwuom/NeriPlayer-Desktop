/**
 * 歌词文本往返工具
 * 有逐字时间轴时导出 YRC，否则 LRC
 */
import type { LyricLine, LyricWord, TrackInfo } from '@/stores/player'

/** Only source-provided text that adds content to the original is a visible translation. */
export function visibleLyricTranslation(line: Pick<LyricLine, 'text' | 'words' | 'translation'>): string {
  const translation = line.translation?.trim() ?? ''
  const comparable = (value: string) => value.normalize('NFKC').replace(/[^\p{L}\p{N}]/gu, '').toLowerCase()
  const translatedText = comparable(translation)
  const original = line.text || (line.words ?? []).map(word => word.text).join('')
  return translatedText && translatedText !== comparable(original) ? translation : ''
}

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

/** 音译轨同翻译一样按 LRC 导出 */
export function toEditableRomanizationText(lines: LyricLine[]): string {
  return lines
    .filter(line => !!line.roman)
    .map(line => lyricLineToLrc(line, line.roman || ''))
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

const TRANSLATION_ALIGNMENT_TOLERANCE_MS = 450
const TRANSLATION_CLOSEST_MATCH_TOLERANCE_MS = 2000

const LYRIC_CREDIT_METADATA_RE = /^\s*(.*?)\s*[:：]\s*(\S.*?)\s*$/u
const LYRIC_CREDIT_METADATA_ROLES = new Set([
  '作词', '作詞', '填词', '填詞', '作曲', '词曲', '詞曲', '编曲', '編曲', '制作人', '製作人', '制作', '製作',
  '出品', '出品人', '联合出品', '聯合出品', '营销', '營銷', '策划', '策劃', '企划', '企劃', '监制', '監製',
  '统筹', '統籌', '发行', '發行', '宣传', '宣傳', '推广', '推廣', '宣传推广', '宣傳推廣', '宣发', '宣發',
  '混音', '母带', '母帶', '录音', '錄音', '和声', '和聲', '和音', '配唱', '演唱', '歌手', '原唱', '翻唱',
  '词', '詞', '曲', '吉他', '电吉他', '電吉他', '木吉他', '贝斯', '貝斯', '鼓', '键盘', '鍵盤', '钢琴', '鋼琴',
  '弦乐', '弦樂', '弦乐编写', '弦樂編寫', '弦乐编曲', '弦樂編曲', '和声编写', '和聲編寫', '和声编唱', '和聲編唱',
  '和声演唱', '和聲演唱', '和声设计', '和聲設計', '和声编配', '和聲編配', '口哨', 'midi程序', 'midi编程', 'midi編程',
  '录音师', '錄音師', '混音师', '混音師', '母带工程师', '母帶工程師', '录音室', '錄音室', '录音棚', '錄音棚',
  '混音室', '母带后期', '母帶後期', '人声编辑', '人聲編輯', '音频编辑', '音頻編輯', '配唱制作人', '配唱製作人',
  '音乐制作', '音樂製作', '音乐总监', '音樂總監', '制作公司', '製作公司', '版权', '版權', '版权所有', '版權所有',
  '鸣谢', '鳴謝', '特别鸣谢', '特別鳴謝', '封面设计', '封面設計', '文案', 'op', 'sp',
  'lyric', 'lyrics', 'word', 'words', 'music', 'lyricist', 'composer', 'arranger', 'producer', 'co-producer',
  'mix', 'mixing', 'mixengineer', 'mixingengineer', 'mastering', 'masteringengineer', 'recording', 'recordingengineer',
  'vocal', 'vocals', 'vocalist', 'vocalproduction', 'vocalproducer', 'vocalarrangement', 'vocalediting',
  'backingvocals', 'backgroundvocals', 'harmony', 'harmonies', 'guitar', 'guitars', 'acousticguitar', 'electricguitar',
  'bass', 'drum', 'drums', 'keyboard', 'keyboards', 'piano', 'strings', 'stringarrangement', 'midiprogramming', 'programming',
  'artist', 'singer', 'title', 'album', 'copyright', 'publisher', 'label', 'distribution', 'promotion', 'credits',
  'lyricby', 'lyricsby', 'wordby', 'wordsby', 'writtenby', 'composedby', 'arrangedby', 'producedby', 'performedby', 'sungby',
  'musicby', 'mixedby', 'masteredby', 'recordedby', 'publishedby', 'distributedby', 'by',
])
const AMBIGUOUS_CREDIT_ROLES = new Set([
  '词', '詞', '曲', '鼓', '吉他', '贝斯', '貝斯', '键盘', '鍵盤', '弦乐', '弦樂', '和声', '和聲', '和音', '口哨',
  'lyric', 'lyrics', 'word', 'words', 'music', 'vocal', 'vocals', 'guitar', 'guitars', 'bass', 'drum', 'drums',
  'keyboard', 'keyboards', 'strings',
])
const CONTRIBUTOR_NAME_RE = /^(?:[\p{Script=Han}]{2,4}|@\S+|[\p{Lu}\p{N}][\p{L}\p{N}.'_-]*(?:\s+(?:[\p{Lu}\p{N}][\p{L}\p{N}.'_-]*|de|van|von|da|del)){0,5})$/u
const SEPARATED_CREDIT_RE = /^\s*(作词|作詞|填词|填詞|作曲|编曲|編曲|制作人|製作人|producer|composer|arranger|lyricist)\s*[-/／]\s*(\S.*?)\s*$/iu
const ATTRIBUTION_CREDIT_RE = /^\s*((?:lyrics?|words?|written|composed|arranged|produced|performed|sung|music|mixed|mastered|recorded|published|distributed)\s+by)\s+(\S.*?)\s*$/iu
const CONTRIBUTOR_SENTENCE_PREFIX_RE = /^(?:i|you|he|she|it|we|they|me|us|my|your|his|her|its|our|their|a|an|the|this|that|these|those)\b|^(?:我|你|他|她|它|想|爱|愛|在|是|不|别|別|没|沒)/iu

function isContributorList(value: string): boolean {
  const names = value.replace(/[/／&、,，+;；\s]+$/u, '').trim()
  return names.split(/\s+and\s+|[/／&、,，+;；]/iu).every(name => {
    const plain = name.trim().split(/[（(]/u)[0].trim()
    const nameOnly = plain.startsWith('@') ? plain : plain.split('@')[0].trim()
    return !CONTRIBUTOR_SENTENCE_PREFIX_RE.test(nameOnly) && CONTRIBUTOR_NAME_RE.test(nameOnly)
  })
}

/** 与后端正文过滤共用同等规则，短标签还需符合署名格式以免误伤歌词 */
export function isLyricCreditMetadataLine(text: string, hasContext = false): boolean {
  const labelled = LYRIC_CREDIT_METADATA_RE.exec(text)
  if (labelled) {
    return labelled[1].split(/[/／&、,，+]/u).every(role => {
      const normalized = role.replace(/\s/g, '').toLowerCase()
      return LYRIC_CREDIT_METADATA_ROLES.has(normalized)
        && (hasContext || !['词', '詞', '曲'].includes(normalized))
        && (!(AMBIGUOUS_CREDIT_ROLES.has(normalized) || normalized.endsWith('by')) || isContributorList(labelled[2]))
    })
  }
  if (SEPARATED_CREDIT_RE.test(text)) return true
  if (!hasContext) return false
  const attribution = ATTRIBUTION_CREDIT_RE.exec(text)
  if (!attribution) return false
  const name = attribution[2].trim()
  return isContributorList(name)
}

/** 含糊标签必须与明确的制作署名组成连续信息块 */
function lyricCreditMetadataMask(texts: string[]): boolean[] {
  const credits = texts.map(text => isLyricCreditMetadataLine(text))
  const possible = texts.map((text, index) => credits[index] || isLyricCreditMetadataLine(text, true))
  let start = 0
  while (start < texts.length) {
    if (!possible[start]) { start++; continue }
    let end = start + 1
    while (end < texts.length && possible[end]) end++
    if (credits.slice(start, end).some(Boolean)) credits.fill(true, start, end)
    start = end
  }
  return credits
}

const LEADING_METADATA_SCAN_LINES = 8
const TRAILING_METADATA_SCAN_LINES = 4
const TITLE_FUNCTION_WORDS = new Set(['a', 'an', 'the', 'in', 'on', 'at', 'of', 'to', 'for', 'and'])

function normalizeLyricIdentity(text: string): string {
  return text.normalize('NFKC').toLowerCase().replace(/&/g, ' and ')
    .replace(/\b(?:feat|ft|featuring)\.?\b/g, ' ')
    .replace(/[^\p{L}\p{N}]+/gu, ' ').trim().replace(/\s+/g, ' ')
}

function songIdentityContext(track: Pick<TrackInfo, 'title' | 'artist'> & Partial<Pick<TrackInfo, 'album'>>) {
  const title = normalizeLyricIdentity(track.title)
  const exact = new Set<string>()
  const shortened = new Set<string>()
  if (!title) return { exact, shortened }
  const artists = new Set([track.artist, ...track.artist.split(/[/,，、&+]|\b(?:feat\.?|ft\.?|featuring)\b|\s+x\s+/iu)]
    .map(normalizeLyricIdentity).filter(term => term && term !== title))
  const addIdentity = (name: string, songTitle: string) => {
    exact.add(`${name}\u0000${songTitle}`)
    exact.add(`${songTitle}\u0000${name}`)
  }
  for (const name of artists) addIdentity(name, title)
  const album = normalizeLyricIdentity(track.album || '')
  if (album && album !== title) addIdentity(album, title)
  // 漏掉一个内部功能词的长英文标题仍须精确匹配歌手，避免泛化到普通歌词
  const words = title.split(' ')
  if (words.length >= 4 && words.every(word => /^[a-z]+$/.test(word))) {
    words.forEach((word, index) => {
      if (index === 0 || index === words.length - 1 || !TITLE_FUNCTION_WORDS.has(word)) return
      const songTitle = words.filter((_, wordIndex) => wordIndex !== index).join(' ')
      for (const name of artists) {
        shortened.add(`${name}\u0000${songTitle}`)
        shortened.add(`${songTitle}\u0000${name}`)
      }
    })
  }
  return { exact, shortened }
}

/** 只生成显示结果，保留原始歌词及正文的逐字时间轴 */
export function filterLyricCreditMetadataLines(
  lines: LyricLine[],
  track?: (Pick<TrackInfo, 'title' | 'artist'> & Partial<Pick<TrackInfo, 'album'>>) | null,
): LyricLine[] {
  const credits = lyricCreditMetadataMask(lines.map(line => line.text.trim() || line.words.map(word => word.text).join('')))
  const translationCredits = lyricCreditMetadataMask(lines.map(line => line.translation || ''))
  const romanCredits = lyricCreditMetadataMask(lines.map(line => line.roman || ''))
  const displayed = lines.flatMap((line, index) => {
    if (credits[index]) return []
    const hasCreditTranslation = translationCredits[index]
    const hasCreditRoman = romanCredits[index]
    if (!hasCreditTranslation && !hasCreditRoman) return [line]
    return [{
      ...line,
      translation: hasCreditTranslation ? undefined : line.translation,
      roman: hasCreditRoman ? undefined : line.roman,
    }]
  })
  if (!track) return displayed
  const identities = songIdentityContext(track)
  const body = displayed.filter((line, index) => {
    if (index >= LEADING_METADATA_SCAN_LINES && index + TRAILING_METADATA_SCAN_LINES < displayed.length) return true
    const text = line.text.trim() || line.words.map(word => word.text).join('')
    if (!/[-－–—/／|]/u.test(text)) return true
    for (const separator of text.matchAll(/[-－–—/／|]/gu)) {
      const left = normalizeLyricIdentity(text.slice(0, separator.index))
      const right = normalizeLyricIdentity(text.slice(separator.index + 1))
      const identity = `${left}\u0000${right}`
      if (identities.exact.has(identity) || identities.shortened.has(identity)) return false
    }
    return true
  })
  return body.some(line => line.text.trim() || line.words.some(word => word.text.trim())) ? body : displayed
}

/** 「//」之类的未翻译占位，占住这一行但不显示 */
function isUntranslatedPlaceholderText(text: string): boolean {
  const normalized = text.replace(/／/g, '/').replace(/\s/g, '')
  return normalized.length >= 2 && /^\/+$/.test(normalized)
}

interface TimedSpan { start: number; end: number }

function lineSpan(line: LyricLine): TimedSpan {
  const start = Number(line.startMs) || 0
  const words = Array.isArray(line.words) ? line.words : []
  const wordEnd = words.reduce((max, w) => Math.max(max, (Number(w.startMs) || 0) + (Number(w.durationMs) || 0)), 0)
  const end = Math.max(start + (Number(line.durationMs) || 0), wordEnd)
  return { start, end: end > start ? end : start + 1 }
}

function startDistanceToSpan(timestamp: number, span: TimedSpan): number {
  if (timestamp < span.start) return span.start - timestamp
  if (timestamp >= span.end) return timestamp - span.end + 1
  return 0
}

function spanOverlap(a: TimedSpan, b: TimedSpan): number {
  return Math.min(a.end, b.end) - Math.max(a.start, b.start)
}

type TranslationDecision = 'skip' | 'match' | 'stop'

function decideTranslationForLine(line: TimedSpan, next: TimedSpan | undefined, translation: TimedSpan): TranslationDecision {
  const currentDistance = startDistanceToSpan(translation.start, line)
  const nextDistance = next ? startDistanceToSpan(translation.start, next) : Infinity
  const currentOverlap = spanOverlap(line, translation)
  const nextOverlap = next ? spanOverlap(next, translation) : 0
  if (
    translation.start < line.start
    && currentDistance > TRANSLATION_CLOSEST_MATCH_TOLERANCE_MS
    && currentOverlap <= 0
  ) return 'skip'
  const matches = (currentOverlap > 0 && currentOverlap >= nextOverlap)
    || (currentDistance <= TRANSLATION_ALIGNMENT_TOLERANCE_MS && currentDistance <= nextDistance)
    || (currentDistance <= TRANSLATION_CLOSEST_MATCH_TOLERANCE_MS && currentDistance <= nextDistance)
  return matches ? 'match' : 'stop'
}

/**
 * 翻译行 → 原文行下标，移植 Android matchTranslationsToLineIndices
 *
 * 按时间顺序双指针推进：先看区间重叠，再按 450ms / 2000ms 容差取比下一行更近的那行；
 * 同一时间戳的多行（制作信息与正文同刻）作为一组，翻译向组尾对齐；制作信息翻译直接丢弃。
 * 网易云翻译与 YRC 行首常差 0.5~1s，单纯 450ms 最近匹配会整行丢翻译。
 */
export function matchTranslationsToLineIndices(
  lines: LyricLine[],
  translations: LyricLine[],
): Map<number, string> {
  const matches = new Map<number, string>()
  if (!lines.length || !translations.length) return matches
  const translationCredits = lyricCreditMetadataMask(translations.map(line => line.text || ''))
  const effective = translations
    .filter((tl, index) => (tl.text || '').trim() && !translationCredits[index])
    .sort((a, b) => (Number(a.startMs) || 0) - (Number(b.startMs) || 0))
  if (!effective.length) return matches
  // 原文里的制作信息行不接收翻译：它常紧挨着第一句正文，会把第一句的翻译抢走
  const credits = lyricCreditMetadataMask(lines.map(line => line.text || ''))
  const bodyIndices = lines.map((_, index) => index).filter(index => !credits[index])
  const candidates = bodyIndices.length ? bodyIndices : lines.map((_, index) => index)
  const spans = candidates.map(index => lineSpan(lines[index]))
  let translationIndex = 0
  let lineIndex = 0
  while (lineIndex < candidates.length && translationIndex < effective.length) {
    const groupStart = spans[lineIndex].start
    let groupEnd = lineIndex
    while (groupEnd < candidates.length && spans[groupEnd].start === groupStart) groupEnd++
    const groupSize = groupEnd - lineIndex
    const representative = spans[groupEnd - 1]
    const next = spans[groupEnd]
    const group: Array<string | null> = []
    while (translationIndex < effective.length && group.length < groupSize) {
      const translation = effective[translationIndex]
      const decision = decideTranslationForLine(representative, next, lineSpan(translation))
      if (decision === 'stop') break
      if (decision === 'match') {
        const text = translation.text || ''
        group.push(isUntranslatedPlaceholderText(text) ? null : text)
      }
      translationIndex++
    }
    group.forEach((text, offset) => {
      if (text != null) matches.set(candidates[groupEnd - group.length + offset], text)
    })
    lineIndex = groupEnd
  }
  return matches
}

/** 合并原词与翻译解析结果（匹配规则见 matchTranslationsToLineIndices） */
export function mergeParsedLyricsWithTranslations(
  original: LyricLine[],
  translations: LyricLine[],
): LyricLine[] {
  if (!translations.length) return original
  const result = original.map(line => ({ ...line }))
  for (const [index, text] of matchTranslationsToLineIndices(result, translations)) {
    result[index].translation = text
  }
  // 未被新翻译覆盖的行保留原 translation
  return result.map((line, i) => ({
    ...line,
    translation: line.translation || original[i].translation || undefined,
  }))
}

/** 音译行按时间轴并到原文行上，与翻译用同一套容差匹配 */
export function mergeParsedLyricsWithRomanization(
  original: LyricLine[],
  romanized: LyricLine[],
): LyricLine[] {
  if (!romanized.length) return original
  const merged = mergeParsedLyricsWithTranslations(
    original.map(line => ({ ...line, translation: line.roman })),
    romanized,
  )
  return original.map((line, index) => ({ ...line, roman: merged[index].translation || undefined }))
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

/** null 与缺省同义（Android `matchedRomanizedLyric == null` 即 ABSENT），只有空串才是有意清空 */
function readPayloadString(
  payload: Record<string, unknown>,
  camel: string,
  snake: string,
): string | undefined {
  for (const key of [camel, snake]) {
    const value = payload[key]
    if (value != null) return typeof value === 'string' ? value : String(value)
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

export function resolveStoredRomanizedLyricStateFromPayload(
  payload: Record<string, unknown> | undefined | null,
): StoredLyricState {
  return resolveStoredLyricState(
    payload,
    'matchedRomanizedLyric',
    'matched_romanized_lyric',
    'originalRomanizedLyric',
    'original_romanized_lyric',
  )
}

/**
 * syncPayload 里的歌词落地成歌词行：原文 + 翻译 + 音译
 *
 * null 表示没有本地歌词（可在线拉取），[] 表示有意清空；原文解析失败照常抛出，
 * 翻译或音译解析失败只丢掉那一轨。parse 的第二个参数区分原文与翻译/音译轨。
 */
export async function materializeStoredLyrics(
  payload: Record<string, unknown> | undefined | null,
  parse: (text: string, track: 'original' | 'secondary') => Promise<LyricLine[]>,
  onSecondaryError?: (error: unknown) => void,
): Promise<LyricLine[] | null> {
  const stored = resolveStoredLyricStateFromPayload(payload)
  if (stored.kind === 'absent') return null
  if (stored.kind === 'cleared') return []
  let lines = await parse(stored.text, 'original')
  const secondary = [
    [resolveStoredTranslatedLyricStateFromPayload(payload), mergeParsedLyricsWithTranslations],
    [resolveStoredRomanizedLyricStateFromPayload(payload), mergeParsedLyricsWithRomanization],
  ] as const
  for (const [state, merge] of secondary) {
    if (state.kind !== 'present' || !state.text.trim()) continue
    try {
      lines = merge(lines, await parse(state.text, 'secondary'))
    } catch (error) {
      onSecondaryError?.(error)
    }
  }
  return lines
}

/**
 * 能直接取网易云歌词的歌曲 ID，对齐 Android resolveKnownNeteaseLyricSongId：
 * 网易云来源的匹配 ID 优先，其次曲目本身是网易云的
 */
export function resolveKnownNeteaseLyricSongId(
  track: { id?: string | null; syncPayload?: Record<string, unknown> | null } | null | undefined,
): number | null {
  const payload = track?.syncPayload
  const source = String(payload?.matchedLyricSource ?? payload?.matched_lyric_source ?? '')
    .toLowerCase().replace(/[^a-z]/g, '')
  if (source === 'cloudmusic' || source === 'netease') {
    const matched = Number(payload?.matchedSongId ?? payload?.matched_song_id)
    if (Number.isSafeInteger(matched) && matched > 0) return matched
  }
  const id = track?.id ?? ''
  if (!id.startsWith('netease:')) return null
  const direct = Number(id.slice('netease:'.length))
  return Number.isSafeInteger(direct) && direct > 0 ? direct : null
}

/**
 * 歌词来自同步载荷又没有音译时，要不要去网易云补音译（Android loadNeteaseRomanizedFallback）：
 * 用户确认过音译（含有意清空）就不补
 */
export function shouldBackfillNeteaseRomanization(
  payload: Record<string, unknown> | undefined | null,
  lines: LyricLine[],
): boolean {
  if (!lines.length || lines.some(line => !!line.roman)) return false
  const edited = payload?.lyricSyncEdited ?? payload?.lyric_sync_edited
  return !(edited === true && resolveStoredRomanizedLyricStateFromPayload(payload).kind !== 'absent')
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
 *
 * nextRomanized 为 undefined 时不动音译（只改原文/翻译的旧调用方）；null 或空串清空音译，
 * 换了一份没有音译的歌词时要清掉，免得旧音译挂在新歌词上
 */
export function withUpdatedLyricsPayload(
  payload: Record<string, unknown> | undefined | null,
  nextLyric: string | null,
  nextTranslated: string | null,
  source?: string | null,
  now: number = Date.now(),
  nextRomanized?: string | null,
): Record<string, unknown> {
  const base: Record<string, unknown> = { ...(payload || {}) }
  const prevMatched = typeof base.matchedLyric === 'string'
    ? base.matchedLyric
    : (typeof base.matched_lyric === 'string' ? base.matched_lyric : null)
  const prevTranslated = typeof base.matchedTranslatedLyric === 'string'
    ? base.matchedTranslatedLyric
    : (typeof base.matched_translated_lyric === 'string' ? base.matched_translated_lyric : null)
  const prevRomanized = typeof base.matchedRomanizedLyric === 'string'
    ? base.matchedRomanizedLyric
    : (typeof base.matched_romanized_lyric === 'string' ? base.matched_romanized_lyric : null)

  if (
    nextRomanized !== undefined
    && base.originalRomanizedLyric == null
    && base.original_romanized_lyric == null
    && prevRomanized
  ) {
    base.originalRomanizedLyric = prevRomanized
  }
  if (nextRomanized !== undefined) {
    base.matchedRomanizedLyric = nextRomanized?.trim() ? nextRomanized : ''
    delete base.matched_romanized_lyric
  }

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
