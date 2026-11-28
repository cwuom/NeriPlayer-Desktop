import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  extractTrackVideoId,
  isYouTubePlaylistIncomplete,
  parseYouTubeLibraryPlaylists,
  parseYouTubePlaylistMeta,
  parseYouTubePlaylistTracks,
} from '@/modules/youtube/youtubePlaylistParse'

// 库内歌单：grid + twoRow
const libraryRoot = {
  contents: {
    singleColumnBrowseResultsRenderer: {
      tabs: [{
        tabRenderer: {
          content: {
            sectionListRenderer: {
              contents: [{
                gridRenderer: {
                  items: [{
                    musicTwoRowItemRenderer: {
                      title: { runs: [{ text: 'My Playlist' }] },
                      subtitle: { runs: [{ text: '12 songs' }] },
                      navigationEndpoint: {
                        browseEndpoint: {
                          browseId: 'VLplaylist1',
                          browseEndpointContextSupportedConfigs: {
                            browseEndpointContextMusicConfig: {
                              pageType: 'MUSIC_PAGE_TYPE_PLAYLIST',
                            },
                          },
                        },
                      },
                      thumbnailRenderer: {
                        musicThumbnailRenderer: {
                          thumbnail: {
                            thumbnails: [{ url: 'https://img/cover.jpg' }],
                          },
                        },
                      },
                    },
                  }],
                },
              }],
            },
          },
        },
      }],
    },
  },
}

// 歌单详情：secondaryContents + playlistItemData videoId
const detailRoot = {
  header: {
    musicDetailHeaderRenderer: {
      title: { runs: [{ text: 'Detail PL' }] },
      subtitle: { runs: [{ text: 'Album - 2024' }] },
      thumbnail: {
        croppedSquareThumbnailRenderer: {
          thumbnail: { thumbnails: [{ url: 'https://img/pl.jpg' }] },
        },
      },
    },
  },
  contents: {
    twoColumnBrowseResultsRenderer: {
      secondaryContents: {
        sectionListRenderer: {
          contents: [{
            musicPlaylistShelfRenderer: {
              contents: [{
                musicResponsiveListItemRenderer: {
                  playlistItemData: { videoId: 'vid001' },
                  flexColumns: [
                    {
                      musicResponsiveListItemFlexColumnRenderer: {
                        text: { runs: [{ text: 'Song A' }] },
                      },
                    },
                    {
                      musicResponsiveListItemFlexColumnRenderer: {
                        text: { runs: [{ text: 'Artist A' }] },
                      },
                    },
                  ],
                  fixedColumns: [{
                    musicResponsiveListItemFixedColumnRenderer: {
                      text: { runs: [{ text: '3:21' }] },
                    },
                  }],
                  thumbnail: {
                    musicThumbnailRenderer: {
                      thumbnail: { thumbnails: [{ url: 'https://img/track.jpg' }] },
                    },
                  },
                },
              }],
            },
          }],
        },
      },
    },
  },
}

// 新版 YTM: header 在 twoColumn tabs section 的 musicResponsiveHeaderRenderer 中
const responsiveHeaderRoot = {
  contents: {
    twoColumnBrowseResultsRenderer: {
      tabs: [{
        tabRenderer: {
          content: {
            sectionListRenderer: {
              contents: [{
                musicResponsiveHeaderRenderer: {
                  title: { runs: [{ text: 'Responsive PL' }] },
                  subtitle: { runs: [{ text: '12 songs' }] },
                  thumbnail: {
                    musicThumbnailRenderer: {
                      thumbnail: { thumbnails: [{ url: 'https://img/responsive-cover.jpg' }] },
                    },
                  },
                },
              }],
            },
          },
        },
      }],
      secondaryContents: {
        sectionListRenderer: {
          contents: [{
            musicPlaylistShelfRenderer: {
              contents: [{
                musicResponsiveListItemRenderer: {
                  playlistItemData: { videoId: 'vid-r1' },
                  flexColumns: [
                    { musicResponsiveListItemFlexColumnRenderer: { text: { runs: [{ text: 'R1' }] } } },
                    { musicResponsiveListItemFlexColumnRenderer: { text: { runs: [{ text: 'A1' }] } } },
                  ],
                  fixedColumns: [{
                    musicResponsiveListItemFixedColumnRenderer: {
                      text: { runs: [{ text: '1:00' }] },
                    },
                  }],
                  thumbnail: {
                    musicThumbnailRenderer: {
                      thumbnail: { thumbnails: [{ url: 'https://img/t1.jpg' }] },
                    },
                  },
                },
              }],
            },
          }],
        },
      },
    },
  },
}

