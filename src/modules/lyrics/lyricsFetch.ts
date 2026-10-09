// 向后端取歌词：结果带上歌词实际来自哪个来源（netease、qq、kugou、lrclib、amll_ttml、youtube、local）
import { invoke } from '@tauri-apps/api/core'
import type { LyricLine } from '@/stores/player'
import type { DefaultLyricSource } from '@/stores/settings'
import { mapBackendLyrics, toEditableRomanizationText } from './lyricsFormat'
import { defaultLyricMatchKeyword, matchLyrics, type LyricMatchSource } from './lyricMatch'
import { hasWordTimedLyrics } from './lyricsRequest'

export interface FetchedLyrics {
  /** 没找到歌词时为空 */
  source: string | null
  lines: LyricLine[]
}

export interface FetchLyricsArgs {
  title: string
  artist: string
  durationSecs: number
  audioPath: string | null
  neteaseId: number | null
  qqSongMid: string | null
  youtubeVideoId: string | null
}

interface BackendFetchedLyrics {
  source?: string | null
  lines?: unknown[]
}

function fromBackend(raw: BackendFetchedLyrics | null | undefined): FetchedLyrics {
  const lines = mapBackendLyrics(Array.isArray(raw?.lines) ? raw.lines : [])
  return { source: lines.length ? raw?.source ?? null : null, lines }
}

/** 多源瀑布取歌词（平台 id、LRCLIB、搜索匹配、YouTube 原生、AMLL/酷狗兜底） */
export async function fetchLyrics(args: FetchLyricsArgs): Promise<FetchedLyrics> {
  return fromBackend(await invoke<BackendFetchedLyrics>('fetch_lyrics', { ...args }))
}

interface AutomaticLyricsTrack {
  id: string
  title: string
  artist: string
  durationMs?: number | null
  audioUrl?: string | null
}

export function fetchLyricsArgsFor(track: AutomaticLyricsTrack): FetchLyricsArgs {
  const neteaseId = track.id.startsWith('netease:') ? Number.parseInt(track.id.slice('netease:'.length), 10) : NaN
  return {
    title: track.title,
    artist: track.artist,
    // 换歌瞬间 player.durationMs 仍是上一首的值, 会误触发后端时长硬门槛拒掉正确
    // 歌词。只用曲目自带时长，未知时传 0（后端对 0 不设门槛）（LY-12）
    durationSecs: Math.floor((track.durationMs || 0) / 1000),
    audioPath: track.audioUrl || null,
    neteaseId: Number.isFinite(neteaseId) && neteaseId > 0 ? neteaseId : null,
    qqSongMid: track.id.startsWith('qq:') ? track.id.slice('qq:'.length) || null : null,
    youtubeVideoId: track.id.startsWith('youtube:') ? track.id.slice('youtube:'.length) || null : null,
  }
}

/**
 * 自动取词的在线部分（同步载荷、本地缓存、默认歌词源都没给出结果之后）。
 * 正在播放页、桌面歌词、下一首预取共用，经 loadLyricsSingleFlight 合并时三处拿到的是同一种结果
 */
export async function fetchAutomaticLyrics(
  track: AutomaticLyricsTrack,
  playbackSource: string | null | undefined,
  preferWordTimed: boolean,
): Promise<FetchedLyrics> {
  if (prefersWordTimedLyricsFirst(playbackSource, preferWordTimed)) {
    const wordTimed = await fetchWordTimedLyrics({
      title: track.title, artist: track.artist, durationMs: track.durationMs || 0,
    }).catch(() => null)
    if (wordTimed && hasWordTimedLyrics(wordTimed.lines)) return wordTimed
  }
  return fetchLyrics(fetchLyricsArgsFor(track))
}

const PREFERRED_MATCH_SOURCES: Record<Exclude<DefaultLyricSource, 'automatic'>, LyricMatchSource> = {
  cloud_music: 'netease',
  kugou: 'kugou',
  qq_music: 'qq',
  lrclib: 'lrclib',
  amll_ttml: 'amll_ttml',
}

/**
 * 默认歌词源要先试的平台，对齐 Android shouldTryPreferredLyricSource：自动、本地曲目不试；
 * 网易云/QQ 曲目选了自家平台时，自动取词本来就先按 ID 取，也不必另试
 */
