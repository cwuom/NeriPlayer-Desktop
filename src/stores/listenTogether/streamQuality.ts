// 一起听共享直链的音质标记与挑选（对齐 Android ListenTogetherQualityPolicy /
// PlayerManagerListenTogetherStreamExtensions）
//
// 房主给每条直链加 `#neriplayer-ltw-quality=<来源>:<音质>` 片段，听众按自己的音质偏好挑最接近的一条。
// URL 片段不会随 HTTP 请求发出，带着标记播放不受影响
import { LtChannels } from './protocol'

const QUALITY_FRAGMENT_KEY = 'neriplayer-ltw-quality='

const QUALITY_ORDER: Record<string, readonly string[]> = {
  [LtChannels.NETEASE]: ['standard', 'higher', 'exhigh', 'lossless', 'hires', 'jyeffect', 'sky', 'jymaster'],
  [LtChannels.BILIBILI]: ['low', 'medium', 'high', 'lossless', 'hires', 'dolby'],
  [LtChannels.YOUTUBE_MUSIC]: ['low', 'medium', 'high', 'very_high'],
}

const SOURCE_KEY: Record<string, string> = {
  [LtChannels.NETEASE]: 'netease',
  [LtChannels.BILIBILI]: 'bili',
  [LtChannels.YOUTUBE_MUSIC]: 'youtube',
}

/** 每个平台最多分享几条直链 */
export const LT_SHARE_LIMITS: Record<string, number> = {
  [LtChannels.NETEASE]: 3,
  [LtChannels.BILIBILI]: 2,
  [LtChannels.YOUTUBE_MUSIC]: 1,
}

const NETEASE_SHARE_GROUPS: readonly (readonly string[])[] = [
  ['exhigh', 'higher', 'standard'],
  ['lossless'],
  ['sky'],
]
const BILI_HIGH_FALLBACK = ['high', 'medium', 'low']

/** 播放层的来源（youtube）换成一起听频道（youtubeMusic） */
export function ltChannelForSource(source?: string | null): string | null {
  if (source === 'netease') return LtChannels.NETEASE
  if (source === 'bilibili') return LtChannels.BILIBILI
  if (source === 'youtube') return LtChannels.YOUTUBE_MUSIC
  return null
}

function normalizeQuality(channelId: string, quality?: string | null): string | null {
  const key = quality?.trim().toLowerCase()
  return key && QUALITY_ORDER[channelId]?.includes(key) ? key : null
}

function qualityRank(channelId: string, quality: string | null): number | null {
  if (!quality) return null
  const rank = QUALITY_ORDER[channelId]?.indexOf(quality) ?? -1
  return rank >= 0 ? rank : null
}

function fragmentParts(url: string): string[] {
  const hash = url.indexOf('#')
  return hash < 0 ? [] : url.slice(hash + 1).split('&').filter(Boolean)
}

/** 链接里是否已经带有音质标记（房主给的，不能改写） */
export function hasLtStreamQuality(url: string): boolean {
  return fragmentParts(url).some(part => part.startsWith(QUALITY_FRAGMENT_KEY))
}

export function ltStreamQuality(url: string, channelId: string): string | null {
  const sourceKey = SOURCE_KEY[channelId]
  if (!sourceKey) return null
  const prefix = `${QUALITY_FRAGMENT_KEY}${sourceKey}:`
  const part = fragmentParts(url).find(item => item.startsWith(prefix))
  return part ? normalizeQuality(channelId, part.slice(prefix.length)) : null
}

/** 加上（或替换成）音质标记；未知来源或音质时原样返回 */
export function decorateLtStreamUrl(url: string, channelId: string, quality?: string | null): string {
  const sourceKey = SOURCE_KEY[channelId]
  const normalized = normalizeQuality(channelId, quality)
  if (!sourceKey || !normalized) return url
  const hash = url.indexOf('#')
  const base = hash < 0 ? url : url.slice(0, hash)
  const kept = fragmentParts(url).filter(part => !part.startsWith(QUALITY_FRAGMENT_KEY))
  return `${base}#${[...kept, `${QUALITY_FRAGMENT_KEY}${sourceKey}:${normalized}`].join('&')}`
}

/**
 * 听众按自己的音质偏好排候选：离偏好最近的在前，同样近时不超过偏好的优先、再按音质从高到低，
 * 没有标记的排最后并保持原顺序
 */
export function orderLtStreamUrlsForPreference(
  urls: readonly string[],
  channelId: string,
  preferredQuality: string,
): string[] {
  const preferredRank = qualityRank(channelId, normalizeQuality(channelId, preferredQuality)) ?? 0
  const seenBases = new Set<string>()
  const items = urls
    .map(url => url.trim())
    .filter((url) => {
      const base = url.split('#')[0]
      if (!url || seenBases.has(base)) return false
      seenBases.add(base)
      return true
    })
    .map((url, index) => ({ url, index, rank: qualityRank(channelId, ltStreamQuality(url, channelId)) }))
  const distance = (rank: number | null) => rank === null ? Number.MAX_SAFE_INTEGER : Math.abs(rank - preferredRank)
  const side = (rank: number | null) => rank === null ? 2 : rank <= preferredRank ? 0 : 1
  return items
    .sort((left, right) =>
      distance(left.rank) - distance(right.rank)
      || side(left.rank) - side(right.rank)
      || (right.rank ?? -1) - (left.rank ?? -1)
      || left.index - right.index)
    .map(item => item.url)
}

/** 网易云分享时依次尝试的音质组：偏好音质一组，其余是 极高/较高/标准、无损、超清母带，最多三组 */
export function neteaseShareQualityGroups(preferredQuality: string): string[][] {
  const preferred = normalizeQuality(LtChannels.NETEASE, preferredQuality) ?? 'exhigh'
  const preferredGroup = preferred === 'exhigh' ? [...NETEASE_SHARE_GROUPS[0]] : [preferred]
  const groups = [preferredGroup]
  for (const group of NETEASE_SHARE_GROUPS) {
    if (!groups.some(existing => existing.join() === group.join())) groups.push([...group])
  }
  return groups.slice(0, 3)
}

/** B 站分享的音质：偏好音质、高→中→低里第一个可用的、无损，最多两条 */
export function biliShareQualityOrder(preferredQuality: string, available: ReadonlySet<string>): string[] {
  const preferred = preferredQuality.trim().toLowerCase() || 'high'
  const selected: string[] = []
  const addFirstAvailable = (qualities: readonly string[]) => {
    const quality = qualities.find(item => available.has(item))
    if (quality && !selected.includes(quality)) selected.push(quality)
    return quality
  }
  if (preferred !== 'high') addFirstAvailable([preferred])
  const primary = addFirstAvailable(BILI_HIGH_FALLBACK)
  if (selected.length < 2) addFirstAvailable(['lossless'])
  if (selected.length < 2 && !available.has('high') && primary) {
    addFirstAvailable(BILI_HIGH_FALLBACK.slice(BILI_HIGH_FALLBACK.indexOf(primary) + 1))
  }
  return selected.slice(0, 2)
}
