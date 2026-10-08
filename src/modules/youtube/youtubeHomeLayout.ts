import type { TrackInfo } from '@/stores/player'
import type { HomeFeedItem, HomeFeedShelf } from '@/stores/recommend'
import type { HomePlaylist } from '@/modules/library/neteaseHome'

export interface YoutubeHomeSection {
  key: string
  title: string
  titleKey: string
  icon: string
  kind: 'songs' | 'playlists'
  songs: TrackInfo[]
  playlists: HomePlaylist[]
}

const GUESS_KEYWORDS = ['猜你喜欢', 'guess you like', 'recommended for you']
const DAILY_KEYWORDS = ['每日发现', 'daily discover', 'discover daily']
const SONG_KEYWORDS = [
  '再听一遍', '老歌重温', '翻唱与混音', '每日发现', '猜你喜欢',
  'listen again', 'oldies', 'covers and remixes', 'daily discover',
]
const SONG_TYPE_LABELS = new Set(['song', 'songs', '歌曲', '曲', 'video', 'videos', '视频', 'mv'])
const STAT_KEYWORDS = ['播放', '观看', 'views', 'view', 'listeners', 'listener', 'monthly', '观众', '订阅者', 'subscriber']

function matchesKeywords(title: string, keywords: string[]): boolean {
  const normalize = (value: string) => value.toLowerCase().replace(/[\s·•・/\\|:_-]+/g, '')
  const normalized = normalize(title)
  return keywords.some(keyword => normalized.includes(normalize(keyword)))
}

function isSongShelf(shelf: HomeFeedShelf): boolean {
  if (!shelf.items.length) return false
  const playableCount = shelf.items.filter(item => item.videoId?.trim()).length
  return playableCount > 0 && (playableCount === shelf.items.length || matchesKeywords(shelf.title, SONG_KEYWORDS))
}

function toPlaylist(item: HomeFeedItem): HomePlaylist | null {
  const id = item.browseId?.trim() ?? ''
  if (!id) return null
  const pageType = item.pageType?.trim().toUpperCase() ?? ''
  if (pageType ? !pageType.includes('PLAYLIST') : !id.startsWith('VL')) return null
  return { id, name: item.title, coverUrl: item.coverUrl, trackCount: 0, playCount: 0 }
}

function toSong(item: HomeFeedItem, sectionTitle: string): TrackInfo | null {
  const videoId = item.videoId?.trim()
  if (!videoId) return null
  const metadata = item.subtitle.split(/[•·|]/).map(part => part.trim()).filter(part => {
    const normalized = part.toLowerCase()
    return part && !SONG_TYPE_LABELS.has(normalized.replace(/ /g, ''))
      && !/^\d+(?::\d+)+$/.test(part)
      && !STAT_KEYWORDS.some(keyword => normalized.includes(keyword))
  })
  const artist = metadata[0] || 'YouTube Music'
  const album = metadata[1] || sectionTitle
  const durationMs = typeof item.durationMs === 'number' && Number.isFinite(item.durationMs) && item.durationMs >= 0 ? item.durationMs : 0
  return {
    id: `youtube:${videoId}`, title: item.title, artist, album, durationMs,
    coverUrl: item.coverUrl, audioUrl: '', source: 'youtube',
    syncPayload: {
      id: videoId, name: item.title, artist, album, durationMs, coverUrl: item.coverUrl,
      mediaUri: `ytmusic://video/${videoId}`, channelId: 'youtube_music', audioId: videoId,
    },
  }
}

function songSection(shelf: HomeFeedShelf, key: string, titleKey = '', title = shelf.title, icon = 'explore'): YoutubeHomeSection {
  return {
    key, title, titleKey, icon, kind: 'songs',
    songs: shelf.items.flatMap((item, index) => {
      const song = toSong(item, shelf.title)
      return song ? [{ ...song, playlistKey: `youtube-home:${key}:${index}:${song.id}` }] : []
    }),
    playlists: [],
  }
}

export function buildYoutubeHomeSections(shelves: HomeFeedShelf[]): YoutubeHomeSection[] {
  const sections: YoutubeHomeSection[] = []
  const guessIndex = shelves.findIndex(shelf => isSongShelf(shelf) && matchesKeywords(shelf.title, GUESS_KEYWORDS))
  const dailyIndex = shelves.findIndex((shelf, index) => index !== guessIndex && isSongShelf(shelf) && matchesKeywords(shelf.title, DAILY_KEYWORDS))
  if (guessIndex >= 0) sections.push(songSection(shelves[guessIndex], 'youtube_guess', 'home.youtube_guess', '猜你喜欢', 'radar'))
  if (dailyIndex >= 0) sections.push(songSection(shelves[dailyIndex], 'youtube_daily', 'home.youtube_daily', '每日发现'))

  const recommendations: HomePlaylist[] = []
  const seen = new Set<string>()
  for (const shelf of shelves) {
    for (const item of shelf.items) {
      const playlist = toPlaylist(item)
      if (!playlist || seen.has(playlist.id)) continue
      seen.add(playlist.id)
      recommendations.push(playlist)
      if (recommendations.length === 24) break
    }
    if (recommendations.length === 24) break
  }
  if (recommendations.length) sections.push({
    key: 'youtube_more', title: '更多推荐', titleKey: 'home.youtube_more', icon: 'star',
    kind: 'playlists', songs: [], playlists: recommendations,
  })

  shelves.forEach((shelf, index) => {
    if (index === guessIndex || index === dailyIndex) return
    const key = `youtube_shelf:${index}:${shelf.title}`
    if (isSongShelf(shelf)) {
      sections.push(songSection(shelf, key))
      return
    }
    const playlists = shelf.items.flatMap(item => {
      const playlist = toPlaylist(item)
      return playlist ? [playlist] : []
    })
    if (playlists.length) sections.push({ key, title: shelf.title, titleKey: '', icon: 'explore', kind: 'playlists', songs: [], playlists })
  })
  return sections
}
