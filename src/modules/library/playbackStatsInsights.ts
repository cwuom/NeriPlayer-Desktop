// 播放统计洞察：对齐 Android PlaybackStatsInsightsQuery / PlaybackStatsSort，
// 但桌面端没有按天分桶的明细，只能从当前区间的曲目汇总里派生排行与平均值
import { displayAlbum } from '@/modules/library/albumDisplay'

/** 统计页需要的曲目汇总字段，store 的 TrackStat 结构上兼容 */
export interface StatRecord {
  identityKey: string
  name: string
  artist: string
  album: string
  albumId: string
  coverUrl: string | null
  totalListenMs: number
  playCount: number
  lastPlayedAt: number
  firstPlayedAt: number
}

export type StatsSortMode = 'play_count' | 'listen_time' | 'recent' | 'first_played'

export const STATS_SORT_MODES: StatsSortMode[] = ['play_count', 'listen_time', 'recent', 'first_played']

export interface StatsRank {
  key: string
  name: string
  trackCount: number
  totalPlayCount: number
  totalListenMs: number
  /** 占本期总收听时长的比例，0..1 */
  share: number
  coverUrl: string | null
}

export interface StatsInsights<T extends StatRecord> {
  topTrack: T | null
  artists: StatsRank[]
  albums: StatsRank[]
  earliestListenAt: number | null
  averageListenMsPerPlay: number
}

// 只认规范的「 / 」分隔，AC/DC 这类名字里的斜杠不能被拆开（与 Android 一致）
const ARTIST_SEPARATOR = ' / '
const UNKNOWN_ARTISTS = new Set(['', 'unknown', 'unknown artist', '未知艺术家', '未知歌手'])
const PLACEHOLDER_ALBUMS = new Set([
  '', 'netease', 'bilibili', 'youtube music', 'youtubemusic', 'local files', 'local music', 'local_files',
  '__local_files__', 'qq music', 'unknown album', '本地音乐', '本地文件', '未知专辑',
])

function positive(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0
}

export function statShare(value: number, total: number): number {
  const denominator = positive(total)
  if (!denominator) return 0
  return Math.min(1, positive(value) / denominator)
}

/**
 * 后端字段的实际类型不总与声明一致（网易云 albumId 是数字，缺省值可能是 null），
 * 统计页在真实数据上曾因对数字调用 .trim() 抛错导致整页卡死，这里统一转成字符串
 */
export function statText(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'bigint') return String(value)
  return ''
}

export function splitStatArtists(raw: unknown): string[] {
  const seen = new Set<string>()
  const names: string[] = []
  for (const part of statText(raw).split(ARTIST_SEPARATOR)) {
    const name = part.trim()
    const key = name.toLowerCase()
    if (UNKNOWN_ARTISTS.has(key) || seen.has(key)) continue
    seen.add(key)
    names.push(name)
  }
  return names
}

/** 专辑展示名；平台占位名返回空串，不参与排行 */
export function statAlbumName(album: unknown): string {
  const name = displayAlbum(statText(album))
  return PLACEHOLDER_ALBUMS.has(name.toLowerCase()) ? '' : name
}

interface RankGroup {
  key: string
  name: string
  trackCount: number
  totalPlayCount: number
  totalListenMs: number
  coverUrl: string | null
  coverListenMs: number
}

function compareRanks(left: StatsRank, right: StatsRank): number {
  return right.totalListenMs - left.totalListenMs
    || right.totalPlayCount - left.totalPlayCount
    || left.name.toLowerCase().localeCompare(right.name.toLowerCase())
    || left.key.localeCompare(right.key)
}

