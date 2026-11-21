import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { TrackInfo } from '@/stores/player'
import { useRecommendStore } from '@/stores/recommend'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import { FAVORITES_PLAYLIST_NAMES, isFavoritesPlaylist } from '@/modules/library/localPlaylists'
import { resolveBiliPartCid, resolveBiliVideoReference } from '@/modules/playback/biliVideoParts'
import { isBiliVideoSkipTrack } from '@/modules/playback/biliVideoSkip'
import { useBiliVideoPartsStore, type BiliCollectionIdentity } from '@/stores/biliVideoParts'
import { createLogger } from '@/utils/logger'

const log = createLogger('liked-songs')

interface PlaylistInfo {
  id: number
  name: string
  track_count?: number
}

type LikedTrack = Pick<TrackInfo, 'id' | 'album' | 'source' | 'syncPayload' | 'playlistKey'>

const DEFAULT_LIKED_PLAYLIST_NAME = FAVORITES_PLAYLIST_NAMES[0]

export const useLikedSongsStore = defineStore('likedSongs', () => {
  const likedPlaylistId = ref<number | null>(null)
  const likedTracks = ref<LikedTrack[]>([])
  const isLoading = ref(false)
  const isReady = ref(false)

  let loadPromise: Promise<boolean> | null = null
  let unlistenPlaylistsChanged: UnlistenFn | null = null

  function inferTrackSource(trackId: string) {
    if (trackId.startsWith('netease:')) return 'netease'
    if (trackId.startsWith('qq:')) return 'qq'
    if (trackId.startsWith('bilibili:')) return 'bilibili'
    if (trackId.startsWith('youtube:')) return 'youtube'
    return 'local'
  }

  function toBackendTrack(track: TrackInfo) {
    return {
      id: track.id,
      title: track.title,
      artist: track.artist,
      album: track.album || '',
      duration_ms: track.durationMs || 0,
      cover_url: track.coverUrl || null,
      url: track.audioUrl || '',
      source: inferTrackSource(track.id),
      added_at: Math.max(0, Math.round(track.addedAt || 0)),
      sync_payload: track.syncPayload ?? null,
      playlist_key: track.playlistKey ?? null,
    }
  }

  function getNeteaseSongId(track: TrackInfo) {
    if (!track.id.startsWith('netease:')) return null
    const id = Number.parseInt(track.id.replace('netease:', ''), 10)
    return Number.isFinite(id) ? id : null
  }

  async function readLikedTracks(id: number) {
    const tracks = await invoke<Array<{ id: string; album: string; source?: string; sync_payload?: TrackInfo['syncPayload']; playlist_key?: string }>>('get_playlist_tracks', { id })
    likedTracks.value = tracks.map(track => ({
      id: track.id, album: track.album, source: track.source,
      syncPayload: track.sync_payload, playlistKey: track.playlist_key,
    }))
  }

  async function loadLikedPlaylist() {
    if (loadPromise) return loadPromise

    loadPromise = (async () => {
      isLoading.value = true
      try {
        const playlists = await invoke<PlaylistInfo[]>('list_playlists')
        const liked = playlists.find(isFavoritesPlaylist)
        if (!liked) {
          likedPlaylistId.value = null
          likedTracks.value = []
          return true
        }

        likedPlaylistId.value = liked.id
        await readLikedTracks(liked.id)
        return true
      } catch (e) {
        log.error('loadLikedPlaylist:', e)
        return false
      } finally {
        isReady.value = true
        isLoading.value = false
        loadPromise = null
      }
    })()

    return loadPromise
  }

  async function ensureLikedPlaylist() {
    if (likedPlaylistId.value !== null) return likedPlaylistId.value

    // 后端以固定 id -1001 创建，和 Android 的"我喜欢的音乐"是同一个同步歌单
    const favorites = await invoke<PlaylistInfo>('ensure_favorites_playlist', { name: DEFAULT_LIKED_PLAYLIST_NAME })
    likedPlaylistId.value = favorites.id
    // 期间完成的同步可能已经带来了收藏，有曲目时重新读取，免得已收藏的歌显示成未收藏
    if (favorites.track_count) {
      await readLikedTracks(favorites.id)
    } else {
      likedTracks.value = []
    }
    return favorites.id
  }

  function findLikedTrack(
    track: LikedTrack,
    includeWhole = false,
    video = resolveBiliVideoReference(track),
    cid = resolveBiliPartCid(track),
  ) {
    if (!video) return likedTracks.value.find(item => item.id === track.id && resolveBiliPartCid(item) === cid)
    return likedTracks.value.find(item => {
      const itemCid = resolveBiliPartCid(item)
      if (itemCid !== cid && !(includeWhole && itemCid === null)) return false
      const candidate = resolveBiliVideoReference(item)
      return !!candidate && ((video.bvid != null && video.bvid === candidate.bvid)
        || (video.avid != null && video.avid === candidate.avid))
    })
  }

  function isTrackLiked(track?: TrackInfo | null, includeWhole = false) {
    return !!track && !!findLikedTrack(track, includeWhole)
  }

  function requestToggleTrack(track?: TrackInfo | null) {
    if (!track?.id) return
    void useBiliVideoPartsStore().requestCollection(track, (target, identity) => {
      void toggleTrack(target, identity)
    }, (target, video) => !!findLikedTrack(target, false, video))
  }

  async function toggleTrack(track: TrackInfo, identity?: BiliCollectionIdentity) {
    if (!await loadLikedPlaylist()) return false
    const cid = resolveBiliPartCid(track) ?? identity?.singlePageCid
    const existing = findLikedTrack(track, false, identity?.video, cid)
      ?? (identity?.allowWholeFavorite ? findLikedTrack(track, true, identity.video, cid) : undefined)
    const shouldLike = !existing
    const neteaseSongId = getNeteaseSongId(track)

    try {
      if (shouldLike) {
        const playlistId = await ensureLikedPlaylist()
        await invoke('add_to_playlist', { playlistId, track: toBackendTrack(track) })
        // 等待可能在写入前开始的读取，再取后端生成的分 P 歌单键
        await loadPromise
        await loadLikedPlaylist()
      } else if (likedPlaylistId.value !== null) {
        const trackId = existing.playlistKey || (!isBiliVideoSkipTrack(existing) ? existing.id : '')
        if (!trackId) return false
        await invoke('remove_from_playlist', {
          playlistId: likedPlaylistId.value,
          trackId,
        })
        likedTracks.value = likedTracks.value.filter(item => item.playlistKey && existing.playlistKey
          ? item.playlistKey !== existing.playlistKey : item.id !== existing.id || resolveBiliPartCid(item) !== resolveBiliPartCid(existing))
      }
      if (neteaseSongId !== null) {
        // 网易云端红心上报失败（会话失效/业务码非 200）不能静默：本地已收藏但云端未同步，
        // 两端红心会无提示地漂移，明确提示用户（CL-2）
        useRecommendStore()
          .toggleLikeSong(neteaseSongId, shouldLike)
          .then((ok) => {
            if (!ok) {
              useToastStore().show(
                i18n.global.t('player.like_cloud_sync_failed'),
                'info',
              )
            }
          })
          .catch(() => {
            useToastStore().show(
              i18n.global.t('player.like_cloud_sync_failed'),
              'info',
            )
          })
      }
      return true
    } catch (e) {
      log.error('toggleLikedTrack:', e)
      await loadLikedPlaylist()
      return false
    }
  }

  async function start() {
    if (!unlistenPlaylistsChanged) {
      try {
        unlistenPlaylistsChanged = await listen('playlists-changed', () => {
          loadLikedPlaylist()
        })
      } catch (e) {
        log.error('listen playlists-changed for liked songs:', e)
      }
    }
    await loadLikedPlaylist()
  }

  function stop() {
    if (unlistenPlaylistsChanged) {
      unlistenPlaylistsChanged()
      unlistenPlaylistsChanged = null
    }
  }

  return {
    likedPlaylistId,
    isLoading,
    isReady,
    loadLikedPlaylist,
    isTrackLiked,
    requestToggleTrack,
    start,
    stop,
  }
})
