import type { RouteLocationRaw } from 'vue-router'
import { collectionWebUrl } from '@/utils/collectionLinks'
import {
  favoriteArtistRoute,
  isArtistFavoriteSource,
  type FavoritePlaylist,
} from '@/modules/library/favoriteArtists'
import { biliPlaylistRoute, parseBiliPlaylistReference } from '@/modules/library/biliPlaylistReference'

/// 对齐 Android LibraryScreen: 按 source 跳平台详情页懒加载曲目,
/// 无法定位平台页时才退回同步曲目快照的本地详情
export function favoriteRoute(fpl: FavoritePlaylist): RouteLocationRaw | null {
  if (isArtistFavoriteSource(fpl.source)) {
    return favoriteArtistRoute(fpl)
  }
  switch (fpl.source) {
    case 'netease':
      return { name: 'netease-playlist', params: { id: fpl.id } }
    case 'neteaseAlbum':
      return { name: 'netease-album', params: { id: fpl.id } }
    case 'youtubeMusic': {
      const browseId = fpl.browseId || (fpl.playlistId ? `VL${fpl.playlistId}` : '')
      if (browseId) {
        return { name: 'youtube-playlist', params: { browseId } }
      }
      break
    }
    case 'bili': {
      // 合集、视频列表不是收藏夹，按收藏夹 id 去查只会拿到空的默认收藏夹
      const reference = parseBiliPlaylistReference(fpl.browseId)
      const target = biliPlaylistRoute({
        id: fpl.id, kind: reference?.kind, mid: reference?.mid,
        name: fpl.name, coverUrl: fpl.coverUrl, trackCount: fpl.trackCount, uploader: fpl.subtitle,
      })
      if (target) {
        return target
      }
      break
    }
  }
  return { name: 'favorite-playlist', params: { id: fpl.id } }
}

/** 右键菜单「复制链接 / 在浏览器中打开」用的网页地址；本地兜底的收藏不编造外部地址 */
export function favoriteWebUrl(fpl: FavoritePlaylist): string | undefined {
  let webUrl: string | undefined
  if (fpl.source === 'netease' || fpl.source === 'neteaseAlbum' || fpl.source === 'neteaseArtist') {
    webUrl = collectionWebUrl('netease', fpl.source === 'neteaseAlbum' ? 'album' : fpl.source === 'neteaseArtist' ? 'artist' : 'playlist', fpl.id)
  } else if (fpl.source === 'biliArtist') {
    webUrl = collectionWebUrl('bilibili', 'artist', fpl.id)
  } else if (fpl.source === 'youtubeMusicArtist') {
    webUrl = collectionWebUrl('youtube', 'artist', fpl.browseId)
  } else if (fpl.source === 'youtubeMusic') {
    webUrl = collectionWebUrl('youtube', 'playlist', fpl.playlistId || fpl.browseId)
  } else if (fpl.source === 'bili') {
    const reference = parseBiliPlaylistReference(fpl.browseId)
    if (reference && ['COLLECTION', 'SERIES'].includes(reference.kind)) {
      if (/^[1-9]\d*$/.test(reference.mid) && /^[1-9]\d*$/.test(fpl.id)) {
        webUrl = `https://space.bilibili.com/${reference.mid}/lists/${fpl.id}?type=${reference.kind === 'SERIES' ? 'series' : 'season'}`
      }
    } else webUrl = collectionWebUrl('bilibili', 'playlist', fpl.id)
  }
  return webUrl
}
