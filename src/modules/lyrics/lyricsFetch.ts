// 向后端取歌词：结果带上歌词实际来自哪个来源（netease、qq、kugou、lrclib、amll_ttml、youtube、local）
import { invoke } from '@tauri-apps/api/core'
import type { LyricLine } from '@/stores/player'
import { mapBackendLyrics } from './lyricsFormat'

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

/** 只要逐字时间轴的歌词（AMLL TTML，其次酷狗 KRC） */
export async function fetchWordTimedLyrics(args: {
  title: string
  artist: string
  durationMs: number
}): Promise<FetchedLyrics> {
  return fromBackend(await invoke<BackendFetchedLyrics>('fetch_word_timed_lyrics', { ...args }))
}
