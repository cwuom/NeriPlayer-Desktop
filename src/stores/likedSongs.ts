import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { TrackInfo } from '@/stores/player'
import { useRecommendStore } from '@/stores/recommend'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import { FAVORITES_PLAYLIST_NAMES, isFavoritesPlaylist } from '@/modules/library/localPlaylists'
import { createLogger } from '@/utils/logger'

const log = createLogger('liked-songs')

interface PlaylistInfo {
  id: number
  name: string
  track_count?: number
}

const DEFAULT_LIKED_PLAYLIST_NAME = FAVORITES_PLAYLIST_NAMES[0]

export const useLikedSongsStore = defineStore('likedSongs', () => {
  const likedPlaylistId = ref<number | null>(null)
  const likedTrackIds = ref<Set<string>>(new Set())
  const isLoading = ref(false)
  const isReady = ref(false)

  let loadPromise: Promise<void> | null = null
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

  function setTrackLiked(trackId: string, liked: boolean) {
    const next = new Set(likedTrackIds.value)
    if (liked) {
      next.add(trackId)
    } else {
      next.delete(trackId)
    }
    likedTrackIds.value = next
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
          likedTrackIds.value = new Set()
          return
        }

        likedPlaylistId.value = liked.id
        const tracks = await invoke<Array<{ id?: string }>>('get_playlist_tracks', { id: liked.id })
        likedTrackIds.value = new Set(tracks.map(t => t.id || '').filter(Boolean))
      } catch (e) {
        log.error('loadLikedPlaylist:', e)
      } finally {
        isReady.value = true
        isLoading.value = false
        loadPromise = null
      }
    })()

    return loadPromise
  }

  async function ensureLikedPlaylist() {
    await loadLikedPlaylist()
    if (likedPlaylistId.value !== null) return likedPlaylistId.value

    // 后端以固定 id -1001 创建，和 Android 的"我喜欢的音乐"是同一个同步歌单
    const favorites = await invoke<PlaylistInfo>('ensure_favorites_playlist', { name: DEFAULT_LIKED_PLAYLIST_NAME })
    likedPlaylistId.value = favorites.id
    // 期间完成的同步可能已经带来了收藏，有曲目时重新读取，免得已收藏的歌显示成未收藏
    if (favorites.track_count) {
      const tracks = await invoke<Array<{ id?: string }>>('get_playlist_tracks', { id: favorites.id })
      likedTrackIds.value = new Set(tracks.map(t => t.id || '').filter(Boolean))
    } else {
      likedTrackIds.value = new Set()
    }
    return favorites.id
  }

  function isTrackLiked(track?: TrackInfo | null) {
    if (!track?.id) return false
    return likedTrackIds.value.has(track.id)
  }

  async function toggleTrack(track?: TrackInfo | null) {
    if (!track?.id) return false

    await loadLikedPlaylist()
    const shouldLike = !isTrackLiked(track)
    const neteaseSongId = getNeteaseSongId(track)

    try {
      if (shouldLike) {
        const playlistId = await ensureLikedPlaylist()
        await invoke('add_to_playlist', { playlistId, track: toBackendTrack(track) })
        setTrackLiked(track.id, true)
      } else if (likedPlaylistId.value !== null) {
        await invoke('remove_from_playlist', {
          playlistId: likedPlaylistId.value,
          trackId: track.id,
        })
        setTrackLiked(track.id, false)
      }
      if (neteaseSongId !== null) {
        // 网易云端红心上报失败（会话失效/业务码非 200）不能静默：本地已收藏但云端未同步，
        // 两端红心会无提示地漂移，明确提示用户（CL-2）
        useRecommendStore()
          .toggleLikeSong(neteaseSongId, shouldLike)
          .then((ok) => {
            if (!ok) {
              useToastStore().show(
                (i18n.global as any).t('player.like_cloud_sync_failed'),
                'info',
              )
            }
          })
          .catch(() => {
            useToastStore().show(
              (i18n.global as any).t('player.like_cloud_sync_failed'),
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
    likedTrackIds,
    isLoading,
    isReady,
    loadLikedPlaylist,
    isTrackLiked,
    toggleTrack,
    start,
    stop,
  }
})
