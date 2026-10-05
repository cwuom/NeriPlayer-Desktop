/**
 * YouTube Music InnerTube 响应解析
 * 覆盖 library / playlist detail 多种布局
 */
import type { TrackInfo } from '@/stores/player'
import { upgradeYouTubeThumbnailUrl } from '@/utils/trackCover'

export interface YoutubePlaylistMeta {
  playlistName: string
  subtitle: string
  coverUrl: string
}

export interface YoutubeLibraryPlaylist {
  id: string
  name: string
  coverUrl: string
  trackCount: number
  description?: string
}

export interface YoutubeHomeFeedShelf {
  title: string
  items: Array<{ title: string; subtitle: string; coverUrl: string; browseId?: string; videoId?: string }>
}

export function parseYouTubeHomeFeed(data: any): YoutubeHomeFeedShelf[] {
  const shelves: YoutubeHomeFeedShelf[] = []
  const layout = data?.contents?.singleColumnBrowseResultsRenderer || data?.contents?.twoColumnBrowseResultsRenderer
  const contents = layout?.tabs?.[0]?.tabRenderer?.content?.sectionListRenderer?.contents
    || data?.contents?.sectionListRenderer?.contents || []
  for (const section of (Array.isArray(contents) ? contents : [])) {
    const shelf = section?.musicCarouselShelfRenderer
    if (!shelf) continue
    const title = extractText(shelf?.header?.musicCarouselShelfBasicHeaderRenderer?.title)
    const items = (Array.isArray(shelf.contents) ? shelf.contents : []).flatMap((item: any) => {
      const renderer = item?.musicTwoRowItemRenderer || item?.musicResponsiveListItemRenderer
      if (!renderer) return []
      const responsive = !!item.musicResponsiveListItemRenderer
      const itemTitle = responsive
        ? extractColumnText(renderer.flexColumns, 0, 'musicResponsiveListItemFlexColumnRenderer')
        : extractText(renderer.title)
      if (!itemTitle) return []
      return [{
        title: itemTitle,
        subtitle: responsive
          ? extractColumnText(renderer.flexColumns, 1, 'musicResponsiveListItemFlexColumnRenderer')
          : extractText(renderer.subtitle),
        coverUrl: extractMusicThumbnailUrl(renderer.thumbnailRenderer || renderer.thumbnail),
        browseId: renderer.navigationEndpoint?.browseEndpoint?.browseId,
        videoId: extractTrackVideoId(renderer) || undefined,
      }]
    })
    if (title && items.length) shelves.push({ title, items })
  }
  return shelves
}

function joinRuns(runs: any[] | undefined | null): string {
  if (!Array.isArray(runs)) return ''
  return runs.map((r: any) => r?.text || '').join('')
}

function extractText(node: any): string {
  if (!node) return ''
  if (typeof node === 'string') return node
  if (Array.isArray(node.runs)) return joinRuns(node.runs)
  if (typeof node.simpleText === 'string') return node.simpleText
  return ''
}

function lastThumbnailUrl(thumbnails: any[] | undefined | null): string {
  if (!Array.isArray(thumbnails) || thumbnails.length === 0) return ''
  return String(thumbnails[thumbnails.length - 1]?.url || '')
}

function extractMusicThumbnailUrl(thumbnailNode: any): string {
  if (!thumbnailNode) return ''
  const candidates = [
    thumbnailNode?.musicThumbnailRenderer?.thumbnail?.thumbnails,
    thumbnailNode?.croppedSquareThumbnailRenderer?.thumbnail?.thumbnails,
    thumbnailNode?.thumbnail?.thumbnails,
    thumbnailNode?.thumbnails,
  ]
  for (const list of candidates) {
    const url = lastThumbnailUrl(list)
    if (url) return upgradeYouTubeThumbnailUrl(url)
  }
  return ''
}

