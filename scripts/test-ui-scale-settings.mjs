import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as pinia from 'pinia'
import * as vue from 'vue'
import ts from 'typescript'
import { loadAudioEffectsModel } from './audio-effects-model-loader.mjs'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
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
let bridgeAvailable = true
const saved = []
const bridge = {
  async invoke(command, args) {
    if (!bridgeAvailable) throw new Error('browser preview has no native bridge')
    if (command === 'get_settings') return loadResult
    assert.equal(command, 'save_settings')
    saved.push(args.settings)
    return args.settings
  },
}

function load(source, dependencies = {}) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `unexpected dependency: ${name}`)
    return dependencies[name]
  }, exports)
  return exports
}

const settingsModule = load(await read('../src/stores/settings.ts'), {
  pinia,
  vue,
  '@tauri-apps/api/core': bridge,
  '@/utils/logger': { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) },
  '@/modules/audioEffects/model': await loadAudioEffectsModel(),
  '@/modules/desktopLyrics/style': load(await read('../src/modules/desktopLyrics/style.ts')),
  '@/modules/shortcuts/bindings': load(await read('../src/modules/shortcuts/bindings.ts'), {
    './platform': load(await read('../src/modules/shortcuts/platform.ts')),
  }),
})

let cases = 0
async function regression(name, run, setup = () => {}) {
  storage.clear()
  timers.clear()
  saved.length = 0
  bridgeAvailable = true
  loadResult = { settings: {}, persisted: false }
  setup()
  const instance = pinia.createPinia()
  pinia.setActivePinia(instance)
  try {
    await run(settingsModule.useSettingsStore())
    await vue.nextTick()
    cases++
    console.log(`ok - ${name}`)
  } finally {
    pinia.disposePinia(instance)
  }
}

async function flushPersist() {
  await vue.nextTick()
  for (const callback of [...timers.values()]) callback()
  timers.clear()
  await new Promise(resolve => setImmediate(resolve))
}

await regression('old settings default to 100 percent and include it in saved snapshots', async store => {
  await store.hydrate()
  assert.equal(store.uiScale, 100)
  assert.equal(store.snapshot().uiScale, 100)
  assert.equal(saved.at(-1).uiScale, 100)
  assert.equal(storage.get('neri:ui_scale'), '100')
})

await regression('the UI scale contract exports its default, bounds, and step', () => {
  assert.equal(settingsModule.UI_SCALE_DEFAULT, 100)
  assert.equal(settingsModule.UI_SCALE_MIN, 25)
  assert.equal(settingsModule.UI_SCALE_MAX, 1000)
  assert.equal(settingsModule.UI_SCALE_STEP, 5)
})

const normalizationCases = [
  [-Number.MAX_VALUE, 25], [-1, 25], [0, 25], [24.9, 25], [25, 25],
  [27.49, 25], [27.5, 30], [74.9, 75], [75, 75],
  [77.49, 75], [77.5, 80], [100, 100], [122.49, 120], [122.5, 125],
  [127.4, 125], [127.5, 130], [197.49, 195], [197.5, 200],
  [200, 200], [200.1, 200], [202.5, 205], [997.49, 995], [997.5, 1000],
  [1000, 1000], [1000.1, 1000], [Number.MAX_VALUE, 1000],
  [Number.NaN, 100], [Number.POSITIVE_INFINITY, 100], [Number.NEGATIVE_INFINITY, 100],
  ['125', 100], ['', 100], [null, 100], [undefined, 100], [true, 100],
  [false, 100], [[], 100], [[125], 100], [{ uiScale: 125 }, 100],
]

await regression('normalization clamps raw numbers before rounding to 5 percent steps', () => {
  for (const [value, expected] of normalizationCases) {
    assert.equal(settingsModule.normalizeUiScale(value), expected, String(value))
  }
  for (let value = 25; value <= 1000; value += 5) {
    assert.equal(settingsModule.normalizeUiScale(value), value)
  }
})

await regression('snapshot imports normalize scale without changing independent lyric settings', store => {
  for (const [value, expected] of normalizationCases) {
    store.applySnapshot({ uiScale: value, lyricFontScale: 1.25 })
    assert.equal(store.uiScale, expected, String(value))
    assert.equal(store.snapshot().uiScale, expected, String(value))
    assert.equal(store.lyricFontScale, 1.25)
  }
})

await regression('persisted scale hydrates with normalization and refreshes the browser shadow', async store => {
  loadResult = { persisted: true, settings: { uiScale: 127.4 } }
  await store.hydrate()
  assert.equal(store.uiScale, 125)
  assert.equal(storage.get('neri:ui_scale'), '125')
})

await regression('malformed persisted values restore the default instead of accepting numeric strings', async store => {
  loadResult = { persisted: true, settings: { uiScale: '150' } }
  await store.hydrate()
  assert.equal(store.uiScale, 100)
  assert.equal(storage.get('neri:ui_scale'), '100')
})

await regression('legacy browser scale is available immediately and migrates to native persistence', async store => {
  assert.equal(store.uiScale, 130)
  await store.hydrate()
  assert.equal(store.uiScale, 130)
  assert.equal(saved.at(-1).uiScale, 130)
  assert.equal(storage.get('neri:ui_scale'), '130')
}, () => storage.set('neri:ui_scale', '127.5'))

await regression('scale changes persist normalized values and survive exported snapshot import', async store => {
  await store.hydrate()
  saved.length = 0
  timers.clear()
  store.uiScale = 137.4
  await flushPersist()
  assert.equal(saved.length, 1)
  assert.equal(saved[0].uiScale, 135)
  assert.equal(storage.get('neri:ui_scale'), '135')
  const exported = JSON.parse(JSON.stringify(store.snapshot()))
  assert.equal(exported.uiScale, 135)
  store.applySnapshot({ uiScale: 75 })
  store.applySnapshot(exported)
  assert.equal(store.uiScale, 135)
  assert.equal(store.snapshot().uiScale, 135)
})

await regression('saved browser scale survives reload and default restoration', async store => {
  await store.hydrate()
  store.uiScale = 175
  await flushPersist()
  loadResult = { persisted: false, settings: {} }
  const reloadedInstance = pinia.createPinia()
  pinia.setActivePinia(reloadedInstance)
  try {
    const reloaded = settingsModule.useSettingsStore()
    assert.equal(reloaded.uiScale, 175)
    await reloaded.hydrate()
    assert.equal(reloaded.uiScale, 175)
    reloaded.applySnapshot({})
    assert.equal(reloaded.uiScale, 100)
    await flushPersist()
    assert.equal(saved.at(-1).uiScale, 100)
    assert.equal(storage.get('neri:ui_scale'), '100')
  } finally {
    pinia.disposePinia(reloadedInstance)
  }
})

await regression('browser preview retains its scale when the native bridge is unavailable', async store => {
  bridgeAvailable = false
  await store.hydrate()
  assert.equal(store.uiScale, 150)
  store.uiScale = 162.5
  await flushPersist()
  assert.equal(store.snapshot().uiScale, 165)
  assert.equal(storage.get('neri:ui_scale'), '165')
}, () => storage.set('neri:ui_scale', '150'))

console.log(`UI scale settings regressions: ${cases} passed`)
