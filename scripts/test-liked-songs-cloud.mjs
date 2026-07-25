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
const tauriCoreModuleUrl = dataModule(`
  export const invoke = (command, args) => globalThis.__likedSongsInvoke(command, args)
`)
const tauriEventModuleUrl = dataModule(`
  export const listen = async () => () => {}
`)
const recommendModuleUrl = dataModule(`
  export const useRecommendStore = () => globalThis.__recommendStore
`)
const loggerModuleUrl = dataModule(`
  export const createLogger = () => ({ error() {}, warn() {}, info() {} })
`)

const sourceUrl = new URL('../src/stores/likedSongs.ts', import.meta.url)
let source = await readFile(sourceUrl, 'utf8')
const nowPlayingSource = await readFile(new URL('../src/components/NowPlaying.vue', import.meta.url), 'utf8')
assert.match(
  source,
  /let currentPromise: Promise<void> \| null = null/,
  'loadLikedPlaylist must initialize its self-referenced promise before creating the async closure',
)
assert.match(nowPlayingSource, /const\s+canToggleFavorite\s*=\s*computed/)
assert.match(nowPlayingSource, /:disabled="!canToggleFavorite"/)
assert.match(nowPlayingSource, /neteaseAuthorized:\s*auth\.canMutateNetease/)
source = source
  .replace("from 'pinia'", `from '${piniaModuleUrl}'`)
  .replace("from 'vue'", `from '${vueModuleUrl}'`)
  .replace("from '@tauri-apps/api/core'", `from '${tauriCoreModuleUrl}'`)
  .replace("from '@tauri-apps/api/event'", `from '${tauriEventModuleUrl}'`)
  .replace("from '@/stores/recommend'", `from '${recommendModuleUrl}'`)
  .replace("from '@/utils/logger'", `from '${loggerModuleUrl}'`)

const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const storeModule = await import(dataModule(compiled))

let cloudIds = new Set([42])
let cloudRefreshes = 0
let cloudRefreshSucceeds = true
const playlistTrackIds = new Set(['qq:local-liked', 'netease:42'])
const mutationCommands = []
let cloudToggleResult = false
globalThis.__recommendStore = {
  likedSongIds: cloudIds,
  async fetchLikedSongIds() {
    cloudRefreshes++
    if (!cloudRefreshSucceeds) return false
    this.likedSongIds = new Set(cloudIds)
    return true
  },
  async toggleLikeSong(songId, like) {
    mutationCommands.push(['cloud', songId, like])
    return cloudToggleResult
  },
}
globalThis.__likedSongsInvoke = async (command, args) => {
  if (command === 'list_playlists') return [{ id: 7, name: 'Liked Songs' }]
  if (command === 'get_playlist_tracks') return [...playlistTrackIds].map(id => ({ id }))
  if (command === 'add_to_playlist') {
    mutationCommands.push(['local', 'add', args.track.id])
    playlistTrackIds.add(args.track.id)
    return undefined
  }
  if (command === 'remove_from_playlist') {
    mutationCommands.push(['local', 'remove', args.trackId])
    playlistTrackIds.delete(args.trackId)
    return undefined
  }
  throw new Error(`Unexpected command: ${command}`)
}

const store = storeModule.useLikedSongsStore()
await store.start()

assert.equal(cloudRefreshes, 0, 'startup must load local likes without refreshing unverified cloud state')
assert.equal(store.isReady, true)
assert.equal(store.isTrackLiked({ id: 'qq:local-liked' }), true)

await store.loadLikedPlaylist()

assert.equal(cloudRefreshes, 0, 'local playlist reloads must not bypass the auth-owned cloud gate')
assert.equal(store.isTrackLiked({ id: 'qq:local-liked' }), true)
assert.equal(store.isTrackLiked({ id: 'netease:42' }), true)

assert.equal(typeof store.refreshCloudLikes, 'function')
await store.refreshCloudLikes()
assert.equal(cloudRefreshes, 1)

cloudIds = new Set([99])
await store.refreshCloudLikes()

assert.equal(cloudRefreshes, 2)
assert.equal(store.isTrackLiked({ id: 'netease:42' }), false)
assert.equal(store.isTrackLiked({ id: 'netease:99' }), true)

cloudRefreshSucceeds = false
await store.refreshCloudLikes()
assert.equal(store.isTrackLiked({ id: 'netease:99' }), false)
assert.equal(store.isTrackLiked({ id: 'netease:42' }), false)
assert.equal(store.isTrackLiked({ id: 'qq:local-liked' }), true)
cloudRefreshSucceeds = true

cloudIds = new Set()
globalThis.__recommendStore.likedSongIds = cloudIds
cloudToggleResult = false
mutationCommands.length = 0
const blockedUnverifiedLike = await store.toggleTrack({
  id: 'netease:123',
  title: 'cloud song',
  artist: 'tester',
  album: '',
  durationMs: 180000,
  coverUrl: '',
  audioUrl: '',
})
assert.equal(blockedUnverifiedLike, false)
assert.deepEqual(mutationCommands, [], 'unverified Netease favorites must have no local or cloud side effects')

const failedLike = await store.toggleTrack({
  id: 'netease:123',
  title: 'cloud song',
  artist: 'tester',
  album: '',
  durationMs: 180000,
  coverUrl: '',
  audioUrl: '',
}, { neteaseAuthorized: true })
assert.equal(failedLike, false)
assert.equal(store.isTrackLiked({ id: 'netease:123' }), false)
assert.deepEqual(mutationCommands, [
  ['local', 'add', 'netease:123'],
  ['cloud', 123, true],
  ['local', 'remove', 'netease:123'],
])

playlistTrackIds.add('netease:321')
cloudIds = new Set([321])
globalThis.__recommendStore.likedSongIds = cloudIds
await store.refreshCloudLikes()
mutationCommands.length = 0
const failedUnlike = await store.toggleTrack({
  id: 'netease:321',
  title: 'cloud song',
  artist: 'tester',
  album: '',
  durationMs: 180000,
  coverUrl: '',
  audioUrl: '',
}, { neteaseAuthorized: true })
assert.equal(failedUnlike, false)
assert.equal(store.isTrackLiked({ id: 'netease:321' }), true)
assert.deepEqual(mutationCommands, [
  ['local', 'remove', 'netease:321'],
  ['cloud', 321, false],
  ['local', 'add', 'netease:321'],
])

playlistTrackIds.add('netease:777')
cloudIds = new Set([888])
globalThis.__recommendStore.likedSongIds = cloudIds
await store.refreshCloudLikes()
assert.equal(store.isTrackLiked({ id: 'netease:888' }), true)
assert.equal(typeof store.clearCloudLikes, 'function')
store.clearCloudLikes()
assert.equal(store.isTrackLiked({ id: 'netease:888' }), false)
assert.equal(store.isTrackLiked({ id: 'qq:local-liked' }), true)
assert.equal(
  store.isTrackLiked({ id: 'netease:777' }),
  false,
  'clearing an account must not reveal locally cached Netease hearts from the previous account',
)

console.log('liked songs cloud tests passed')