function parseDurationTextToMs(text: string): number {
  if (!text) return 0
  const parts = text.split(':').map(Number)
  if (parts.some(n => Number.isNaN(n))) return 0
  if (parts.length === 2) return (parts[0] * 60 + parts[1]) * 1000
  if (parts.length === 3) return (parts[0] * 3600 + parts[1] * 60 + parts[2]) * 1000
  return 0
}

function looksLikeDuration(text: string): boolean {
  if (!text || !text.includes(':')) return false
  const parts = text.split(':')
  return parts.length >= 2 && parts.every(p => /^\d+$/.test(p.trim()))
}

function extractColumnText(columns: any[] | undefined | null, index: number, rendererKey: string): string {
  if (!Array.isArray(columns) || index < 0 || index >= columns.length) return ''
  const renderer = columns[index]?.[rendererKey]
  return extractText(renderer?.text)
}

/// 在 runs 里从后往前找"3:45"式时长片段
/// 布局改版后时长常混在副标题 runs（"Artist • Album • 3:45"）里，
/// 整段拼接后无法通过 looksLikeDuration，需要按 run 粒度扫描
function extractDurationFromRuns(textNode: any): string {
  const runs = textNode?.runs
  if (!Array.isArray(runs)) return ''
  for (let i = runs.length - 1; i >= 0; i--) {
    const text = String(runs[i]?.text || '').trim()
    if (looksLikeDuration(text)) return text
  }
  return ''
}

/// 多路径提取曲目时长文本：fixedColumns 任意列 → flexColumns 整列 → flexColumns runs
function extractTrackDurationText(renderer: any): string {
  const fixed = renderer?.fixedColumns
  if (Array.isArray(fixed)) {
    for (let i = 0; i < fixed.length; i++) {
      const text = extractColumnText(fixed, i, 'musicResponsiveListItemFixedColumnRenderer')
      if (looksLikeDuration(text)) return text
    }
  }

  const flex = renderer?.flexColumns
  if (Array.isArray(flex)) {
    for (let i = 0; i < flex.length; i++) {
      const text = extractColumnText(flex, i, 'musicResponsiveListItemFlexColumnRenderer')
      if (looksLikeDuration(text)) return text
    }
    // 跳过第 0 列（标题），避免把形如"3:05"的歌名误判成时长
    for (let i = 1; i < flex.length; i++) {
      const text = extractDurationFromRuns(
        flex[i]?.musicResponsiveListItemFlexColumnRenderer?.text,
      )
      if (text) return text
    }
  }

  // 普通 YouTube 视频布局兜底（playlistVideoRenderer 等）：
  // 时长不在列里，而在 lengthText / lengthSeconds 字段
  const lengthText = extractText(renderer?.lengthText)
  if (looksLikeDuration(lengthText)) return lengthText
  const lengthSeconds = Number(renderer?.lengthSeconds)
  if (Number.isFinite(lengthSeconds) && lengthSeconds > 0) {
    const total = Math.floor(lengthSeconds)
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
  }
  return ''
}

