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

await regression('old settings default to automatic and opt-in fallbacks', async store => {
  await store.hydrate()
  assert.equal(store.youtubePlaybackSource, 'automatic')
  assert.equal(store.neteaseAutoSourceSwitch, false)
  assert.equal(store.neteaseLocalSourceFallback, false)
  assert.equal(saved[0].youtubePlaybackSource, 'automatic')
})

await regression('source normalization matches Android canonical values and legacy aliases', store => {
  for (const [value, expected] of [
    ...exports.YOUTUBE_PLAYBACK_SOURCES.map(value => [value, value]),
    [' VISION_OS ', 'visionos'],
    ['AndroidVR', 'android_vr'],
    ['Creator', 'web_creator'],
    [' WEB_REMIX ', 'web_remix'],
    ['invalid', 'automatic'],
    ['constructor', 'automatic'],
    ['__proto__', 'automatic'],
    ['', 'automatic'],
    [null, 'automatic'],
  ]) {
    store.applySnapshot({ youtubePlaybackSource: value })
    assert.equal(store.youtubePlaybackSource, expected, String(value))
    assert.equal(store.snapshot().youtubePlaybackSource, expected)
  }
})

await regression('malformed fallback flags cannot enable alternate sources', store => {
  store.applySnapshot({ neteaseAutoSourceSwitch: 'true', neteaseLocalSourceFallback: 1 })
  assert.equal(store.neteaseAutoSourceSwitch, false)
  assert.equal(store.neteaseLocalSourceFallback, false)
})

await regression('hydration retains persisted source and independent fallback controls', async store => {
  loadResult = {
    persisted: true,
    settings: {
      youtubePlaybackSource: ' TV_HTML5 ',
      neteaseAutoSourceSwitch: false,
      neteaseLocalSourceFallback: true,
    },
  }
  await store.hydrate()
  assert.equal(store.youtubePlaybackSource, 'tv_html5')
  assert.equal(store.neteaseAutoSourceSwitch, false)
  assert.equal(store.neteaseLocalSourceFallback, true)
  assert.equal(storage.get('neri:youtube_playback_source'), '"tv_html5"')
  assert.equal(storage.get('neri:netease_local_source_fallback'), 'true')
})

await regression('legacy browser source values migrate with canonical storage names', async store => {
  storage.set('neri:youtube_playback_source', '"Vision_OS"')
  storage.set('neri:netease_auto_source_switch', 'true')
  storage.set('neri:netease_local_source_fallback', 'false')
  await store.hydrate()
  assert.equal(store.youtubePlaybackSource, 'visionos')
  assert.equal(saved[0].youtubePlaybackSource, 'visionos')
  assert.equal(saved[0].neteaseAutoSourceSwitch, true)
  assert.equal(saved[0].neteaseLocalSourceFallback, false)
})

await regression('changed source and fallback switches persist together to Rust and browser shadow', async store => {
  await store.hydrate()
  saved.length = 0
  timers.clear()
  store.youtubePlaybackSource = 'android_vr'
  store.neteaseAutoSourceSwitch = true
  store.neteaseLocalSourceFallback = true
  await vue.nextTick()
  assert.equal(timers.size, 1)
  for (const callback of timers.values()) callback()
  timers.clear()
  await Promise.resolve()
  assert.equal(saved.length, 1)
  assert.equal(saved[0].youtubePlaybackSource, 'android_vr')
  assert.equal(saved[0].neteaseAutoSourceSwitch, true)
  assert.equal(saved[0].neteaseLocalSourceFallback, true)
  assert.equal(storage.get('neri:youtube_playback_source'), '"android_vr"')
  assert.equal(storage.get('neri:netease_auto_source_switch'), 'true')
  assert.equal(storage.get('neri:netease_local_source_fallback'), 'true')
})

console.log(`Playback settings regressions: ${cases} passed`)