describe('extractTrackVideoId', () => {
  it('reads the videoId from playlistItemData', () => {
    expect(
      extractTrackVideoId({
        playlistItemData: { videoId: 'abc123' },
      }),
    ).toBe('abc123')
  })

  it('reads the videoId from the thumbnail overlay play button', () => {
    expect(
      extractTrackVideoId({
        overlay: {
          musicItemThumbnailOverlayRenderer: {
            content: {
              musicPlayButtonRenderer: {
                playNavigationEndpoint: {
                  watchEndpoint: { videoId: 'fromOverlay' },
                },
              },
            },
          },
        },
      }),
    ).toBe('fromOverlay')
  })
})

describe('parseYouTubeLibraryPlaylists', () => {
  it('parses grid twoRow playlist items', () => {
    const playlists = parseYouTubeLibraryPlaylists(libraryRoot)
    expect(playlists.length).toBe(1)
    expect(playlists[0].id).toBe('VLplaylist1')
    expect(playlists[0].name).toBe('My Playlist')
    expect(playlists[0].trackCount).toBe(12)
    expect(playlists[0].coverUrl).toBe('https://img/cover.jpg')
  })
})

describe('playlist detail parsing', () => {
  it('reads meta from the legacy musicDetailHeaderRenderer', () => {
    const meta = parseYouTubePlaylistMeta(detailRoot)
    expect(meta.playlistName).toBe('Detail PL')
    expect(meta.coverUrl).toBe('https://img/pl.jpg')
  })

  it('parses tracks from secondaryContents with playlistItemData videoIds', () => {
    const tracks = parseYouTubePlaylistTracks(detailRoot)
    expect(tracks.length).toBe(1)
    expect(tracks[0].id).toBe('youtube:vid001')
    expect(tracks[0].title).toBe('Song A')
    expect(tracks[0].artist).toBe('Artist A')
    expect(tracks[0].durationMs).toBe((3 * 60 + 21) * 1000)
    expect(tracks[0].syncPayload?.audioId).toBe('vid001')
  })

  it('reads meta from a musicResponsiveHeaderRenderer inside twoColumn tabs', () => {
    const responsiveMeta = parseYouTubePlaylistMeta(responsiveHeaderRoot)
    expect(responsiveMeta.playlistName).toBe('Responsive PL')
    expect(responsiveMeta.coverUrl).toBe('https://img/responsive-cover.jpg')
    expect(parseYouTubePlaylistTracks(responsiveHeaderRoot).length).toBe(1)
  })
})

describe('shared parser wiring', () => {
  it('is used by YouTubePlaylistView and the recommend store', () => {
    const viewSource = readFileSync(
      new URL('../../../views/YouTubePlaylistView.vue', import.meta.url),
      'utf8',
    )
    expect(viewSource).toMatch(/parseYouTubePlaylistTracks/)
    expect(viewSource).toMatch(/parseYouTubePlaylistMeta/)

    const recommendSource = readFileSync(
      new URL('../../../stores/recommend.ts', import.meta.url),
      'utf8',
    )
    expect(recommendSource).toMatch(/parseYouTubeLibraryPlaylistsShared/)
  })
})

describe('isYouTubePlaylistIncomplete', () => {
  it('只认后端写入的分页中断标记', () => {
    expect(isYouTubePlaylistIncomplete({ _neriContinuationIncomplete: true })).toBe(true)
    expect(isYouTubePlaylistIncomplete({ _neriContinuationIncomplete: 'true' })).toBe(false)
    expect(isYouTubePlaylistIncomplete({ contents: {} })).toBe(false)
    expect(isYouTubePlaylistIncomplete(null)).toBe(false)
  })
})
