/**
 * YouTube Music InnerTube 响应解析
 * 覆盖 library / playlist detail 多种布局
 */
import type { TrackInfo } from '@/types/track'
import { asArray, asRecord, asString, dig, objectChildren } from '@/utils/json'
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
  items: Array<{ title: string; subtitle: string; coverUrl: string; browseId?: string; videoId?: string; pageType?: string; durationMs?: number }>
}

export function parseYouTubeHomeFeed(data: unknown): YoutubeHomeFeedShelf[] {
  const shelves: YoutubeHomeFeedShelf[] = []
  const layout = dig(data, 'contents', 'singleColumnBrowseResultsRenderer') || dig(data, 'contents', 'twoColumnBrowseResultsRenderer')
  const contents = dig(layout, 'tabs', 0, 'tabRenderer', 'content', 'sectionListRenderer', 'contents')
    || dig(data, 'contents', 'sectionListRenderer', 'contents')
  for (const section of asArray(contents)) {
    const shelf = asRecord(dig(section, 'musicCarouselShelfRenderer'))
    if (!shelf) continue
    const title = extractText(dig(shelf, 'header', 'musicCarouselShelfBasicHeaderRenderer', 'title'))
    const items = asArray(shelf.contents).flatMap((item) => {
      const renderer = asRecord(dig(item, 'musicTwoRowItemRenderer') || dig(item, 'musicResponsiveListItemRenderer'))
      if (!renderer) return []
      const responsive = !!dig(item, 'musicResponsiveListItemRenderer')
      const itemTitle = responsive
        ? extractColumnText(renderer.flexColumns, 0, 'musicResponsiveListItemFlexColumnRenderer')
        : extractText(renderer.title)
      if (!itemTitle) return []
      const browseEndpoint = dig(renderer, 'navigationEndpoint', 'browseEndpoint')
      return [{
        title: itemTitle,
        subtitle: responsive
          ? extractColumnText(renderer.flexColumns, 1, 'musicResponsiveListItemFlexColumnRenderer')
          : extractText(renderer.subtitle),
        coverUrl: extractMusicThumbnailUrl(renderer.thumbnailRenderer || renderer.thumbnail),
        browseId: asString(dig(browseEndpoint, 'browseId')),
        videoId: extractTrackVideoId(renderer) || undefined,
        pageType: asString(dig(browseEndpoint, 'browseEndpointContextSupportedConfigs', 'browseEndpointContextMusicConfig', 'pageType')),
        durationMs: parseDurationTextToMs(extractTrackDurationText(renderer) || extractDurationFromRuns(renderer.subtitle)),
      }]
    })
    if (title && items.length) shelves.push({ title, items })
  }
  return shelves
}

function joinRuns(runs: unknown[]): string {
  return runs.map((r) => dig(r, 'text') || '').join('')
}

function extractText(node: unknown): string {
  if (typeof node === 'string') return node
  const runs = dig(node, 'runs')
  if (Array.isArray(runs)) return joinRuns(asArray(runs))
  return asString(dig(node, 'simpleText')) ?? ''
}

function lastThumbnailUrl(thumbnails: unknown): string {
  const list = asArray(thumbnails)
  if (list.length === 0) return ''
  return String(dig(list[list.length - 1], 'url') || '')
}

