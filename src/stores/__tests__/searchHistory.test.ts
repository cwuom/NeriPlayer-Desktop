import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import { useSearchHistoryStore } from '@/stores/searchHistory'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'

const mocks = vi.hoisted(() => ({
  settings: null as { exploreSearchHistoryEnabled: boolean } | null,
}))

vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => mocks.settings,
}))

const STORAGE_KEY = 'neri:explore-search-history'

describe('searchHistory store', () => {
  let storage: MemoryStorage

  beforeEach(() => {
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    mocks.settings = reactive({ exploreSearchHistoryEnabled: true })
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('normalizes what was stored by older versions: trims, dedupes case-insensitively and drops junk', () => {
    storage.setItem(STORAGE_KEY, JSON.stringify([' Jay ', 'jay', 42, '', null, 'Eason', 'EASON ']))

    const history = useSearchHistoryStore()

    expect(history.entries).toEqual(['Jay', 'Eason'])
  })

  it('starts empty when the stored value is corrupt or not a list', () => {
    storage.setItem(STORAGE_KEY, '{not json')
    expect(useSearchHistoryStore().entries).toEqual([])

    setActivePinia(createPinia())
    storage.setItem(STORAGE_KEY, JSON.stringify({ 0: 'a' }))
    expect(useSearchHistoryStore().entries).toEqual([])
  })

  it('moves a repeated keyword to the front, keeps 15 entries and persists them', () => {
    const history = useSearchHistoryStore()
    for (let index = 0; index < 16; index++) history.record(`q${index}`)

    history.record('  Q3  ')

    expect(history.entries).toHaveLength(15)
    expect(history.entries[0]).toBe('Q3')
    expect(history.entries.filter(entry => entry.toLowerCase() === 'q3')).toHaveLength(1)
    expect(history.entries).not.toContain('q0')
    expect(JSON.parse(storage.getItem(STORAGE_KEY) ?? '[]')).toEqual(history.entries)
  })

  it('ignores blank keywords', () => {
    const history = useSearchHistoryStore()

    history.record('   ')

    expect(history.entries).toEqual([])
    expect(storage.getItem(STORAGE_KEY)).toBeNull()
  })

  it('neither records nor shows history while the setting is off, and shows it again when re-enabled', () => {
    storage.setItem(STORAGE_KEY, JSON.stringify(['kept']))
    const history = useSearchHistoryStore()

    mocks.settings!.exploreSearchHistoryEnabled = false
    history.record('secret')

    expect(history.visible).toEqual([])
    expect(history.entries).toEqual(['kept'])
    expect(JSON.parse(storage.getItem(STORAGE_KEY) ?? '[]')).toEqual(['kept'])

    mocks.settings!.exploreSearchHistoryEnabled = true
    expect(history.visible).toEqual(['kept'])
  })

  it('clears memory and storage', () => {
    const history = useSearchHistoryStore()
    history.record('a')

    history.clear()

    expect(history.entries).toEqual([])
    expect(storage.getItem(STORAGE_KEY)).toBeNull()
  })

  it('keeps recording in memory when storage refuses writes', () => {
    const history = useSearchHistoryStore()
    vi.spyOn(storage, 'setItem').mockImplementation(() => { throw new Error('QuotaExceededError') })
    vi.spyOn(storage, 'removeItem').mockImplementation(() => { throw new Error('SecurityError') })

    expect(() => history.record('offline')).not.toThrow()
    expect(history.entries).toEqual(['offline'])
    expect(() => history.clear()).not.toThrow()
    expect(history.entries).toEqual([])
  })
})