function firstNonBlank(...values: Array<string | null | undefined>): string {
  for (const value of values) {
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return ''
}

function extractVideoIdFromRuns(runs: any[] | undefined | null): string {
  if (!Array.isArray(runs)) return ''
  for (const run of runs) {
    const videoId = run?.navigationEndpoint?.watchEndpoint?.videoId
    if (typeof videoId === 'string' && videoId.trim()) return videoId.trim()
  }
  return ''
}

function extractVideoIdFromMenu(menu: any): string {
  const items = menu?.menuRenderer?.items
  if (!Array.isArray(items)) return ''
  for (const item of items) {
    const nav = item?.menuNavigationItemRenderer?.navigationEndpoint?.watchEndpoint?.videoId
    if (typeof nav === 'string' && nav.trim()) return nav.trim()
    const queue = item?.menuServiceItemRenderer?.serviceEndpoint?.queueAddEndpoint
      ?.queueTarget?.videoId
    if (typeof queue === 'string' && queue.trim()) return queue.trim()
  }
  return ''
}

/** 多路径提取 videoId：overlay / playlistItemData / runs / menu */
export function extractTrackVideoId(renderer: any): string {
  if (!renderer) return ''
  return firstNonBlank(
    renderer?.overlay?.musicItemThumbnailOverlayRenderer
      ?.content?.musicPlayButtonRenderer
      ?.playNavigationEndpoint?.watchEndpoint?.videoId,
    renderer?.playlistItemData?.videoId,
    extractVideoIdFromRuns(
      renderer?.flexColumns?.[0]
        ?.musicResponsiveListItemFlexColumnRenderer
        ?.text?.runs,
    ),
    extractVideoIdFromMenu(renderer?.menu),
    renderer?.navigationEndpoint?.watchEndpoint?.videoId,
  )
}

function isMusicPlaylistBrowseId(browseId: string, browseEndpoint?: any): boolean {
  if (!browseId) return false
  if (browseId.startsWith('VL') || browseId.startsWith('PL') || browseId.startsWith('OLAK5uy_')) {
    return true
  }
  const pageType = browseEndpoint
    ?.browseEndpointContextSupportedConfigs
    ?.browseEndpointContextMusicConfig
    ?.pageType
  return pageType === 'MUSIC_PAGE_TYPE_PLAYLIST' || pageType === 'MUSIC_PAGE_TYPE_AUDIOBOOK'
}

function parseTrackCount(subtitle: string): number {
  if (!subtitle) return 0
  const match = subtitle.match(/([\d,]+)\s*(songs?|tracks?|首|曲)/i)
    || subtitle.match(/([\d,]+)/)
  if (!match) return 0
  return Number(String(match[1]).replace(/,/g, '')) || 0
}

function walkSections(root: any): any[] {
  const sections: any[] = []
  const pushContents = (contents: any) => {
    if (Array.isArray(contents)) sections.push(...contents)
  }

  const singleTabs = root?.contents?.singleColumnBrowseResultsRenderer?.tabs
  const twoTabs = root?.contents?.twoColumnBrowseResultsRenderer?.tabs
  for (const tabs of [singleTabs, twoTabs]) {
    if (!Array.isArray(tabs)) continue
    for (const tab of tabs) {
      pushContents(tab?.tabRenderer?.content?.sectionListRenderer?.contents)
      pushContents(
        tab?.tabRenderer?.content?.richGridRenderer?.contents
          ?.map((c: any) => c?.richSectionRenderer?.content || c)
          .filter(Boolean),
      )
    }
  }

  // secondaryContents 常见于双栏歌单详情
  pushContents(
    root?.contents?.twoColumnBrowseResultsRenderer
      ?.secondaryContents?.sectionListRenderer?.contents,
  )
  pushContents(root?.contents?.sectionListRenderer?.contents)

  return sections
}

function collectGridItems(root: any): any[] {
  const items: any[] = []
  for (const section of walkSections(root)) {
    const gridItems = section?.gridRenderer?.items
    if (Array.isArray(gridItems)) items.push(...gridItems)
    const shelfItems = section?.musicShelfRenderer?.contents
    if (Array.isArray(shelfItems)) items.push(...shelfItems)
    const carouselItems = section?.musicCarouselShelfRenderer?.contents
    if (Array.isArray(carouselItems)) items.push(...carouselItems)
  }

  // 深搜 musicTwoRowItemRenderer（库页布局多变）
  const stack: any[] = [root]
  const seen = new Set<any>()
  while (stack.length) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    if (node.musicTwoRowItemRenderer) {
      items.push({ musicTwoRowItemRenderer: node.musicTwoRowItemRenderer })
    }
    if (Array.isArray(node)) {
      for (const child of node) stack.push(child)
    } else {
      for (const value of Object.values(node)) {
        if (value && typeof value === 'object') stack.push(value)
      }
    }
    if (seen.size > 5000) break
  }
  return items
}

