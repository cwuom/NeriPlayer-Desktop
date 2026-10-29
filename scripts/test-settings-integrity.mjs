import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'
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
  '@/modules/audioEffects/model': await loadAudioEffectsModel(),
  pinia,
  vue,
  '@tauri-apps/api/core': bridge,
  '@/utils/logger': { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) },
  '@/modules/desktopLyrics/style': load(await read('../src/modules/desktopLyrics/style.ts')),
  '@/modules/shortcuts/bindings': load(await read('../src/modules/shortcuts/bindings.ts'), {
    './platform': load(await read('../src/modules/shortcuts/platform.ts')),
  }),
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

await regression('old settings preserve single-click playback and the default local library order', store => {
  store.applySnapshot({})
  assert.equal(store.doubleClickPlay, false)
  assert.equal(store.localLibrarySortBy, 'default')
  assert.equal(store.localLibrarySortDescending, false)
})

await regression('library interaction preferences normalize malformed snapshots', store => {
  store.applySnapshot({ doubleClickPlay: 'true', localLibrarySortBy: 'invalid', localLibrarySortDescending: 1 })
  assert.equal(store.doubleClickPlay, false)
  assert.equal(store.localLibrarySortBy, 'default')
  assert.equal(store.localLibrarySortDescending, false)
  for (const mode of ['default', 'modified', 'title', 'artist', 'album', 'duration']) {
    store.applySnapshot({ doubleClickPlay: true, localLibrarySortBy: mode, localLibrarySortDescending: true })
    assert.equal(store.snapshot().doubleClickPlay, true)
    assert.equal(store.snapshot().localLibrarySortBy, mode)
    assert.equal(store.snapshot().localLibrarySortDescending, true)
  }
})

await regression('library interaction preferences survive hydration, IPC save and legacy reload', async store => {
  loadResult = { settings: { doubleClickPlay: true, localLibrarySortBy: 'modified', localLibrarySortDescending: true }, persisted: true }
  await store.hydrate()
  assert.equal(store.doubleClickPlay, true)
  assert.equal(store.localLibrarySortBy, 'modified')
  assert.equal(store.localLibrarySortDescending, true)
  store.localLibrarySortBy = 'title'
  await flushPersist()
  assert.equal(saved.at(-1).doubleClickPlay, true)
  assert.equal(saved.at(-1).localLibrarySortBy, 'title')
  assert.equal(saved.at(-1).localLibrarySortDescending, true)
  loadResult = { settings: {}, persisted: false }
  const reloadedInstance = pinia.createPinia()
  pinia.setActivePinia(reloadedInstance)
  try {
    const reloaded = settingsModule.useSettingsStore()
    await reloaded.hydrate()
    assert.equal(reloaded.doubleClickPlay, true)
    assert.equal(reloaded.localLibrarySortBy, 'title')
    assert.equal(reloaded.localLibrarySortDescending, true)
  } finally {
    pinia.disposePinia(reloadedInstance)
  }
})

await regression('translation scale defaults preserve the existing size and independent lyric settings', store => {
  store.applySnapshot({ lyricFontScale: 1.25, showTranslation: false, showRomanization: true })
  assert.equal(store.lyricTranslationScale, 1)
  assert.equal(store.snapshot().lyricTranslationScale, 1)
  assert.equal(store.lyricFontScale, 1.25)
  assert.equal(store.showTranslation, false)
  assert.equal(store.showRomanization, true)
  assert.equal(settingsModule.LYRIC_TRANSLATION_SCALE_DEFAULT, 1)
  assert.equal(settingsModule.LYRIC_TRANSLATION_SCALE_MIN, 0.5)
  assert.equal(settingsModule.LYRIC_TRANSLATION_SCALE_MAX, 2)
  assert.equal(settingsModule.LYRIC_TRANSLATION_SCALE_STEP, 0.05)
})

await regression('translation scale clamps numbers and defaults invalid snapshots without altering lyric size', store => {
  for (const [value, expected] of [
    [-1, 0.5], [0.5, 0.5], [1.234, 1.234], [2, 2], [3, 2],
    [NaN, 1], [Infinity, 1], [-Infinity, 1], ['1.5', 1], [null, 1],
    [undefined, 1], [true, 1], [[], 1], [{}, 1],
  ]) {
    store.applySnapshot({ lyricTranslationScale: value, lyricFontScale: 1.25 })
    assert.equal(store.lyricTranslationScale, expected, String(value))
    assert.equal(store.snapshot().lyricTranslationScale, expected, String(value))
    assert.equal(store.lyricFontScale, 1.25)
  }
})

