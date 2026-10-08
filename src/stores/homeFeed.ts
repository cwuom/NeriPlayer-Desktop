import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  NETEASE_HOME_SECTIONS,
  normalizeHomeSection,
  type HomeSectionState,
  type NeteaseHomeSource,
} from '@/modules/library/neteaseHome'

const CACHE_KEY = 'neri:home-feed:v1'
const CACHE_VERSION = 1
const CACHE_TTL_MS = 30 * 60 * 1000
const MAX_CONCURRENT_REQUESTS = 3

const emptySection = (): HomeSectionState => ({ songs: [], playlists: [], loading: false, error: null })
const initialSections = () => Object.fromEntries(NETEASE_HOME_SECTIONS.map(section => [section.key, emptySection()])) as Record<NeteaseHomeSource, HomeSectionState>

interface SectionRequest {
  source: NeteaseHomeSource
  generation: number
  promise: Promise<void>
  resolve: () => void
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

export const useHomeFeedStore = defineStore('homeFeed', () => {
  const sections = ref(initialSections())
  const fetchedAt = new Map<NeteaseHomeSource, number>()
  const activeRequests = new Set<SectionRequest>()
  let pendingRequests: SectionRequest[] = []
  let generation = 0
  let activated = false
  let hasContext = false
  let hasRefreshed = false
  let context = ''
  let loggedIn = false
  let refreshing: Promise<void> | null = null

  function isFresh(source: NeteaseHomeSource): boolean {
    const timestamp = fetchedAt.get(source)
    const age = timestamp === undefined ? Infinity : Date.now() - timestamp
    return age >= 0 && age < CACHE_TTL_MS && !sections.value[source].error
  }

  function clearPrivateSections() {
    for (const section of NETEASE_HOME_SECTIONS) {
      if (!section.requiresLogin) continue
      sections.value[section.key] = emptySection()
      fetchedAt.delete(section.key)
    }
  }

  function loadCache() {
    try {
      const raw = localStorage.getItem(CACHE_KEY)
      const cache = raw ? record(JSON.parse(raw)) : null
      if (cache?.version !== CACHE_VERSION || cache.context !== context) return
      const cachedSections = record(cache.sections)
      for (const definition of NETEASE_HOME_SECTIONS) {
        if (definition.requiresLogin) continue
        const cached = record(cachedSections?.[definition.key])
        const timestamp = cached?.fetchedAt
        if (typeof timestamp !== 'number' || !Number.isFinite(timestamp) || timestamp > Date.now() || Date.now() - timestamp >= CACHE_TTL_MS) continue
        // 使用同一解析器校验缓存，播放地址不会进入首页缓存
        const songs = Array.isArray(cached?.songs) ? cached.songs.map(value => {
          const song = record(value)
          return { id: String(song?.id ?? '').replace(/^netease:/, ''), name: song?.title, artists: [{ name: song?.artist }], album: { name: song?.album, picUrl: song?.coverUrl }, duration: song?.durationMs }
        }) : []
        const playlists = Array.isArray(cached?.playlists) ? cached.playlists : []
        const content = normalizeHomeSection({ code: 200, result: definition.kind === 'songs' ? songs : playlists }, definition.kind)
        sections.value[definition.key] = { ...content, loading: false, error: null }
        fetchedAt.set(definition.key, timestamp)
      }
    } catch {
      // 缓存损坏不影响独立分区请求
    }
  }

  function saveCache() {
    const publicSections: Partial<Record<NeteaseHomeSource, Pick<HomeSectionState, 'songs' | 'playlists'> & { fetchedAt: number }>> = {}
    for (const definition of NETEASE_HOME_SECTIONS) {
      const timestamp = fetchedAt.get(definition.key)
      if (definition.requiresLogin || timestamp === undefined) continue
      const section = sections.value[definition.key]
      publicSections[definition.key] = { songs: section.songs, playlists: section.playlists, fetchedAt: timestamp }
    }
    try {
      localStorage.setItem(CACHE_KEY, JSON.stringify({ version: CACHE_VERSION, context, sections: publicSections }))
    } catch {
      // 存储不可用时继续保留当前内存内容
    }
  }

  function cancelPending() {
    generation++
    for (const request of pendingRequests) request.resolve()
    pendingRequests = []
    for (const request of activeRequests) request.resolve()
    for (const section of Object.values(sections.value)) section.loading = false
    refreshing = null
  }

  function isCurrent(request: SectionRequest): boolean {
    return activated && request.generation === generation
  }

  async function loadSection(request: SectionRequest) {
    try {
      const raw = await invoke<unknown>('get_netease_home_section', { source: request.source })
      if (!isCurrent(request)) return
      const definition = NETEASE_HOME_SECTIONS.find(section => section.key === request.source)!
      const content = normalizeHomeSection(raw, definition.kind)
      sections.value[request.source] = { ...content, loading: false, error: null }
      fetchedAt.set(request.source, Date.now())
      saveCache()
    } catch (error) {
      if (isCurrent(request)) sections.value[request.source].error = error instanceof Error ? error.message : String(error)
    } finally {
      if (isCurrent(request)) sections.value[request.source].loading = false
      activeRequests.delete(request)
      request.resolve()
      drainRequests()
    }
  }

  function drainRequests() {
    // 已发出的 IPC 无法取消，换号后的请求仍需等待旧请求释放并发名额
    while (activated && activeRequests.size < MAX_CONCURRENT_REQUESTS && pendingRequests.length) {
      const request = pendingRequests.shift()!
      if (!isCurrent(request)) {
        request.resolve()
        continue
      }
      activeRequests.add(request)
      void loadSection(request)
    }
  }

  function queueSection(source: NeteaseHomeSource): SectionRequest {
    let resolve!: () => void
    const promise = new Promise<void>(done => { resolve = done })
    const request = { source, generation, promise, resolve }
    sections.value[source].loading = true
    sections.value[source].error = null
    pendingRequests.push(request)
    return request
  }

  function refresh(nextLoggedIn: boolean, accountKey: string, force = false): Promise<void> {
    const nextContext = nextLoggedIn ? `account:${accountKey.trim()}` : 'guest'
    if (activated && hasContext && context === nextContext && !force) {
      if (refreshing) return refreshing
      const running = [...pendingRequests, ...activeRequests].filter(isCurrent)
      if (running.length) return Promise.all(running.map(request => request.promise)).then(() => {})
    }
    const firstRefresh = !hasRefreshed
    const contextChanged = hasContext && context !== nextContext
    cancelPending()
    activated = true
    loggedIn = nextLoggedIn
    context = nextContext
    hasContext = true
    hasRefreshed = true
    if (firstRefresh || contextChanged) {
      sections.value = initialSections()
      fetchedAt.clear()
      if (firstRefresh) loadCache()
    }
    if (!loggedIn) clearPrivateSections()
    const requests = NETEASE_HOME_SECTIONS
      .filter(section => (!section.requiresLogin || loggedIn) && (force || contextChanged || !isFresh(section.key)))
      .map(section => queueSection(section.key))
    const task = Promise.all(requests.map(request => request.promise)).then(() => {})
    refreshing = task
    void task.then(() => { if (refreshing === task) refreshing = null })
    drainRequests()
    return task
  }

  function retry(source: NeteaseHomeSource): Promise<void> {
    const definition = NETEASE_HOME_SECTIONS.find(section => section.key === source)
    if (!activated || !definition || (definition.requiresLogin && !loggedIn)) return Promise.resolve()
    const running = [...pendingRequests, ...activeRequests].find(request => isCurrent(request) && request.source === source)
    if (running) return running.promise
    const request = queueSection(source)
    drainRequests()
    return request.promise
  }

  function deactivate(nextLoggedIn?: boolean, accountKey = '') {
    activated = false
    cancelPending()
    if (nextLoggedIn === undefined) return
    const nextContext = nextLoggedIn ? `account:${accountKey.trim()}` : 'guest'
    if (!hasContext || context !== nextContext) {
      sections.value = initialSections()
      fetchedAt.clear()
    }
    context = nextContext
    loggedIn = nextLoggedIn
    hasContext = true
  }

  return { sections, refresh, retry, deactivate }
})
