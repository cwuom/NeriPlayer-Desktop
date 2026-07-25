import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { TrackInfo } from '@/stores/player'
import { useRecommendStore } from '@/stores/recommend'
import { createLogger } from '@/utils/logger'

const log = createLogger('liked-songs')

interface PlaylistInfo {
  id: number
  name: string
}

interface ToggleTrackOptions {
  neteaseAuthorized?: boolean
}

const DEFAULT_LIKED_PLAYLIST_NAME = '我喜欢的音乐'
const SYSTEM_LIKED_PLAYLIST_ID = -1001
const LIKED_PLAYLIST_NAMES = [
  DEFAULT_LIKED_PLAYLIST_NAME,
  '我喜歡的音樂',
  'お気に入りの曲',
  'Liked Songs',
  'My Favorite Music',
]

export const useLikedSongsStore = defineStore('likedSongs', () => {
  const likedPlaylistId = ref<number | null>(null)
  const likedTrackIds = ref<Set<string>>(new Set())
  const isLoading = ref(false)
  const isReady = ref(false)

  let localLoadPromise: Promise<void> | null = null
  let cloudLoadPromise: Promise<void> | null = null
  let unlistenPlaylistsChanged: UnlistenFn | null = null
  let likedRequestGeneration = 0
  let localLikedTrackIds = new Set<string>()
  let cloudLikedTrackIds = new Set<string>()
  let cloudLikedTrackIdsLoaded = false

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
    const id = Number(track.id.slice('netease:'.length))
    return Number.isSafeInteger(id) && id > 0 ? id : null
  }

  function setLocalTrackLiked(trackId: string, liked: boolean) {
    const next = new Set(localLikedTrackIds)
    if (liked) {
      next.add(trackId)
    } else {
      next.delete(trackId)
    }
    localLikedTrackIds = next
    rebuildLikedTrackIds()
  }

  function setCloudTrackLiked(songId: number, liked: boolean) {
    const trackId = `netease:${songId}`
    const next = new Set(cloudLikedTrackIds)
    if (liked) {
      next.add(trackId)
    } else {
      next.delete(trackId)
    }
    cloudLikedTrackIds = next
    rebuildLikedTrackIds()
  }

  function rebuildLikedTrackIds() {
    const next = new Set(
      [...localLikedTrackIds]
        .filter(trackId => !cloudLikedTrackIdsLoaded || !trackId.startsWith('netease:')),
    )
    for (const trackId of cloudLikedTrackIds) next.add(trackId)
    likedTrackIds.value = next
  }

  function clearCloudLikes() {
    likedRequestGeneration++
    cloudLoadPromise = null
    cloudLikedTrackIds = new Set()
    cloudLikedTrackIdsLoaded = true
    rebuildLikedTrackIds()
  }

  async function loadLocalLikedPlaylist() {
    if (localLoadPromise) return localLoadPromise

    let currentPromise: Promise<void> | null = null
    currentPromise = (async () => {
      isLoading.value = true
      try {
        const playlists = await invoke<PlaylistInfo[]>('list_playlists')
        const liked = playlists.find(
          p => p.id === SYSTEM_LIKED_PLAYLIST_ID || LIKED_PLAYLIST_NAMES.includes(p.name),
        )
        if (!liked) {
          likedPlaylistId.value = null
          localLikedTrackIds = new Set()
        } else {
          likedPlaylistId.value = liked.id
          const tracks = await invoke<Array<{ id?: string }>>('get_playlist_tracks', { id: liked.id })
          localLikedTrackIds = new Set(tracks.map(t => t.id || '').filter(Boolean))
        }
      } catch (e) {
        log.error('loadLikedPlaylist:', e)
      } finally {
        rebuildLikedTrackIds()
        isReady.value = true
        isLoading.value = false
        if (localLoadPromise === currentPromise) localLoadPromise = null
      }
    })()
    localLoadPromise = currentPromise

    return localLoadPromise
  }

  async function refreshCloudLikes() {
    if (cloudLoadPromise) return cloudLoadPromise

    const requestGeneration = likedRequestGeneration
    let currentPromise: Promise<void> | null = null
    currentPromise = (async () => {
      const isCurrent = () => requestGeneration === likedRequestGeneration
      try {
        const recommend = useRecommendStore()
        const refreshed = await recommend.fetchLikedSongIds()
        if (!isCurrent()) return

        cloudLikedTrackIds = refreshed
          ? new Set(
              [...recommend.likedSongIds]
                .filter(id => Number.isSafeInteger(id) && id > 0)
                .map(id => `netease:${id}`),
            )
          : new Set()
        cloudLikedTrackIdsLoaded = true
      } catch (e) {
        if (!isCurrent()) return
        log.error('loadLikedPlaylist cloud refresh:', e)
        cloudLikedTrackIds = new Set()
        cloudLikedTrackIdsLoaded = true
      } finally {
        if (isCurrent()) rebuildLikedTrackIds()
        if (cloudLoadPromise === currentPromise) cloudLoadPromise = null
      }
    })()
    cloudLoadPromise = currentPromise

    return cloudLoadPromise
  }

  async function loadLikedPlaylist() {
    await loadLocalLikedPlaylist()
  }

  async function ensureLikedPlaylist() {
    await loadLikedPlaylist()
    if (likedPlaylistId.value !== null) return likedPlaylistId.value

    const created = await invoke<PlaylistInfo>('create_playlist', { name: DEFAULT_LIKED_PLAYLIST_NAME })
    likedPlaylistId.value = created.id
    localLikedTrackIds = new Set()
    rebuildLikedTrackIds()
    return created.id
  }

  function isTrackLiked(track?: TrackInfo | null) {
    if (!track?.id) return false
    return likedTrackIds.value.has(track.id)
  }

  async function toggleTrack(track?: TrackInfo | null, options: ToggleTrackOptions = {}) {
    if (!track?.id) return false

    const isNeteaseTrack = track.id.startsWith('netease:')
    const neteaseSongId = getNeteaseSongId(track)
    if (isNeteaseTrack && (!options.neteaseAuthorized || neteaseSongId === null)) return false

    await loadLikedPlaylist()
    const shouldLike = !isTrackLiked(track)
    const wasLocallyLiked = localLikedTrackIds.has(track.id)
    let localMutation: 'added' | 'removed' | null = null
    let localMutationPlaylistId: number | null = null

    try {
      if (shouldLike) {
        const playlistId = await ensureLikedPlaylist()
        if (!wasLocallyLiked) {
          await invoke('add_to_playlist', { playlistId, track: toBackendTrack(track) })
          localMutation = 'added'
          localMutationPlaylistId = playlistId
          setLocalTrackLiked(track.id, true)
        }
      } else if (wasLocallyLiked && likedPlaylistId.value !== null) {
        const playlistId = likedPlaylistId.value
        await invoke('remove_from_playlist', {
          playlistId,
          trackId: track.id,
        })
        localMutation = 'removed'
        localMutationPlaylistId = playlistId
        setLocalTrackLiked(track.id, false)
      }
      if (neteaseSongId !== null) {
        const updated = await useRecommendStore().toggleLikeSong(neteaseSongId, shouldLike)
        if (!updated) throw new Error('Netease like update rejected')
        setCloudTrackLiked(neteaseSongId, shouldLike)
      }
      return true
    } catch (e) {
      log.error('toggleLikedTrack:', e)
      try {
        if (localMutation === 'added' && localMutationPlaylistId !== null) {
          await invoke('remove_from_playlist', {
            playlistId: localMutationPlaylistId,
            trackId: track.id,
          })
          setLocalTrackLiked(track.id, false)
        } else if (localMutation === 'removed' && localMutationPlaylistId !== null) {
          await invoke('add_to_playlist', {
            playlistId: localMutationPlaylistId,
            track: toBackendTrack(track),
          })
          setLocalTrackLiked(track.id, true)
        } else {
          rebuildLikedTrackIds()
        }
      } catch (rollbackError) {
        log.error('toggleLikedTrack rollback:', rollbackError)
        await loadLikedPlaylist()
      }
      return false
    }
  }

  async function start() {
    if (!unlistenPlaylistsChanged) {
      try {
        unlistenPlaylistsChanged = await listen('playlists-changed', () => {
          void loadLocalLikedPlaylist()
        })
      } catch (e) {
        log.error('listen playlists-changed for liked songs:', e)
      }
    }
    await loadLocalLikedPlaylist()
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
    refreshCloudLikes,
    clearCloudLikes,
    isTrackLiked,
    toggleTrack,
    start,
    stop,
  }
})
