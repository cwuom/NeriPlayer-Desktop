import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const piniaModuleUrl = dataModule(`
  export const defineStore = (_id, setup) => {
    let instance
    return () => {
      if (!instance) {
        const raw = setup()
        instance = new Proxy(raw, {
          get(target, key) {
            const value = Reflect.get(target, key)
            return value && value.__testRef ? value.value : value
          },
        })
      }
      return instance
    }
  }
`)
const vueModuleUrl = dataModule(`
  export const ref = value => ({ __testRef: true, value })
`)
const tauriModuleUrl = dataModule(`
  export const invoke = (command, args) => globalThis.__recommendInvoke(command, args)
`)
const loggerModuleUrl = dataModule(`
  export const createLogger = () => ({ error() {}, warn() {}, info() {} })
`)
const youtubeModuleUrl = dataModule(`
  export const parseYouTubeLibraryPlaylists = () => []
`)

globalThis.localStorage = {
  getItem() { return null },
  setItem() {},
}

const sourceUrl = new URL('../src/stores/recommend.ts', import.meta.url)
let source = await readFile(sourceUrl, 'utf8')
source = source
  .replace("from 'pinia'", `from '${piniaModuleUrl}'`)
  .replace("from 'vue'", `from '${vueModuleUrl}'`)
  .replace("from '@tauri-apps/api/core'", `from '${tauriModuleUrl}'`)
  .replace("from '@/utils/logger'", `from '${loggerModuleUrl}'`)
  .replace(
    "from '@/modules/youtube/youtubePlaylistParse'",
    `from '${youtubeModuleUrl}'`,
  )

const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const storeModule = await import(dataModule(compiled))
const store = storeModule.useRecommendStore()

let response = { code: 200, ids: [7, '8', 0, 'invalid'] }
let rejection = null
globalThis.__recommendInvoke = async (command) => {
  if (rejection) throw rejection
  if (command === 'get_liked_song_ids' || command === 'like_song') return response
  throw new Error(`Unexpected command: ${command}`)
}

assert.equal(await store.fetchLikedSongIds(), true)
assert.deepEqual([...store.likedSongIds], [7, 8])

response = { code: 301 }
assert.equal(await store.fetchLikedSongIds(), false)
assert.deepEqual([...store.likedSongIds], [])

response = { code: 200, ids: [9] }
assert.equal(await store.fetchLikedSongIds(), true)
rejection = new Error('logged out')
assert.equal(await store.fetchLikedSongIds(), false)
assert.deepEqual([...store.likedSongIds], [])

rejection = null
response = { code: '200' }
const beforeToggle = store.likedSongIds
assert.equal(await store.toggleLikeSong(10, true), true)
assert.notEqual(store.likedSongIds, beforeToggle)
assert.deepEqual([...store.likedSongIds], [10])

console.log('recommend liked state tests passed')
