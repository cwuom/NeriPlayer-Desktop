import { describe, expect, it } from 'vitest'
import {
  buildStatsInsights,
  rankStatAlbums,
  rankStatArtists,
  sortStatTracks,
  splitStatArtists,
  statAlbumName,
  statChartValue,
  statShare,
  type StatRecord,
} from '@/modules/library/playbackStatsInsights'

function stat(id: string, patch: Partial<StatRecord> = {}): StatRecord {
  return {
    identityKey: id,
    name: `song ${id}`,
    artist: 'Artist',
    album: '',
    albumId: '',
    coverUrl: null,
    totalListenMs: 0,
    playCount: 0,
    lastPlayedAt: 0,
    firstPlayedAt: 0,
    ...patch,
  }
}

describe('splitStatArtists', () => {
  it('splits only on the canonical spaced separator and keeps names like AC/DC intact', () => {
    expect(splitStatArtists('AC/DC / Queen')).toEqual(['AC/DC', 'Queen'])
  })

  it('drops unknown placeholders and case-insensitive duplicates', () => {
    expect(splitStatArtists('未知歌手 / YOASOBI / yoasobi / Unknown Artist / ')).toEqual(['YOASOBI'])
  })
})

describe('statAlbumName', () => {
  it('cleans platform prefixes and hides placeholder albums', () => {
    expect(statAlbumName('Netease 夜に駆ける')).toBe('夜に駆ける')
    expect(statAlbumName('Bilibili|BV1xx')).toBe('')
    expect(statAlbumName('本地音乐')).toBe('')
    expect(statAlbumName('  Local Files ')).toBe('')
  })
})

describe('rankings', () => {
  const items = [
    stat('a', { artist: 'YOASOBI / Ayase', album: 'THE BOOK', albumId: '1', totalListenMs: 600_000, playCount: 3, coverUrl: 'cover-a' }),
    stat('b', { artist: 'YOASOBI', album: 'THE BOOK', albumId: '1', totalListenMs: 900_000, playCount: 4, coverUrl: 'cover-b' }),
    stat('c', { artist: 'Oasis', album: 'Morning Glory', totalListenMs: 300_000, playCount: 1 }),
    stat('d', { artist: 'Blur', album: 'Morning Glory', totalListenMs: 100_000, playCount: 1 }),
  ]
  const total = 1_900_000

  it('credits every artist of a track and orders by listening time', () => {
    const artists = rankStatArtists(items, total)
    expect(artists.map(rank => rank.name)).toEqual(['YOASOBI', 'Ayase', 'Oasis', 'Blur'])
    expect(artists[0]).toMatchObject({ trackCount: 2, totalPlayCount: 7, totalListenMs: 1_500_000, coverUrl: 'cover-b' })
    expect(artists[0].share).toBeCloseTo(1_500_000 / total)
  })

  it('groups albums by id, and by name plus primary artist when the id is missing', () => {
    const albums = rankStatAlbums(items, total)
    expect(albums.map(rank => [rank.name, rank.trackCount])).toEqual([
      ['THE BOOK', 2],
      ['Morning Glory', 1],
      ['Morning Glory', 1],
    ])
  })

  it('clamps shares into 0..1 and tolerates an empty total', () => {
    expect(statShare(5, 0)).toBe(0)
    expect(statShare(10, 5)).toBe(1)
    expect(statShare(-3, 10)).toBe(0)
  })
})

describe('sortStatTracks', () => {
  const items = [
    stat('a', { playCount: 2, totalListenMs: 100, lastPlayedAt: 30, firstPlayedAt: 0 }),
    stat('b', { playCount: 5, totalListenMs: 50, lastPlayedAt: 10, firstPlayedAt: 5 }),
    stat('c', { playCount: 2, totalListenMs: 300, lastPlayedAt: 20, firstPlayedAt: 2 }),
  ]

  it.each([
    ['play_count', ['b', 'c', 'a']],
    ['listen_time', ['c', 'a', 'b']],
    ['recent', ['a', 'c', 'b']],
    ['first_played', ['c', 'b', 'a']],
  ] as const)('sorts by %s like Android', (mode, expected) => {
    expect(sortStatTracks(items, mode).map(item => item.identityKey)).toEqual(expected)
  })

  it('does not mutate the input and charts listening time only for that mode', () => {
    sortStatTracks(items, 'listen_time')
    expect(items.map(item => item.identityKey)).toEqual(['a', 'b', 'c'])
    expect(statChartValue(items[0], 'listen_time')).toBe(100)
    expect(statChartValue(items[0], 'recent')).toBe(2)
  })
})

describe('buildStatsInsights', () => {
  it('picks the longest-listened track and the earliest recorded listen', () => {
    const items = [
      stat('a', { totalListenMs: 100, firstPlayedAt: 50 }),
      stat('b', { totalListenMs: 900, firstPlayedAt: 0 }),
      stat('c', { totalListenMs: 300, firstPlayedAt: 20 }),
    ]
    const insights = buildStatsInsights(items, { totalPlayCount: 4, totalListenMs: 1_300 })
    expect(insights.topTrack?.identityKey).toBe('b')
    expect(insights.earliestListenAt).toBe(20)
    expect(insights.averageListenMsPerPlay).toBe(325)
  })

  it('returns an empty spotlight when nothing has listening time', () => {
    const insights = buildStatsInsights([stat('a', { playCount: 1 })], { totalPlayCount: 1, totalListenMs: 0 })
    expect(insights.topTrack).toBeNull()
    expect(insights.earliestListenAt).toBeNull()
    expect(insights.averageListenMsPerPlay).toBe(0)
  })
})

describe('real backend payload shapes', () => {
  // 回归：网易云 albumId 实际是数字，曾在 .trim() 处抛错导致统计页整页卡死
  it('groups albums by numeric ids and tolerates null text fields', () => {
    const raw = [
      stat('a', { album: 'I Look To You', albumId: 3_084_207 as unknown as string, artist: 'Whitney Houston', totalListenMs: 300 }),
      stat('b', { album: 'I Look To You', albumId: 3_084_207 as unknown as string, artist: 'Whitney Houston', totalListenMs: 100 }),
      stat('c', { album: null as unknown as string, albumId: null as unknown as string, artist: null as unknown as string, totalListenMs: 50 }),
    ]
    expect(() => buildStatsInsights(raw, { totalPlayCount: 3, totalListenMs: 450 })).not.toThrow()
    const albums = rankStatAlbums(raw, 450)
    expect(albums).toHaveLength(1)
    expect(albums[0]).toMatchObject({ key: 'id:3084207', name: 'I Look To You', trackCount: 2 })
    expect(rankStatArtists(raw, 450).map(rank => rank.name)).toEqual(['Whitney Houston'])
    expect(() => sortStatTracks(raw.map(item => ({ ...item, identityKey: null as unknown as string })), 'play_count')).not.toThrow()
  })
})