function parseLibraryPlaylistRenderer(
  renderer: any,
  requirePlaylistEndpoint: boolean,
): YoutubeLibraryPlaylist | null {
  const browseEndpoint = renderer?.navigationEndpoint?.browseEndpoint
  const browseId = String(browseEndpoint?.browseId || '').trim()
  const name = extractText(renderer?.title)
  if (!browseId || !name) return null
  if (requirePlaylistEndpoint && !isMusicPlaylistBrowseId(browseId, browseEndpoint)) {
    return null
  }
  const subtitle = extractText(renderer?.subtitle)
  return {
    id: browseId,
    name,
    coverUrl: extractMusicThumbnailUrl(renderer?.thumbnailRenderer || renderer?.thumbnail),
    trackCount: parseTrackCount(subtitle),
    description: subtitle || undefined,
  }
}

/// 在任意深度找第一个满足条件的节点
///
/// YouTube 的 ViewModel 结构层级深且改版频繁，写死路径极易失效，
/// 这里按"找得到就用"的方式解析，结构小改不会直接把列表打空。
function findFirst(root: any, predicate: (node: any) => boolean): any {
  const stack: any[] = [root]
  const seen = new Set<any>()
  while (stack.length) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    if (predicate(node)) return node
    for (const value of Object.values(node)) {
      if (value && typeof value === 'object') stack.push(value)
    }
  }
  return null
}

function collectStrings(root: any, key: string, limit = 8): string[] {
  const out: string[] = []
  const stack: any[] = [root]
  const seen = new Set<any>()
  while (stack.length && out.length < limit) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    const value = node[key]
    if (typeof value === 'string' && value.trim()) out.push(value.trim())
    for (const nested of Object.values(node)) {
      if (nested && typeof nested === 'object') stack.push(nested)
    }
  }
  return out
}

/// 解析 lockupViewModel（YouTube 的新 ViewModel 架构）
///
/// 库页在 2026 年已从 musicTwoRowItemRenderer 迁到 lockupViewModel，
/// 只认旧渲染器会让「已登录但云端歌单为空」——请求成功、响应有内容，
/// 但一条都解析不出来。
function parseLockupViewModel(lockup: any): YoutubeLibraryPlaylist | null {
  if (!lockup) return null

  const browseEndpoint = findFirst(lockup, (node) => typeof node?.browseId === 'string')
  const rawId = String(browseEndpoint?.browseId || lockup?.contentId || '').trim()
  if (!rawId) return null
  // contentId 可能是裸的 PL...，YouTube Music 的歌单 browseId 需要 VL 前缀
  const id = rawId.startsWith('VL') || rawId.startsWith('OLAK5uy_') ? rawId : `VL${rawId}`

  const titleNode = findFirst(
    lockup?.metadata ?? lockup,
    (node) => typeof node?.content === 'string' && node.content.trim().length > 0,
  )
  const name = String(titleNode?.content || '').trim()
  if (!name) return null

  const imageNode = findFirst(
    lockup?.contentImage ?? lockup,
    (node) => Array.isArray(node?.sources) && node.sources.length > 0,
  )
  const coverUrl = String(
    imageNode?.sources?.[imageNode.sources.length - 1]?.url
      || imageNode?.sources?.[0]?.url
      || '',
  )

  // 第一条 content 是标题本身，后面的才是「N 首歌曲」这类描述
  const subtitle = collectStrings(lockup?.metadata ?? lockup, 'content')
    .filter((text) => text !== name)
    .join(' · ')

  return {
    id,
    name,
    coverUrl,
    trackCount: parseTrackCount(subtitle),
    description: subtitle || undefined,
  }
}

