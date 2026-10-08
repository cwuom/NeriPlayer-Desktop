export interface StorageUsageItem {
  id: string
  sizeBytes: number
  fileCount: number
  path?: string | null
  cacheKind?: string | null
  /** 存在用户数据库中的条目按记录数展示 */
  recordCount?: number | null
}

export interface StorageUsageSection {
  id: string
  items: StorageUsageItem[]
}

export interface StorageUsageSummary {
  sections: StorageUsageSection[]
  totalSizeBytes: number
  totalFileCount: number
}

export interface StorageCacheClearOptions {
  audioCache: boolean
  imageCache: boolean
  downloadStaging: boolean
  sharedMedia: boolean
  platformList: boolean
  lyricsCache: boolean
}

// 歌单详情与歌词缓存已迁入用户数据库，由后端统计；这里只剩首屏用的推荐快照
const BROWSER_CACHE_KEYS = {
  platformList: ['neri:recommend:cache', 'neri:home-feed:v1'],
  other: [],
} as const

export function mergeBrowserCacheUsage(summary: StorageUsageSummary): StorageUsageSummary {
  const sections = summary.sections.map(section => ({
    ...section,
    items: section.items.map(item => ({ ...item })),
  }))
  const cleanable = sections.find(section => section.id === 'cleanable_cache')
  if (!cleanable) return summary

  addBrowserCacheUsage(cleanable.items, 'platform_list_cache', BROWSER_CACHE_KEYS.platformList)
  addBrowserCacheUsage(cleanable.items, 'other_cache', BROWSER_CACHE_KEYS.other)

  return withTotals(sections)
}

export function clearBrowserCache(options: Pick<StorageCacheClearOptions, 'platformList'>) {
  if (typeof localStorage === 'undefined' || !options.platformList) return
  for (const key of BROWSER_CACHE_KEYS.platformList) localStorage.removeItem(key)
}

export function formatStorageSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  if (bytes < 1024) return `${Math.round(bytes)} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

function addBrowserCacheUsage(items: StorageUsageItem[], id: string, keys: readonly string[]) {
  const item = items.find(entry => entry.id === id)
  if (!item || typeof localStorage === 'undefined') return

  for (const key of keys) {
    const value = localStorage.getItem(key)
    if (value === null) continue
    item.sizeBytes += value.length * 2
    item.fileCount += 1
  }
}

function withTotals(sections: StorageUsageSection[]): StorageUsageSummary {
  return {
    sections,
    totalSizeBytes: sections.flatMap(section => section.items).reduce((sum, item) => sum + item.sizeBytes, 0),
    totalFileCount: sections.flatMap(section => section.items).reduce((sum, item) => sum + item.fileCount, 0),
  }
}
