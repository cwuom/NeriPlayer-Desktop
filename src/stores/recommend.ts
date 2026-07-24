import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { createLogger } from '@/utils/logger'
import { parseYouTubeLibraryPlaylists as parseYouTubeLibraryPlaylistsShared } from '@/modules/youtube/youtubePlaylistParse'

const log = createLogger('recommend')

export interface PlaylistInfo {
  id: string | number
  name: string
  coverUrl: string
  trackCount: number
  playCount?: number
  description?: string
  creator?: string
}

export interface UserAlbumInfo {
  id: string | number
  name: string
  coverUrl: string
  artist: string
  trackCount: number
}

export interface HomeFeedShelf {
  title: string
  items: HomeFeedItem[]
}

export interface HomeFeedItem {
  title: string
  subtitle: string
  coverUrl: string
  browseId?: string
  videoId?: string
}

export interface HomeRecommendationSong {
  id: string
  title: string
  artist: string
  album: string
  duration_ms: number
  source: string
  cover_url: string | null
}

export interface HomeSongSection {
  items: HomeRecommendationSong[]
  loading: boolean
  error: string | null
}

export interface RecommendCacheValue {
  recommendedPlaylists?: unknown[]
  userPlaylists?: Record<string, unknown[]>
  [key: string]: unknown
}

export function clearPlatformRecommendCache<T extends RecommendCacheValue>(
  cache: T,
  platform: string,
): T {
  const userPlaylists = { ...(cache.userPlaylists ?? {}) }
  delete userPlaylists[platform]

  return {
    ...cache,
    userPlaylists,
    ...(platform === 'netease'
      ? {
          recommendedPlaylists: [],
          homeHotSongs: { items: [], loading: false, error: null },
          homeRadarSongs: { items: [], loading: false, error: null },
        }
      : {}),
  }
}

type HomeSongSectionKey = 'hot' | 'radar'

const HOME_SEARCH_KEYWORDS: Record<HomeSongSectionKey, string> = {
  hot: '热歌',
  radar: '私人雷达',
}

const emptyHomeSongSection = (): HomeSongSection => ({
  items: [],
  loading: false,
  error: null,
})

