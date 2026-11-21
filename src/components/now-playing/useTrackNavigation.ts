import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { artistNavigationTargetKey, artistSearchRoute } from '@/modules/library/artistTrackNavigation'
import { createLogger } from '@/utils/logger'
import { primaryArtistOf } from './nowPlayingHelpers'
import type { NowPlayingSheetContext } from './types'

const log = createLogger('now-playing')

/** 更多面板里的「查看专辑 / 查看歌手」（仅网易云曲目有精确的专辑与歌手 ID） */
export function useTrackNavigation(context: NowPlayingSheetContext) {
  const { ipc, emit, hideMoreSheet, albumName, quality } = context
  const { currentSource } = quality
  const player = usePlayerStore()
  const toast = useToastStore()
  const router = useRouter()
  const { t } = useI18n()

  const currentNeteaseSongNumericId = computed(() => {
    const id = player.currentTrack?.id || ''
    if (!id.startsWith('netease:')) return null
    const num = Number(id.replace('netease:', ''))
    return Number.isFinite(num) && num > 0 ? num : null
  })

  const canViewNeteaseAlbum = computed(() => currentSource.value === 'netease' && !!albumName.value && !!currentNeteaseSongNumericId.value)

  const primaryArtistName = computed(() => primaryArtistOf(player.currentTrack?.artist || ''))
  const canViewNeteaseArtist = computed(() =>
    currentSource.value === 'netease' && !!primaryArtistName.value && !!currentNeteaseSongNumericId.value)

  async function openCurrentAlbum() {
    const songId = currentNeteaseSongNumericId.value
    if (!songId) return
    try {
      const detail = await ipc.getNeteaseSongDetail(songId)
      const song = detail?.songs?.[0]
      const albumId = song?.al?.id || song?.album?.id
      if (!albumId) {
        toast.error(t('player.not_available'))
        return
      }
      hideMoreSheet()
      // 先收起正在播放页，再跳转专辑，避免详情盖在 NP 下面
      emit('collapse')
      await router.push({ name: 'netease-album', params: { id: String(albumId) } })
    } catch (e) {
      log.error('Open album failed:', e)
      toast.error(String(e))
    }
  }

  async function openCurrentArtist() {
    const track = player.currentTrack
    if (!track) return
    const targetKey = artistNavigationTargetKey(track)
    try {
      const searchRoute = artistSearchRoute(track)
      if (searchRoute) {
        hideMoreSheet()
        emit('collapse')
        await router.push(searchRoute)
        return
      }
      const songId = currentNeteaseSongNumericId.value
      if (!songId) return
      const detail = await ipc.getNeteaseSongDetail(songId)
      if (artistNavigationTargetKey(player.currentTrack) !== targetKey) return
      const song = detail?.songs?.[0]
      const artist = (song?.ar || []).find(a => a?.id)
      if (!artist?.id) {
        toast.error(t('player.not_available'))
        return
      }
      hideMoreSheet()
      // 先收起正在播放页，再跳转歌手页，避免详情盖在 NP 下面
      emit('collapse')
      await router.push({
        name: 'netease-artist',
        params: { id: String(artist.id) },
        query: { name: artist.name || primaryArtistName.value },
      })
    } catch (e) {
      if (artistNavigationTargetKey(player.currentTrack) !== targetKey) return
      log.error('Open artist failed:', e)
      toast.error(String(e))
    }
  }

  return {
    canViewNeteaseAlbum,
    primaryArtistName,
    canViewNeteaseArtist,
    openCurrentAlbum,
    openCurrentArtist,
  }
}