function extractMusicThumbnailUrl(thumbnailNode: unknown): string {
  if (!thumbnailNode) return ''
  const candidates = [
    dig(thumbnailNode, 'musicThumbnailRenderer', 'thumbnail', 'thumbnails'),
    dig(thumbnailNode, 'croppedSquareThumbnailRenderer', 'thumbnail', 'thumbnails'),
    dig(thumbnailNode, 'thumbnail', 'thumbnails'),
    dig(thumbnailNode, 'thumbnails'),
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

function extractColumnText(columns: unknown, index: number, rendererKey: string): string {
  return extractText(dig(columns, index, rendererKey, 'text'))
}

/// 在 runs 里从后往前找"3:45"式时长片段
/// 布局改版后时长常混在副标题 runs（"Artist • Album • 3:45"）里，
/// 整段拼接后无法通过 looksLikeDuration，需要按 run 粒度扫描
function extractDurationFromRuns(textNode: unknown): string {
  const runs = asArray(dig(textNode, 'runs'))
  for (let i = runs.length - 1; i >= 0; i--) {
    const text = String(dig(runs[i], 'text') || '').trim()
    if (looksLikeDuration(text)) return text
  }
  return ''
}

/// 多路径提取曲目时长文本：fixedColumns 任意列 → flexColumns 整列 → flexColumns runs
function extractTrackDurationText(renderer: unknown): string {
  const fixed = asArray(dig(renderer, 'fixedColumns'))
  for (let i = 0; i < fixed.length; i++) {
    const text = extractColumnText(fixed, i, 'musicResponsiveListItemFixedColumnRenderer')
    if (looksLikeDuration(text)) return text
  }

  const flex = asArray(dig(renderer, 'flexColumns'))
  for (let i = 0; i < flex.length; i++) {
    const text = extractColumnText(flex, i, 'musicResponsiveListItemFlexColumnRenderer')
    if (looksLikeDuration(text)) return text
  }
  // 跳过第 0 列（标题），避免把形如"3:05"的歌名误判成时长
  for (let i = 1; i < flex.length; i++) {
    const text = extractDurationFromRuns(dig(flex, i, 'musicResponsiveListItemFlexColumnRenderer', 'text'))
    if (text) return text
  }

  // 普通 YouTube 视频布局兜底（playlistVideoRenderer 等）：
  // 时长不在列里，而在 lengthText / lengthSeconds 字段
  const lengthText = extractText(dig(renderer, 'lengthText'))
  if (looksLikeDuration(lengthText)) return lengthText
  const lengthSeconds = Number(dig(renderer, 'lengthSeconds'))
  if (Number.isFinite(lengthSeconds) && lengthSeconds > 0) {
    const total = Math.floor(lengthSeconds)
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
  }
  return ''
}

function firstNonBlank(...values: unknown[]): string {
  for (const value of values) {
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return ''
}

function extractVideoIdFromRuns(runs: unknown): string {
  for (const run of asArray(runs)) {
    const videoId = dig(run, 'navigationEndpoint', 'watchEndpoint', 'videoId')
    if (typeof videoId === 'string' && videoId.trim()) return videoId.trim()
  }
  return ''
}

function extractVideoIdFromMenu(menu: unknown): string {
  for (const item of asArray(dig(menu, 'menuRenderer', 'items'))) {
    const nav = dig(item, 'menuNavigationItemRenderer', 'navigationEndpoint', 'watchEndpoint', 'videoId')
    if (typeof nav === 'string' && nav.trim()) return nav.trim()
    const queue = dig(item, 'menuServiceItemRenderer', 'serviceEndpoint', 'queueAddEndpoint', 'queueTarget', 'videoId')
    if (typeof queue === 'string' && queue.trim()) return queue.trim()
  }
  return ''
}

/** 多路径提取 videoId：overlay / playlistItemData / runs / menu */
export function extractTrackVideoId(renderer: unknown): string {
  if (!renderer) return ''
  return firstNonBlank(
    dig(renderer, 'overlay', 'musicItemThumbnailOverlayRenderer', 'content', 'musicPlayButtonRenderer',
      'playNavigationEndpoint', 'watchEndpoint', 'videoId'),
    dig(renderer, 'playlistItemData', 'videoId'),
    extractVideoIdFromRuns(dig(renderer, 'flexColumns', 0, 'musicResponsiveListItemFlexColumnRenderer', 'text', 'runs')),
    extractVideoIdFromMenu(dig(renderer, 'menu')),
    dig(renderer, 'navigationEndpoint', 'watchEndpoint', 'videoId'),
  )
}

function isMusicPlaylistBrowseId(browseId: string, browseEndpoint?: unknown): boolean {
  if (!browseId) return false
  if (browseId.startsWith('VL') || browseId.startsWith('PL') || browseId.startsWith('OLAK5uy_')) {
    return true
  }
  const pageType = dig(browseEndpoint, 'browseEndpointContextSupportedConfigs', 'browseEndpointContextMusicConfig', 'pageType')
  return pageType === 'MUSIC_PAGE_TYPE_PLAYLIST' || pageType === 'MUSIC_PAGE_TYPE_AUDIOBOOK'
}

function parseTrackCount(subtitle: string): number {
  if (!subtitle) return 0
  const match = subtitle.match(/([\d,]+)\s*(songs?|tracks?|首|曲)/i)
    || subtitle.match(/([\d,]+)/)
  if (!match) return 0
  return Number(String(match[1]).replace(/,/g, '')) || 0
}

function walkSections(root: unknown): unknown[] {
  const sections: unknown[] = []

  const singleTabs = dig(root, 'contents', 'singleColumnBrowseResultsRenderer', 'tabs')
  const twoTabs = dig(root, 'contents', 'twoColumnBrowseResultsRenderer', 'tabs')
  for (const tabs of [singleTabs, twoTabs]) {
    for (const tab of asArray(tabs)) {
      const content = dig(tab, 'tabRenderer', 'content')
      sections.push(...asArray(dig(content, 'sectionListRenderer', 'contents')))
      sections.push(
        ...asArray(dig(content, 'richGridRenderer', 'contents'))
          .map((c) => dig(c, 'richSectionRenderer', 'content') || c)
          .filter(Boolean),
      )
    }
  }

  // secondaryContents 常见于双栏歌单详情
  sections.push(...asArray(dig(root, 'contents', 'twoColumnBrowseResultsRenderer', 'secondaryContents', 'sectionListRenderer', 'contents')))
  sections.push(...asArray(dig(root, 'contents', 'sectionListRenderer', 'contents')))

  return sections
}

function collectGridItems(root: unknown): unknown[] {
  const items: unknown[] = []
  for (const section of walkSections(root)) {
    items.push(...asArray(dig(section, 'gridRenderer', 'items')))
    items.push(...asArray(dig(section, 'musicShelfRenderer', 'contents')))
    items.push(...asArray(dig(section, 'musicCarouselShelfRenderer', 'contents')))
  }

  // 深搜 musicTwoRowItemRenderer（库页布局多变）
  const stack: unknown[] = [root]
  const seen = new Set<object>()
  while (stack.length) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    const twoRow = dig(node, 'musicTwoRowItemRenderer')
    if (twoRow) items.push({ musicTwoRowItemRenderer: twoRow })
    stack.push(...objectChildren(node))
    if (seen.size > 5000) break
  }
  return items
}

function parseLibraryPlaylistRenderer(
  renderer: unknown,
  requirePlaylistEndpoint: boolean,
): YoutubeLibraryPlaylist | null {
  const browseEndpoint = dig(renderer, 'navigationEndpoint', 'browseEndpoint')
  const browseId = String(dig(browseEndpoint, 'browseId') || '').trim()
  const name = extractText(dig(renderer, 'title'))
  if (!browseId || !name) return null
  if (requirePlaylistEndpoint && !isMusicPlaylistBrowseId(browseId, browseEndpoint)) {
    return null
  }
  const subtitle = extractText(dig(renderer, 'subtitle'))
  return {
    id: browseId,
    name,
    coverUrl: extractMusicThumbnailUrl(dig(renderer, 'thumbnailRenderer') || dig(renderer, 'thumbnail')),
    trackCount: parseTrackCount(subtitle),
    description: subtitle || undefined,
  }
}

/// 在任意深度找第一个满足条件的节点
///
/// YouTube 的 ViewModel 结构层级深且改版频繁，写死路径极易失效，
/// 这里按"找得到就用"的方式解析，结构小改不会直接把列表打空。
function findFirst(root: unknown, predicate: (node: object) => boolean): object | null {
  const stack: unknown[] = [root]
  const seen = new Set<object>()
  while (stack.length) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    if (predicate(node)) return node
    stack.push(...objectChildren(node))
  }
  return null
}

function collectStrings(root: unknown, key: string, limit = 8): string[] {
  const out: string[] = []
  const stack: unknown[] = [root]
  const seen = new Set<object>()
  while (stack.length && out.length < limit) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || seen.has(node)) continue
    seen.add(node)
    const value = dig(node, key)
    if (typeof value === 'string' && value.trim()) out.push(value.trim())
    stack.push(...objectChildren(node))
  }
  return out
}

