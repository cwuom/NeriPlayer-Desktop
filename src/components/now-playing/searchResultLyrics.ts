import type { LyricLine } from '@/types/track'
import { mapBackendLyrics, mergeParsedLyricsWithTranslations } from '@/modules/lyrics/lyricsFormat'
import type { InfoSearchResult } from './types'

/** 歌词来源标记: 对齐 Android MusicPlatform 枚举名, 其余平台名 Android 侧解析为 null 亦无害 */
export function lyricSourceForPlatform(source?: string | null): string {
  const key = String(source || '').toLowerCase()
  if (key.includes('netease')) return 'CLOUD_MUSIC'
  if (key.includes('qq')) return 'QQ_MUSIC'
  return key ? key.toUpperCase() : 'LOCAL_EDIT'
}

// 「获取歌曲信息」的搜索结果自带歌词时直接解析
export async function parseLyricsFromSearchResult(
  result: InfoSearchResult,
  parseLrc: (content: string) => Promise<unknown[]>,
): Promise<{
  lines: LyricLine[]
  rawLyric: string
  rawTranslated: string | null
} | null> {
  if (result.synced_lyrics) {
    const rawLyric = String(result.synced_lyrics)
    const rawTranslated = result.translated_lyrics ? String(result.translated_lyrics) : null
    const parsed = await parseLrc(rawLyric)
    let parsedTranslations: unknown[] = []
    if (rawTranslated) {
      try {
        parsedTranslations = await parseLrc(rawTranslated)
      } catch {
        // 翻译解析失败不影响原文，按无翻译处理
      }
    }
    return {
      lines: mergeParsedLyricsWithTranslations(
        mapBackendLyrics(parsed),
        mapBackendLyrics(parsedTranslations),
      ),
      rawLyric,
      rawTranslated,
    }
  }

  if (result.plain_lyrics) {
    const rawLyric = String(result.plain_lyrics)
    const lines = rawLyric
      .split(/\r?\n/)
      .map((line: string) => line.trim())
      .filter(Boolean)
      .map((line: string, index: number) => ({
        startMs: index * 3000,
        durationMs: 3000,
        words: [] as LyricLine['words'],
        text: line,
        translation: undefined as string | undefined,
      }))
    return { lines, rawLyric, rawTranslated: null }
  }

  return null
}
