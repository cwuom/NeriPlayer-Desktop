import { computed, ref, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import {
  createContextMenuItem,
  createContextMenuSeparator,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import { writeClipboardText } from '@/utils/clipboard'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { readImageBytes, safeFileName, saveImageWithDialog } from '@/utils/imageTransfer'
import { createLogger } from '@/utils/logger'
import { neteaseSongArtists } from '@/modules/library/artistNavigation'
import { artistNavigationTargetKey, artistSearchRoute, resolveBiliArtistUploader } from '@/modules/library/artistTrackNavigation'
import { splitArtistNames } from '@/modules/library/localArtists'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'
import type { ArtistLink, NowPlayingIpc } from './types'

const log = createLogger('now-playing')

interface ContextMenuDeps {
  ipc: NowPlayingIpc
  emit: (event: 'collapse') => void
  coverUrl: Readonly<Ref<string>>
  shareSong: () => Promise<void>
  openAddToPlaylist: () => void
}

/** 右键菜单（歌曲名/歌手复制 + 封面保存）与点击歌手名跳转歌手页 */
export function useNowPlayingContextMenu({ ipc, emit, coverUrl, shareSong, openAddToPlaylist }: ContextMenuDeps) {
  const player = usePlayerStore()
  const settings = useSettingsStore()
  const toast = useToastStore()
  const router = useRouter()
  const { t } = useI18n()

  const contextMenu = ref({ show: false, x: 0, y: 0, type: '' as 'title' | 'artist' | 'cover' | 'artist-page' })
  const artistLinks = ref<ArtistLink[]>([])
  const artistLinkTrackId = ref('')
  const artistLinksLoading = ref(false)

  async function openArtistPage(event: MouseEvent) {
    const track = player.currentTrack
    if (!track?.artist.trim() || artistLinksLoading.value) return
    const targetKey = artistNavigationTargetKey(track)
    const source = getPlaybackSourceKind(track)
    const searchRoute = artistSearchRoute(track)
    artistLinksLoading.value = true
    try {
      if (searchRoute || source === 'qq') {
        await router.push(searchRoute ?? { name: 'explore', query: { q: track.artist, platform: 'netease', kind: 'artists' } })
        emit('collapse')
        return
      }
      let links: typeof artistLinks.value
      if (source === 'youtube') {
        // 对齐 Android resolveYouTubeMusicCreators：按名字精确匹配创作者，匹配不到再退回创作者搜索
        const creators = await ipc.resolveYoutubeCreators(track.artist)
        links = creators.map(creator => ({
          id: creator.id, name: creator.name,
          route: { name: 'youtube-artist', params: { browseId: creator.id }, query: { name: creator.name, cover: creator.cover_url || '' } },
        }))
        if (!links.length) {
          if (artistNavigationTargetKey(player.currentTrack) !== targetKey) return
          await router.push({ name: 'explore', query: { q: track.artist, platform: 'youtube', kind: 'creators' } })
          emit('collapse')
          return
        }
      } else if (source === 'bilibili') {
        // 对齐 Android resolveBiliUploader：B 站的「歌手」就是视频的 UP 主
        const uploader = await resolveBiliArtistUploader(track)
        if (artistNavigationTargetKey(player.currentTrack) !== targetKey) return
        if (!uploader) {
          toast.show(t('player.bili_uploader_unavailable'), 'info')
          return
        }
        const sharedArtistRoute = artistSearchRoute(track, uploader.name)
        if (sharedArtistRoute) {
          await router.push(sharedArtistRoute)
          emit('collapse')
          return
        }
        links = [{
          id: uploader.id, name: uploader.name,
          route: { name: 'bili-artist', params: { mid: uploader.id }, query: { name: uploader.name, cover: uploader.cover_url || '' } },
        }]
      } else if (source === 'netease') {
        const rawId = track.syncPayload?.audioId ?? track.syncPayload?.audio_id ?? track.id.replace(/^netease:/i, '')
        const songId = Number(rawId)
        if (!Number.isSafeInteger(songId) || songId <= 0) return
        const detail = await ipc.getNeteaseSongDetail(songId)
        links = neteaseSongArtists(detail, songId).map(artist => ({
          id: String(artist.id), name: artist.name,
          route: { name: 'netease-artist', params: { id: String(artist.id) } },
        }))
      } else {
        links = splitArtistNames(track.artist).map(name => ({
          id: name, name, route: { name: 'local-artist', params: { name } },
        }))
      }
      if (artistNavigationTargetKey(player.currentTrack) !== targetKey) return
      if (links.length === 1) {
        await router.push(links[0]!.route)
        emit('collapse')
      } else if (links.length > 1) {
        artistLinks.value = links
        artistLinkTrackId.value = track.id
        contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type: 'artist-page' }
      }
    } catch (error) {
      if (artistNavigationTargetKey(player.currentTrack) !== targetKey) return
      log.warn('failed to open artist page:', error)
      toast.error(t('player.artist_load_failed'))
    } finally {
      artistLinksLoading.value = false
    }
  }

  const contextMenuItems = computed<ContextMenuItem[]>(() => {
    if (contextMenu.value.type === 'artist-page') {
      return artistLinks.value.map(artist => ({ id: artist.id, label: artist.name, icon: 'person' }))
    }
    const track = player.currentTrack
    if (contextMenu.value.type === 'title') {
      return [
        createContextMenuItem(t('player.copy_title'), { id: 'copy-title', icon: 'content_copy' }),
        createContextMenuItem(t('player.copy_track_info'), { id: 'copy-info', icon: 'copy_all', disabled: !track?.artist.trim() }),
        createContextMenuItem(t('player.share'), { id: 'share', icon: 'share' }),
        createContextMenuSeparator('title-actions'),
        createContextMenuItem(t('player.add_to_playlist'), { id: 'add-to-playlist', icon: 'playlist_add' }),
      ]
    }
    if (contextMenu.value.type === 'artist') {
      return [
        createContextMenuItem(t('player.open_artist'), { id: 'open-artist', icon: 'person', disabled: artistLinksLoading.value || !track?.artist.trim() }),
        createContextMenuItem(t('player.copy_artist'), { id: 'copy-artist', icon: 'content_copy', disabled: !track?.artist.trim() }),
      ]
    }
    return [
      createContextMenuItem(t('player.save_cover'), { id: 'save-cover', icon: 'save', disabled: !coverUrl.value }),
      createContextMenuSeparator('cover-style'),
      createContextMenuItem(t('settings.cover_style_card'), { id: 'cover-style-card', icon: settings.coverStyle === 'card' ? 'check' : 'crop_square' }),
      createContextMenuItem(t('settings.cover_style_disc'), { id: 'cover-style-disc', icon: settings.coverStyle === 'disc' ? 'check' : 'album' }),
    ]
  })

  function openContextMenu(e: MouseEvent, type: 'title' | 'artist' | 'cover') {
    e.preventDefault()
    contextMenu.value = { show: true, x: e.clientX, y: e.clientY, type }
  }

  function closeContextMenu() {
    contextMenu.value.show = false
  }

  async function copyText(text: string) {
    try {
      await writeClipboardText(text)
      toast.success(t('player.copied'))
    } catch {
      toast.error(t('player.copy_failed'))
    }
    closeContextMenu()
  }

  function handleContextMenuClick(item: ContextMenuActionItem) {
    if (contextMenu.value.type === 'artist-page') {
      const artist = artistLinks.value.find(link => link.id === item.id)
      if (artist && player.currentTrack?.id === artistLinkTrackId.value) {
        void router.push(artist.route).then(() => emit('collapse'))
      }
      closeContextMenu()
      return
    }
    const track = player.currentTrack
    const { x, y } = contextMenu.value
    switch (item.id) {
      case 'copy-title': void copyText(track?.title || ''); break
      case 'copy-info': void copyText(track ? `${track.title} - ${track.artist}` : ''); break
      case 'copy-artist': void copyText(track?.artist || ''); break
      case 'share': closeContextMenu(); void shareSong(); break
      case 'add-to-playlist': closeContextMenu(); openAddToPlaylist(); break
      case 'open-artist': closeContextMenu(); void openArtistPage(new MouseEvent('click', { clientX: x, clientY: y })); break
      case 'save-cover': void saveCoverArt(); break
      case 'cover-style-card': settings.coverStyle = 'card'; closeContextMenu(); break
      case 'cover-style-disc': settings.coverStyle = 'disc'; closeContextMenu(); break
    }
  }

  async function saveCoverArt() {
    closeContextMenu()
    // Export the original artwork, not the detail-sized URL used for display.
    const src = getTrackCoverUrl(player.currentTrack, 'original') || coverUrl.value || player.currentTrack?.coverUrl
    if (!src) return

    try {
      const { bytes, extension } = await readImageBytes(src)
      const baseName = safeFileName(player.currentTrack?.title || '', 'cover')
      if (await saveImageWithDialog(bytes, baseName, extension)) toast.success(t('player.cover_saved'))
    } catch (e) {
      log.error('Save cover failed:', e)
      toast.error(t('player.cover_save_failed'))
    }
  }

  return {
    contextMenu,
    artistLinksLoading,
    contextMenuItems,
    openArtistPage,
    openContextMenu,
    closeContextMenu,
    copyText,
    handleContextMenuClick,
  }
}