/// 解析 lockupViewModel（YouTube 的新 ViewModel 架构）
///
/// 库页在 2026 年已从 musicTwoRowItemRenderer 迁到 lockupViewModel，
/// 只认旧渲染器会让「已登录但云端歌单为空」——请求成功、响应有内容，
/// 但一条都解析不出来。
function parseLockupViewModel(lockup: unknown): YoutubeLibraryPlaylist | null {
  if (!lockup) return null

  const browseEndpoint = findFirst(lockup, (node) => typeof dig(node, 'browseId') === 'string')
  const rawId = String(dig(browseEndpoint, 'browseId') || dig(lockup, 'contentId') || '').trim()
  if (!rawId) return null
  // contentId 可能是裸的 PL...，YouTube Music 的歌单 browseId 需要 VL 前缀
  const id = rawId.startsWith('VL') || rawId.startsWith('OLAK5uy_') ? rawId : `VL${rawId}`

  const titleNode = findFirst(
    dig(lockup, 'metadata') ?? lockup,
    (node) => (asString(dig(node, 'content'))?.trim().length ?? 0) > 0,
  )
  const name = String(dig(titleNode, 'content') || '').trim()
  if (!name) return null

  const imageNode = findFirst(
    dig(lockup, 'contentImage') ?? lockup,
    (node) => asArray(dig(node, 'sources')).length > 0,
  )
  const sources = asArray(dig(imageNode, 'sources'))
  const coverUrl = String(
    dig(sources, sources.length - 1, 'url')
      || dig(sources, 0, 'url')
      || '',
  )

  // 第一条 content 是标题本身，后面的才是「N 首歌曲」这类描述
  const subtitle = collectStrings(dig(lockup, 'metadata') ?? lockup, 'content')
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
export function parseYouTubeLibraryPlaylists(data: unknown): YoutubeLibraryPlaylist[] {
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
    for (const item of asArray(dig(section, 'gridRenderer', 'items'))) {
      tryPush(
        parseLibraryPlaylistRenderer(dig(item, 'musicTwoRowItemRenderer'), false),
        false,
      )
      tryPush(parseLockupViewModel(dig(item, 'lockupViewModel')), false)
    }
  }

  // 回退：全量 twoRow + shelf
  for (const item of collectGridItems(data)) {
    tryPush(
      parseLibraryPlaylistRenderer(dig(item, 'musicTwoRowItemRenderer'), true),
      true,
    )
  }

  // 最后回退：深搜 lockupViewModel
  // 新架构下 grid/shelf 的外层容器也换了名字，按类型深搜才不会漏
  const stack: unknown[] = [data]
  const visited = new Set<object>()
  while (stack.length) {
    const node = stack.pop()
    if (!node || typeof node !== 'object' || visited.has(node)) continue
    visited.add(node)
    const lockup = dig(node, 'lockupViewModel')
    if (lockup) {
      tryPush(parseLockupViewModel(lockup), false)
    }
    stack.push(...objectChildren(node))
  }
  return playlists
}

