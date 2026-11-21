import type { TrackInfo } from '@/types/track'
import { asArray, asRecord, asString, dig, objectChildren, type JsonRecord } from '@/utils/json'

export interface YouTubeArtistHeader { name: string; coverUrl: string; subtitle: string; description: string; subscribers: string; listeners: string }
export interface YouTubeArtistItem {
  kind: 'song' | 'video' | 'album' | 'playlist' | 'artist'
  title: string; subtitle: string; coverUrl: string; videoId: string; browseId: string
  artist: string; album: string; durationMs: number
}
export interface YouTubeArtistEndpoint { browseId: string; params: string }
export interface YouTubeArtistSection { title: string; items: YouTubeArtistItem[]; moreEndpoint: YouTubeArtistEndpoint | null }
export interface YouTubeArtistDetail { header: YouTubeArtistHeader; sections: YouTubeArtistSection[]; tracks: TrackInfo[] }

function text(node: unknown): string {
  if (typeof node === 'string') return node
  const runs = dig(node, 'runs')
  return Array.isArray(runs) ? asArray(runs).map(run => dig(run, 'text') || '').join('') : String(dig(node, 'simpleText') || '')
}
function nodes(root: unknown): object[] {
  const result: object[] = []
  const queue: unknown[] = [root]
  const seen = new Set<object>()
  for (let index = 0; index < queue.length && result.length < 12000; index++) {
    const node = queue[index]
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    result.push(node)
    queue.push(...objectChildren(node))
  }
  return result
}
function thumbnail(node: unknown): string {
  const list = asArray(dig(nodes(node).find(value => Array.isArray(dig(value, 'thumbnails'))), 'thumbnails'))
  const url = String(dig(list, list.length - 1, 'url') || '')
  return url.startsWith('//') ? `https:${url}` : url.replace(/^http:\/\//, 'https://')
}
function endpoint(node: unknown): JsonRecord | undefined {
  return asRecord(nodes(node).find(value => typeof dig(value, 'browseId') === 'string'))
}
function durationMs(node: unknown): number {
  const parts = text(node).split(/\s*[•·]\s*/).filter(value => /^\d+(?::\d{1,2}){1,2}$/.test(value.trim()))
  const duration = parts[parts.length - 1]
  return duration ? duration.split(':').reduce((total, value) => total * 60 + Number(value), 0) * 1000 : 0
}
function runBrowseId(run: unknown): string {
  return String(dig(run, 'navigationEndpoint', 'browseEndpoint', 'browseId') || '')
}

function parseItem(node: unknown): YouTubeArtistItem | null {
  const responsive = asRecord(dig(node, 'musicResponsiveListItemRenderer'))
  const renderer = responsive || asRecord(dig(node, 'musicTwoRowItemRenderer'))
  if (!renderer) return null
  const columns = renderer.flexColumns
  const titleNode = responsive ? dig(columns, 0, 'musicResponsiveListItemFlexColumnRenderer', 'text') : renderer.title
  const title = text(titleNode).trim()
  if (!title) return null
  const subtitleNode = responsive ? dig(columns, 1, 'musicResponsiveListItemFlexColumnRenderer', 'text') : renderer.subtitle
  const browse = endpoint(renderer.navigationEndpoint) || endpoint(titleNode)
  const browseId = String(browse?.browseId || '')
  const videoId = String(dig(renderer, 'playlistItemData', 'videoId') || dig(renderer, 'navigationEndpoint', 'watchEndpoint', 'videoId') ||
    dig(renderer, 'overlay', 'musicItemThumbnailOverlayRenderer', 'content', 'musicPlayButtonRenderer', 'playNavigationEndpoint', 'watchEndpoint', 'videoId') ||
    dig(nodes(titleNode).find(value => dig(value, 'watchEndpoint', 'videoId')), 'watchEndpoint', 'videoId') || '')
  if (!videoId && !browseId) return null
  const pageType = String(dig(browse, 'browseEndpointContextSupportedConfigs', 'browseEndpointContextMusicConfig', 'pageType') || '')
  const kind = videoId ? (responsive ? 'song' : 'video')
    : /ARTIST|USER_CHANNEL/.test(pageType) || browseId.startsWith('UC') ? 'artist'
    : /ALBUM/.test(pageType) || browseId.startsWith('MPRE') ? 'album' : 'playlist'
  const metadata = asArray(dig(subtitleNode, 'runs'))
  const artistRuns = metadata.filter(run => runBrowseId(run).startsWith('UC')).map(run => dig(run, 'text')).filter(Boolean)
  const albumRun = metadata.find(run => runBrowseId(run).startsWith('MPRE'))
  const metadataParts = text(subtitleNode).split(/\s*[•·]\s*/).filter(part =>
    part.trim() && !/^\d+(?::\d{1,2}){1,2}$/.test(part.trim()) && !/^(song|video|歌曲|视频)$/i.test(part.trim()))
  const duration = asArray(renderer.fixedColumns).map(column => durationMs(dig(column, 'musicResponsiveListItemFixedColumnRenderer', 'text'))).find(value => value > 0)
    || durationMs(subtitleNode)
  return {
    kind, title, subtitle: text(subtitleNode), videoId, browseId,
    coverUrl: thumbnail(renderer.thumbnailRenderer || renderer.thumbnail),
    artist: artistRuns.join(' / ') || metadataParts[0] || '',
    album: asString(dig(albumRun, 'text')) || text(dig(columns, 2, 'musicResponsiveListItemFlexColumnRenderer', 'text')),
    durationMs: duration || 0,
  }
}

function items(contents: unknown): YouTubeArtistItem[] {
  if (!Array.isArray(contents)) return []
  const seen = new Set<string>()
  return asArray(contents).flatMap(node => {
    const item = parseItem(node)
    if (!item) return []
    const key = `${item.kind}:${item.videoId || item.browseId}`
    if (seen.has(key)) return []
    seen.add(key)
    return [item]
  })
}

export function youtubeArtistItemTrack(item: YouTubeArtistItem, fallbackArtist = ''): TrackInfo | null {
  if (!item.videoId) return null
  const artist = item.artist || fallbackArtist || item.subtitle || 'YouTube'
  const coverUrl = item.coverUrl || `https://i.ytimg.com/vi/${item.videoId}/hqdefault.jpg`
  return {
    id: `youtube:${item.videoId}`, title: item.title, artist, album: item.album || 'YouTube Music',
    durationMs: item.durationMs, coverUrl, audioUrl: '', source: 'youtube',
    syncPayload: {
      id: item.videoId, name: item.title, artist, album: item.album || 'YouTube Music', durationMs: item.durationMs,
      coverUrl, mediaUri: `ytmusic://video/${item.videoId}`, channelId: 'youtubeMusic', audioId: item.videoId,
    },
  }
}

const HEADER_RENDERER_KEYS = ['musicImmersiveHeaderRenderer', 'musicVisualHeaderRenderer', 'musicDetailHeaderRenderer', 'musicResponsiveHeaderRenderer']
const ITEM_CONTAINER_KEYS = [
  'musicShelfContinuation', 'musicPlaylistShelfContinuation', 'gridContinuation', 'appendContinuationItemsAction',
  'musicShelfRenderer', 'musicPlaylistShelfRenderer', 'gridRenderer', 'musicCarouselShelfRenderer',
]

/** 按 keys 顺序取第一个非空字段，等价于 `node?.a || node?.b || ...` */
function firstField(node: unknown, keys: readonly string[]): unknown {
  for (const key of keys) {
    const value = dig(node, key)
    if (value) return value
  }
  return undefined
}

export function parseYouTubeArtistDetail(raw: unknown, fallback: Partial<YouTubeArtistHeader> = {}): YouTubeArtistDetail {
  const allNodes = nodes(raw)
  const headerNode = allNodes.find(node => firstField(node, HEADER_RENDERER_KEYS))
  const renderer = firstField(headerNode, HEADER_RENDERER_KEYS)
  const header: YouTubeArtistHeader = {
    name: text(dig(renderer, 'title')) || fallback.name || '',
    coverUrl: thumbnail(dig(renderer, 'thumbnail') || dig(renderer, 'thumbnailRenderer')) || fallback.coverUrl || '',
    subtitle: text(dig(renderer, 'subtitle')) || text(dig(renderer, 'straplineTextOne')) || fallback.subtitle || '',
    description: text(dig(renderer, 'description'))
      || text(dig(allNodes.find(node => dig(node, 'musicDescriptionShelfRenderer')), 'musicDescriptionShelfRenderer', 'description')),
    subscribers: text(dig(renderer, 'shortSubscriberCountText')), listeners: text(dig(renderer, 'monthlyListenerCount')),
  }
  const sections: YouTubeArtistSection[] = []
  for (const node of allNodes) {
    const shelf = asRecord(dig(node, 'musicShelfRenderer') || dig(node, 'musicCarouselShelfRenderer'))
    if (!shelf) continue
    const basic = dig(shelf, 'header', 'musicCarouselShelfBasicHeaderRenderer')
    const title = text(shelf.title) || text(dig(basic, 'title')) || text(dig(shelf, 'header', 'title'))
    const sectionItems = items(shelf.contents)
    if (!title || !sectionItems.length) continue
    const more = endpoint(shelf.bottomEndpoint) || endpoint(dig(basic, 'moreContentButton')) || endpoint(dig(basic, 'title')) || endpoint(shelf.title)
    sections.push({ title, items: sectionItems, moreEndpoint: more ? { browseId: String(more.browseId), params: String(more.params || '') } : null })
  }
  const tracks = sections.flatMap(section => section.items.flatMap(item => {
    const track = youtubeArtistItemTrack(item, header.name)
    return track ? [track] : []
  }))
  return { header, sections, tracks }
}

export function parseYouTubeArtistItems(raw: unknown): { items: YouTubeArtistItem[]; continuation: string } {
  const allNodes = nodes(raw)
  const source = allNodes.find(node => firstField(node, ITEM_CONTAINER_KEYS))
  const renderer = firstField(source, ITEM_CONTAINER_KEYS)
  const parsed = items(dig(renderer, 'contents') || dig(renderer, 'items') || dig(renderer, 'continuationItems'))
  const next = nodes(renderer).find(node => dig(node, 'nextContinuationData', 'continuation') || dig(node, 'continuationCommand', 'token'))
  return { items: parsed, continuation: String(dig(next, 'nextContinuationData', 'continuation') || dig(next, 'continuationCommand', 'token') || '') }
}
