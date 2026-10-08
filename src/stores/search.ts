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
  // 只有最近一次请求能写回结果，较慢的旧关键词 / 旧平台响应直接丢弃
  let requestSeq = 0

  async function search(q: string, p?: string) {
    const requestId = ++requestSeq
    if (!q.trim()) {
      results.value = []
      error.value = null
      isSearching.value = false
      return
    }

    query.value = q
    if (p) platform.value = p
    isSearching.value = true
    error.value = null

    try {
      const r = await invoke<SearchResult[]>('search', {
        query: q,
        platform: platform.value,
      })
      if (requestId !== requestSeq) return
      results.value = r
    } catch (e) {
      if (requestId !== requestSeq) return
      log.error('Search failed:', e)
      results.value = []
      error.value = String(e)
    } finally {
      if (requestId === requestSeq) isSearching.value = false
    }
  }

  function clear() {
    requestSeq++
    results.value = []
    error.value = null
    query.value = ''
    isSearching.value = false
  }

  return { results, isSearching, error, query, platform, search, clear }
})
