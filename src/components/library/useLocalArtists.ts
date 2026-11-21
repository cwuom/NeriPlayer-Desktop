import { computed, ref, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import {
  filterLocalArtists,
  groupLocalArtists,
  loadArtistSourceTracks,
  sortLocalArtists,
  type LocalArtistSortMode,
  type LocalArtistSummary,
} from '@/modules/library/localArtists'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { createLogger } from '@/utils/logger'

const log = createLogger('library-view')

// 本地库分类：歌单 / 歌手（对齐 Android LOCAL_CATEGORY_ARTIST）
export type LocalCategory = 'playlists' | 'artists' | 'files'

export interface LocalArtistsOptions {
  isMultiSelectMode: Ref<boolean>
  exitMultiSelect: () => void
}

/** 本地页的分类切换与歌手视图（由全部歌单曲目聚合出的歌手列表） */
export function useLocalArtists(options: LocalArtistsOptions) {
  const { isMultiSelectMode, exitMultiSelect } = options
  const router = useRouter()
  const player = usePlayerStore()
  const { t } = useI18n()

  const localCategory = ref<LocalCategory>('playlists')
  const localArtistSort = ref<LocalArtistSortMode>('song_count')
  const localArtistQuery = ref('')
  const localArtistTracks = ref<TrackInfo[]>([])
  const localArtistsLoading = ref(false)
  const showLocalArtistSort = ref(false)

  const localArtistSortModes: LocalArtistSortMode[] = ['song_count', 'name', 'recent']

  const localArtists = computed(() =>
    sortLocalArtists(
      filterLocalArtists(
        groupLocalArtists(localArtistTracks.value, t('library.local_artist_unknown')),
        localArtistQuery.value,
      ),
      localArtistSort.value,
    ),
  )

  async function loadLocalArtistTracks(force = false) {
    if (localArtistsLoading.value) return
    if (localArtistTracks.value.length > 0 && !force) return
    localArtistsLoading.value = true
    try {
      // 与歌手详情页共用同一个来源，避免两边口径漂移
      localArtistTracks.value = await loadArtistSourceTracks()
    } catch (e) {
      log.error('Load local artist tracks failed:', e)
      localArtistTracks.value = []
    } finally {
      localArtistsLoading.value = false
    }
  }

  /** 歌单变更后调用：本地歌手由歌单曲目聚合而来；不在歌手视图时清空，下次进入重新加载 */
  function refreshLocalArtistsAfterPlaylistChange() {
    if (localCategory.value === 'artists') void loadLocalArtistTracks(true)
    else localArtistTracks.value = []
  }

  function switchLocalCategory(category: LocalCategory) {
    if (localCategory.value === category) return
    if (isMultiSelectMode.value) exitMultiSelect()
    localCategory.value = category
    if (category === 'artists') void loadLocalArtistTracks()
  }

  function openLocalArtist(artist: LocalArtistSummary) {
    router.push({ name: 'local-artist', params: { name: artist.name } })
  }

  function playLocalArtist(artist: LocalArtistSummary) {
    if (!artist.tracks.length) return
    player.playAll(artist.tracks)
  }

  return {
    localCategory,
    localArtistSort,
    localArtistQuery,
    localArtistsLoading,
    showLocalArtistSort,
    localArtistSortModes,
    localArtists,
    switchLocalCategory,
    openLocalArtist,
    playLocalArtist,
    refreshLocalArtistsAfterPlaylistChange,
  }
}