export const useRecommendStore = defineStore('recommend', () => {
  // 网易云推荐歌单
  const recommendedPlaylists = ref<PlaylistInfo[]>([])
  const recommendedSongs = ref<any[]>([])
  const homeHotSongs = ref<HomeSongSection>(emptyHomeSongSection())
  const homeRadarSongs = ref<HomeSongSection>(emptyHomeSongSection())

  // YouTube 首页 shelf
  const homeFeedShelves = ref<HomeFeedShelf[]>([])

  // 用户歌单
  const userPlaylists = ref<Record<string, PlaylistInfo[]>>({})

  // 用户收藏专辑（网易云）
  const userAlbums = ref<UserAlbumInfo[]>([])

  // 用户喜欢的歌曲 ID 集合
  const likedSongIds = ref<Set<number>>(new Set())

  const isLoading = ref(false)
  const error = ref<string | null>(null)

  // Keep loading state correct when several shelves are requested together.
  // Requests are tracked individually so invalidating Netease work cannot
  // accidentally finish an unrelated platform request.
  let activeLoadingRequests = 0
  let loadingRequestId = 0
  const loadingRequests = new Map<number, 'netease' | undefined>()

  function syncLoadingState() {
    activeLoadingRequests = loadingRequests.size
    isLoading.value = activeLoadingRequests > 0
  }

  function beginLoading(platform?: 'netease') {
    const id = ++loadingRequestId
    loadingRequests.set(id, platform)
    syncLoadingState()
    return id
  }

  function endLoading(id: number) {
    loadingRequests.delete(id)
    syncLoadingState()
  }

  // stale-while-revalidate 缓存
  const CACHE_KEY = 'neri:recommend:cache'
  const CACHE_MAX_AGE_MS = 30 * 60 * 1000 // 30 分钟
  let neteaseRequestGeneration = 0

  function isCurrentNeteaseRequest(generation: number) {
    return generation === neteaseRequestGeneration
  }

  function loadCache() {
    try {
      const raw = localStorage.getItem(CACHE_KEY)
      if (!raw) return
      const cache = JSON.parse(raw)
      if (cache.recommendedPlaylists?.length) recommendedPlaylists.value = cache.recommendedPlaylists
      if (cache.userPlaylists && Object.keys(cache.userPlaylists).length) userPlaylists.value = cache.userPlaylists
      if (cache.homeFeedShelves?.length) homeFeedShelves.value = cache.homeFeedShelves
      if (cache.homeHotSongs?.items?.length) homeHotSongs.value = { ...emptyHomeSongSection(), ...cache.homeHotSongs }
      if (cache.homeRadarSongs?.items?.length) homeRadarSongs.value = { ...emptyHomeSongSection(), ...cache.homeRadarSongs }
    } catch { /* 缓存损坏则忽略 */ }
  }

  function saveCache() {
    try {
      localStorage.setItem(CACHE_KEY, JSON.stringify({
        recommendedPlaylists: recommendedPlaylists.value,
        userPlaylists: userPlaylists.value,
        homeFeedShelves: homeFeedShelves.value,
        homeHotSongs: homeHotSongs.value,
        homeRadarSongs: homeRadarSongs.value,
        timestamp: Date.now(),
      }))
    } catch { /* localStorage 已满则忽略 */ }
  }

  function isCacheFresh(): boolean {
    try {
      const raw = localStorage.getItem(CACHE_KEY)
      if (!raw) return false
      const cache = JSON.parse(raw)
      return Date.now() - (cache.timestamp || 0) < CACHE_MAX_AGE_MS
    } catch { return false }
  }

  function clearPlatformCache(platform: string) {
    if (platform === 'netease') {
      neteaseRequestGeneration++
      for (const [id, requestPlatform] of loadingRequests) {
        if (requestPlatform === 'netease') loadingRequests.delete(id)
      }
      syncLoadingState()
    }
    const cleared = clearPlatformRecommendCache({
      recommendedPlaylists: recommendedPlaylists.value,
      userPlaylists: userPlaylists.value,
      homeHotSongs: homeHotSongs.value,
      homeRadarSongs: homeRadarSongs.value,
    }, platform)

    userPlaylists.value = (cleared.userPlaylists ?? {}) as Record<string, PlaylistInfo[]>
    if (platform === 'netease') {
      recommendedPlaylists.value = []
      recommendedSongs.value = []
      homeHotSongs.value = emptyHomeSongSection()
      homeRadarSongs.value = emptyHomeSongSection()
      userAlbums.value = []
      likedSongIds.value = new Set()
    }
    saveCache()
  }

  // 启动时立即加载缓存
  loadCache()

  /** 获取网易云推荐歌单 */
  async function fetchRecommendedPlaylists(limit = 30) {
    const requestGeneration = neteaseRequestGeneration
    const loadingRequest = beginLoading('netease')
    error.value = null
    try {
      const data = await invoke<any>('get_recommended_playlists', { limit })
      if (!isCurrentNeteaseRequest(requestGeneration)) return
      if (Number(data?.code) !== 200) {
        throw new Error(`Netease recommended playlists request failed with code ${data?.code ?? 'unknown'}`)
      }
      const result = data?.result
      if (!Array.isArray(result)) throw new Error('Netease recommended playlists response was invalid')
      recommendedPlaylists.value = result.map((p: any) => ({
        id: p.id,
        name: p.name,
        coverUrl: p.picUrl || p.coverImgUrl || '',
        trackCount: p.trackCount || 0,
        description: p.copywriter || '',
      }))
      saveCache()
    } catch (e: any) {
      if (!isCurrentNeteaseRequest(requestGeneration)) return
      error.value = e?.toString() || 'Failed to fetch recommendations'
      log.error('fetchRecommendedPlaylists:', e)
      throw e
    } finally {
      endLoading(loadingRequest)
    }
  }

  /** 获取网易云每日推荐歌曲 */
  async function fetchRecommendedSongs() {
    const requestGeneration = neteaseRequestGeneration
    const loadingRequest = beginLoading('netease')
    try {
      const data = await invoke<any>('get_recommended_songs')
      if (!isCurrentNeteaseRequest(requestGeneration)) return
      if (Number(data?.code) !== 200) {
        throw new Error(`Netease recommended songs request failed with code ${data?.code ?? 'unknown'}`)
      }
      const dailySongs = data?.data?.dailySongs
      if (!Array.isArray(dailySongs)) throw new Error('Netease recommended songs response was invalid')
      recommendedSongs.value = dailySongs
    } catch (e) {
      if (!isCurrentNeteaseRequest(requestGeneration)) return
      log.error('fetchRecommendedSongs:', e)
      throw e
    } finally {
      endLoading(loadingRequest)
    }
  }

  async function fetchHomeSearchSection(section: HomeSongSectionKey, force = false) {
    const requestGeneration = neteaseRequestGeneration
    const target = section === 'hot' ? homeHotSongs : homeRadarSongs
    if (!force && (target.value.loading || target.value.items.length > 0)) return

    target.value = { ...target.value, loading: true, error: null }
    const loadingRequest = beginLoading('netease')

    try {
      const items = await invoke<HomeRecommendationSong[]>('search', {
        query: HOME_SEARCH_KEYWORDS[section],
        platform: 'netease',
      })
      if (!isCurrentNeteaseRequest(requestGeneration)) return
      target.value = {
        items: items.slice(0, 30),
        loading: false,
        error: null,
      }
      saveCache()
    } catch (e: any) {
      if (!isCurrentNeteaseRequest(requestGeneration)) return
      target.value = {
        items: target.value.items,
        loading: false,
        error: e?.toString() || 'Failed to fetch home recommendations',
      }
      log.error(`fetchHomeSearchSection(${section}):`, e)
    } finally {
      endLoading(loadingRequest)
    }
  }

  async function fetchHomeSearchRecommendations(force = false) {
    await Promise.allSettled([
      fetchHomeSearchSection('hot', force),
      fetchHomeSearchSection('radar', force),
    ])
  }

  function clearHomeSearchRecommendations() {
    homeHotSongs.value = emptyHomeSongSection()
    homeRadarSongs.value = emptyHomeSongSection()
    saveCache()
  }

  /** 获取用户歌单 */
  async function fetchUserPlaylists(platform: string): Promise<boolean> {
    const requestGeneration = platform === 'netease' ? neteaseRequestGeneration : null
    const loadingRequest = beginLoading(platform === 'netease' ? 'netease' : undefined)
    try {
      const data = await invoke<any>('get_user_playlists', { platform })

      let playlists: PlaylistInfo[] = []
      if (platform === 'netease') {
        if (data?.code != null && Number(data.code) !== 200) {
          throw new Error(`Netease playlist request failed (${data.code})`)
        }
        const list = data?.playlist
        if (!Array.isArray(list)) throw new Error('Netease playlist response was invalid')
        playlists = list.map((p: any) => ({
          id: p.id,
          name: p.name,
          coverUrl: p.coverImgUrl || '',
          trackCount: p.trackCount || 0,
          playCount: Number(p.playCount) || 0,
          creator: p.creator?.nickname || '',
        }))
      } else if (platform === 'bilibili') {
        const list = data?.data?.list || []
        playlists = list.map((f: any) => ({
          id: f.id,
          name: f.title,
          coverUrl: f.cover || '',
          trackCount: f.media_count || 0,
        }))

        // For folders missing cover, fetch from folder info API
        const needCover = playlists.filter(p => !p.coverUrl && p.trackCount > 0)
        if (needCover.length > 0) {
          await Promise.allSettled(
            needCover.map(async (p) => {
              try {
                const info = await invoke<any>('get_bili_fav_folder_info', { mediaId: p.id })
                const cover = info?.data?.cover || ''
                if (cover) {
                  p.coverUrl = cover
                }
              } catch (err) {
                log.warn('cover failed for', p.id, err)
              }
            })
          )
        }
      } else if (platform === 'youtube') {
        // YouTube browse 响应需要解析 sectionListRenderer
        playlists = parseYouTubeLibraryPlaylistsShared(data)
      }

      if (requestGeneration !== null && !isCurrentNeteaseRequest(requestGeneration)) return false
      userPlaylists.value[platform] = playlists
      saveCache()
      return true
    } catch (e) {
      if (requestGeneration !== null && !isCurrentNeteaseRequest(requestGeneration)) return false
      log.error(`fetchUserPlaylists(${platform}):`, e)
      return false
    } finally {
      endLoading(loadingRequest)
    }
  }

  /** 获取 YouTube 首页信息流 */
  async function fetchHomeFeed() {
    const loadingRequest = beginLoading()
    try {
      const data = await invoke<any>('get_home_feed')
      homeFeedShelves.value = parseYouTubeHomeFeed(data)
      saveCache()
    } catch (e) {
      log.error('fetchHomeFeed:', e)
    } finally {
      endLoading(loadingRequest)
    }
  }

  /** 获取精品歌单 */
  async function fetchHighQualityPlaylists(cat?: string, limit = 30) {
    const loadingRequest = beginLoading('netease')
    try {
      const data = await invoke<any>('get_high_quality_playlists', { cat, limit })
      if (Number(data?.code) !== 200) {
        throw new Error(`Netease high quality playlists request failed with code ${data?.code ?? 'unknown'}`)
      }
      const list = data?.playlists
      if (!Array.isArray(list)) throw new Error('Netease high quality playlists response was invalid')
      return list.map((p: any) => ({
        id: p.id,
        name: p.name,
        coverUrl: p.coverImgUrl || '',
        trackCount: p.trackCount || 0,
        description: p.description || '',
        creator: p.creator?.nickname || '',
      }))
    } catch (e) {
      log.error('fetchHighQualityPlaylists:', e)
      throw e
    } finally {
      endLoading(loadingRequest)
    }
  }

  /** 获取精品歌单分类标签 */
  async function fetchHighQualityTags(): Promise<string[]> {
    try {
      const data = await invoke<any>('get_high_quality_tags')
      const tags = data?.tags || []
      return tags.map((t: any) => t.name || t)
    } catch (e) {
      log.error('fetchHighQualityTags:', e)
      return []
    }
  }

  /** 获取用户喜欢的歌曲 ID 列表 */
  async function fetchLikedSongIds(): Promise<boolean> {
    const requestGeneration = neteaseRequestGeneration
    try {
      const data = await invoke<any>('get_liked_song_ids')
      if (!isCurrentNeteaseRequest(requestGeneration)) return false
      const rawIds = data?.ids ?? data?.data?.ids
      const responseCode = data?.code
      if (!Array.isArray(rawIds) || (responseCode != null && Number(responseCode) !== 200)) {
        likedSongIds.value = new Set()
        return false
      }
      const ids: number[] = rawIds
        .map((id: unknown) => typeof id === 'number' ? id : Number(id))
        .filter((id: number) => Number.isSafeInteger(id) && id > 0)
      likedSongIds.value = new Set(ids)
      return true
    } catch (e) {
      if (!isCurrentNeteaseRequest(requestGeneration)) return false
      log.error('fetchLikedSongIds:', e)
      likedSongIds.value = new Set()
      return false
    }
  }

  /** 喜欢/取消喜欢歌曲 */
  async function toggleLikeSong(songId: number, like: boolean): Promise<boolean> {
    const requestGeneration = neteaseRequestGeneration
    try {
      const data = await invoke<any>('like_song', { songId, like })
      if (!isCurrentNeteaseRequest(requestGeneration)) return false
      if (Number(data?.code) === 200) {
        if (like) {
          likedSongIds.value = new Set(likedSongIds.value).add(songId)
        } else {
          const next = new Set(likedSongIds.value)
          next.delete(songId)
          likedSongIds.value = next
        }
        return true
      }
      return false
    } catch (e) {
      if (!isCurrentNeteaseRequest(requestGeneration)) return false
      log.error('toggleLikeSong:', e)
      return false
    }
  }

  /** 获取专辑详情 */
  async function fetchAlbumDetail(albumId: number) {
    try {
      return await invoke<any>('get_album_detail', { albumId })
    } catch (e) {
      log.error('fetchAlbumDetail:', e)
      return null
    }
  }

  /** 获取用户收藏的专辑列表（网易云） */
  async function fetchUserAlbums(): Promise<boolean> {
    const requestGeneration = neteaseRequestGeneration
    try {
      const data = await invoke<any>('get_user_stared_albums', {})
      if (!isCurrentNeteaseRequest(requestGeneration)) return false
      if (data?.code != null && Number(data.code) !== 200) {
        throw new Error(`Netease album request failed (${data.code})`)
      }
      const list = data?.data
      if (!Array.isArray(list)) throw new Error('Netease album response was invalid')
      userAlbums.value = list.map((a: any) => ({
        id: a.id,
        name: a.name,
        coverUrl: a.picUrl || '',
        artist: a.artists?.map((ar: any) => ar.name).join(', ') || '',
        trackCount: a.size || 0,
      }))
      return true
    } catch (e) {
      if (!isCurrentNeteaseRequest(requestGeneration)) return false
      log.error('fetchUserAlbums:', e)
      return false
    }
  }

  /** 获取 B站收藏夹内容 */
  async function fetchBiliFavoriteItems(mediaId: number, page = 1) {
    try {
      return await invoke<any>('get_bili_favorite_items', { mediaId, page })
    } catch (e) {
      log.error('fetchBiliFavoriteItems:', e)
      return null
    }
  }

  /** 验证平台登录状态 */
  async function validateAuth(platform: string): Promise<boolean> {
    try {
      return await invoke<boolean>('validate_auth', { platform })
    } catch {
      return false
    }
  }

  return {
    recommendedPlaylists, recommendedSongs, homeHotSongs, homeRadarSongs,
    homeFeedShelves, userPlaylists,
    userAlbums, likedSongIds, isLoading, error, isCacheFresh,
    fetchRecommendedPlaylists, fetchRecommendedSongs, fetchUserPlaylists,
    fetchHomeSearchRecommendations, clearHomeSearchRecommendations, clearPlatformCache,
    fetchHomeFeed, fetchHighQualityPlaylists, fetchHighQualityTags,
    fetchLikedSongIds, toggleLikeSong, fetchAlbumDetail, fetchUserAlbums,
    fetchBiliFavoriteItems, validateAuth,
  }
})

