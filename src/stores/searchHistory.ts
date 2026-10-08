// 探索页搜索历史：只存本机（Android 同样不同步），关闭设置后既不显示也不记录
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { useSettingsStore } from './settings'
import { normalizeSearchHistory, updatedSearchHistory } from '@/modules/search/searchHistory'

const STORAGE_KEY = 'neri:explore-search-history'

function readStored(): string[] {
  try {
    return normalizeSearchHistory(JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]'))
  } catch {
    return []
  }
}

export const useSearchHistoryStore = defineStore('searchHistory', () => {
  const settings = useSettingsStore()
  const entries = ref<string[]>(readStored())
  const visible = computed(() => settings.exploreSearchHistoryEnabled ? entries.value : [])

  function record(query: string) {
    if (!settings.exploreSearchHistoryEnabled || !query.trim()) return
    entries.value = updatedSearchHistory(entries.value, query)
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(entries.value))
    } catch {
      // 存储不可用时只是记不住历史
    }
  }

  function clear() {
    entries.value = []
    try {
      localStorage.removeItem(STORAGE_KEY)
    } catch {
      // 同上
    }
  }

  return { entries, visible, record, clear }
})
