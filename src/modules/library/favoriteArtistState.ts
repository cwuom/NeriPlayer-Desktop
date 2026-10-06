import { onUnmounted, ref, watch, type ComputedRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { parseFavoritePlaylists, type ArtistFavoriteSource } from './favoriteArtists'

export interface ArtistFavoriteInput {
  source: ArtistFavoriteSource
  id: string
  name: string
  coverUrl: string
  trackCount: number
  browseId?: string
  subtitle?: string
}

export function useArtistFavorite(artist: ComputedRef<ArtistFavoriteInput>) {
  const following = ref(false)
  const changing = ref(false)
  let version = 0
  let unmounted = false
  let unlisten: UnlistenFn | undefined

  async function refresh() {
    const request = ++version
    const current = artist.value
    const favorites = parseFavoritePlaylists(await invoke('list_favorite_playlists'))
    if (unmounted || request !== version) return
    following.value = favorites.some(favorite => favorite.source === current.source &&
      (current.source === 'youtubeMusicArtist' ? favorite.browseId === current.browseId : favorite.id === current.id))
  }

  async function toggle() {
    if (changing.value || !artist.value.name) return
    changing.value = true
    const identity = `${artist.value.source}:${artist.value.browseId || artist.value.id}`
    try {
      const value = await invoke<boolean>('set_artist_favorite', { artist: artist.value, following: !following.value })
      if (`${artist.value.source}:${artist.value.browseId || artist.value.id}` === identity) following.value = value
    } finally {
      changing.value = false
    }
  }

  watch(() => `${artist.value.source}:${artist.value.browseId || artist.value.id}`, () => {
    following.value = false
    void refresh().catch(() => {})
  }, { immediate: true })
  void listen('playlists-changed', () => { void refresh().catch(() => {}) }).then(stop => {
    if (unmounted) stop()
    else unlisten = stop
  }).catch(() => {})
  onUnmounted(() => { unmounted = true; version++; unlisten?.() })

  return { following, changing, toggle }
}
