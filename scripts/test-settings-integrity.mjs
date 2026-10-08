import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

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
let saveResponse = null
const saved = []
const bridge = {
  async invoke(command, args) {
    if (command === 'get_settings') return loadResult
    assert.equal(command, 'save_settings')
    saved.push(args.settings)
    return saveResponse ? saveResponse(args.settings) : undefined
  },
}

function load(source, dependencies = {}) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    if (name in dependencies) return dependencies[name]
    throw new Error(`Unexpected dependency: ${name}`)
  }, exports)
  return exports
}

const settingsModule = load(await read('../src/stores/settings.ts'), {
  pinia,
  vue,
  '@tauri-apps/api/core': bridge,
  '@/utils/logger': { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) },
})
const { roundToStepPrecision } = load(await read('../src/utils/editableRange.ts'))

let cases = 0
async function regression(name, run) {
  storage.clear()
  timers.clear()
  saved.length = 0
  saveResponse = null
  loadResult = { settings: {}, persisted: false }
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

const camel = name => name.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
const between = (text, start, end) => {
  const from = text.indexOf(start)
  assert.ok(from >= 0, `missing ${start}`)
  return text.slice(from, text.indexOf(end, from + start.length))
}
const rust = await read('../src-tauri/src/settings/store.rs')
const settingsView = await read('../src/views/SettingsView.vue')
const nowPlaying = await read('../src/components/NowPlaying.vue')
const rustStruct = between(rust, 'pub struct AppSettings', 'impl Default for AppSettings')
const RUST_INTEGER_KEYS = [...rustStruct.matchAll(/pub (\w+): (?:i32|u32),/g)]
  .map(match => camel(match[1]))
  .filter(key => key !== 'formatVersion')

await regression('integer-backed settings never reach save_settings as fractions', async store => {
  await store.hydrate()
  await flushPersist()
  saved.length = 0
  for (const key of RUST_INTEGER_KEYS) store[key] += 0.4
  store.equalizerBands = [1.4, -2.6, 0, 3.5, 4]
  await flushPersist()
  assert.equal(saved.length, 1)
  for (const key of RUST_INTEGER_KEYS) assert.ok(Number.isInteger(saved[0][key]), `${key}=${saved[0][key]}`)
  assert.ok(saved[0].equalizerBands.every(Number.isInteger))
})

await regression('TypeScript defaults match AppSettings::default() except the detected locale', store => {
  const defaults = store.snapshot()
  const mismatches = []
  const body = between(rust, 'impl Default for AppSettings', 'impl AppSettings')
  for (const [, field, raw] of body.matchAll(/^\s+(\w+): (.+),\r?$/gm)) {
    const key = camel(field)
    if (key === 'formatVersion' || key === 'locale' || !(key in defaults)) continue
    let expected
    if (raw === 'true' || raw === 'false') expected = raw === 'true'
    else if (raw === 'String::new()') expected = ''
    else if (/^"[^"]*"\.into\(\)$/.test(raw)) expected = raw.slice(1, raw.lastIndexOf('"'))
    else if (/^-?\d+(\.\d+)?$/.test(raw)) expected = Number(raw)
    else continue
    if (!Object.is(defaults[key], expected)) mismatches.push(`${key}: ts=${JSON.stringify(defaults[key])} rust=${JSON.stringify(expected)}`)
  }
  assert.deepEqual(mismatches, [])
})

await regression('NetEase playback quality uses Android "higher" everywhere', store => {
  for (const value of ['high', ' high ', 'higher']) {
    store.applySnapshot({ neteaseQuality: value })
    assert.equal(store.neteaseQuality, 'higher', JSON.stringify(value))
  }
  const settingsValues = [...between(settingsView, 'const neteaseQualityOptions', '])').matchAll(/value: '(\w+)'/g)].map(m => m[1])
  const sheetValues = [...between(nowPlaying, 'const neteaseQualities', ']').matchAll(/key: '(\w+)'/g)].map(m => m[1])
  assert.deepEqual(settingsValues, sheetValues)
})

await regression('lyric font scale uses the Android 0.5-1.6 range in every editor', store => {
  for (const value of [0.5, 1.6]) {
    store.applySnapshot({ lyricFontScale: value })
    assert.equal(store.snapshot().lyricFontScale, value)
  }
  assert.equal(settingsModule.LYRIC_FONT_SCALE_MIN, 0.5)
  assert.equal(settingsModule.LYRIC_FONT_SCALE_MAX, 1.6)
  assert.match(rust, /clamp_f32\(self\.lyric_font_scale, 0\.5, 1\.6, 1\.0\)/)
  const fontSheet = between(nowPlaying, "moreSheetView === 'fontsize'", '</template>')
  assert.match(fontSheet, /:min="LYRIC_FONT_SCALE_MIN"/)
  assert.match(fontSheet, /:max="LYRIC_FONT_SCALE_MAX"/)
  assert.doesNotMatch(settingsView, /v-model\.number="lyricFontScale" min="0\.5" max="1\.5"/)
})

await regression('cover blur stays within the rendered pixel budget', store => {
  store.applySnapshot({ coverBlurAmount: 500 })
  const px = store.snapshot().coverBlurAmount * settingsModule.COVER_BLUR_PX_PER_UNIT
  assert.ok(px <= 240, `cover blur renders ${px}px`)
  assert.match(rust, /clamp_f32\(self\.cover_blur_amount, 0\.0, 8\.0, 1\.5\)/)
})

await regression('the old seamless switch merges into next-track crossfade once', store => {
  store.applySnapshot({ crossfade: true, crossfadeNext: false, fadeInDuration: 800, fadeOutDuration: 300 })
  const snapshot = store.snapshot()
  assert.equal(snapshot.crossfade, false)
  assert.equal(snapshot.crossfadeNext, true)
  assert.equal(snapshot.crossfadeInDuration, 800)
  assert.equal(snapshot.crossfadeOutDuration, 300)
  assert.doesNotMatch(settingsView, /v-model="crossfade"/)
})

await regression('Rust-normalized values are adopted when nothing changed during the save', async store => {
  await store.hydrate()
  await flushPersist()
  saveResponse = settings => ({ ...settings, ltServerUrl: settings.ltServerUrl.trim().replace(/\/+$/, '') })
  store.ltServerUrl = ' https://example.test/ '
  await flushPersist()
  assert.equal(store.ltServerUrl, 'https://example.test')
})

for (const [value, step, min, max, expected] of [
  [500.5, 100, 0, 10000, 501],
  [-12.5, 50, -5000, 5000, -12],
  [1049.4, 256, 256, 524288, 1049],
  [1.234, 0.05, 0.5, 1.6, 1.23],
  [1.7, 0.05, 0.5, 1.6, 1.6],
  [0.333, 0.01, 0, 1, 0.33],
]) {
  assert.equal(roundToStepPrecision(value, step, min, max), expected, `${value} step ${step}`)
}
cases++
console.log('ok - typed values are rounded to the step precision and clamped')

console.log(`Settings integrity regressions: ${cases} passed`)
