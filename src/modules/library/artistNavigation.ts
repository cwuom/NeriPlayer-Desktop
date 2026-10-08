export interface NeteaseArtistLink {
  id: number
  name: string
}

export function neteaseSongArtists(detail: unknown, songId: number): NeteaseArtistLink[] {
  if (!detail || typeof detail !== 'object') return []
  const songs = (detail as { songs?: unknown }).songs
  if (!Array.isArray(songs)) return []
  const song = songs.find(value => value && typeof value === 'object' && Number(value.id) === songId)
  const artists = song?.ar ?? song?.artists
  if (!Array.isArray(artists)) return []
  const result: NeteaseArtistLink[] = []
  for (const artist of artists) {
    if (!artist || typeof artist !== 'object') continue
    const id = Number(artist.id)
    const name = typeof artist.name === 'string' ? artist.name.trim() : ''
    if (!Number.isSafeInteger(id) || id <= 0 || !name || result.some(item => item.id === id)) continue
    result.push({ id, name })
  }
  return result
}
