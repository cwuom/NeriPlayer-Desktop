import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const moduleUrl = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`

async function read(relativePath) {
  return readFile(new URL(relativePath, root), 'utf8')
}

function compile(source) {
  return ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
}

function deferred() {
  let resolve
  let reject
  const promise = new Promise((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

async function flushMicrotasks(count = 12) {
  for (let index = 0; index < count; index += 1) await Promise.resolve()
}

function loggedOutStatus() {
  return {
    netease: {
      platform: 'netease',
      logged_in: false,
      nickname: null,
      avatar_url: null,
      account_id: null,
    },
    bilibili: { platform: 'bilibili', logged_in: false, nickname: null, avatar_url: null },
    youtube: { platform: 'youtube', logged_in: false, nickname: null, avatar_url: null },
  }
}

function loggedInStatus() {
  return {
    ...loggedOutStatus(),
    netease: {
      platform: 'netease',
      logged_in: true,
      nickname: 'verified-user',
      avatar_url: 'verified-avatar',
      account_id: '42',
    },
  }
}

async function compileStores() {
  const piniaUrl = moduleUrl(`
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
  const vueUrl = moduleUrl(`
    export const ref = value => ({ __testRef: true, value })
    export const computed = getter => ({ __testRef: true, get value() { return getter() } })
  `)
  const tauriCoreUrl = moduleUrl(`
    export const invoke = (...args) => globalThis.__likedStartupAuth.invoke(...args)
  `)
  const tauriEventUrl = moduleUrl(`
    export const listen = async () => () => {}
  `)
  const recommendUrl = moduleUrl(`
    export const useRecommendStore = () => globalThis.__likedStartupAuth.recommend
  `)
  const loggerUrl = moduleUrl(`
    export const createLogger = () => ({ error() {}, warn() {}, info() {} })
  `)
  const toastUrl = moduleUrl(`
    export const useToastStore = () => ({ success() {}, error() {}, show() {} })
  `)
  const i18nUrl = moduleUrl(`
    export default { global: { t: key => key } }
  `)

  let likedSource = await read('src/stores/likedSongs.ts')
  likedSource = likedSource
    .replace(/from ['"]pinia['"]/, `from ${JSON.stringify(piniaUrl)}`)
    .replace(/from ['"]vue['"]/, `from ${JSON.stringify(vueUrl)}`)
    .replace(/from ['"]@tauri-apps\/api\/core['"]/, `from ${JSON.stringify(tauriCoreUrl)}`)
    .replace(/from ['"]@tauri-apps\/api\/event['"]/, `from ${JSON.stringify(tauriEventUrl)}`)
    .replace(/from ['"]@\/stores\/recommend['"]/, `from ${JSON.stringify(recommendUrl)}`)
    .replace(/from ['"]@\/utils\/logger['"]/, `from ${JSON.stringify(loggerUrl)}`)
  const likedStoreUrl = moduleUrl(compile(likedSource))

  const [authSource, authStatusSource, authMutationSource] = await Promise.all([
    read('src/stores/auth.ts'),
    read('src/modules/auth/authStatusRequest.ts'),
    read('src/modules/auth/authMutationRequest.ts'),
  ])
  const authStatusUrl = moduleUrl(compile(authStatusSource))
  const authMutationUrl = moduleUrl(compile(authMutationSource))
  const rewrittenAuth = authSource
    .replace(/from ['"]pinia['"]/, `from ${JSON.stringify(piniaUrl)}`)
    .replace(/from ['"]vue['"]/, `from ${JSON.stringify(vueUrl)}`)
    .replace(/from ['"]@tauri-apps\/api\/core['"]/, `from ${JSON.stringify(tauriCoreUrl)}`)
    .replace(/from ['"]\.\/toast['"]/, `from ${JSON.stringify(toastUrl)}`)
    .replace(/from ['"]\.\/recommend['"]/, `from ${JSON.stringify(recommendUrl)}`)
    .replace(/from ['"]\.\/likedSongs['"]/, `from ${JSON.stringify(likedStoreUrl)}`)
    .replace(/from ['"]@\/i18n['"]/, `from ${JSON.stringify(i18nUrl)}`)
    .replace(/from ['"]@\/utils\/logger['"]/, `from ${JSON.stringify(loggerUrl)}`)
    .replace(/from ['"]@\/modules\/auth\/authStatusRequest['"]/, `from ${JSON.stringify(authStatusUrl)}`)
    .replace(/from ['"]@\/modules\/auth\/authMutationRequest['"]/, `from ${JSON.stringify(authMutationUrl)}`)

  const [likedModule, authModule] = await Promise.all([
    import(likedStoreUrl),
    import(moduleUrl(compile(rewrittenAuth))),
  ])
  return {
    likedSongs: likedModule.useLikedSongsStore(),
    auth: authModule.useAuthStore(),
  }
}

const appSource = await read('src/App.vue')
assert.match(
  appSource,
  /Promise\.allSettled\(\[\s*syncStore\.loadConfigs\(\),\s*authStore\.checkStatus\(\),\s*likedSongs\.start\(\),\s*\]\)/,
  'startup regression must mirror the auth and liked-store concurrency used by App.vue',
)

const pendingStatus = deferred()
let statusLoader = () => pendingStatus.promise
let playlistTracksLoader = () => Promise.resolve([
  { id: 'qq:local-liked' },
  { id: 'netease:previous-account' },
])
const commands = []
globalThis.__likedStartupAuth = {
  invoke: async (command) => {
    commands.push(command)
    if (command === 'check_auth_status') return statusLoader()
    if (command === 'list_playlists') return [{ id: 7, name: 'Liked Songs' }]
    if (command === 'get_playlist_tracks') return playlistTracksLoader()
    if (command === 'get_liked_song_ids') return { code: 200, ids: [42] }
    throw new Error(`unexpected command: ${command}`)
  },
  recommend: {
    likedSongIds: new Set(),
    clearPlatformCache() {},
    async fetchLikedSongIds() {
      const data = await globalThis.__likedStartupAuth.invoke('get_liked_song_ids')
      this.likedSongIds = new Set(data.ids)
      return true
    },
  },
}

const { auth, likedSongs } = await compileStores()
const authStartup = auth.checkStatus()
const localStartup = likedSongs.start()
await localStartup

assert.equal(auth.canMutateNetease, false)
assert.equal(likedSongs.isReady, true, 'local likes must become ready while auth is still pending')
assert.equal(likedSongs.isTrackLiked({ id: 'qq:local-liked' }), true)
assert.equal(
  likedSongs.isTrackLiked({ id: 'netease:previous-account' }),
  false,
  'unverified startup must not reveal cached Netease hearts from a previous account',
)
assert.equal(commands.filter(command => command === 'get_liked_song_ids').length, 0)

pendingStatus.reject(new Error('status offline'))
await authStartup
assert.equal(commands.filter(command => command === 'get_liked_song_ids').length, 0)
assert.equal(likedSongs.isTrackLiked({ id: 'qq:local-liked' }), true)

statusLoader = () => Promise.resolve(loggedOutStatus())
await auth.checkStatus()
assert.equal(auth.canMutateNetease, false)
assert.equal(commands.filter(command => command === 'get_liked_song_ids').length, 0)

const pendingLocalReload = deferred()
playlistTracksLoader = () => pendingLocalReload.promise
const localReload = likedSongs.loadLikedPlaylist()
await flushMicrotasks()

statusLoader = () => Promise.resolve(loggedInStatus())
await auth.checkStatus()
await flushMicrotasks()

assert.equal(auth.canMutateNetease, true)
assert.equal(commands.filter(command => command === 'get_liked_song_ids').length, 1)
pendingLocalReload.resolve([
  { id: 'qq:local-liked' },
  { id: 'netease:previous-account' },
])
await localReload
assert.equal(likedSongs.isTrackLiked({ id: 'qq:local-liked' }), true)
assert.equal(likedSongs.isTrackLiked({ id: 'netease:42' }), true)
assert.equal(likedSongs.isTrackLiked({ id: 'netease:previous-account' }), false)

console.log('liked songs startup auth tests passed')
