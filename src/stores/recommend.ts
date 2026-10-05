import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToastStore } from './toast'
import { createLogger } from '@/utils/logger'
import { parseYouTubeLibraryPlaylists as parseYouTubeLibraryPlaylistsShared, parseYouTubeHomeFeed } from '@/modules/youtube/youtubePlaylistParse'

const log = createLogger('recommend')

export interface PlaylistInfo {
  id: string | number
  name: string
  coverUrl: string
  trackCount: number
  description?: string
  creator?: string
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
  const userAlbums = ref<any[]>([])

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
        userAlbums: userAlbums.value,
        homeFeedShelves: homeFeedShelves.value,
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
    if (platform === 'netease') {
      userAlbums.value = []
      likedSongIds.value = new Set()
    }
    // 内存清了也要落盘，否则重启后 loadCache 又把旧数据恢复回来
    saveCache()
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
      const data = await invoke<any>('get_recommended_playlists', { limit })
      const result = data?.result || []
      recommendedPlaylists.value = result.map((p: any) => ({
        id: p.id,
        name: p.name,
        coverUrl: p.picUrl || p.coverImgUrl || '',
        trackCount: p.trackCount || 0,
        description: p.copywriter || '',
      }))
      saveCache()
    } catch (e: any) {
      error.value = e?.toString() || 'Failed to fetch recommendations'
      log.error('fetchRecommendedPlaylists:', e)
    } finally {
      isLoading.value = false
    }
  }

  /** 获取网易云每日推荐歌曲 */
  async function fetchRecommendedSongs() {
    isLoading.value = true
    try {
      const data = await invoke<any>('get_recommended_songs')
      recommendedSongs.value = data?.data?.dailySongs || []
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
    } catch (e: any) {
      target.value = {
        items: target.value.items,
        loading: false,
        error: e?.toString() || 'Failed to fetch home recommendations',
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

  /** 获取用户歌单 */
  async function fetchUserPlaylists(platform: string) {
    isLoading.value = true
    try {
      const data = await invoke<any>('get_user_playlists', { platform })

      let playlists: PlaylistInfo[] = []
      if (platform === 'netease') {
        const list = data?.playlist || []
        playlists = list.map((p: any) => ({
          id: p.id,
          name: p.name,
          coverUrl: p.coverImgUrl || '',
          trackCount: p.trackCount || 0,
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

      userPlaylists.value[platform] = playlists
      saveCache()
    } catch (e) {
      // 静默失败会渲染成"暂无云端歌单"，把登录失效之类的问题藏起来
      log.error(`fetchUserPlaylists(${platform}):`, e)
      error.value = String(e)
      useToastStore().error(String(e))
    } finally {
      isLoading.value = false
    }
  }

  /** 获取 YouTube 首页信息流 */
  async function fetchHomeFeed() {
    isLoading.value = true
    try {
      const data = await invoke<any>('get_home_feed')
      homeFeedShelves.value = parseYouTubeHomeFeed(data)
      saveCache()
    } catch (e) {
      log.error('fetchHomeFeed:', e)
    } finally {
      isLoading.value = false
    }
  }

  /** 获取精品歌单 */
  async function fetchHighQualityPlaylists(cat?: string, limit = 30) {
    isLoading.value = true
    try {
      const data = await invoke<any>('get_high_quality_playlists', { cat, limit })
      const list = data?.playlists || []
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
      return []
    } finally {
      isLoading.value = false
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
  async function fetchLikedSongIds() {
    try {
      const data = await invoke<any>('get_liked_song_ids')
      const ids: number[] = data?.ids || []
      likedSongIds.value = new Set(ids)
    } catch (e) {
      log.error('fetchLikedSongIds:', e)
    }
  }

  /** 喜欢/取消喜欢歌曲 */
  async function toggleLikeSong(songId: number, like: boolean): Promise<boolean> {
    try {
      const data = await invoke<any>('like_song', { songId, like })
      if (data?.code === 200) {
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
      return await invoke<any>('get_album_detail', { albumId })
    } catch (e) {
      log.error('fetchAlbumDetail:', e)
      return null
    }
  }

  /** 获取用户收藏的专辑列表（网易云） */
  async function fetchUserAlbums() {
    try {
      const data = await invoke<any>('get_user_stared_albums', {})
      const list = data?.data || []
      userAlbums.value = list.map((a: any) => ({
        id: a.id,
        name: a.name,
        coverUrl: a.picUrl || '',
        artist: a.artists?.map((ar: any) => ar.name).join(', ') || '',
        trackCount: a.size || 0,
      }))
      saveCache()
    } catch (e) {
      log.error('fetchUserAlbums:', e)
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
    fetchHomeSearchRecommendations, clearHomeSearchRecommendations,
    fetchHomeFeed, fetchHighQualityPlaylists, fetchHighQualityTags,
    fetchLikedSongIds, toggleLikeSong, fetchAlbumDetail, fetchUserAlbums,
    invalidatePlatform,
    fetchBiliFavoriteItems, validateAuth,
  }
})
