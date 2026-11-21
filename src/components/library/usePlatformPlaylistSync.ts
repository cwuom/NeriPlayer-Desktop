import { computed, onMounted, onUnmounted, watch, type Ref } from 'vue'
import { searchMatches } from '@/modules/search/textMatcher'
import { AUTH_CHANGED_EVENT, useAuthStore } from '@/stores/auth'
import { useRecommendStore, type CloudListStatus } from '@/stores/recommend'
import type { NeteaseCategory } from './useLibraryTabs'

type CloudPlatform = 'netease' | 'bilibili' | 'youtube'

/** recommend store 中网易云收藏专辑的条目（store 侧尚未声明类型） */
export interface NeteaseAlbumSummary {
  id: string | number
  name: string
  coverUrl: string
  artist: string
  trackCount: number
}

export interface PlatformPlaylistSyncOptions {
  activeTab: Ref<number>
  neteaseCategory: Ref<NeteaseCategory>
  /** 当前 tab 的搜索关键词 */
  query: Ref<string>
  /** 登录态变化后顺带刷新收藏 tab */
  reloadFavorites: () => Promise<void>
}

function isCloudPlatform(platform: string): platform is CloudPlatform {
  return platform === 'netease' || platform === 'bilibili' || platform === 'youtube'
}

/// 登录态检查返回之前还不知道要不要拉，按加载中显示，不先闪一下「暂无」
const PENDING_AUTH: CloudListStatus = { loading: true, error: null }

/// 平台数据按需拉取
///
/// 绝不能只在 onMounted 判一次登录态：登录状态是启动后异步拉回来的，
/// 挂载那一刻通常还是 false，判完就再没有东西重新触发，
/// 表现就是「明明登录了，云端歌单一直空着」。
/// 这里改成对（登录态 × 当前 tab）响应式求值，任一变化都会补拉。
/// 有缓存时先显示缓存，本次启动第一次打开时在后台刷新；同一平台的请求由 store 合并。
export function usePlatformPlaylistSync(options: PlatformPlaylistSyncOptions) {
  const { activeTab, neteaseCategory, query, reloadFavorites } = options
  const recommend = useRecommendStore()
  const auth = useAuthStore()

  // 网易云用户歌单
  const neteasePlaylists = computed(() => recommend.userPlaylists['netease'] || [])
  // 哔哩哔哩收藏夹
  const biliPlaylists = computed(() => recommend.userPlaylists['bilibili'] || [])
  // YouTube Music 资料库歌单
  const youtubePlaylists = computed(() => recommend.userPlaylists['youtube'] || [])

  const filteredNeteasePlaylists = computed(() =>
    neteasePlaylists.value.filter((npl) => searchMatches(query.value, npl.name)),
  )
  const filteredNeteaseAlbums = computed<NeteaseAlbumSummary[]>(() =>
    recommend.userAlbums.filter((album: NeteaseAlbumSummary) => searchMatches(query.value, album.name, album.artist)),
  )
  const filteredBiliPlaylists = computed(() =>
    biliPlaylists.value.filter((bpl) => searchMatches(query.value, bpl.name)),
  )
  const filteredYoutubePlaylists = computed(() =>
    youtubePlaylists.value.filter((ypl) => searchMatches(query.value, ypl.name)),
  )

  function ensurePlatformData(platform: string, force = false) {
    if (!isCloudPlatform(platform) || !auth[platform].loggedIn) return
    // 专辑和歌单分开判断：歌单从缓存恢复了，专辑也要照样补拉
    if (platform === 'netease') void (force ? recommend.fetchUserAlbums() : recommend.ensureUserAlbums())
    void (force ? recommend.fetchUserPlaylists(platform) : recommend.ensureUserPlaylists(platform))
  }

  function syncActiveTabData(force = false) {
    switch (activeTab.value) {
      case 3:
        ensurePlatformData('netease', force)
        break
      case 4:
        ensurePlatformData('bilibili', force)
        break
      case 5:
        ensurePlatformData('youtube', force)
        break
      default:
        break
    }
  }

  /// 登录态还没查回来时按「已登录」对待：有缓存就先显示，免得启动瞬间闪一下登录提示
  function cloudSignedIn(platform: CloudPlatform): boolean {
    return !auth.statusChecked || auth[platform].loggedIn
  }

  function cloudPlaylistStatus(platform: CloudPlatform): CloudListStatus | undefined {
    return auth.statusChecked ? recommend.userPlaylistsStatus[platform] : PENDING_AUTH
  }

  const neteaseAlbumsStatus = computed(() => auth.statusChecked ? recommend.userAlbumsStatus : PENDING_AUTH)

  /// 网易云分类栏右侧的刷新状态：只在已有列表时显示，没有列表时由占位区显示
  const neteaseRefreshStatus = computed(() => {
    if (neteaseCategory.value === 'albums') {
      return recommend.userAlbums.length > 0 ? recommend.userAlbumsStatus : undefined
    }
    return neteasePlaylists.value.length > 0 ? recommend.userPlaylistsStatus.netease : undefined
  })

  function retryNeteaseRefresh() {
    if (neteaseCategory.value === 'albums') void recommend.fetchUserAlbums()
    else void recommend.fetchUserPlaylists('netease')
  }

  watch(
    () => [
      activeTab.value,
      auth.netease.loggedIn,
      auth.bilibili.loggedIn,
      auth.youtube.loggedIn,
    ],
    () => syncActiveTabData(),
    { immediate: true },
  )

  /// 登录/登出后强制重新拉取该平台数据；重新登录看到旧的空列表是最常见的抱怨
  function handleAuthChanged(event: Event) {
    const platform = (event as CustomEvent<{ platform?: string }>).detail?.platform
    if (!platform) return
    ensurePlatformData(platform, true)
    void reloadFavorites()
  }

  onMounted(() => window.addEventListener(AUTH_CHANGED_EVENT, handleAuthChanged))
  onUnmounted(() => window.removeEventListener(AUTH_CHANGED_EVENT, handleAuthChanged))

  return {
    neteasePlaylists,
    biliPlaylists,
    youtubePlaylists,
    filteredNeteasePlaylists,
    filteredNeteaseAlbums,
    filteredBiliPlaylists,
    filteredYoutubePlaylists,
    cloudSignedIn,
    cloudPlaylistStatus,
    neteaseAlbumsStatus,
    neteaseRefreshStatus,
    retryNeteaseRefresh,
  }
}
