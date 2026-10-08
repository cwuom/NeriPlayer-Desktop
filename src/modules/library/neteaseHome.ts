import type { TrackInfo } from '@/stores/player'

export const NETEASE_HOME_SECTIONS = [
  { key: 'personal_radar', titleKey: 'home.radar', kind: 'songs', requiresLogin: false, icon: 'radar' },
  { key: 'radar_playlists', titleKey: 'home.radar_playlists', kind: 'radar', requiresLogin: false, icon: 'explore' },
  { key: 'daily_recommend', titleKey: 'home.daily_recommend', kind: 'songs', requiresLogin: true, icon: 'today' },
  { key: 'private_fm', titleKey: 'home.private_fm', kind: 'songs', requiresLogin: true, icon: 'radio' },
  { key: 'top_soaring', titleKey: 'home.trending', kind: 'songs', requiresLogin: false, icon: 'bolt' },
  { key: 'personalized_new_songs', titleKey: 'home.new_songs', kind: 'songs', requiresLogin: false, icon: 'new_releases' },
  { key: 'top_hot', titleKey: 'home.hot_rank', kind: 'songs', requiresLogin: false, icon: 'local_fire_department' },
  { key: 'top_new', titleKey: 'home.new_rank', kind: 'songs', requiresLogin: false, icon: 'trending_up' },
  { key: 'personalized', titleKey: 'home.for_you', kind: 'playlists', requiresLogin: false, icon: 'favorite' },
  { key: 'daily_resource', titleKey: 'home.daily_playlists', kind: 'playlists', requiresLogin: true, icon: 'event_note' },
  { key: 'high_quality', titleKey: 'home.high_quality_playlists', kind: 'playlists', requiresLogin: false, icon: 'workspace_premium' },
  { key: 'hot_playlists', titleKey: 'home.hot_playlists', kind: 'playlists', requiresLogin: false, icon: 'whatshot' },
  { key: 'acg_playlists', titleKey: 'home.acg_playlists', kind: 'playlists', requiresLogin: false, icon: 'animation' },
] as const

export type NeteaseHomeSource = typeof NETEASE_HOME_SECTIONS[number]['key']
export type HomeSectionKind = typeof NETEASE_HOME_SECTIONS[number]['kind']

export interface HomePlaylist {
  id: string
  name: string
  coverUrl: string
  trackCount: number
  playCount: number
}

export interface HomeSectionState {
  songs: TrackInfo[]
  playlists: HomePlaylist[]
  loading: boolean
  error: string | null
}

type JsonRecord = Record<string, unknown>

function record(value: unknown): JsonRecord | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as JsonRecord : null
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function positiveId(value: unknown): string | null {
  if (typeof value === 'number') return Number.isSafeInteger(value) && value > 0 ? String(value) : null
  if (typeof value !== 'string' || !/^\d+$/.test(value.trim())) return null
  const id = BigInt(value.trim())
  return id > 0n ? id.toString() : null
}

function nonnegativeInteger(value: unknown, fallback = 0): number {
  if (value === null || value === undefined || value === '') return fallback
  const number = Number(value)
  return Number.isSafeInteger(number) && number >= 0 ? number : fallback
}

function coverUrl(value: string): string {
  return value.replace(/^http:\/\//i, 'https://')
}

function firstArray(values: unknown[]): unknown[] {
  return values.find(Array.isArray) as unknown[] | undefined ?? []
}

function artists(value: unknown): string[] {
  return Array.isArray(value) ? value.map(item => text(record(item)?.name)).filter(Boolean) : []
}

function parseSong(value: unknown): TrackInfo | null {
  const container = record(value)
  const song = record(container?.song) ?? container
  if (!song) return null
  const id = positiveId(song.id), title = text(song.name)
  if (!id || !title) return null
  const album = record(song.al) ?? record(song.album)
  const primaryArtists = artists(song.ar)
  return {
    id: `netease:${id}`, title,
    artist: (primaryArtists.length ? primaryArtists : artists(song.artists)).join(' / '),
    album: text(album?.name),
    durationMs: nonnegativeInteger(song.dt, nonnegativeInteger(song.duration)),
    coverUrl: coverUrl(text(album?.picUrl) || text(album?.picUrl_str)),
    audioUrl: '', source: 'netease',
  }
}

function parsePlaylist(value: unknown): HomePlaylist | null {
  const playlist = record(value)
  if (!playlist) return null
  const id = positiveId(playlist.id), name = text(playlist.name)
  if (!id || !name) return null
  return {
    id, name,
    coverUrl: coverUrl(text(playlist.picUrl) || text(playlist.coverImgUrl) || text(playlist.coverUrl)),
    trackCount: nonnegativeInteger(playlist.trackCount, nonnegativeInteger(playlist.songCount)),
    playCount: nonnegativeInteger(playlist.playCount, nonnegativeInteger(playlist.playcount)),
  }
}

function uniqueItems<T extends { id: string }>(raw: unknown[], parse: (value: unknown) => T | null): T[] {
  const seen = new Set<string>(), items: T[] = []
  for (const value of raw) {
    const item = parse(value)
    if (!item || seen.has(item.id)) continue
    seen.add(item.id)
    items.push(item)
    if (items.length === 30) break
  }
  return items
}

export function normalizeHomeSection(raw: unknown, kind: HomeSectionKind): Pick<HomeSectionState, 'songs' | 'playlists'> {
  const root = record(typeof raw === 'string' ? JSON.parse(raw) as unknown : raw)
  if (!root || Number(root.code) !== 200) throw new Error(`api_code=${root?.code ?? -1}`)
  const data = record(root.data)
  if (kind === 'songs') {
    const songs = firstArray([data?.dailySongs, data?.songs, root.data, root.result, root.songs, record(root.playlist)?.tracks])
    return { songs: uniqueItems(songs, parseSong), playlists: [] }
  }
  const playlists = firstArray([root.result, root.recommend, root.playlists, data?.playlists, data?.list])
  return { songs: [], playlists: uniqueItems(playlists, parsePlaylist) }
}
