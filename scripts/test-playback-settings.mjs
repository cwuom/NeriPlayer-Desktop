import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'
import { loadAudioEffectsModel } from './audio-effects-model-loader.mjs'

const audioEffectsModel = await loadAudioEffectsModel()

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
const desktopLyricsStyle = {}
new Function('exports', ts.transpileModule(await readFile(new URL('../src/modules/desktopLyrics/style.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(desktopLyricsStyle)
const desktopPlatform = {}
new Function('exports', ts.transpileModule(await readFile(new URL('../src/utils/platform.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(desktopPlatform)
const shortcutPlatform = {}
new Function('require', 'exports', ts.transpileModule(await readFile(new URL('../src/modules/shortcuts/platform.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(() => desktopPlatform, shortcutPlatform)
const lyricSourcePreference = {}
new Function('exports', ts.transpileModule(await readFile(new URL('../src/modules/lyrics/lyricSourcePreference.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(lyricSourcePreference)
const uiScaleRange = {}
new Function('exports', ts.transpileModule(await readFile(new URL('../src/modules/settings/uiScaleRange.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(uiScaleRange)
const textScale = {}
new Function('exports', ts.transpileModule(await readFile(new URL('../src/modules/settings/textScale.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(textScale)
const cleanScheme = {}
new Function('exports', ts.transpileModule(await readFile(new URL('../src/utils/theme/cleanScheme.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(cleanScheme)
const shortcutBindings = {}
new Function('require', 'exports', ts.transpileModule(await readFile(new URL('../src/modules/shortcuts/bindings.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(() => shortcutPlatform, shortcutBindings)
const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === 'pinia') return pinia
  if (name === 'vue') return vue
  if (name === '@tauri-apps/api/core') return bridge
  if (name === '@/utils/logger') return { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) }
  if (name === '@/modules/desktopLyrics/style') return desktopLyricsStyle
  if (name === '@/modules/shortcuts/bindings') return shortcutBindings
  if (name === '@/modules/audioEffects/model') return audioEffectsModel
  if (name === '@/modules/lyrics/lyricSourcePreference') return lyricSourcePreference
  if (name === '@/modules/settings/uiScaleRange') return uiScaleRange
  if (name === '@/modules/settings/textScale') return textScale
  if (name === '@/utils/theme/cleanScheme') return cleanScheme
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

await regression('channel balance is clamped and rounded like Android, and centers on bad input', store => {
  for (const [value, expected] of [
    [0.354, 0.35],
    [-0.35, -0.35],
    [1.7, 1],
    [-3, -1],
    [-0.004, 0],
    [Number.NaN, 0],
    ['0.5', 0],
    [null, 0],
  ]) {
    store.applySnapshot({ volumeBalance: value })
    assert.equal(store.volumeBalance, expected, String(value))
    assert.ok(!Object.is(store.volumeBalance, -0), `${value} must not round to -0`)
  }
  assert.equal(exports.normalizeVolumeBalance(Number.POSITIVE_INFINITY), 0)
})

await regression('legacy settings restore neutral pitch independently from speed', async store => {
  loadResult = { persisted: true, settings: { playbackSpeed: 0.85 } }
  await store.hydrate()
  assert.equal(store.playbackSpeed, 0.85)
  assert.equal(store.playbackPitch, 1)
  assert.equal(store.snapshot().playbackPitch, 1)
})

await regression('speed and pitch follow Android ranges and 0.05 steps', store => {
  for (const [speed, pitch, expectedSpeed, expectedPitch] of [
    [0.1, 0.25, 0.1, 0.25], [4, 2, 4, 2],
    [0.01, 0.01, 0.1, 0.25], [8, 8, 4, 2],
    [1.234, 0.876, 1.25, 0.9], [Number.NaN, Number.POSITIVE_INFINITY, 1, 1],
    ['2', '0.5', 1, 1],
  ]) {
    store.applySnapshot({ playbackSpeed: speed, playbackPitch: pitch })
    assert.equal(store.playbackSpeed, expectedSpeed)
    assert.equal(store.playbackPitch, expectedPitch)
  }
})

await regression('independent speed and pitch persist and survive snapshot import', async store => {
  await store.hydrate()
  saved.length = 0
  timers.clear()
  store.playbackSpeed = 1.5
  store.playbackPitch = 0.75
  await vue.nextTick()
  for (const callback of timers.values()) callback()
  timers.clear()
  await Promise.resolve()
  assert.equal(saved.at(-1).playbackSpeed, 1.5)
  assert.equal(saved.at(-1).playbackPitch, 0.75)
  assert.equal(storage.get('neri:playback_pitch'), '0.75')
  store.applySnapshot(saved.at(-1))
  assert.equal(store.playbackSpeed, 1.5)
  assert.equal(store.playbackPitch, 0.75)
})

await regression('vinyl effect defaults off and persists independently from manual pitch', async store => {
  await store.hydrate()
  assert.equal(store.vinylEffectEnabled, false)
  store.vinylEffectEnabled = true
  store.playbackSpeed = 0.95
  store.playbackPitch = 1.25
  await vue.nextTick()
  for (const callback of timers.values()) callback()
  timers.clear()
  await Promise.resolve()
  const snapshot = saved.at(-1)
  assert.equal(snapshot.vinylEffectEnabled, true)
  assert.equal(snapshot.playbackSpeed, 0.95)
  assert.equal(snapshot.playbackPitch, 1.25)
  store.applySnapshot(snapshot)
  assert.equal(store.vinylEffectEnabled, true)
})

await regression('full DSP defaults and imported values remain bounded and independently persisted', async store => {
  await store.hydrate()
  assert.equal(store.audioEffects.main.enabled, false)
  assert.equal(store.audioEffects.main.sound.equalizerBandsDb.length, 10)
  store.applySnapshot({ audioEffects: { main: { enabled: true, presetId: 'custom', sound: {
    preampDb: -24, bassDb: 8, parametricEnabled: true,
    parametricBands: [{ frequencyHz: 50, gainDb: 24, q: 20 }],
  } }, perOutputEnabled: true, outputProfiles: {
    Headphones: { enabled: true, sound: { stereoWidth: 1.4 } },
  }, userPresets: [{ id: 'user-one', name: '耳机配置', sound: { bassDb: 3 } }] } })
  const snapshot = store.snapshot()
  assert.equal(snapshot.audioEffects.main.sound.preampDb, -24)
  assert.equal(snapshot.audioEffects.main.sound.parametricBands[0].q, 20)
  assert.equal(snapshot.audioEffects.outputProfiles.Headphones.sound.stereoWidth, 1.4)
  assert.equal(snapshot.audioEffects.userPresets[0].name, '耳机配置')
  snapshot.audioEffects.main.sound.equalizerBandsDb[0] = 12
  assert.equal(store.audioEffects.main.sound.equalizerBandsDb[0], 0, 'snapshot cannot mutate live DSP')
})

await regression('legacy EQ migrates once without stacking native and legacy filters', store => {
  store.applySnapshot({ equalizerEnabled: true, equalizerPresetId: 'bass_boost', equalizerBands: [700, 500, 200, -100, -300] })
  assert.equal(store.equalizerEnabled, false)
  assert.equal(store.audioEffects.main.enabled, true)
  assert.equal(store.audioEffects.main.sound.parametricEnabled, true)
  assert.deepEqual(store.audioEffects.main.sound.parametricBands.map(band => band.gainDb), [0, -2, -5, -8, -10])
  const saved = store.snapshot()
  saved.audioEffects.main.enabled = false
  saved.equalizerEnabled = true
  store.applySnapshot(saved)
  assert.equal(store.audioEffects.main.enabled, false, 'explicitly disabled new profile stays off')
  assert.equal(store.equalizerEnabled, false)
})

console.log(`Playback settings regressions: ${cases} passed`)
