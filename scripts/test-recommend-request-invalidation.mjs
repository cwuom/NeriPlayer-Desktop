import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const piniaUrl = dataModule('export const defineStore = (_id, setup) => setup')
const vueUrl = dataModule('export const ref = value => ({ value })')
const tauriUrl = dataModule('export const invoke = (command, args) => globalThis.__recommendInvoke(command, args)')
const loggerUrl = dataModule('export const createLogger = () => ({ error() {}, warn() {}, info() {} })')
const parserUrl = dataModule(`
  export const parseYouTubeLibraryPlaylists = data =>
    Array.isArray(data?.playlists) ? data.playlists : []
`)

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
const invocationKey = (command, args) => (
  command === 'get_user_playlists' ? `${command}:${args?.platform ?? ''}` : command
)
globalThis.__recommendInvoke = (command, args) => new Promise((resolve, reject) => {
  pending.set(invocationKey(command, args), { resolve, reject })
})

const { useRecommendStore } = await import(dataModule(compiled))
const store = useRecommendStore()

const playlistRequest = store.fetchUserPlaylists('netease')
await Promise.resolve()
store.clearPlatformCache('netease')
assert.equal(store.isLoading.value, false)
pending.get('get_user_playlists:netease').resolve({
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

const biliRequest = store.fetchUserPlaylists('bilibili')
const youtubeRequest = store.fetchUserPlaylists('youtube')
await Promise.resolve()
assert.equal(store.isLoading.value, true, 'parallel platform requests must set loading state')

store.clearPlatformCache('bilibili')
assert.equal(
  store.isLoading.value,
  true,
  'invalidating Bilibili must not finish the active YouTube request',
)
pending.get('get_user_playlists:bilibili').resolve({
  code: 0,
  data: { list: [{ id: 8, title: 'stale Bilibili', cover: 'cover', media_count: 1 }] },
})
assert.equal(await biliRequest, false, 'an invalidated Bilibili request must report stale')
assert.deepEqual(
  store.userPlaylists.value.bilibili,
  undefined,
  'a stale Bilibili response must not repopulate the cleared platform cache',
)

pending.get('get_user_playlists:youtube').resolve({
  playlists: [{ id: 'PL-current', name: 'Current YouTube', coverUrl: '', trackCount: 2 }],
})
assert.equal(await youtubeRequest, true)
assert.equal(store.userPlaylists.value.youtube[0].id, 'PL-current')
assert.equal(store.isLoading.value, false)

const staleYoutubeRequest = store.fetchUserPlaylists('youtube')
await Promise.resolve()
store.clearPlatformCache('youtube')
pending.get('get_user_playlists:youtube').resolve({
  playlists: [{ id: 'PL-stale', name: 'Stale YouTube', coverUrl: '', trackCount: 1 }],
})
assert.equal(await staleYoutubeRequest, false, 'an invalidated YouTube request must report stale')
assert.deepEqual(
  store.userPlaylists.value.youtube,
  undefined,
  'a stale YouTube response must not repopulate the cleared platform cache',
)

const biliBusinessError = store.fetchUserPlaylists('bilibili')
await Promise.resolve()
pending.get('get_user_playlists:bilibili').resolve({ code: -101, data: { list: [] } })
assert.equal(await biliBusinessError, false, 'Bilibili non-zero business codes must fail')
assert.deepEqual(store.userPlaylists.value.bilibili, undefined)

const youtubeBusinessError = store.fetchUserPlaylists('youtube')
await Promise.resolve()
pending.get('get_user_playlists:youtube').resolve({
  error: { code: 401, message: 'YouTube session expired' },
})
assert.equal(await youtubeBusinessError, false, 'YouTube error payloads must fail')
assert.deepEqual(store.userPlaylists.value.youtube, undefined)

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
