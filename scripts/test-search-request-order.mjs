import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const piniaUrl = dataModule('export const defineStore = (_id, setup) => setup')
const vueUrl = dataModule('export const ref = value => ({ value })')
const pending = []
const tauriUrl = dataModule('export const invoke = (command, args) => globalThis.__searchInvoke(command, args)')
const loggerUrl = dataModule('export const createLogger = () => ({ error() {}, warn() {}, info() {} })')

let source = await readFile(new URL('../src/stores/search.ts', import.meta.url), 'utf8')
source = source
  .replace("from 'pinia'", `from '${piniaUrl}'`)
  .replace("from 'vue'", `from '${vueUrl}'`)
  .replace("from '@tauri-apps/api/core'", `from '${tauriUrl}'`)
  .replace("from '@/utils/logger'", `from '${loggerUrl}'`)

const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText

globalThis.__searchInvoke = (command, args) => {
  const request = {}
  pending.push(request)
  return new Promise((resolve, reject) => {
    request.command = command
    request.args = args
    request.resolve = resolve
    request.reject = reject
  })
}

const { useSearchStore } = await import(dataModule(compiled))
const store = useSearchStore()

const first = store.search('old query', 'netease')
await Promise.resolve()
const second = store.search('new query', 'netease')
await Promise.resolve()
assert.equal(pending.length, 2)

pending[1].resolve([{
  id: 'new-song',
  title: 'new song',
  artist: 'new artist',
  album: '',
  duration_ms: 1000,
  source: 'netease',
  cover_url: null,
}])
await second
assert.equal(store.isSearching.value, false, 'latest search should finish loading')
assert.equal(store.error.value, null, 'successful search should clear error')
assert.equal(store.results.value[0].id, 'new-song')

pending[0].resolve([{
  id: 'old-song',
  title: 'old song',
  artist: 'old artist',
  album: '',
  duration_ms: 1000,
  source: 'netease',
  cover_url: null,
}])
await first
assert.equal(store.results.value[0].id, 'new-song', 'stale search must not overwrite newer results')
assert.equal(store.isSearching.value, false, 'stale search must not change loading state')

const failed = store.search('broken query', 'netease')
await Promise.resolve()
assert.equal(pending.length, 3)
pending[2].reject(new Error('search backend offline'))
await failed
assert.match(store.error.value, /search backend offline/)
assert.equal(store.isSearching.value, false)

console.log('search request order tests passed')