/** 解析 FEmusic_liked_playlists / 库内歌单列表 */
export function parseYouTubeLibraryPlaylists(data: any): YoutubeLibraryPlaylist[] {
  const playlists: YoutubeLibraryPlaylist[] = []
  const seen = new Set<string>()

  const tryPush = (item: YoutubeLibraryPlaylist | null, strict: boolean) => {
    if (!item) return
    if (strict && !isMusicPlaylistBrowseId(item.id)) return
    if (!item.id || seen.has(item.id)) return
    seen.add(item.id)
    playlists.push(item)
  }

  // 优先 grid
  for (const section of walkSections(data)) {
    const gridItems = section?.gridRenderer?.items
    if (!Array.isArray(gridItems)) continue
    for (const item of gridItems) {
      tryPush(
        parseLibraryPlaylistRenderer(item?.musicTwoRowItemRenderer, false),
        false,
      )
      tryPush(parseLockupViewModel(item?.lockupViewModel), false)
    }
  }

  // 回退：全量 twoRow + shelf
  for (const item of collectGridItems(data)) {
    tryPush(
      parseLibraryPlaylistRenderer(item?.musicTwoRowItemRenderer, true),
      true,
    )
  }

  // 最后回退：深搜 lockupViewModel
  // 新架构下 grid/shelf 的外层容器也换了名字，按类型深搜才不会漏
  const stack: any[] = [data]
  const visited = new Set<any>()
  while (stack.length) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || visited.has(node)) continue
    visited.add(node)
    if (node.lockupViewModel) {
      tryPush(parseLockupViewModel(node.lockupViewModel), false)
    }
    for (const value of Object.values(node)) {
      if (value && typeof value === 'object') stack.push(value)
    }
  }
  return playlists
}

function findPlaylistShelfContents(root: any): any[] {
  const sections = walkSections(root)
  for (const section of sections) {
    const playlistShelf = section?.musicPlaylistShelfRenderer?.contents
    if (Array.isArray(playlistShelf) && playlistShelf.length) return playlistShelf
    const musicShelf = section?.musicShelfRenderer?.contents
    if (Array.isArray(musicShelf) && musicShelf.length) return musicShelf
  }

  const continuation =
    root?.continuationContents?.musicPlaylistShelfContinuation?.contents
    || root?.continuationContents?.musicShelfContinuation?.contents
  if (Array.isArray(continuation)) return continuation

  const actions = root?.onResponseReceivedActions
  if (Array.isArray(actions)) {
    for (const action of actions) {
      const items = action?.appendContinuationItemsAction?.continuationItems
      if (Array.isArray(items) && items.length) return items
    }
  }

  // 后端分页合并后的兜底字段
  const merged = root?._neriMergedContinuationItems
  if (Array.isArray(merged) && merged.length) return merged

  return []
}

function parseListItemTrack(renderer: any): TrackInfo | null {
  const videoId = extractTrackVideoId(renderer)
  const title = extractColumnText(
    renderer?.flexColumns,
    0,
    'musicResponsiveListItemFlexColumnRenderer',
  ) || extractText(renderer?.flexColumns?.[0]?.musicResponsiveListItemFlexColumnRenderer?.text)
  if (!videoId || !title.trim()) return null

  const durationMs = parseDurationTextToMs(extractTrackDurationText(renderer))
  const coverUrl = extractMusicThumbnailUrl(renderer?.thumbnail)

  const artist = extractColumnText(
    renderer?.flexColumns,
    1,
    'musicResponsiveListItemFlexColumnRenderer',
  )
  const album = extractColumnText(
    renderer?.flexColumns,
    2,
    'musicResponsiveListItemFlexColumnRenderer',
  )

  return {
    id: `youtube:${videoId}`,
    title: title.trim(),
    artist: artist || '',
    album: album || '',
    durationMs,
    coverUrl,
    audioUrl: '',
    source: 'youtube',
    syncPayload: {
      id: videoId,
      name: title.trim(),
      artist: artist || '',
      album: album || '',
      durationMs,
      coverUrl,
      mediaUri: `ytmusic://video/${videoId}`,
      channelId: 'youtube_music',
      audioId: videoId,
    },
  }
}

