export type LibraryTabKey = 'local' | 'favorites' | 'downloads' | 'netease' | 'bilibili' | 'youtube'
export type NeteaseLibraryCategory = 'playlists' | 'albums'

export interface LibraryLocation {
  tab: LibraryTabKey
  category: NeteaseLibraryCategory
}

const LIBRARY_TABS = new Set<LibraryTabKey>([
  'local',
  'favorites',
  'downloads',
  'netease',
  'bilibili',
  'youtube',
])
const NETEASE_CATEGORIES = new Set<NeteaseLibraryCategory>(['playlists', 'albums'])

function firstQueryValue(value: unknown): string | undefined {
  if (Array.isArray(value)) return firstQueryValue(value[0])
  return typeof value === 'string' ? value : undefined
}

export function resolveLibraryLocation(rawTab: unknown, rawCategory: unknown): LibraryLocation {
  const tabValue = firstQueryValue(rawTab)
  const categoryValue = firstQueryValue(rawCategory)

  if (tabValue === 'netease_playlists') return { tab: 'netease', category: 'playlists' }
  if (tabValue === 'netease_albums') return { tab: 'netease', category: 'albums' }

  const tab = LIBRARY_TABS.has(tabValue as LibraryTabKey)
    ? tabValue as LibraryTabKey
    : 'local'
  const category = tab === 'netease' && NETEASE_CATEGORIES.has(categoryValue as NeteaseLibraryCategory)
    ? categoryValue as NeteaseLibraryCategory
    : 'playlists'
  return { tab, category }
}

export function buildLibraryQuery(location: LibraryLocation): Record<string, string> {
  return location.tab === 'netease'
    ? { tab: 'netease', category: location.category }
    : { tab: location.tab }
}

export function isCanonicalLibraryLocation(rawTab: unknown, rawCategory: unknown): boolean {
  if (typeof rawTab !== 'string') return false
  if (rawCategory !== undefined && typeof rawCategory !== 'string') return false

  const tab = firstQueryValue(rawTab)
  const category = firstQueryValue(rawCategory)
  const resolved = resolveLibraryLocation(rawTab, rawCategory)
  const canonical = buildLibraryQuery(resolved)
  return tab === canonical.tab && category === canonical.category
}
