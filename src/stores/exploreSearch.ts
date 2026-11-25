import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { createLogger } from '@/utils/logger'

const log = createLogger('explore-search')

export type ExplorePlatform = 'netease' | 'bilibili' | 'youtube' | 'link'
export type ExploreKind = 'songs' | 'playlists' | 'artists' | 'videos' | 'creators'

/// 对齐 Android：网易云可搜歌曲/歌单/歌手，YouTube Music 与 B 站可搜歌曲/视频/创作者
export const EXPLORE_KINDS: Record<Exclude<ExplorePlatform, 'link'>, ExploreKind[]> = {
  netease: ['songs', 'playlists', 'artists'],
  youtube: ['songs', 'videos', 'creators'],
  bilibili: ['songs', 'videos', 'creators'],
}

export interface ExploreSong {
  kind: 'song'
  id: string
  title: string
  artist: string
  album: string
  duration_ms: number
  source: string
  cover_url: string | null
}

export interface ExploreCollection {
  kind: 'playlist' | 'artist'
  platform: 'netease' | 'bilibili' | 'youtube'
  id: string
  name: string
  cover_url: string | null
  subtitle: string
  track_count?: number
  /** 仅 B 站创作者搜索结果携带 */
  follower_count?: number
  video_count?: number
}

export interface ExploreNotice {
  kind: 'notice'
  reason: 'no_link' | 'unsupported' | 'not_found' | string
}

export type ExploreItem = ExploreSong | ExploreCollection | ExploreNotice

interface ExplorePage {
  items: ExploreItem[]
  has_more: boolean
}

export const useExploreSearchStore = defineStore('exploreSearch', () => {
  const items = ref<ExploreItem[]>([])
  const isSearching = ref(false)
  const isLoadingMore = ref(false)
  const hasMore = ref(false)
  const error = ref<string | null>(null)
  let page = 1
  let context = ''
  // 只有最近一次请求能写回：慢的旧关键词、旧平台或旧分类响应直接丢弃
  let requestSeq = 0

  function keyOf(platform: ExplorePlatform, kind: ExploreKind, query: string) {
    return `${platform}|${kind}|${query.trim()}`
  }

  async function search(platform: ExplorePlatform, kind: ExploreKind, query: string) {
    const requestId = ++requestSeq
    const trimmed = query.trim()
    context = keyOf(platform, kind, trimmed)
    page = 1
    hasMore.value = false
    isLoadingMore.value = false
    if (!trimmed) {
      items.value = []
      error.value = null
      isSearching.value = false
      return
    }
    isSearching.value = true
    error.value = null
    try {
      if (platform === 'link') {
        const item = await invoke<ExploreItem>('resolve_share_link', { text: trimmed })
        if (requestId !== requestSeq) return
        items.value = [item]
        return
      }
      const result = await invoke<ExplorePage>('explore_search', { platform, kind, query: trimmed, page: 1 })
      if (requestId !== requestSeq) return
      items.value = result.items
      hasMore.value = result.has_more
    } catch (e) {
      if (requestId !== requestSeq) return
      log.error('Explore search failed:', e)
      items.value = []
      error.value = String(e)
    } finally {
      if (requestId === requestSeq) isSearching.value = false
    }
  }

  async function loadMore(platform: ExplorePlatform, kind: ExploreKind, query: string) {
    if (!hasMore.value || isLoadingMore.value || isSearching.value || platform === 'link') return
    if (context !== keyOf(platform, kind, query)) return
    const requestId = requestSeq
    isLoadingMore.value = true
    try {
      const result = await invoke<ExplorePage>('explore_search', { platform, kind, query: query.trim(), page: page + 1 })
      if (requestId !== requestSeq) return
      page += 1
      const seen = new Set(items.value.map(identity))
      items.value = [...items.value, ...result.items.filter(item => !seen.has(identity(item)))]
      hasMore.value = result.has_more && result.items.length > 0
    } catch (e) {
      if (requestId !== requestSeq) return
      log.warn('Explore load more failed:', e)
      hasMore.value = false
    } finally {
      if (requestId === requestSeq) isLoadingMore.value = false
    }
  }

  function clear() {
    requestSeq++
    items.value = []
    error.value = null
    hasMore.value = false
    isSearching.value = false
    isLoadingMore.value = false
    context = ''
  }

  return { items, isSearching, isLoadingMore, hasMore, error, search, loadMore, clear }
})

function identity(item: ExploreItem): string {
  if (item.kind === 'song') return `song:${item.id}`
  if (item.kind === 'notice') return `notice:${item.reason}`
  return `${item.kind}:${item.platform}:${item.id}`
}
