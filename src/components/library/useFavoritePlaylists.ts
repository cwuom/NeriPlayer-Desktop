import { computed, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { searchMatches } from '@/modules/search/textMatcher'
import {
  filterFavoriteArtists,
  isArtistFavoriteSource,
  parseFavoritePlaylists,
  type ArtistFavoriteSource,
  type FavoritePlaylist,
} from '@/modules/library/favoriteArtists'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { createLogger } from '@/utils/logger'

const log = createLogger('library-view')

export interface FavoritePlaylistsIpc {
  listFavoritePlaylists: () => Promise<unknown>
  importFollowedArtists: (source: ArtistFavoriteSource) => Promise<number>
}

export interface FavoritePlaylistsOptions {
  ipc: FavoritePlaylistsIpc
  /** 当前 tab 的搜索关键词 */
  query: Ref<string>
}

/** 收藏 tab：同步数据中的收藏歌单与关注歌手 */
export function useFavoritePlaylists(options: FavoritePlaylistsOptions) {
  const { ipc, query } = options
  const { t } = useI18n()
  const auth = useAuthStore()
  const toast = useToastStore()

  const favoritePlaylists = ref<FavoritePlaylist[]>([])
  let favoritesLoadVersion = 0

  // 收藏分类: 歌单 / 歌手 (对齐 Android FavoritePlaylistList 的二级分类)
  const favoriteCategory = ref<'playlists' | 'artists'>('playlists')
  const favoriteArtistSource = ref<ArtistFavoriteSource>('neteaseArtist')
  const importingArtists = ref(false)
  const favoriteRenderCount = ref(100)
  const playlistFavorites = computed(() =>
    favoritePlaylists.value.filter((fpl) => !isArtistFavoriteSource(fpl.source)),
  )
  const artistFavorites = computed(() =>
    filterFavoriteArtists(favoritePlaylists.value, favoriteArtistSource.value),
  )
  const filteredFavoritePlaylists = computed(() => {
    if (favoriteCategory.value === 'artists') {
      return filterFavoriteArtists(favoritePlaylists.value, favoriteArtistSource.value, query.value)
    }
    return playlistFavorites.value.filter((fpl) => searchMatches(query.value, fpl.name, fpl.source))
  })
  const visibleFavoritePlaylists = computed(() => filteredFavoritePlaylists.value.slice(0, favoriteRenderCount.value))
  watch([favoriteCategory, favoriteArtistSource, query], () => { favoriteRenderCount.value = 100 })

  function platformLabel(source?: string) {
    switch ((source || '').toLowerCase()) {
      case 'netease': return t('player.source_netease')
      case 'qq': return t('player.source_qq')
      case 'bilibili': return t('player.source_bilibili')
      case 'youtube': return t('player.source_youtube')
      case 'local': return t('player.source_local')
      default: return source || '—'
    }
  }

  /** 收藏来源明文标签 (对齐 Android favoriteSourceLabel, 不露出内部 source 标识) */
  function favoriteSourceLabel(source: string): string {
    switch (source) {
      case 'netease': return t('player.source_netease')
      case 'neteaseAlbum': return `${t('player.source_netease')} · ${t('library.albums')}`
      case 'neteaseArtist': return `${t('player.source_netease')} · ${t('library.local_category_artists')}`
      case 'youtubeMusic': return 'YouTube Music'
      case 'bili': return t('player.source_bilibili')
      case 'qq': return t('player.source_qq')
      default: return platformLabel(source)
    }
  }

  function favoriteArtistPlatformLabel(source: string): string {
    return source === 'neteaseArtist' ? t('player.source_netease')
      : source === 'biliArtist' ? t('player.source_bilibili') : 'YouTube'
  }

  async function loadFavorites() {
    const request = ++favoritesLoadVersion
    try {
      const raw = await ipc.listFavoritePlaylists()
      if (request !== favoritesLoadVersion) return
      favoritePlaylists.value = parseFavoritePlaylists(raw)
    } catch (e) {
      log.error('Load favorites failed:', e)
    }
  }

  /** 页面卸载后丢弃仍在途的收藏请求结果 */
  function cancelPendingFavoritesLoad() {
    favoritesLoadVersion++
  }

  async function importFollowedArtists() {
    if (importingArtists.value) return
    const source = favoriteArtistSource.value
    const loggedIn = source === 'neteaseArtist' ? auth.netease.loggedIn : auth.youtube.loggedIn
    if (!loggedIn) {
      toast.show(t('library.artist_import_login'), 'info')
      return
    }
    importingArtists.value = true
    try {
      const count = await ipc.importFollowedArtists(source)
      await loadFavorites()
      toast.show(t('library.artist_import_success', { count }), 'success')
    } catch (error) {
      toast.show(String(error), 'error')
    } finally {
      importingArtists.value = false
    }
  }

  return {
    favoriteCategory,
    favoriteArtistSource,
    importingArtists,
    favoriteRenderCount,
    artistFavorites,
    filteredFavoritePlaylists,
    visibleFavoritePlaylists,
    favoriteSourceLabel,
    favoriteArtistPlatformLabel,
    loadFavorites,
    cancelPendingFavoritesLoad,
    importFollowedArtists,
  }
}
