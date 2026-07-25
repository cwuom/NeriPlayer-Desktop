export interface SearchableNeteasePlaylist {
  id: string | number
  name: string
  playCount?: number
  trackCount: number
}

export interface SearchableNeteaseAlbum {
  id: string | number
  name: string
  artist?: string
  trackCount: number
}

export function normalizeNeteaseLibrarySearch(value: unknown): string {
  return String(value ?? '').trim().toLocaleLowerCase()
}

function matches(query: string, values: unknown[]): boolean {
  return values.some(value => normalizeNeteaseLibrarySearch(value).includes(query))
}

export function filterNeteasePlaylists<T extends SearchableNeteasePlaylist>(
  items: T[],
  rawQuery: string,
): T[] {
  const query = normalizeNeteaseLibrarySearch(rawQuery)
  if (!query) return items
  return items.filter(item => matches(query, [item.name, item.id, item.playCount ?? 0, item.trackCount]))
}

export function filterNeteaseAlbums<T extends SearchableNeteaseAlbum>(
  items: T[],
  rawQuery: string,
): T[] {
  const query = normalizeNeteaseLibrarySearch(rawQuery)
  if (!query) return items
  return items.filter(item => matches(query, [item.name, item.artist ?? '', item.id, item.trackCount]))
}