export function preferredLyricMatchSource(
  playbackSource: string | null | undefined,
  preference: DefaultLyricSource,
): LyricMatchSource | null {
  if (preference === 'automatic' || !playbackSource) return null
  const source = PREFERRED_MATCH_SOURCES[preference]
  if (!source || (source === 'netease' && playbackSource === 'netease') || (source === 'qq' && playbackSource === 'qq')) {
    return null
  }
  return source
}

/**
 * B 站没有平台歌词：Android 开着「优先使用逐词歌词」时先取 AMLL TTML，没有才回退。
 * 先搜网易云会拿到另一份歌词并套上网易云的默认偏移，两端对不上
 */
export function prefersWordTimedLyricsFirst(playbackSource: string | null | undefined, preferWordTimed: boolean): boolean {
  return preferWordTimed && playbackSource === 'bilibili'
}

/** 按默认歌词源匹配：只收歌名、歌手、时长都对得上的高置信度结果，找不到返回 null 回退自动 */
export async function fetchPreferredSourceLyrics(
  track: { title: string; artist: string; album?: string | null; durationMs?: number | null },
  source: LyricMatchSource,
  preferWordTimed: boolean,
): Promise<FetchedLyrics | null> {
  const durationMs = track.durationMs || 0
  if (durationMs <= 0 || !track.title.trim() || !track.artist.trim()) return null
  const results = await matchLyrics({
    keyword: defaultLyricMatchKeyword(track.title, track.artist),
    title: track.title,
    artist: track.artist,
    album: track.album || '',
    durationMs,
    preferWordTimed,
    sources: [source],
  })
  const best = results.find(result => result.source === source && result.confidence === 'high')
  return best ? { source, lines: best.lines } : null
}

/** 网易云这首歌的音译轨原文（romalrc），没有时为 null */
export async function fetchNeteaseRomanizedLyric(songId: number): Promise<string | null> {
  return (await invoke<string | null>('fetch_netease_romanized_lyric', { songId })) ?? null
}

// 匹配一次要搜网易云，同一首歌（含没找到）在本次运行里只查一次
const MAX_ROMANIZATION_CACHE = 200
const romanizationByMatch = new Map<string, Promise<string | null>>()

/**
 * 网易云音译轨（LRC 文本），对齐 Android loadNeteaseRomanizedFallback：已知网易云 ID 直接取，
 * 否则按歌名、歌手、时长高置信度匹配网易云，取第一个带音译的结果
 */
export async function fetchNeteaseRomanization(
  track: { id?: string | null; title: string; artist: string; album?: string | null; durationMs?: number | null },
  songId: number | null,
): Promise<string | null> {
  if (songId) {
    // 按 ID 取不到（接口失败、后端还没有这条命令）时退回匹配：匹配结果里的网易云歌词已并好音译
    const byId = await fetchNeteaseRomanizedLyric(songId).catch(() => null)
    if (byId) return byId
  }
  const durationMs = track.durationMs || 0
  if (durationMs <= 0 || !track.title.trim() || !track.artist.trim()) return null
  const key = [track.id ?? '', track.title.trim(), track.artist.trim(), durationMs].join('\u0000')
  let pending = romanizationByMatch.get(key)
  if (!pending) {
    pending = matchLyrics({
      keyword: defaultLyricMatchKeyword(track.title, track.artist),
      title: track.title,
      artist: track.artist,
      album: track.album || '',
      durationMs,
      preferWordTimed: false,
      sources: ['netease'],
    }).then((results) => {
      const withRoman = results.filter(result => result.source === 'netease' && result.hasRomanization)
      // 已知 ID 时认准这首；否则只收高置信度
      const best = (songId ? withRoman.find(result => result.id === String(songId)) : null)
        ?? withRoman.find(result => result.confidence === 'high')
      return best ? toEditableRomanizationText(best.lines) || null : null
    })
    // 网络失败不记住，下次还能再试
    pending.catch(() => romanizationByMatch.delete(key))
    romanizationByMatch.set(key, pending)
    if (romanizationByMatch.size > MAX_ROMANIZATION_CACHE) {
      const oldest = romanizationByMatch.keys().next().value
      if (oldest !== undefined) romanizationByMatch.delete(oldest)
    }
  }
  return pending
}

/** 只要逐字时间轴的歌词（AMLL TTML，其次酷狗 KRC） */
export async function fetchWordTimedLyrics(args: {
  title: string
  artist: string
  durationMs: number
}): Promise<FetchedLyrics> {
  return fromBackend(await invoke<BackendFetchedLyrics>('fetch_word_timed_lyrics', { ...args }))
}
