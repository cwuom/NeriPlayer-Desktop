import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

const storage = new Map()
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
}
const timers = new Map()
let timerId = 0
globalThis.setTimeout = callback => {
  const id = ++timerId
  timers.set(id, callback)
  return id
}
globalThis.clearTimeout = id => timers.delete(id)
let loadResult = { settings: {}, persisted: false }
const saved = []
const bridge = {
  async invoke(command, args) {
    if (command === 'get_settings') return loadResult
    assert.equal(command, 'save_settings')
    saved.push(args.settings)
  },
}

const source = await readFile(new URL('../src/stores/settings.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === 'pinia') return pinia
  if (name === 'vue') return vue
  if (name === '@tauri-apps/api/core') return bridge
  if (name === '@/utils/logger') return { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) }
  throw new Error(`Unexpected settings dependency: ${name}`)
}, exports)

let cases = 0
async function regression(name, run) {
  storage.clear()
  timers.clear()
  saved.length = 0
  loadResult = { settings: {}, persisted: false }
  const instance = pinia.createPinia()
  pinia.setActivePinia(instance)
  try {
    await run(exports.useSettingsStore())
    await vue.nextTick()
    cases++
    console.log(`ok - ${name}`)
  } finally {
    pinia.disposePinia(instance)
  }
}

await regression('old snapshots adopt Android download defaults', async store => {
  await store.hydrate()
  assert.equal(store.downloadParallelism, 6)
  assert.equal(store.downloadAutoFillMetadata, true)
  assert.equal(store.downloadEmbedLyrics, false)
  assert.equal(store.downloadFollowPlaybackQuality, true)
  assert.equal(store.downloadNameTemplate, '%title% - %artist% - %album% - %source%')
  assert.equal(saved[0].downloadParallelism, 6)
})

await regression('parallelism is an integer from one to eight', store => {
  for (const [value, expected] of [[-1, 1], [0, 1], [2.6, 3], [6, 6], [99, 8], [NaN, 6], [Infinity, 6]]) {
    store.applySnapshot({ downloadParallelism: value })
    assert.equal(store.downloadParallelism, expected)
    assert.equal(store.snapshot().downloadParallelism, expected)
  }
})

await regression('invalid flags and quality choices use defaults', store => {
  store.applySnapshot({
    downloadAutoFillMetadata: 'false', downloadEmbedLyrics: 'true',
    downloadFollowPlaybackQuality: 0, downloadNeteaseQuality: 'invalid',
    downloadQqMusicQuality: 'invalid', downloadYoutubeQuality: 'invalid', downloadBiliQuality: 'invalid',
  })
  assert.equal(store.downloadAutoFillMetadata, true)
  assert.equal(store.downloadEmbedLyrics, false)
  assert.equal(store.downloadFollowPlaybackQuality, true)
  assert.equal(store.downloadNeteaseQuality, 'exhigh')
  assert.equal(store.downloadQqMusicQuality, 'high')
  assert.equal(store.downloadYoutubeQuality, 'high')
  assert.equal(store.downloadBiliQuality, 'high')
})

await regression('NetEase download high alias becomes Android higher', store => {
  for (const value of ['high', ' high ', 'higher', ' higher ']) {
    store.applySnapshot({ downloadNeteaseQuality: value })
    assert.equal(store.downloadNeteaseQuality, 'higher')
    assert.equal(store.snapshot().downloadNeteaseQuality, 'higher')
  }
})

await regression('persisted choices retain custom templates and normalize directory', async store => {
  loadResult = {
    persisted: true,
    settings: {
      downloadParallelism: 2, downloadAutoFillMetadata: false, downloadEmbedLyrics: true,
      downloadFollowPlaybackQuality: false, downloadNeteaseQuality: 'lossless',
      downloadQqMusicQuality: 'lossless', downloadYoutubeQuality: 'medium', downloadBiliQuality: 'hires',
      downloadNameTemplate: '  {artist} - {title}  ', downloadDir: '  E:\\Music  ',
    },
  }
  await store.hydrate()
  assert.equal(store.downloadParallelism, 2)
  assert.equal(store.downloadAutoFillMetadata, false)
  assert.equal(store.downloadEmbedLyrics, true)
  assert.equal(store.downloadFollowPlaybackQuality, false)
  assert.equal(store.downloadNameTemplate, '{artist} - {title}')
  assert.equal(store.downloadDir, 'E:\\Music')
  assert.equal(store.downloadNeteaseQuality, 'lossless')
  assert.equal(storage.get('neri:download_parallelism'), '2')
  assert.equal(storage.get('neri:download_embed_lyrics'), 'true')
})

await regression('download settings round trip through exported snapshots', store => {
  store.downloadParallelism = 4
  store.downloadAutoFillMetadata = false
  store.downloadEmbedLyrics = true
  store.downloadFollowPlaybackQuality = false
  store.downloadNeteaseQuality = 'hires'
  store.downloadQqMusicQuality = 'lossless'
  store.downloadYoutubeQuality = 'low'
  store.downloadBiliQuality = 'lossless'
  const snapshot = JSON.parse(JSON.stringify(store.snapshot()))
  store.applySnapshot({})
  store.applySnapshot(snapshot)
  assert.deepEqual(store.snapshot(), snapshot)
})

await regression('download changes persist to Rust and browser shadow together', async store => {
  await store.hydrate()
  saved.length = 0
  timers.clear()
  store.downloadParallelism = 8
  store.downloadAutoFillMetadata = false
  store.downloadEmbedLyrics = true
  store.downloadFollowPlaybackQuality = false
  store.downloadYoutubeQuality = 'high'
  await vue.nextTick()
  assert.equal(timers.size, 1)
  for (const callback of timers.values()) callback()
  timers.clear()
  await Promise.resolve()
  assert.equal(saved.length, 1)
  assert.equal(saved[0].downloadParallelism, 8)
  assert.equal(saved[0].downloadAutoFillMetadata, false)
  assert.equal(saved[0].downloadEmbedLyrics, true)
  assert.equal(saved[0].downloadFollowPlaybackQuality, false)
  assert.equal(saved[0].downloadYoutubeQuality, 'high')
  assert.equal(storage.get('neri:download_parallelism'), '8')
  assert.equal(storage.get('neri:download_auto_fill_metadata'), 'false')
})

console.log(`Download settings regressions: ${cases} passed`)
