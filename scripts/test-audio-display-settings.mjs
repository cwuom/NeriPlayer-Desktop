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

const granularKeys = ['showAudioBitrate', 'showAudioFormat', 'showAudioChannels', 'showAudioSampleRate', 'showAudioBitDepth']
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

await regression('new installations display bitrate and format by default', async store => {
  await store.hydrate()
  for (const key of granularKeys) {
    assert.equal(store[key], key === 'showAudioBitrate' || key === 'showAudioFormat', key)
    assert.equal(saved[0][key], key === 'showAudioBitrate' || key === 'showAudioFormat', key)
  }
  assert.equal(store.showQualitySwitch, false)
  assert.equal(saved[0].showQualitySwitch, false)
})

await regression('old grouped preferences migrate without resetting user choices', store => {
  for (const enabled of [false, true]) {
    store.applySnapshot({ showAudioCodec: enabled, showAudioSpec: enabled, showQualitySwitch: enabled })
    assert.equal(store.showAudioBitrate, true)
    assert.equal(store.showAudioFormat, enabled)
    assert.equal(store.showAudioChannels, enabled)
    assert.equal(store.showAudioSampleRate, enabled)
    assert.equal(store.showAudioBitDepth, enabled)
    assert.equal(store.showQualitySwitch, enabled)
    assert.equal(store.showAudioCodec, enabled)
    assert.equal(store.showAudioSpec, enabled)
  }
})

await regression('explicit granular values override old grouped preferences', store => {
  store.applySnapshot({
    showAudioCodec: true, showAudioSpec: true, showAudioBitrate: false,
    showAudioFormat: false, showAudioChannels: false, showAudioSampleRate: true, showAudioBitDepth: false,
  })
  assert.equal(store.showAudioBitrate, false)
  assert.equal(store.showAudioFormat, false)
  assert.equal(store.showAudioChannels, false)
  assert.equal(store.showAudioSampleRate, true)
  assert.equal(store.showAudioBitDepth, false)
})

await regression('legacy browser preferences migrate into granular persisted fields', async store => {
  storage.set('neri:audio_codec', 'true')
  storage.set('neri:audio_spec', 'false')
  storage.set('neri:quality_switch', 'true')
  await store.hydrate()
  assert.equal(store.showAudioFormat, true)
  assert.equal(store.showAudioChannels, false)
  assert.equal(store.showAudioSampleRate, false)
  assert.equal(store.showAudioBitDepth, false)
  assert.equal(store.showQualitySwitch, true)
  assert.equal(saved[0].showAudioFormat, true)
  assert.equal(storage.get('neri:audio_format'), 'true')
})

await regression('new field values round trip independently and preserve old quality choices', store => {
  store.applySnapshot({
    showAudioBitrate: false, showAudioFormat: true, showAudioChannels: false,
    showAudioSampleRate: true, showAudioBitDepth: false, showQualitySwitch: true,
  })
  const snapshot = JSON.parse(JSON.stringify(store.snapshot()))
  store.applySnapshot({})
  store.applySnapshot(snapshot)
  assert.equal(store.showAudioBitrate, false)
  assert.equal(store.showAudioFormat, true)
  assert.equal(store.showAudioChannels, false)
  assert.equal(store.showAudioSampleRate, true)
  assert.equal(store.showAudioBitDepth, false)
  assert.equal(store.showQualitySwitch, true)
  assert.deepEqual(store.snapshot(), snapshot)
})

await regression('hydration keeps granular preferences and writes compatibility shadow keys', async store => {
  loadResult = {
    persisted: true,
    settings: { showAudioBitrate: false, showAudioFormat: true, showAudioChannels: true, showAudioSampleRate: false, showAudioBitDepth: true, showQualitySwitch: true },
  }
  await store.hydrate()
  assert.equal(store.showAudioBitrate, false)
  assert.equal(store.showAudioFormat, true)
  assert.equal(store.showAudioChannels, true)
  assert.equal(store.showAudioSampleRate, false)
  assert.equal(store.showAudioBitDepth, true)
  assert.equal(store.showQualitySwitch, true)
  assert.equal(storage.get('neri:audio_bitrate'), 'false')
  assert.equal(storage.get('neri:audio_format'), 'true')
  assert.equal(storage.get('neri:audio_channels'), 'true')
  assert.equal(storage.get('neri:audio_sample_rate'), 'false')
  assert.equal(storage.get('neri:audio_bit_depth'), 'true')
})

await regression('malformed display flags cannot enable hidden details', store => {
  store.applySnapshot({
    showAudioBitrate: 'false', showAudioFormat: 'true', showAudioChannels: 1,
    showAudioSampleRate: null, showAudioBitDepth: [], showQualitySwitch: 'true',
  })
  for (const key of granularKeys) assert.equal(store[key], key === 'showAudioBitrate' || key === 'showAudioFormat', key)
  assert.equal(store.showQualitySwitch, false)
})

await regression('each display preference persists to Rust and browser storage', async store => {
  await store.hydrate()
  saved.length = 0
  timers.clear()
  store.showAudioBitrate = false
  store.showAudioFormat = true
  store.showAudioChannels = true
  store.showAudioSampleRate = true
  store.showAudioBitDepth = true
  store.showQualitySwitch = true
  await vue.nextTick()
  assert.equal(timers.size, 1)
  for (const callback of timers.values()) callback()
  timers.clear()
  await Promise.resolve()
  assert.equal(saved.length, 1)
  for (const key of granularKeys) assert.equal(saved[0][key], key !== 'showAudioBitrate', key)
  assert.equal(saved[0].showQualitySwitch, true)
  assert.equal(storage.get('neri:audio_bitrate'), 'false')
  assert.equal(storage.get('neri:audio_bit_depth'), 'true')
})

console.log(`Audio display settings regressions: ${cases} passed`)
