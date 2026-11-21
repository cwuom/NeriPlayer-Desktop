import { describe, expect, it } from 'vitest'
import { favoriteRoute, favoriteWebUrl } from '@/components/library/favoriteLinks'
import type { FavoritePlaylist } from '@/modules/library/favoriteArtists'

function favorite(overrides: Partial<FavoritePlaylist>): FavoritePlaylist {
  return {
    id: '123', name: 'Name', coverUrl: '', trackCount: 0, source: 'netease', songs: [],
    addedTime: 0, modifiedAt: 0, isDeleted: false, sortOrder: 0, browseId: '', playlistId: '', subtitle: '',
    ...overrides,
  }
}

describe('favoriteRoute', () => {
  it('routes NetEase playlists and albums to their platform pages', () => {
    expect(favoriteRoute(favorite({ source: 'netease', id: '1' }))).toEqual({ name: 'netease-playlist', params: { id: '1' } })
    expect(favoriteRoute(favorite({ source: 'neteaseAlbum', id: '2' }))).toEqual({ name: 'netease-album', params: { id: '2' } })
  })

  it('prefers the YouTube browseId and derives one from the playlistId', () => {
    expect(favoriteRoute(favorite({ source: 'youtubeMusic', browseId: 'VLPLdemo' })))
      .toEqual({ name: 'youtube-playlist', params: { browseId: 'VLPLdemo' } })
    expect(favoriteRoute(favorite({ source: 'youtubeMusic', playlistId: 'PLdemo' })))
      .toEqual({ name: 'youtube-playlist', params: { browseId: 'VLPLdemo' } })
  })

  it('routes Bilibili folders and archive collections through the bilibili playlist page', () => {
    expect(favoriteRoute(favorite({ source: 'bili', id: '345' })))
      .toEqual({ name: 'bili-playlist', params: { mediaId: '345' } })
    expect(favoriteRoute(favorite({ source: 'bili', id: '456', browseId: 'bili-playlist/v1/COLLECTION/456/567' })))
      .toMatchObject({ name: 'bili-playlist', params: { mediaId: '456' } })
  })

  it('routes followed artists to the artist pages and rejects unusable artist ids', () => {
    expect(favoriteRoute(favorite({ source: 'neteaseArtist', id: '678', name: 'Artist' })))
      .toMatchObject({ name: 'netease-artist', params: { id: '678' } })
    expect(favoriteRoute(favorite({ source: 'youtubeMusicArtist', id: 'hash', browseId: 'UCdemo' })))
      .toMatchObject({ name: 'youtube-artist', params: { browseId: 'UCdemo' } })
    expect(favoriteRoute(favorite({ source: 'neteaseArtist', id: 'not-a-number' }))).toBeNull()
  })

  it('falls back to the synced snapshot when no platform page can be located', () => {
    expect(favoriteRoute(favorite({ source: 'unknown', id: '789' }))).toEqual({ name: 'favorite-playlist', params: { id: '789' } })
    expect(favoriteRoute(favorite({ source: 'youtubeMusic', id: 'hash' }))).toEqual({ name: 'favorite-playlist', params: { id: 'hash' } })
    expect(favoriteRoute(favorite({ source: 'bili', id: '0' }))).toEqual({ name: 'favorite-playlist', params: { id: '0' } })
  })
})

describe('favoriteWebUrl', () => {
  it('builds platform links for NetEase sources', () => {
    expect(favoriteWebUrl(favorite({ source: 'netease', id: '1' }))).toBe('https://music.163.com/playlist?id=1')
    expect(favoriteWebUrl(favorite({ source: 'neteaseAlbum', id: '2' }))).toBe('https://music.163.com/album?id=2')
    expect(favoriteWebUrl(favorite({ source: 'neteaseArtist', id: '3' }))).toBe('https://music.163.com/artist?id=3')
  })

  it('builds YouTube links from the playlist identity first', () => {
    expect(favoriteWebUrl(favorite({ source: 'youtubeMusic', browseId: 'VLPLdemo' }))).toBe('https://music.youtube.com/playlist?list=PLdemo')
    expect(favoriteWebUrl(favorite({ source: 'youtubeMusic', browseId: 'MPREb_demo', playlistId: 'OLAK5uy_demo' })))
      .toBe('https://music.youtube.com/playlist?list=OLAK5uy_demo')
    expect(favoriteWebUrl(favorite({ source: 'youtubeMusicArtist', browseId: 'UCdemo' }))).toBe('https://music.youtube.com/channel/UCdemo')
  })

  it('builds Bilibili space links for collections and medialist links for folders', () => {
    expect(favoriteWebUrl(favorite({ source: 'bili', id: '456', browseId: 'bili-playlist/v1/COLLECTION/456/567' })))
      .toBe('https://space.bilibili.com/567/lists/456?type=season')
    expect(favoriteWebUrl(favorite({ source: 'bili', id: '456', browseId: 'bili-playlist/v1/SERIES/456/567' })))
      .toBe('https://space.bilibili.com/567/lists/456?type=series')
    expect(favoriteWebUrl(favorite({ source: 'bili', id: '345' }))).toBe('https://www.bilibili.com/medialist/detail/ml345')
    expect(favoriteWebUrl(favorite({ source: 'biliArtist', id: '99' }))).toBe('https://space.bilibili.com/99')
  })

  it('does not invent a link for unknown sources or invalid collection ids', () => {
    expect(favoriteWebUrl(favorite({ source: 'unknown', id: '789' }))).toBeUndefined()
    expect(favoriteWebUrl(favorite({ source: 'bili', id: '0', browseId: 'bili-playlist/v1/SERIES/0/567' }))).toBeUndefined()
  })
})
