import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToastStore } from './toast'
import { asArray, dig } from '@/utils/json'
import { createLogger } from '@/utils/logger'
import { resolveNeteaseCover } from '@/utils/neteaseCover'
import { parseYouTubeLibraryPlaylists as parseYouTubeLibraryPlaylistsShared, parseYouTubeHomeFeed } from '@/modules/youtube/youtubePlaylistParse'
import type { HomeFeedItem, HomeFeedShelf } from '@/modules/youtube/youtubeHomeLayout'

const log = createLogger('recommend')

export type { HomeFeedItem, HomeFeedShelf }

export interface PlaylistInfo {
  id: string | number
  name: string
  coverUrl: string
  trackCount: number
  description?: string
  creator?: string
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

/** 网易云收藏专辑摘要（由 get_user_stared_albums 的原始响应映射） */
export interface NeteaseUserAlbum {
  id: string | number
  name: string
  coverUrl: string
  artist: string
  trackCount: number
}

/** 云端列表（用户歌单、收藏专辑）的拉取状态；有缓存时 loading 表示在后台刷新 */
export interface CloudListStatus {
  loading: boolean
  /** 最近一次拉取失败的原因，成功后清空 */
  error: string | null
}

export interface CloudListLoadOptions {
  /** 后台预取：失败只记在状态里，不弹提示 */
  quiet?: boolean
}

type HomeSongSectionKey = 'hot' | 'radar'

const HOME_SEARCH_KEYWORDS: Record<HomeSongSectionKey, string> = {
  hot: '热歌',
  radar: '私人雷达',
}

/** 平台原始 id：数字原样保留，其它值转成字符串 */
function platformId(value: unknown): string | number {
  return typeof value === 'number' ? value : String(value ?? '')
}

/** 平台原始 JSON 的文本字段，缺省为空串 */
function rawText(value: unknown): string {
  return String(value ?? '')
}

function neteasePlaylistInfo(raw: unknown, coverFields: readonly [string, string]): PlaylistInfo {
  return {
    id: platformId(dig(raw, 'id')),
    name: rawText(dig(raw, 'name')),
    coverUrl: resolveNeteaseCover(dig(raw, coverFields[0]), dig(raw, coverFields[1])),
    trackCount: Number(dig(raw, 'trackCount')) || 0,
  }
}

const emptyHomeSongSection = (): HomeSongSection => ({
  items: [],
  loading: false,
  error: null,
})

export const useRecommendStore = defineStore('recommend', () => {
  // 网易云推荐歌单
  const recommendedPlaylists = ref<PlaylistInfo[]>([])
  const recommendedSongs = ref<unknown[]>([])
  const homeHotSongs = ref<HomeSongSection>(emptyHomeSongSection())
  const homeRadarSongs = ref<HomeSongSection>(emptyHomeSongSection())

  // YouTube 首页 shelf
  const homeFeedShelves = ref<HomeFeedShelf[]>([])
  let homeFeedRequestGeneration = 0
  const homeFeedLoading = ref(false)

  // 用户歌单
  const userPlaylists = ref<Record<string, PlaylistInfo[]>>({})
  const userPlaylistsStatus = ref<Record<string, CloudListStatus>>({})

  // 用户收藏专辑（网易云）
  const userAlbums = ref<NeteaseUserAlbum[]>([])
  const userAlbumsStatus = ref<CloudListStatus>({ loading: false, error: null })

  // 同一平台同时只发一个请求；登录态变化时代际递增，换账号前发出的请求回来也不写入
  const cloudRequests = new Map<string, Promise<void>>()
  const cloudGenerations = new Map<string, number>()
  // 本次启动已向平台确认过的列表：缓存先顶上，每次启动后第一次用到时在后台刷新一次
  const revalidatedCloudLists = new Set<string>()
  // 有页面在等结果的请求；只有后台预取在等时失败不弹提示，页面上的状态照样会显示失败
  const cloudRequestsToNotify = new Set<string>()
  const ALBUMS_KEY = 'netease-albums'

  // 用户喜欢的歌曲 ID 集合
  const likedSongIds = ref<Set<number>>(new Set())

  const isLoading = ref(false)
  const error = ref<string | null>(null)

  // stale-while-revalidate 缓存
  const CACHE_KEY = 'neri:recommend:cache'
  const CACHE_MAX_AGE_MS = 30 * 60 * 1000 // 30 分钟

  function loadCache() {
    try {
      const raw = localStorage.getItem(CACHE_KEY)
      if (!raw) return
      const cache = JSON.parse(raw)
      if (cache.recommendedPlaylists?.length) recommendedPlaylists.value = cache.recommendedPlaylists
      if (cache.userPlaylists && Object.keys(cache.userPlaylists).length) userPlaylists.value = cache.userPlaylists
      // 专辑与歌单同样入缓存，重启后无需等网络即可显示
      if (cache.userAlbums?.length) userAlbums.value = cache.userAlbums
      if (cache.homeHotSongs?.items?.length) homeHotSongs.value = { ...emptyHomeSongSection(), ...cache.homeHotSongs }
      if (cache.homeRadarSongs?.items?.length) homeRadarSongs.value = { ...emptyHomeSongSection(), ...cache.homeRadarSongs }
    } catch { /* 缓存损坏则忽略 */ }
  }

  function saveCache() {
    try {
      localStorage.setItem(CACHE_KEY, JSON.stringify({
        recommendedPlaylists: recommendedPlaylists.value,
        userPlaylists: userPlaylists.value,
        userAlbums: userAlbums.value,
        homeHotSongs: homeHotSongs.value,
        homeRadarSongs: homeRadarSongs.value,
        timestamp: Date.now(),
      }))
    } catch { /* localStorage 已满则忽略 */ }
  }

  /// 登录态变化后清掉该平台的缓存，下一次进入库页会重新拉取
  ///
  /// 不清的话 LibraryView 的 `!length` 判断会一直命中旧数据，
  /// 重新登录后看到的还是登录前的空列表。
  function invalidatePlatform(platform: string) {
    const next = { ...userPlaylists.value }
    delete next[platform]
    userPlaylists.value = next
    forgetCloudList(platform)
    const status = { ...userPlaylistsStatus.value }
    delete status[platform]
    userPlaylistsStatus.value = status
    if (platform === 'netease') {
      userAlbums.value = []
      forgetCloudList(ALBUMS_KEY)
      userAlbumsStatus.value = { loading: false, error: null }
      likedSongIds.value = new Set()
    } else if (platform === 'youtube') {
      homeFeedRequestGeneration++
      homeFeedShelves.value = []
      if (homeFeedLoading.value) {
        homeFeedLoading.value = false
        isLoading.value = false
      }
    }
    // 内存清了也要落盘，否则重启后 loadCache 又把旧数据恢复回来
    saveCache()
  }

  function forgetCloudList(key: string) {
    cloudGenerations.set(key, cloudGeneration(key) + 1)
    cloudRequests.delete(key)
    cloudRequestsToNotify.delete(key)
    revalidatedCloudLists.delete(key)
  }

  function cloudGeneration(key: string): number {
    return cloudGenerations.get(key) ?? 0
  }

  /// 拉取一个云端列表：同一列表合并成一个请求，拉取期间换了账号则丢弃结果
  ///
  /// 失败时保留已有的列表（缓存或上次结果），把原因记在状态里由页面显示；
  /// 页面上什么都没有时再弹提示，免得失败看起来像「暂无歌单」。
  function loadCloudList(
    key: string,
    setStatus: (status: CloudListStatus) => void,
    hasData: () => boolean,
    load: () => Promise<(() => void) | undefined>,
    { quiet = false }: CloudListLoadOptions = {},
  ): Promise<void> {
    if (!quiet) cloudRequestsToNotify.add(key)
    const pending = cloudRequests.get(key)
    if (pending) return pending
    const generation = cloudGeneration(key)
    revalidatedCloudLists.add(key)
    setStatus({ loading: true, error: null })
    const request: Promise<void> = (async () => {
      try {
        const commit = await load()
        if (generation !== cloudGeneration(key)) return
        commit?.()
        setStatus({ loading: false, error: null })
        saveCache()
      } catch (e) {
        if (generation !== cloudGeneration(key)) return
        log.error(`load ${key}:`, e)
        error.value = String(e)
        setStatus({ loading: false, error: String(e) })
        if (!hasData() && cloudRequestsToNotify.has(key)) useToastStore().error(String(e))
      }
    })().finally(() => {
      if (cloudRequests.get(key) !== request) return
      cloudRequests.delete(key)
      cloudRequestsToNotify.delete(key)
    })
    cloudRequests.set(key, request)
    return request
  }

  function isCacheFresh(): boolean {
    try {
      const raw = localStorage.getItem(CACHE_KEY)
      if (!raw) return false
      const cache = JSON.parse(raw)
      return Date.now() - (cache.timestamp || 0) < CACHE_MAX_AGE_MS
    } catch { return false }
  }

  // 启动时立即加载缓存
  loadCache()

  /** 获取网易云推荐歌单 */
  async function fetchRecommendedPlaylists(limit = 30) {
    isLoading.value = true
    error.value = null
    try {
      const data = await invoke<unknown>('get_recommended_playlists', { limit })
      recommendedPlaylists.value = asArray(dig(data, 'result')).map(p => ({
        ...neteasePlaylistInfo(p, ['picUrl', 'coverImgUrl']),
        description: rawText(dig(p, 'copywriter') || ''),
      }))
      saveCache()
    } catch (e) {
      error.value = String(e ?? '') || 'Failed to fetch recommendations'
      log.error('fetchRecommendedPlaylists:', e)
    } finally {
      isLoading.value = false
    }
  }

  /** 获取网易云每日推荐歌曲 */
  async function fetchRecommendedSongs() {
    isLoading.value = true
    try {
      const data = await invoke<unknown>('get_recommended_songs')
      recommendedSongs.value = asArray(dig(data, 'data', 'dailySongs'))
    } catch (e) {
      log.error('fetchRecommendedSongs:', e)
    } finally {
      isLoading.value = false
    }
  }

  async function fetchHomeSearchSection(section: HomeSongSectionKey, force = false) {
    const target = section === 'hot' ? homeHotSongs : homeRadarSongs
    if (!force && (target.value.loading || target.value.items.length > 0)) return

    target.value = { ...target.value, loading: true, error: null }
    isLoading.value = true

    try {
      const items = await invoke<HomeRecommendationSong[]>('search', {
        query: HOME_SEARCH_KEYWORDS[section],
        platform: 'netease',
      })
      target.value = {
        items: items.slice(0, 30),
        loading: false,
        error: null,
      }
      saveCache()
    } catch (e) {
      target.value = {
        items: target.value.items,
        loading: false,
        error: String(e ?? '') || 'Failed to fetch home recommendations',
      }
      log.error(`fetchHomeSearchSection(${section}):`, e)
    } finally {
      isLoading.value = false
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

  /** 向平台拉取用户歌单；正在拉时返回同一个请求 */
  function fetchUserPlaylists(platform: string, options?: CloudListLoadOptions): Promise<void> {
    return loadCloudList(
      platform,
      (status) => { userPlaylistsStatus.value = { ...userPlaylistsStatus.value, [platform]: status } },
      () => (userPlaylists.value[platform]?.length ?? 0) > 0,
      async () => {
        const playlists = await requestUserPlaylists(platform)
        return () => { userPlaylists.value = { ...userPlaylists.value, [platform]: playlists } }
      },
      options,
    )
  }

  /** 先显示缓存的歌单；本次启动还没向平台确认过时在后台刷新一次 */
  function ensureUserPlaylists(platform: string, options?: CloudListLoadOptions): Promise<void> {
    if (revalidatedCloudLists.has(platform) && !cloudRequests.has(platform)) return Promise.resolve()
    return fetchUserPlaylists(platform, options)
  }

  async function requestUserPlaylists(platform: string): Promise<PlaylistInfo[]> {
    isLoading.value = true
    try {
      const data = await invoke<unknown>('get_user_playlists', { platform })

      let playlists: PlaylistInfo[] = []
      if (platform === 'netease') {
        playlists = asArray(dig(data, 'playlist')).map(p => ({
          ...neteasePlaylistInfo(p, ['coverImgUrl', 'picUrl']),
          creator: rawText(dig(p, 'creator', 'nickname') || ''),
        }))
      } else if (platform === 'bilibili') {
        playlists = asArray(dig(data, 'data', 'list')).map(f => ({
          id: platformId(dig(f, 'id')),
          name: rawText(dig(f, 'title')),
          coverUrl: rawText(dig(f, 'cover') || ''),
          trackCount: Number(dig(f, 'media_count')) || 0,
        }))

        // For folders missing cover, fetch from folder info API
        const needCover = playlists.filter(p => !p.coverUrl && p.trackCount > 0)
        if (needCover.length > 0) {
          await Promise.allSettled(
            needCover.map(async (p) => {
              try {
                const info = await invoke<unknown>('get_bili_fav_folder_info', { mediaId: p.id })
                const cover = rawText(dig(info, 'data', 'cover') || '')
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
      return playlists
    } finally {
      isLoading.value = false
    }
  }

  /** 获取 YouTube 首页信息流 */
  async function fetchHomeFeed() {
    const requestGeneration = ++homeFeedRequestGeneration
    homeFeedLoading.value = true
    isLoading.value = true
    try {
      const data = await invoke<unknown>('get_home_feed')
      if (requestGeneration !== homeFeedRequestGeneration) return
      homeFeedShelves.value = parseYouTubeHomeFeed(data)
      saveCache()
    } catch (e) {
      if (requestGeneration === homeFeedRequestGeneration) log.error('fetchHomeFeed:', e)
    } finally {
      if (requestGeneration === homeFeedRequestGeneration) {
        homeFeedLoading.value = false
        isLoading.value = false
      }
    }
  }

  /** 获取精品歌单 */
  async function fetchHighQualityPlaylists(cat?: string, limit = 30): Promise<PlaylistInfo[]> {
    isLoading.value = true
    try {
      const data = await invoke<unknown>('get_high_quality_playlists', { cat, limit })
      return asArray(dig(data, 'playlists')).map(p => ({
        ...neteasePlaylistInfo(p, ['coverImgUrl', 'picUrl']),
        description: rawText(dig(p, 'description') || ''),
        creator: rawText(dig(p, 'creator', 'nickname') || ''),
      }))
    } catch (e) {
      log.error('fetchHighQualityPlaylists:', e)
      return []
    } finally {
      isLoading.value = false
    }
  }

  /** 获取精品歌单分类标签 */
  async function fetchHighQualityTags(): Promise<string[]> {
    try {
      const data = await invoke<unknown>('get_high_quality_tags')
      return asArray(dig(data, 'tags')).map(tag => rawText(dig(tag, 'name') || tag))
    } catch (e) {
      log.error('fetchHighQualityTags:', e)
      return []
    }
  }

  /** 获取用户喜欢的歌曲 ID 列表 */
  async function fetchLikedSongIds() {
    try {
      const data = await invoke<unknown>('get_liked_song_ids')
      const ids = asArray(dig(data, 'ids')).filter((id): id is number => typeof id === 'number')
      likedSongIds.value = new Set(ids)
    } catch (e) {
      log.error('fetchLikedSongIds:', e)
    }
  }

  /** 喜欢/取消喜欢歌曲 */
  async function toggleLikeSong(songId: number, like: boolean): Promise<boolean> {
    try {
      const data = await invoke<unknown>('like_song', { songId, like })
      if (dig(data, 'code') === 200) {
        if (like) {
          likedSongIds.value.add(songId)
        } else {
          likedSongIds.value.delete(songId)
        }
        return true
      }
      return false
    } catch (e) {
      log.error('toggleLikeSong:', e)
      return false
    }
  }

  /** 获取专辑详情 */
  async function fetchAlbumDetail(albumId: number) {
    try {
      return await invoke<unknown>('get_album_detail', { albumId })
    } catch (e) {
      log.error('fetchAlbumDetail:', e)
      return null
    }
  }

  /** 向网易云拉取用户收藏的专辑；正在拉时返回同一个请求 */
  function fetchUserAlbums(options?: CloudListLoadOptions): Promise<void> {
    return loadCloudList(
      ALBUMS_KEY,
      (status) => { userAlbumsStatus.value = status },
      () => userAlbums.value.length > 0,
      async () => {
        const data = await invoke<unknown>('get_user_stared_albums', {})
        const albums = asArray(dig(data, 'data')).map((a): NeteaseUserAlbum => ({
          id: platformId(dig(a, 'id')),
          name: rawText(dig(a, 'name')),
          coverUrl: resolveNeteaseCover(dig(a, 'picUrl'), dig(a, 'blurPicUrl')),
          artist: asArray(dig(a, 'artists')).map(ar => dig(ar, 'name')).join(', ') || '',
          trackCount: Number(dig(a, 'size')) || 0,
        }))
        return () => { userAlbums.value = albums }
      },
      options,
    )
  }

  /** 先显示缓存的专辑；本次启动还没向网易云确认过时在后台刷新一次 */
  function ensureUserAlbums(options?: CloudListLoadOptions): Promise<void> {
    if (revalidatedCloudLists.has(ALBUMS_KEY) && !cloudRequests.has(ALBUMS_KEY)) return Promise.resolve()
    return fetchUserAlbums(options)
  }

  /** 获取 B站收藏夹内容 */
  async function fetchBiliFavoriteItems(mediaId: number, page = 1) {
    try {
      return await invoke<unknown>('get_bili_favorite_items', { mediaId, page })
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
    homeFeedShelves, homeFeedLoading, userPlaylists, userPlaylistsStatus,
    userAlbums, userAlbumsStatus, likedSongIds, isLoading, error, isCacheFresh,
    fetchRecommendedPlaylists, fetchRecommendedSongs, fetchUserPlaylists, ensureUserPlaylists,
    fetchHomeSearchRecommendations, clearHomeSearchRecommendations,
    fetchHomeFeed, fetchHighQualityPlaylists, fetchHighQualityTags,
    fetchLikedSongIds, toggleLikeSong, fetchAlbumDetail, fetchUserAlbums, ensureUserAlbums,
    invalidatePlatform,
    fetchBiliFavoriteItems, validateAuth,
  }
})
