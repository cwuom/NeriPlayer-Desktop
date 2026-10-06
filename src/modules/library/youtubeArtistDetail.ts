import type { TrackInfo } from '@/stores/player'

export interface YouTubeArtistHeader { name: string; coverUrl: string; subtitle: string; description: string; subscribers: string; listeners: string }
export interface YouTubeArtistItem {
  kind: 'song' | 'video' | 'album' | 'playlist' | 'artist'
  title: string; subtitle: string; coverUrl: string; videoId: string; browseId: string
  artist: string; album: string; durationMs: number
}
export interface YouTubeArtistEndpoint { browseId: string; params: string }
export interface YouTubeArtistSection { title: string; items: YouTubeArtistItem[]; moreEndpoint: YouTubeArtistEndpoint | null }
export interface YouTubeArtistDetail { header: YouTubeArtistHeader; sections: YouTubeArtistSection[]; tracks: TrackInfo[] }

function text(node: any): string {
  if (typeof node === 'string') return node
  return Array.isArray(node?.runs) ? node.runs.map((run: any) => run?.text || '').join('') : String(node?.simpleText || '')
}
function nodes(root: any): any[] {
  const result: any[] = []
  const queue = [root]
  const seen = new Set<any>()
  for (let index = 0; index < queue.length && result.length < 12000; index++) {
    const node = queue[index]
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    result.push(node)
    queue.push(...Object.values(node).filter(value => value && typeof value === 'object'))
  }
  return result
}
function thumbnail(node: any): string {
  const list = nodes(node).find(value => Array.isArray(value.thumbnails))?.thumbnails
  const url = String(list?.[list.length - 1]?.url || '')
  return url.startsWith('//') ? `https:${url}` : url.replace(/^http:\/\//, 'https://')
}
function endpoint(node: any): any {
  return nodes(node).find(value => typeof value.browseId === 'string')
}
function durationMs(node: any): number {
  const parts = text(node).split(/\s*[•·]\s*/).filter(value => /^\d+(?::\d{1,2}){1,2}$/.test(value.trim()))
  const duration = parts[parts.length - 1]
  return duration ? duration.split(':').reduce((total, value) => total * 60 + Number(value), 0) * 1000 : 0
}

function parseItem(node: any): YouTubeArtistItem | null {
  const responsive = node?.musicResponsiveListItemRenderer
  const renderer = responsive || node?.musicTwoRowItemRenderer
  if (!renderer) return null
  const columns = renderer.flexColumns || []
  const titleNode = responsive ? columns[0]?.musicResponsiveListItemFlexColumnRenderer?.text : renderer.title
  const title = text(titleNode).trim()
  if (!title) return null
  const subtitleNode = responsive ? columns[1]?.musicResponsiveListItemFlexColumnRenderer?.text : renderer.subtitle
  const browse = endpoint(renderer.navigationEndpoint) || endpoint(titleNode)
  const browseId = String(browse?.browseId || '')
  const videoId = String(renderer.playlistItemData?.videoId || renderer.navigationEndpoint?.watchEndpoint?.videoId ||
    renderer.overlay?.musicItemThumbnailOverlayRenderer?.content?.musicPlayButtonRenderer?.playNavigationEndpoint?.watchEndpoint?.videoId ||
    nodes(titleNode).find(value => value.watchEndpoint?.videoId)?.watchEndpoint?.videoId || '')
  if (!videoId && !browseId) return null
  const pageType = String(browse?.browseEndpointContextSupportedConfigs?.browseEndpointContextMusicConfig?.pageType || '')
  const kind = videoId ? (responsive ? 'song' : 'video')
    : /ARTIST|USER_CHANNEL/.test(pageType) || browseId.startsWith('UC') ? 'artist'
    : /ALBUM/.test(pageType) || browseId.startsWith('MPRE') ? 'album' : 'playlist'
  const metadata = Array.isArray(subtitleNode?.runs) ? subtitleNode.runs : []
  const artistRuns = metadata.filter((run: any) => {
    const id = run.navigationEndpoint?.browseEndpoint?.browseId || ''
    return id.startsWith('UC')
  }).map((run: any) => run.text).filter(Boolean)
  const albumRun = metadata.find((run: any) => String(run.navigationEndpoint?.browseEndpoint?.browseId || '').startsWith('MPRE'))
  const metadataParts = text(subtitleNode).split(/\s*[•·]\s*/).filter(part =>
    part.trim() && !/^\d+(?::\d{1,2}){1,2}$/.test(part.trim()) && !/^(song|video|歌曲|视频)$/i.test(part.trim()))
  const duration = (renderer.fixedColumns || []).map((column: any) => durationMs(column.musicResponsiveListItemFixedColumnRenderer?.text)).find((value: number) => value > 0)
    || durationMs(subtitleNode)
  return {
    kind, title, subtitle: text(subtitleNode), videoId, browseId,
    coverUrl: thumbnail(renderer.thumbnailRenderer || renderer.thumbnail),
    artist: artistRuns.join(' / ') || metadataParts[0] || '',
    album: albumRun?.text || text(columns[2]?.musicResponsiveListItemFlexColumnRenderer?.text),
    durationMs: duration || 0,
  }
}

function items(contents: any): YouTubeArtistItem[] {
  if (!Array.isArray(contents)) return []
  const seen = new Set<string>()
  return contents.flatMap(node => {
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

export function parseYouTubeArtistDetail(raw: any, fallback: Partial<YouTubeArtistHeader> = {}): YouTubeArtistDetail {
  const allNodes = nodes(raw)
  const headerNode = allNodes.find(node => node.musicImmersiveHeaderRenderer || node.musicVisualHeaderRenderer || node.musicDetailHeaderRenderer || node.musicResponsiveHeaderRenderer)
  const renderer = headerNode?.musicImmersiveHeaderRenderer || headerNode?.musicVisualHeaderRenderer || headerNode?.musicDetailHeaderRenderer || headerNode?.musicResponsiveHeaderRenderer
  const header: YouTubeArtistHeader = {
    name: text(renderer?.title) || fallback.name || '',
    coverUrl: thumbnail(renderer?.thumbnail || renderer?.thumbnailRenderer) || fallback.coverUrl || '',
    subtitle: text(renderer?.subtitle) || text(renderer?.straplineTextOne) || fallback.subtitle || '',
    description: text(renderer?.description) || text(allNodes.find(node => node.musicDescriptionShelfRenderer)?.musicDescriptionShelfRenderer?.description),
    subscribers: text(renderer?.shortSubscriberCountText), listeners: text(renderer?.monthlyListenerCount),
  }
  const sections: YouTubeArtistSection[] = []
  for (const node of allNodes) {
    const shelf = node.musicShelfRenderer || node.musicCarouselShelfRenderer
    if (!shelf) continue
    const basic = shelf.header?.musicCarouselShelfBasicHeaderRenderer
    const title = text(shelf.title) || text(basic?.title) || text(shelf.header?.title)
    const sectionItems = items(shelf.contents)
    if (!title || !sectionItems.length) continue
    const more = endpoint(shelf.bottomEndpoint) || endpoint(basic?.moreContentButton) || endpoint(basic?.title) || endpoint(shelf.title)
    sections.push({ title, items: sectionItems, moreEndpoint: more ? { browseId: String(more.browseId), params: String(more.params || '') } : null })
  }
  const tracks = sections.flatMap(section => section.items.flatMap(item => {
    const track = youtubeArtistItemTrack(item, header.name)
    return track ? [track] : []
  }))
  return { header, sections, tracks }
}

export function parseYouTubeArtistItems(raw: any): { items: YouTubeArtistItem[]; continuation: string } {
  const allNodes = nodes(raw)
  const source = allNodes.find(node => node.musicShelfContinuation || node.musicPlaylistShelfContinuation || node.gridContinuation || node.appendContinuationItemsAction || node.musicShelfRenderer || node.musicPlaylistShelfRenderer || node.gridRenderer || node.musicCarouselShelfRenderer)
  const renderer = source?.musicShelfContinuation || source?.musicPlaylistShelfContinuation || source?.gridContinuation || source?.appendContinuationItemsAction || source?.musicShelfRenderer || source?.musicPlaylistShelfRenderer || source?.gridRenderer || source?.musicCarouselShelfRenderer
  const parsed = items(renderer?.contents || renderer?.items || renderer?.continuationItems)
  const next = nodes(renderer).find(node => node.nextContinuationData?.continuation || node.continuationCommand?.token)
  return { items: parsed, continuation: String(next?.nextContinuationData?.continuation || next?.continuationCommand?.token || '') }
}
