import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const piniaUrl = dataModule('export const defineStore = (_id, setup) => setup')
const vueUrl = dataModule('export const ref = value => ({ value })')
const tauriUrl = dataModule('export const invoke = (command, args) => globalThis.__recommendInvoke(command, args)')
const loggerUrl = dataModule('export const createLogger = () => ({ error() {}, warn() {}, info() {} })')
const parserUrl = dataModule('export const parseYouTubeLibraryPlaylists = () => []')

let source = await readFile(new URL('../src/stores/recommend.ts', import.meta.url), 'utf8')
source = source
  .replace("from 'pinia'", `from '${piniaUrl}'`)
  .replace("from 'vue'", `from '${vueUrl}'`)
  .replace("from '@tauri-apps/api/core'", `from '${tauriUrl}'`)
  .replace("from '@/utils/logger'", `from '${loggerUrl}'`)
  .replace("from '@/modules/youtube/youtubePlaylistParse'", `from '${parserUrl}'`)

const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText

globalThis.localStorage = {
  getItem() { return null },
  setItem() {},
}

const pending = new Map()
globalThis.__recommendInvoke = command => new Promise((resolve, reject) => {
  pending.set(command, { resolve, reject })
})

const { useRecommendStore } = await import(dataModule(compiled))
const store = useRecommendStore()

const playlistRequest = store.fetchUserPlaylists('netease')
await Promise.resolve()
store.clearPlatformCache('netease')
assert.equal(store.isLoading.value, false)
pending.get('get_user_playlists').resolve({
  playlist: [{ id: 7, name: 'old account', trackCount: 1 }],
})
await playlistRequest
assert.deepEqual(store.userPlaylists.value.netease, undefined)

const likedRequest = store.fetchLikedSongIds()
await Promise.resolve()
store.clearPlatformCache('netease')
pending.get('get_liked_song_ids').resolve({ ids: [42] })
assert.equal(await likedRequest, false)
assert.deepEqual([...store.likedSongIds.value], [])

const qualityRequest = store.fetchHighQualityPlaylists('all')
await Promise.resolve()
pending.get('get_high_quality_playlists').reject(new Error('recommendations offline'))
await assert.rejects(qualityRequest, /recommendations offline/)

const qualityBusinessRequest = store.fetchHighQualityPlaylists('all')
await Promise.resolve()
pending.get('get_high_quality_playlists').resolve({ code: 500, playlists: [] })
await assert.rejects(qualityBusinessRequest, /code 500/)

const recommendedBusinessRequest = store.fetchRecommendedPlaylists()
await Promise.resolve()
pending.get('get_recommended_playlists').resolve({ code: 301, result: [] })
await assert.rejects(recommendedBusinessRequest, /code 301/)

console.log('recommend request invalidation tests passed')
