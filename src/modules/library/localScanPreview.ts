export interface LocalScanTrack {
  id: string
  title: string
  artist: string
  album: string
  durationMs: number
  audioUrl: string
  source?: string
  syncPayload?: Record<string, unknown>
}

function normalizeText(value: string): string {
  return value.trim().toLowerCase().replace(/\s+/g, ' ')
}

export function localReferenceKey(value: unknown): string | null {
  if (typeof value !== 'string' || !value.trim()) return null
  let path = value.trim()
  if (/^file:/i.test(path)) {
    try {
      const uri = new URL(path)
      path = `${uri.hostname && uri.hostname !== 'localhost' ? `//${uri.hostname}` : ''}${decodeURIComponent(uri.pathname)}`
      if (/^\/[a-z]:\//i.test(path)) path = path.slice(1)
    } catch { return null }
  }
  path = path.replace(/^\\\\\?\\UNC\\/i, '//').replace(/^\\\\\?\\/, '').replace(/\\/g, '/')
  if (!/^(?:[a-z]:\/|\/)/i.test(path)) return null
  return /^[a-z]:\//i.test(path) || path.startsWith('//') ? path.toLowerCase() : path
}

function localSourceKeys(track: LocalScanTrack): string[] {
  const payload = track.syncPayload
  const references = [track.audioUrl, payload?.localFilePath, payload?.local_file_path, payload?.mediaUri]
  if (track.id.startsWith('local:')) references.push(track.id.slice(6))
  const keys = references.map(localReferenceKey).filter((key): key is string => key !== null).map(key => `ref:${key}`)
  if (keys.length || track.source === 'local' || track.id.startsWith('local:')) {
    if (track.id.startsWith('local:')) keys.push(`id:${track.id}`)
    const sourceKey = payload?.sourceStableKey ?? payload?.source_stable_key
    if (typeof sourceKey === 'string' && sourceKey.trim()) keys.push(`source:${sourceKey.trim()}`)
  }
  return keys
}

export function scanMetadataFingerprint(track: LocalScanTrack): string | null {
  const title = normalizeText(track.title)
  const artist = normalizeText(track.artist)
  const album = normalizeText(track.album)
  if (!title || !artist || !album || album === '__local_files__' || track.durationMs <= 0) return null
  return JSON.stringify([title, artist, album, track.durationMs])
}

export function duplicateScanTrackIds(tracks: LocalScanTrack[]): Set<string> {
  const seen = new Set<string>()
  const duplicates = new Set<string>()
  for (const track of tracks) {
    const fingerprint = scanMetadataFingerprint(track)
    if (!fingerprint) continue
    if (seen.has(fingerprint)) duplicates.add(track.id)
    else seen.add(fingerprint)
  }
  return duplicates
}

export function existingScanTrackIds(tracks: LocalScanTrack[], playlistTracks: LocalScanTrack[]): Set<string> {
  const keys = new Set(playlistTracks.flatMap(localSourceKeys))
  return new Set(tracks.filter(track => localSourceKeys(track).some(key => keys.has(key))).map(track => track.id))
}

function meaningfulMetadata(track: LocalScanTrack): boolean {
  const unknown = /^(unknown(?: artist| album)?|未知(?:艺术家|歌手|专辑)?|不明|__local_files__)$/i
  return [track.title, track.artist, track.album].every(value => value.trim() && !unknown.test(value.trim()))
}

export interface ScanFilterOptions {
  query: string
  metadataOnly: boolean
  hideExisting: boolean
  hideDuplicates: boolean
  existingIds: Set<string>
  duplicateIds: Set<string>
}

export function filterScanTracks<T extends LocalScanTrack>(tracks: T[], options: ScanFilterOptions): T[] {
  const words = normalizeText(options.query).replace(/\\/g, '/').split(' ').filter(Boolean)
  return tracks.filter(track => {
    if (options.metadataOnly && !meaningfulMetadata(track)) return false
    if (options.hideExisting && options.existingIds.has(track.id)) return false
    if (options.hideDuplicates && options.duplicateIds.has(track.id)) return false
    const haystack = normalizeText(`${track.title} ${track.artist} ${track.album} ${track.audioUrl}`).replace(/\\/g, '/')
    return words.every(word => haystack.includes(word))
  })
}
