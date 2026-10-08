import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

const source = await readFile(new URL('../src/stores/search.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText

const pending = []
function invoke(command, args) {
  assert.equal(command, 'search')
  let resolve
  let reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  pending.push({ args, resolve, reject })
  return promise
}

const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === 'pinia') return pinia
  if (name === 'vue') return vue
  if (name === '@tauri-apps/api/core') return { invoke }
  if (name === '@/utils/logger') return { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) }
  throw new Error(`Unexpected search dependency: ${name}`)
}, exports)

pinia.setActivePinia(pinia.createPinia())
const store = exports.useSearchStore()
const result = id => [{ id, title: id, artist: '', album: '', duration_ms: 0, source: 'netease', cover_url: null }]

// 慢的旧关键词晚于新关键词返回，不能覆盖新结果
const slow = store.search('old', 'netease')
const fast = store.search('new', 'netease')
pending[1].resolve(result('new'))
await fast
pending[0].resolve(result('old'))
await slow
assert.deepEqual(store.results.map(r => r.id), ['new'])
assert.equal(store.isSearching, false)

// 旧请求失败也不能清空新结果或留下错误
const failing = store.search('broken', 'bilibili')
const current = store.search('fine', 'bilibili')
pending[3].resolve(result('fine'))
await current
pending[2].reject(new Error('timeout'))
await failing
assert.deepEqual(store.results.map(r => r.id), ['fine'])
assert.equal(store.error, null)

// 当前请求失败时给出错误，便于界面与「无结果」区分
const failed = store.search('offline', 'youtube')
pending[4].reject(new Error('network down'))
await failed
assert.deepEqual(store.results, [])
assert.match(store.error, /network down/)

// clear() 之后返回的旧响应被丢弃，加载态立即结束
const inFlight = store.search('late', 'netease')
assert.equal(store.isSearching, true)
store.clear()
assert.equal(store.isSearching, false)
pending[5].resolve(result('late'))
await inFlight
assert.deepEqual(store.results, [])
assert.equal(store.query, '')

console.log('search store request ordering tests passed')