/** 解析歌单详情曲目列表（含 secondaryContents / continuation） */
export function parseYouTubePlaylistTracks(data: any): TrackInfo[] {
  const result: TrackInfo[] = []
  const seen = new Set<string>()
  const contents = findPlaylistShelfContents(data)
  for (const item of contents) {
    const renderer = item?.musicResponsiveListItemRenderer
    if (!renderer) continue
    const track = parseListItemTrack(renderer)
    if (!track || seen.has(track.id)) continue
    seen.add(track.id)
    result.push(track)
  }

  // 兜底：深搜 list item
  if (result.length === 0) {
    const stack: any[] = [data]
    const visited = new Set<any>()
    while (stack.length) {
      const node = stack.pop()
      if (!node || typeof node !== 'object' || visited.has(node)) continue
      visited.add(node)
      if (node.musicResponsiveListItemRenderer) {
        const track = parseListItemTrack(node.musicResponsiveListItemRenderer)
        if (track && !seen.has(track.id)) {
          seen.add(track.id)
          result.push(track)
        }
      }
      if (Array.isArray(node)) {
        for (const child of node) stack.push(child)
      } else {
        for (const value of Object.values(node)) {
          if (value && typeof value === 'object') stack.push(value)
        }
      }
      if (visited.size > 8000) break
    }
  }

  return result
}

/**
 * 定位歌单页 header renderer.
 * 新版 YTM 把 musicResponsiveHeaderRenderer 放在
 * contents.twoColumnBrowseResultsRenderer.tabs[0]...sectionListRenderer.contents 里,
 * 旧版才在 data.header 下; 对齐 Android findPlaylistHeaderRenderer.
 */
function findPlaylistHeaderRenderer(data: any): any | null {
  const topLevel =
    data?.header?.musicImmersiveHeaderRenderer
    || data?.header?.musicDetailHeaderRenderer
    || data?.header?.musicEditablePlaylistDetailHeaderRenderer?.header?.musicDetailHeaderRenderer
    || data?.header?.musicEditablePlaylistDetailHeaderRenderer?.header?.musicResponsiveHeaderRenderer
    || data?.header?.musicVisualHeaderRenderer
    || data?.header?.musicResponsiveHeaderRenderer
    || null
  if (topLevel) return topLevel

  const sections =
    data?.contents?.twoColumnBrowseResultsRenderer?.tabs?.[0]
      ?.tabRenderer?.content?.sectionListRenderer?.contents
  if (!Array.isArray(sections)) return null

  for (const section of sections) {
    const responsive = section?.musicResponsiveHeaderRenderer
    if (responsive) return responsive
    const editable =
      section?.musicEditablePlaylistDetailHeaderRenderer?.header?.musicResponsiveHeaderRenderer
      || section?.musicEditablePlaylistDetailHeaderRenderer?.header?.musicDetailHeaderRenderer
    if (editable) return editable
  }
  return null
}

function extractHeaderCoverUrl(header: any): string {
  if (!header) return ''
  // lastThumbnailUrl 路径也走升清，避免 header 直取遗漏
  return firstNonBlank(
    extractMusicThumbnailUrl(header?.thumbnail),
    extractMusicThumbnailUrl(header?.thumbnailRenderer),
    upgradeYouTubeThumbnailUrl(
      lastThumbnailUrl(header?.thumbnail?.musicThumbnailRenderer?.thumbnail?.thumbnails),
    ),
    upgradeYouTubeThumbnailUrl(
      lastThumbnailUrl(header?.thumbnail?.croppedSquareThumbnailRenderer?.thumbnail?.thumbnails),
    ),
    upgradeYouTubeThumbnailUrl(lastThumbnailUrl(header?.thumbnail?.thumbnails)),
    // musicResponsiveHeaderRenderer 偶发把图放在 foregroundThumbnail
    extractMusicThumbnailUrl(header?.foregroundThumbnail),
    extractMusicThumbnailUrl(header?.background),
  )
}

export function parseYouTubePlaylistMeta(data: any): YoutubePlaylistMeta {
  const header = findPlaylistHeaderRenderer(data)

  const playlistName = extractText(header?.title)
  const subtitle = extractText(header?.subtitle) || extractText(header?.secondSubtitle)
  const coverUrl = extractHeaderCoverUrl(header)

  return {
    playlistName,
    subtitle,
    coverUrl,
  }
}
