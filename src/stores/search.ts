import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { createLogger } from '@/utils/logger'

const log = createLogger('search')

export interface SearchResult {
  id: string
  title: string
  artist: string
  album: string
  duration_ms: number
  source: string
  cover_url: string | null
  synced_lyrics?: string | null
  plain_lyrics?: string | null
  translated_lyrics?: string | null
}

export const useSearchStore = defineStore('search', () => {
  const results = ref<SearchResult[]>([])
  const isSearching = ref(false)
  const error = ref<string | null>(null)
  const query = ref('')
  const platform = ref('all') // all | netease | qq | bilibili | youtube
  let searchGeneration = 0

  async function search(q: string, p?: string) {
    const requestGeneration = ++searchGeneration
    if (!q.trim()) {
      results.value = []
      isSearching.value = false
      error.value = null
      return
    }

    query.value = q
    if (p) platform.value = p
    const requestedPlatform = platform.value
    isSearching.value = true
    error.value = null

    try {
      const r = await invoke<SearchResult[]>('search', {
        query: q,
        platform: requestedPlatform,
      })
      if (requestGeneration !== searchGeneration) return
      results.value = r
      error.value = null
    } catch (e) {
      if (requestGeneration !== searchGeneration) return
      log.error('Search failed:', e)
      results.value = []
      error.value = e instanceof Error ? e.message : String(e)
    } finally {
      if (requestGeneration === searchGeneration) {
        isSearching.value = false
      }
    }
  }

  function clear() {
    searchGeneration++
    results.value = []
    query.value = ''
    isSearching.value = false
    error.value = null
  }

  return { results, isSearching, error, query, platform, search, clear }
})