await regression('translation scale survives hydration, IPC save, and legacy browser reload', async store => {
  loadResult = { settings: { lyricTranslationScale: 1.25, lyricFontScale: 1.1 }, persisted: true }
  await store.hydrate()
  assert.equal(store.lyricTranslationScale, 1.25)
  store.lyricTranslationScale = 1.75
  await flushPersist()
  assert.equal(saved.at(-1).lyricTranslationScale, 1.75)
  assert.equal(saved.at(-1).lyricFontScale, 1.1)
  assert.equal(JSON.parse(storage.get('neri:lyric_translation_scale')), 1.75)
  loadResult = { settings: {}, persisted: false }
  const reloadedInstance = pinia.createPinia()
  pinia.setActivePinia(reloadedInstance)
  try {
    const reloaded = settingsModule.useSettingsStore()
    await reloaded.hydrate()
    assert.equal(reloaded.lyricTranslationScale, 1.75)
    assert.equal(reloaded.snapshot().lyricTranslationScale, 1.75)
    assert.equal(saved.at(-1).lyricTranslationScale, 1.75)
  } finally {
    pinia.disposePinia(reloadedInstance)
  }
})

await regression('translation scale survives config export and import and resets with old configs', store => {
  store.applySnapshot({ lyricTranslationScale: 1.5, lyricFontScale: 1.25 })
  const exported = JSON.parse(JSON.stringify({ settings: store.snapshot() }))
  store.applySnapshot({ lyricTranslationScale: 0.75 })
  store.applySnapshot(exported.settings)
  assert.equal(store.lyricTranslationScale, 1.5)
  assert.equal(store.lyricFontScale, 1.25)
  store.applySnapshot({ lyricFontScale: 1.1 })
  assert.equal(store.lyricTranslationScale, 1)
  assert.equal(store.snapshot().lyricTranslationScale, 1)
})

await regression('old settings keep the default app and lyric fonts', store => {
  store.applySnapshot({ showTranslation: false })
  assert.equal(store.appFontFamily, '')
  assert.equal(store.lyricFontFamily, '')
  assert.equal(store.snapshot().appFontFamily, '')
  assert.equal(store.snapshot().lyricFontFamily, '')
})

await regression('font preferences remove CSS syntax and bound Unicode family names', store => {
  for (const [value, expected] of [
    ['  Noto  Sans SC  ', 'Noto Sans SC'],
    ['思源黑体.1-_ Regular', '思源黑体.1-_ Regular'],
    ['A"; color:red/*x*/', 'A colorredx'],
    ['Noto\tSans\nSC', 'NotoSansSC'],
    ['x'.repeat(80), 'x'.repeat(64)],
    ['𠮷'.repeat(80), '𠮷'.repeat(64)],
    [42, ''], [null, ''], [undefined, ''],
  ]) {
    store.applySnapshot({ appFontFamily: value, lyricFontFamily: value })
    assert.equal(store.appFontFamily, expected, String(value))
    assert.equal(store.lyricFontFamily, expected, String(value))
    assert.equal(store.snapshot().appFontFamily, expected, String(value))
    assert.equal(store.snapshot().lyricFontFamily, expected, String(value))
  }
})

await regression('app and lyric fonts survive IPC persistence and browser reload', async store => {
  loadResult = { settings: { appFontFamily: 'Noto Sans SC', lyricFontFamily: '思源宋体' }, persisted: true }
  await store.hydrate()
  assert.equal(store.appFontFamily, 'Noto Sans SC')
  assert.equal(store.lyricFontFamily, '思源宋体')
  store.appFontFamily = '  MiSans  '
  store.lyricFontFamily = '  Noto  Serif SC  '
  await flushPersist()
  assert.equal(saved.at(-1).appFontFamily, 'MiSans')
  assert.equal(saved.at(-1).lyricFontFamily, 'Noto Serif SC')
  assert.equal(JSON.parse(storage.get('neri:app_font_family')), 'MiSans')
  assert.equal(JSON.parse(storage.get('neri:lyric_font_family')), 'Noto Serif SC')
  loadResult = { settings: {}, persisted: false }
  const reloadedInstance = pinia.createPinia()
  pinia.setActivePinia(reloadedInstance)
  try {
    const reloaded = settingsModule.useSettingsStore()
    await reloaded.hydrate()
    assert.equal(reloaded.appFontFamily, 'MiSans')
    assert.equal(reloaded.lyricFontFamily, 'Noto Serif SC')
  } finally {
    pinia.disposePinia(reloadedInstance)
  }
})

