import { describe, expect, it } from 'vitest'
import { extractQuotedSongTitle, lyricSearchTitles } from '@/modules/lyrics/lyricSearchTitle'

describe('extractQuotedSongTitle', () => {
  it('extracts the song name inside book-title marks from video titles', () => {
    expect(extractQuotedSongTitle('【洛天依原创】《歌行四方》PV付')).toBe('歌行四方')
    expect(extractQuotedSongTitle('兰音 《 夜航星 》 翻唱')).toBe('夜航星')
    expect(extractQuotedSongTitle('初音ミク『千本桜』オリジナル')).toBe('千本桜')
    expect(extractQuotedSongTitle('【MV】「夜に駆ける」')).toBe('夜に駆ける')
  })

  it('prefers 《》 over corner brackets and the first non-empty mark', () => {
    expect(extractQuotedSongTitle('「翻唱」《春庭雪》')).toBe('春庭雪')
    expect(extractQuotedSongTitle('《》《赤伶》《其它》')).toBe('赤伶')
  })

  it('ignores marks inside parentheses, unmatched marks and plain titles', () => {
    expect(extractQuotedSongTitle('光年之外 (《太空旅客》电影中国区主题曲)')).toBeNull()
    expect(extractQuotedSongTitle('凉凉（电视剧《三生三世十里桃花》片尾曲）')).toBeNull()
    expect(extractQuotedSongTitle('《未闭合')).toBeNull()
    expect(extractQuotedSongTitle('Lemon')).toBeNull()
    expect(extractQuotedSongTitle(null)).toBeNull()
  })
})

describe('lyricSearchTitles', () => {
  it('tries the extracted song name first and keeps the original title as fallback', () => {
    expect(lyricSearchTitles('【洛天依】《歌行四方》')).toEqual(['歌行四方', '【洛天依】《歌行四方》'])
    expect(lyricSearchTitles('Lemon')).toEqual(['Lemon'])
  })
})
