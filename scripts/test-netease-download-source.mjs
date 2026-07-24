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
  export const computed = getter => ({ __testRef: true, get value() { return getter() } })
`)
const tauriCoreModuleUrl = dataModule(`
  export const invoke = (command, args) => globalThis.__downloadInvoke(command, args)
`)
const tauriEventModuleUrl = dataModule(`
  export const listen = async () => () => {}
`)
const settingsModuleUrl = dataModule(`
  export const useSettingsStore = () => globalThis.__downloadSettings
`)
const toastModuleUrl = dataModule(`
  export const useToastStore = () => globalThis.__downloadToast
`)
const i18nModuleUrl = dataModule(`
  export default { global: { t: key => key } }
`)
const loggerModuleUrl = dataModule(`
  export const createLogger = () => ({ error() {}, warn() {}, info() {} })
`)

const sourceUrl = new URL('../src/stores/download.ts', import.meta.url)
let source = await readFile(sourceUrl, 'utf8')
source = source
  .replace("from 'pinia'", `from '${piniaModuleUrl}'`)
  .replace("from 'vue'", `from '${vueModuleUrl}'`)
  .replace("from '@tauri-apps/api/core'", `from '${tauriCoreModuleUrl}'`)
  .replace("from '@tauri-apps/api/event'", `from '${tauriEventModuleUrl}'`)
  .replace("from './settings'", `from '${settingsModuleUrl}'`)
  .replace("from './toast'", `from '${toastModuleUrl}'`)
  .replace("from '@/i18n'", `from '${i18nModuleUrl}'`)
  .replace("from '@/utils/logger'", `from '${loggerModuleUrl}'`)

const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const storeModule = await import(dataModule(compiled))

globalThis.__downloadSettings = {
  neteaseQuality: 'exhigh',
  qqMusicQuality: 'high',
  downloadDir: '',
  downloadNameTemplate: '{title}',
}
const toastErrors = []
globalThis.__downloadToast = {
  success() {},
  show() {},
  error(message) { toastErrors.push(message) },
}

const store = storeModule.useDownloadStore()
const track = id => ({
  id: `netease:${id}`,
  title: `song-${id}`,
  artist: 'artist',
  album: 'album',
  durationMs: 180000,
  coverUrl: '',
  audioUrl: '',
})

let invokeCalls = []
let neteaseResponse = {
  url: 'https://music.example/preview.mp3',
  is_preview: true,
}
globalThis.__downloadInvoke = async (command, args) => {
  invokeCalls.push([command, args])
  if (command === 'get_netease_song_url' || command === 'get_netease_song_download_url') {
    return neteaseResponse
  }
  if (command === 'download_track') return undefined
  throw new Error(`Unexpected command: ${command}`)
}

await store.downloadTrack(track(10))
assert.deepEqual(invokeCalls.map(([command]) => command), [
  'get_netease_song_download_url',
])
assert.equal(toastErrors.length, 1)

invokeCalls = []
neteaseResponse = {
  url: 'https://music.example/full.flac',
  is_preview: false,
}
await store.downloadTrack(track(11))
assert.deepEqual(invokeCalls.map(([command]) => command), [
  'get_netease_song_download_url',
  'download_track',
])
assert.equal(invokeCalls[1][1].url, 'https://music.example/full.flac')

const settingsCommandSource = await readFile(
  new URL('../src-tauri/src/commands/settings_cmd.rs', import.meta.url),
  'utf8',
)
assert.match(
  settingsCommandSource,
  /pub async fn get_netease_song_download_url[\s\S]*?client\.get_song_download_url\(song_id, &quality\)/,
)
assert.match(
  settingsCommandSource,
  /#\[tauri::command\]\s+pub async fn get_netease_song_download_url/,
  'download URL command must be registered as a Tauri command',
)

const mainSource = await readFile(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8')
assert.match(mainSource, /settings_cmd::get_netease_song_download_url/)

const clientSource = await readFile(
  new URL('../src-tauri/src/api/netease/client.rs', import.meta.url),
  'utf8',
)
assert.match(clientSource, /weapi\/song\/enhance\/download\/url/)

console.log('netease download source tests passed')