function rank(
  items: readonly StatRecord[],
  totalListenMs: number,
  members: (item: StatRecord) => Array<{ key: string; name: string }>,
): StatsRank[] {
  const groups = new Map<string, RankGroup>()
  for (const item of items) {
    const listenMs = positive(item.totalListenMs)
    for (const member of members(item)) {
      let group = groups.get(member.key)
      if (!group) {
        group = { key: member.key, name: member.name, trackCount: 0, totalPlayCount: 0, totalListenMs: 0, coverUrl: null, coverListenMs: -1 }
        groups.set(member.key, group)
      }
      group.trackCount += 1
      group.totalPlayCount += positive(item.playCount)
      group.totalListenMs += listenMs
      // 用组内听得最久的那首的封面代表整组，比任取一张更贴近用户印象
      if (item.coverUrl && listenMs > group.coverListenMs) {
        group.coverUrl = item.coverUrl
        group.coverListenMs = listenMs
      }
    }
  }
  return [...groups.values()]
    .map((group): StatsRank => ({
      key: group.key,
      name: group.name,
      trackCount: group.trackCount,
      totalPlayCount: group.totalPlayCount,
      totalListenMs: group.totalListenMs,
      share: statShare(group.totalListenMs, totalListenMs),
      coverUrl: group.coverUrl,
    }))
    .sort(compareRanks)
}

export function rankStatArtists(items: readonly StatRecord[], totalListenMs: number): StatsRank[] {
  return rank(items, totalListenMs, item => splitStatArtists(item.artist).map(name => ({ key: name.toLowerCase(), name })))
}

export function rankStatAlbums(items: readonly StatRecord[], totalListenMs: number): StatsRank[] {
  return rank(items, totalListenMs, (item) => {
    const name = statAlbumName(item.album)
    if (!name) return []
    const albumId = statText(item.albumId).trim()
    if (albumId && albumId !== '0') return [{ key: `id:${albumId}`, name }]
    // 没有专辑 id 时按「专辑名 + 第一位艺人」区分同名专辑
    const primaryArtist = splitStatArtists(item.artist)[0]?.toLowerCase() || statText(item.identityKey)
    return [{ key: `name:${name.toLowerCase()}\u0000${primaryArtist}`, name }]
  })
}

export function statChartValue(item: StatRecord, mode: StatsSortMode): number {
  return mode === 'listen_time' ? positive(item.totalListenMs) : positive(item.playCount)
}

function byTieBreak(left: StatRecord, right: StatRecord): number {
  return statText(left.identityKey).localeCompare(statText(right.identityKey))
}

export function sortStatTracks<T extends StatRecord>(items: readonly T[], mode: StatsSortMode): T[] {
  const sorted = [...items]
  switch (mode) {
    case 'play_count':
      return sorted.sort((a, b) => positive(b.playCount) - positive(a.playCount)
        || positive(b.totalListenMs) - positive(a.totalListenMs) || byTieBreak(a, b))
    case 'listen_time':
      return sorted.sort((a, b) => positive(b.totalListenMs) - positive(a.totalListenMs)
        || positive(b.playCount) - positive(a.playCount) || byTieBreak(a, b))
    case 'recent':
      return sorted.sort((a, b) => positive(b.lastPlayedAt) - positive(a.lastPlayedAt) || byTieBreak(a, b))
    case 'first_played':
      // 首次播放升序；缺失首次时间的旧记录排在最后
      return sorted.sort((a, b) => (positive(a.firstPlayedAt) || Number.MAX_SAFE_INTEGER)
        - (positive(b.firstPlayedAt) || Number.MAX_SAFE_INTEGER) || byTieBreak(a, b))
  }
}

export function buildStatsInsights<T extends StatRecord>(
  items: readonly T[],
  totals: { totalPlayCount: number; totalListenMs: number },
): StatsInsights<T> {
  let topTrack: T | null = null
  let earliest = 0
  for (const item of items) {
    if (positive(item.totalListenMs) > 0 && (!topTrack || item.totalListenMs > topTrack.totalListenMs)) topTrack = item
    const first = positive(item.firstPlayedAt)
    if (first && (!earliest || first < earliest)) earliest = first
  }
  const plays = positive(totals.totalPlayCount)
  return {
    topTrack,
    artists: rankStatArtists(items, totals.totalListenMs),
    albums: rankStatAlbums(items, totals.totalListenMs),
    earliestListenAt: earliest || null,
    averageListenMsPerPlay: plays ? Math.floor(positive(totals.totalListenMs) / plays) : 0,
  }
}
