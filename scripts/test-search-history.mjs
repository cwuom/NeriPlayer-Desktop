import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

function transpile(source) {
  return ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
}
function load(path, modules = {}) {
  return readFile(new URL(path, import.meta.url), 'utf8').then(source => {
    const exports = {}
    new Function('require', 'exports', transpile(source))(name => {
      assert.ok(name in modules, `unexpected dependency ${name}`)
      return modules[name]
    }, exports)
    return exports
  })
}

const policy = await load('../src/modules/search/searchHistory.ts')
const { updatedSearchHistory, normalizeSearchHistory, SEARCH_HISTORY_LIMIT } = policy

assert.equal(SEARCH_HISTORY_LIMIT, 15)
assert.deepEqual(updatedSearchHistory(['a', 'B'], 'b'), ['b', 'a'], 'case-insensitive duplicates collapse to the new spelling')
assert.deepEqual(updatedSearchHistory(['a'], '  c  '), ['c', 'a'])
assert.deepEqual(updatedSearchHistory(['a'], '   '), ['a'], 'blank queries are not recorded')
assert.deepEqual(updatedSearchHistory([' x ', '', 'X', 'y'], 'z'), ['z', 'x', 'y'])
const full = Array.from({ length: 15 }, (_, index) => `q${index}`)
assert.deepEqual(updatedSearchHistory(full, 'new'), ['new', ...full.slice(0, 14)], 'the oldest entry drops off')
assert.deepEqual(normalizeSearchHistory(['a', 1, null, ' A ', 'b']), ['a', 'b'])
assert.deepEqual(normalizeSearchHistory('broken'), [])

const storage = new Map()
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
  removeItem: key => storage.delete(key),
}
const settings = vue.reactive({ exploreSearchHistoryEnabled: true })
const { useSearchHistoryStore } = await load('../src/stores/searchHistory.ts', {
  pinia, vue,
  './settings': { useSettingsStore: () => settings },
  '@/modules/search/searchHistory': policy,
})

storage.set('neri:explore-search-history', JSON.stringify(['old', 'OLD', '']))
pinia.setActivePinia(pinia.createPinia())
const store = useSearchHistoryStore()
assert.deepEqual(store.visible, ['old'], 'stored history is cleaned on load')
store.record('jay')
assert.deepEqual(JSON.parse(storage.get('neri:explore-search-history')), ['jay', 'old'])
settings.exploreSearchHistoryEnabled = false
assert.deepEqual(store.visible, [], 'a disabled history is hidden')
store.record('ignored')
assert.deepEqual(store.entries, ['jay', 'old'], 'and nothing new is recorded')
settings.exploreSearchHistoryEnabled = true
store.clear()
assert.deepEqual(store.visible, [])
assert.equal(storage.has('neri:explore-search-history'), false)

console.log('search history tests passed')