await regression('font preferences survive exported JSON snapshots and config replacement', store => {
  store.applySnapshot({ appFontFamily: '思源黑体', lyricFontFamily: 'Noto Serif SC', lyricFontScale: 1.25 })
  const exported = JSON.parse(JSON.stringify({ settings: store.snapshot() }))
  assert.equal(exported.settings.appFontFamily, '思源黑体')
  assert.equal(exported.settings.lyricFontFamily, 'Noto Serif SC')
  store.applySnapshot({ appFontFamily: 'MiSans', lyricFontFamily: 'Caveat' })
  store.applySnapshot(exported.settings)
  assert.equal(store.appFontFamily, '思源黑体')
  assert.equal(store.lyricFontFamily, 'Noto Serif SC')
  assert.equal(store.lyricFontScale, 1.25)
  store.applySnapshot({ lyricFontScale: 1 })
  assert.equal(store.appFontFamily, '')
  assert.equal(store.lyricFontFamily, '')
})

await regression('old settings receive the easier long-note threshold', store => {
  store.applySnapshot({ showTranslation: false })
  assert.equal(store.lyricEmphasisThresholdMs, 700)
  assert.equal(store.snapshot().lyricEmphasisThresholdMs, 700)
})

await regression('long-note thresholds reject invalid values and align to the supported range', store => {
  for (const [value, expected] of [
    [-1, 300], [299, 300], [300, 300], [324.6, 350], [325, 350], [724.4, 700], [724.6, 750], [725, 750],
    [2000, 2000], [2001, 2000], [NaN, 700], [Infinity, 700], ['1000', 700],
  ]) {
    store.applySnapshot({ lyricEmphasisThresholdMs: value })
    assert.equal(store.lyricEmphasisThresholdMs, expected, String(value))
    assert.equal(store.snapshot().lyricEmphasisThresholdMs, expected, String(value))
  }
})

await regression('long-note preferences survive hydration, IPC persistence, and browser reload', async store => {
  loadResult = { settings: { lyricEmphasisThresholdMs: 1050 }, persisted: true }
  await store.hydrate()
  assert.equal(store.lyricEmphasisThresholdMs, 1050)
  store.lyricEmphasisThresholdMs = 1450
  await flushPersist()
  assert.equal(saved.at(-1).lyricEmphasisThresholdMs, 1450)
  assert.equal(JSON.parse(storage.get('neri:lyric_emphasis_threshold_ms')), 1450)
  store.applySnapshot(saved.at(-1))
  assert.equal(store.lyricEmphasisThresholdMs, 1450)
  loadResult = { settings: {}, persisted: false }
  const reloadedInstance = pinia.createPinia()
  pinia.setActivePinia(reloadedInstance)
  try {
    const reloaded = settingsModule.useSettingsStore()
    await reloaded.hydrate()
    assert.equal(reloaded.lyricEmphasisThresholdMs, 1450)
  } finally {
    pinia.disposePinia(reloadedInstance)
  }
})

await regression('desktop lyric width survives hydration, IPC persistence, reload, and exported snapshots', async store => {
  loadResult = { settings: { desktopLyrics: { fontSize: 40, maxWidth: 600 } }, persisted: true }
  await store.hydrate()
  assert.equal(store.desktopLyrics.maxWidth, 600)
  store.desktopLyrics = { ...store.desktopLyrics, maxWidth: 1209 }
  await flushPersist()
  assert.equal(saved.at(-1).desktopLyrics.maxWidth, 1200)
  assert.equal(JSON.parse(storage.get('neri:desktop_lyrics')).maxWidth, 1200)
  const exported = JSON.parse(JSON.stringify(store.snapshot()))
  store.applySnapshot({ desktopLyrics: { maxWidth: 360 } })
  store.applySnapshot(exported)
  assert.equal(store.desktopLyrics.maxWidth, 1200)
  loadResult = { settings: {}, persisted: false }
  const reloadedInstance = pinia.createPinia()
  pinia.setActivePinia(reloadedInstance)
  try {
    const reloaded = settingsModule.useSettingsStore()
    await reloaded.hydrate()
    assert.equal(reloaded.desktopLyrics.maxWidth, 1200)
  } finally {
    pinia.disposePinia(reloadedInstance)
  }
  store.applySnapshot({ desktopLyrics: { fontSize: 40 } })
  assert.equal(store.desktopLyrics.maxWidth, 860, '旧配置导入时补上默认最大宽度')
})

await regression('the menu bar lyric preference defaults off and survives persistence', async store => {
  await store.hydrate()
  assert.equal(store.showMenuBarLyrics, false)
  store.showMenuBarLyrics = true
  await flushPersist()
  assert.equal(saved.at(-1).showMenuBarLyrics, true)
  store.applySnapshot(saved.at(-1))
  assert.equal(store.showMenuBarLyrics, true)
  store.applySnapshot({ showMenuBarLyrics: 'true' })
  assert.equal(store.showMenuBarLyrics, false)
})

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