function findPlaylistShelfContents(root: unknown): unknown[] {
  const sections = walkSections(root)
  for (const section of sections) {
    const playlistShelf = asArray(dig(section, 'musicPlaylistShelfRenderer', 'contents'))
    if (playlistShelf.length) return playlistShelf
    const musicShelf = asArray(dig(section, 'musicShelfRenderer', 'contents'))
    if (musicShelf.length) return musicShelf
  }

  const continuation =
    dig(root, 'continuationContents', 'musicPlaylistShelfContinuation', 'contents')
    || dig(root, 'continuationContents', 'musicShelfContinuation', 'contents')
  if (Array.isArray(continuation)) return asArray(continuation)

  for (const action of asArray(dig(root, 'onResponseReceivedActions'))) {
    const items = asArray(dig(action, 'appendContinuationItemsAction', 'continuationItems'))
    if (items.length) return items
  }

  // 后端分页合并后的兜底字段
  const merged = asArray(dig(root, '_neriMergedContinuationItems'))
  if (merged.length) return merged

  return []
}

function parseListItemTrack(renderer: unknown): TrackInfo | null {
  const flexColumns = dig(renderer, 'flexColumns')
  const videoId = extractTrackVideoId(renderer)
  const title = extractColumnText(flexColumns, 0, 'musicResponsiveListItemFlexColumnRenderer')
  if (!videoId || !title.trim()) return null

  const durationMs = parseDurationTextToMs(extractTrackDurationText(renderer))
  const coverUrl = extractMusicThumbnailUrl(dig(renderer, 'thumbnail'))

  const artist = extractColumnText(flexColumns, 1, 'musicResponsiveListItemFlexColumnRenderer')
  const album = extractColumnText(flexColumns, 2, 'musicResponsiveListItemFlexColumnRenderer')

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
export function parseYouTubePlaylistTracks(data: unknown): TrackInfo[] {
  const result: TrackInfo[] = []
  const seen = new Set<string>()
  const contents = findPlaylistShelfContents(data)
  for (const item of contents) {
    const renderer = dig(item, 'musicResponsiveListItemRenderer')
    if (!renderer) continue
    const track = parseListItemTrack(renderer)
    if (!track || seen.has(track.id)) continue
    seen.add(track.id)
    result.push(track)
  }

  // 兜底：深搜 list item
  if (result.length === 0) {
    const stack: unknown[] = [data]
    const visited = new Set<object>()
    while (stack.length) {
      const node = stack.pop()
      if (!node || typeof node !== 'object' || visited.has(node)) continue
      visited.add(node)
      const listItem = dig(node, 'musicResponsiveListItemRenderer')
      if (listItem) {
        const track = parseListItemTrack(listItem)
        if (track && !seen.has(track.id)) {
          seen.add(track.id)
          result.push(track)
        }
      }
      stack.push(...objectChildren(node))
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
function findPlaylistHeaderRenderer(data: unknown): unknown {
  const header = dig(data, 'header')
  const editableHeader = dig(header, 'musicEditablePlaylistDetailHeaderRenderer', 'header')
  const topLevel =
    dig(header, 'musicImmersiveHeaderRenderer')
    || dig(header, 'musicDetailHeaderRenderer')
    || dig(editableHeader, 'musicDetailHeaderRenderer')
    || dig(editableHeader, 'musicResponsiveHeaderRenderer')
    || dig(header, 'musicVisualHeaderRenderer')
    || dig(header, 'musicResponsiveHeaderRenderer')
    || null
  if (topLevel) return topLevel

  const sections = dig(data, 'contents', 'twoColumnBrowseResultsRenderer', 'tabs', 0,
    'tabRenderer', 'content', 'sectionListRenderer', 'contents')
  for (const section of asArray(sections)) {
    const responsive = dig(section, 'musicResponsiveHeaderRenderer')
    if (responsive) return responsive
    const editable = dig(section, 'musicEditablePlaylistDetailHeaderRenderer', 'header')
    const editableHeaderRenderer =
      dig(editable, 'musicResponsiveHeaderRenderer')
      || dig(editable, 'musicDetailHeaderRenderer')
    if (editableHeaderRenderer) return editableHeaderRenderer
  }
  return null
}

function extractHeaderCoverUrl(header: unknown): string {
  if (!header) return ''
  const thumbnail = dig(header, 'thumbnail')
  // lastThumbnailUrl 路径也走升清，避免 header 直取遗漏
  return firstNonBlank(
    extractMusicThumbnailUrl(thumbnail),
    extractMusicThumbnailUrl(dig(header, 'thumbnailRenderer')),
    upgradeYouTubeThumbnailUrl(
      lastThumbnailUrl(dig(thumbnail, 'musicThumbnailRenderer', 'thumbnail', 'thumbnails')),
    ),
    upgradeYouTubeThumbnailUrl(
      lastThumbnailUrl(dig(thumbnail, 'croppedSquareThumbnailRenderer', 'thumbnail', 'thumbnails')),
    ),
    upgradeYouTubeThumbnailUrl(lastThumbnailUrl(dig(thumbnail, 'thumbnails'))),
    // musicResponsiveHeaderRenderer 偶发把图放在 foregroundThumbnail
    extractMusicThumbnailUrl(dig(header, 'foregroundThumbnail')),
    extractMusicThumbnailUrl(dig(header, 'background')),
  )
}

export function parseYouTubePlaylistMeta(data: unknown): YoutubePlaylistMeta {
  const header = findPlaylistHeaderRenderer(data)

  const playlistName = extractText(dig(header, 'title'))
  const subtitle = extractText(dig(header, 'subtitle')) || extractText(dig(header, 'secondSubtitle'))
  const coverUrl = extractHeaderCoverUrl(header)

  return {
    playlistName,
    subtitle,
    coverUrl,
  }
}