// YouTube InnerTube 响应解析
function parseYouTubeHomeFeed(data: any): HomeFeedShelf[] {
  const shelves: HomeFeedShelf[] = []
  try {
    const tabs = data?.contents?.singleColumnBrowseResultsRenderer?.tabs || []
    const contents = tabs[0]?.tabRenderer?.content?.sectionListRenderer?.contents || []
    for (const section of contents) {
      const shelf = section?.musicCarouselShelfRenderer
      if (!shelf) continue
      const title = shelf?.header?.musicCarouselShelfBasicHeaderRenderer?.title?.runs?.[0]?.text || ''
      const items: HomeFeedItem[] = []
      for (const item of (shelf?.contents || [])) {
        const renderer = item?.musicTwoRowItemRenderer || item?.musicResponsiveListItemRenderer
        if (!renderer) continue
        items.push({
          title: renderer?.title?.runs?.[0]?.text || '',
          subtitle: renderer?.subtitle?.runs?.map((r: any) => r.text).join('') || '',
          coverUrl: renderer?.thumbnailRenderer?.musicThumbnailRenderer?.thumbnail?.thumbnails?.slice(-1)?.[0]?.url || '',
          browseId: renderer?.navigationEndpoint?.browseEndpoint?.browseId,
          videoId: renderer?.overlay?.musicItemThumbnailOverlayRenderer?.content?.musicPlayButtonRenderer?.playNavigationEndpoint?.watchEndpoint?.videoId,
        })
      }
      if (title && items.length > 0) {
        shelves.push({ title, items })
      }
    }
  } catch {
    // 解析失败返回空
  }
  return shelves
}
