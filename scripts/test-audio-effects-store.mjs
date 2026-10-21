import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import { ref, reactive, computed, watch, nextTick, effectScope } from 'vue'
import { loadAudioEffectsModel, loadListeningTuning } from './audio-effects-model-loader.mjs'
const model = await loadAudioEffectsModel()
const listening = await loadListeningTuning()

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
const playerSource = await read('../src/stores/player.ts')
const settingsSource = await read('../src/stores/settings.ts')
const parse = (name, source) => ts.createSourceFile(name, source, ts.ScriptTarget.ES2022, true)
const playerTree = parse('player.ts', playerSource)
const settingsTree = parse('settings.ts', settingsSource)
const names = new Set(['setSpeed', 'setPitch', 'effectivePlaybackSpeed', 'setListenTogetherSyncPlaybackRate',
  'setVinylEffect', 'effectivePlaybackPitch', 'resetAudioEffects', 'setLoudnessGain', 'setEqualizer', 'setEqualizerPreset', 'applyPersistedSettings',
  'applyNativeAudioEffects', 'scheduleNativeAudioEffects', 'applyPlaybackParameter', 'updateAudioEffects', 'updateAudioEffectsProfile', 'updateAudioEffectsSound',
  'setAudioEffectsPreset', 'setListeningTuning', 'resetListeningTuning', 'resetAudioEffectsSection', 'saveAudioEffectsPreset', 'deleteAudioEffectsPreset',
  'exportAudioEffectsPreset', 'importAudioEffectsPreset', 'mergeImportedPresets', 'importedPresetId', 'androidRouteLabel',
  'importAutoEq', 'refreshAudioEffectsStats'])
const fragments = []
function collect(node) {
  if (ts.isFunctionDeclaration(node) && names.has(node.name?.text)) fragments.push(node.getText(playerTree))
  if (ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
    && node.expression.expression.getText(playerTree) === 'watch'
    && /settings\.(playbackSpeed|playbackPitch|vinylEffectEnabled|loudnessGainMb|equalizerEnabled|audioEffects|audioOutputDevice)/.test(node.expression.arguments[0]?.getText(playerTree))) {
    fragments.push(node.getText(playerTree))
  }
  ts.forEachChild(node, collect)
}
collect(playerTree)
const normalizers = settingsTree.statements.filter(node => ts.isFunctionDeclaration(node)
  && ['normalizePlaybackSpeed', 'normalizePlaybackPitch', 'normalizePlaybackParameter'].includes(node.name?.text))
  .map(node => node.getText(settingsTree)).join('\n').replaceAll('export ', '')
const presets = playerTree.statements.find(node => ts.isVariableStatement(node)
  && node.declarationList.declarations.some(declaration => declaration.name.getText(playerTree) === 'EQ_PRESETS'))
const compile = code => ts.transpileModule(code, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const presetModule = {}
new Function('exports', compile(presets.getText(playerTree)))(presetModule)
const compiled = compile(`${normalizers}\n${fragments.join('\n')}`)

function runtime(options = {}) {
  const settings = reactive({ volume: 0.7, playbackSpeed: 1, playbackPitch: 1, loudnessGainMb: 0,
    audioEffects: model.normalizeAudioEffectsSettings(undefined),
    equalizerEnabled: false, equalizerBands: [0, 0, 0, 0, 0], equalizerPresetId: 'flat',
    vinylEffectEnabled: false, normalizeVolume: true, volumeBalance: 0.35, multichannelDrc: false, audioOutputDevice: '' })
  const state = {
    audioEffectsOutputDevice: ref('Headphones'),
    settings, playbackSpeed: ref(1), playbackPitch: ref(1), vinylEffectEnabled: ref(false), loudnessGainMb: ref(0),
    equalizerEnabled: ref(false), equalizerBands: ref([0, 0, 0, 0, 0]), equalizerPresetId: ref('flat'), volume: ref(0.7),
  }
  state.audioEffects = computed(() => settings.audioEffects)
  state.audioEffectsProfile = computed(() => model.audioEffectsProfileFor(settings.audioEffects, state.audioEffectsOutputDevice.value))
  const calls = [], timers = new Map()
  let timerSequence = 0
  const context = {
    ...state, ...model, ...listening, resetListeningTuningSound: listening.resetListeningTuning,
    parseAutoEq: model.importAutoEq, ref, computed, watch, EQ_PRESETS: presetModule.EQ_PRESETS,
    i18n: { global: { t: key => key } }, crypto,
    invoke: async (command, args) => { calls.push([command, args]); return options.invoke?.(command, args) },
    setTimeout: callback => { const id = ++timerSequence; timers.set(id, callback); return id },
    clearTimeout: id => timers.delete(id),
    currentRenderedPosition: () => 12_000,
    performance: { now: () => 100 }, interpolatedPositionMs: ref(12_000), lastSeekCommand: ref({ seq: 0 }),
    playbackRequestToken: 1, log: { warn() {} },
  }
  const scope = effectScope()
  const api = scope.run(() => new Function(...Object.keys(context), `
    let _speedInvokeTimer = null, _pitchInvokeTimer = null, listenTogetherSyncRateMultiplier = null;
    let audioEffectsTimer = null, audioEffectsRevision = 0, audioEffectsApplyChain = Promise.resolve();
    let audioEffectsStatsRevision = 0, vinylEffectRevision = 0, vinylEffectApplyChain = Promise.resolve();
    let _interpSpeed = 1, _interpIsPlaying = true, _interpAnchorMs = 12000, _interpAnchorTime = 100, _interpRenderedMs = 12000;
    ${compiled}
    return { setSpeed, setPitch, setVinylEffect, effectivePlaybackSpeed, effectivePlaybackPitch, setListenTogetherSyncPlaybackRate,
      applyNativeAudioEffects, updateAudioEffects, updateAudioEffectsProfile, updateAudioEffectsSound,
      setAudioEffectsPreset, setListeningTuning, resetListeningTuning, resetAudioEffectsSection, saveAudioEffectsPreset, deleteAudioEffectsPreset,
      exportAudioEffectsPreset, importAudioEffectsPreset, importAutoEq, refreshAudioEffectsStats,
      resetAudioEffects, setLoudnessGain, setEqualizerPreset, applyPersistedSettings, interpolationSpeed: () => _interpSpeed,
      seekSequence: () => lastSeekCommand.value.seq, anchor: () => [_interpAnchorMs, _interpAnchorTime, _interpRenderedMs] };
  `)(...Object.values(context)))
  return { ...state, ...api, calls, dispose: () => scope.stop(), async flush() {
    await nextTick()
    for (const [id, callback] of [...timers]) { timers.delete(id); callback() }
    for (let i = 0; i < 12; i++) await Promise.resolve()
  } }
}

let passed = 0
async function regression(name, run, options) {
  const r = runtime(options)
  try { await run(r); passed++; console.log(`ok - ${name}`) }
  finally { r.dispose() }
}

await regression('changing pitch leaves speed, lyric clock and seek anchor unchanged', async r => {
  await r.setSpeed(1.5)
  await r.flush()
  r.calls.length = 0
  const anchor = r.anchor(), seekSequence = r.seekSequence()
  await r.setPitch(0.754)
  await r.flush()
  assert.equal(r.playbackPitch.value, 0.75)
  assert.equal(r.settings.playbackPitch, 0.75)
  assert.equal(r.playbackSpeed.value, 1.5)
  assert.equal(r.interpolationSpeed(), 1.5)
  assert.deepEqual(r.anchor(), anchor)
  assert.equal(r.seekSequence(), seekSequence)
  assert.deepEqual(r.calls, [['set_pitch', { pitch: 0.75 }]])
})

await regression('speed spans Android range independently from pitch', async r => {
  await r.setPitch(2)
  await r.setSpeed(0.1)
  await r.flush()
  assert.equal(r.playbackSpeed.value, 0.1)
  assert.equal(r.playbackPitch.value, 2)
  assert.equal(r.interpolationSpeed(), 0.1)
  await r.setSpeed(4)
  await r.flush()
  assert.equal(r.calls.at(-1)[1].speed, 4)
})

await regression('imported sound settings update the live DSP state', async r => {
  Object.assign(r.settings, { playbackSpeed: 1.7, playbackPitch: 0.65, loudnessGainMb: 450,
    equalizerEnabled: true, equalizerBands: [300, 200, 100, 200, 300], equalizerPresetId: 'acoustic' })
  await r.flush()
  assert.equal(r.playbackSpeed.value, 1.7)
  assert.equal(r.playbackPitch.value, 0.65)
  assert.equal(r.loudnessGainMb.value, 450)
  assert.equal(r.equalizerPresetId.value, 'acoustic')
  assert.deepEqual(r.equalizerBands.value, [300, 200, 100, 200, 300])
  assert.ok(r.calls.some(([command, args]) => command === 'set_pitch' && args.pitch === 0.65))
})

await regression('reset clears queued edits while retaining temporary sync rate and output preferences', async r => {
  r.setListenTogetherSyncPlaybackRate(0.96)
  await r.setSpeed(1.8)
  await r.setPitch(0.7)
  await r.resetAudioEffects()
  await r.flush()
  assert.equal(r.playbackSpeed.value, 1)
  assert.equal(r.playbackPitch.value, 1)
  assert.equal(r.effectivePlaybackSpeed(), 0.96)
  assert.equal(r.interpolationSpeed(), 0.96)
  assert.equal(r.settings.normalizeVolume, true)
  assert.equal(r.settings.volumeBalance, 0.35)
  assert.ok(!r.calls.some(([command]) => command === 'set_pitch'), 'old pitch edit cannot reapply after reset')
  assert.ok(r.calls.some(([command, args], index) => command === 'set_speed' && args.speed === 0.96
    && index > r.calls.findIndex(([command]) => command === 'reset_audio_effects')))
})

await regression('startup reapplies saved pitch and speed through separate IPC commands', async r => {
  r.settings.playbackPitch = 0.8
  r.settings.playbackSpeed = 1.25
  await r.applyPersistedSettings()
  await r.flush()
  assert.ok(r.calls.some(([command, args]) => command === 'set_pitch' && args.pitch === 0.8))
  assert.ok(r.calls.some(([command, args]) => command === 'set_speed' && args.speed === 1.25))
})

await regression('Android EQ presets use the referenced frequency gains', async r => {
  await r.setEqualizerPreset('acoustic')
  assert.deepEqual(r.equalizerBands.value, [300, 200, 100, 200, 300])
  await r.setEqualizerPreset('club')
  assert.deepEqual(r.equalizerBands.value, [500, 300, 0, 300, 400])
  await r.setEqualizerPreset('folk')
  assert.deepEqual(r.equalizerBands.value, [100, 300, 0, 400, 500])
})

await regression('selecting custom retains manual bands and unknown presets are harmless', async r => {
  r.equalizerBands.value = [100, -200, 300, -400, 500]
  await r.setEqualizerPreset('custom')
  assert.deepEqual(r.equalizerBands.value, [100, -200, 300, -400, 500])
  await r.setEqualizerPreset('__proto__')
  assert.equal(r.equalizerPresetId.value, 'flat')
  assert.deepEqual(r.equalizerBands.value, [0, 0, 0, 0, 0])
})

await regression('vinyl mode links fine speeds while preserving the independent pitch', async r => {
  await r.setPitch(1.25)
  await r.setSpeed(0.95)
  await r.setVinylEffect(true)
  await r.flush()
  assert.equal(r.effectivePlaybackPitch(), 0.95)
  assert.equal(r.playbackPitch.value, 1.25)
  assert.equal(r.settings.playbackPitch, 1.25)
  await r.setSpeed(1.05)
  await r.flush()
  assert.equal(r.effectivePlaybackPitch(), 1.05)
  await r.setSpeed(4)
  await r.flush()
  assert.equal(r.effectivePlaybackPitch(), 4, 'vinyl must span the entire speed range')
  await r.setVinylEffect(false)
  await r.flush()
  assert.equal(r.effectivePlaybackPitch(), 1.25)
  assert.ok(r.calls.some(([command, args]) => command === 'set_vinyl_effect' && args.enabled))
})

await regression('native edits coalesce and preserve A/B bypass, presets enable the effect', async r => {
  r.updateAudioEffectsSound({ preampDb: -20 })
  r.updateAudioEffectsSound({ bassDb: 6 })
  assert.equal(r.audioEffectsProfile.value.enabled, false)
  await r.flush()
  const native = r.calls.filter(([command]) => command === 'set_audio_effects')
  assert.equal(native.length, 1)
  assert.equal(native[0][1].settings.main.sound.preampDb, -20)
  assert.equal(native[0][1].settings.main.sound.bassDb, 6)
  assert.equal(native[0][1].outputDevice, 'Headphones')
  r.setAudioEffectsPreset('deep_bass')
  assert.equal(r.audioEffectsProfile.value.enabled, true)
  assert.equal(r.audioEffectsProfile.value.sound.bassDb, 8)
})

await regression('device profiles follow actual output and edit that device only', async r => {
  r.updateAudioEffects({ perOutputEnabled: true })
  r.updateAudioEffectsSound({ bassDb: 4 })
  r.audioEffectsOutputDevice.value = 'Speakers'
  assert.equal(r.audioEffectsProfile.value.sound.bassDb, 0)
  r.updateAudioEffectsSound({ bassDb: -2 })
  r.audioEffectsOutputDevice.value = 'Headphones'
  assert.equal(r.audioEffectsProfile.value.sound.bassDb, 4)
  assert.equal(r.settings.audioEffects.main.sound.bassDb, 0)
})

await regression('AutoEQ enables full correction and saved presets survive global reset', async r => {
  r.updateAudioEffectsSound({ reverb: 0.4, equalizerEnabled: false })
  const result = r.importAutoEq('Preamp: -20 dB\nFilter 1: ON PK Fc 1000 Hz Gain -4 dB Q 2')
  assert.equal(result.imported, 1)
  assert.equal(r.audioEffectsProfile.value.enabled, true)
  assert.equal(r.audioEffectsProfile.value.sound.equalizerEnabled, true)
  assert.equal(r.audioEffectsProfile.value.sound.preampDb, -20)
  assert.equal(r.audioEffectsProfile.value.sound.reverb, 0.4)
  const id = r.saveAudioEffectsPreset('Headphone correction')
  const text = r.exportAudioEffectsPreset(id)
  r.resetAudioEffectsSection('space')
  assert.equal(r.audioEffectsProfile.value.sound.reverb, 0)
  assert.equal(r.audioEffectsProfile.value.sound.preampDb, -20)
  await r.resetAudioEffects()
  await r.flush()
  assert.equal(r.settings.audioEffects.userPresets[0].id, id)
  assert.equal(r.audioEffectsProfile.value.enabled, false)
  const summary = r.importAudioEffectsPreset(text)
  assert.equal(r.audioEffectsProfile.value.sound.preampDb, -20)
  assert.equal(r.audioEffectsProfile.value.presetId, id, 'an identical saved preset is reselected instead of duplicated')
  assert.equal(summary.addedPresets, 0)
  assert.equal(r.settings.audioEffects.userPresets.length, 1)
  const before = JSON.stringify(r.settings.audioEffects)
  assert.throws(() => r.importAutoEq('not a filter'))
  assert.equal(JSON.stringify(r.settings.audioEffects), before)
})

await regression('Android config backup imports the main sound, user presets, route profiles and speaker settings', async r => {
  const android = {
    version: 2, perOutputEnabled: true, powerMode: 'high', autoHeadroom: false,
    main: { enabled: true, presetId: 'user_phone', sound: { bassDb: 3, vocal: 0.4 } },
    outputProfiles: { wired: { enabled: true, presetId: 'flat', sound: { trebleDb: -2 } }, bogus: { sound: {} } },
    userPresets: [{ id: 'user_phone', name: '手机预设', sound: { bassDb: 3, vocal: 0.4 } }],
    speaker: { enabled: true, size: 'tablet' }, speedPitchLinked: true, storedSpeed: 1.25,
  }
  const backup = JSON.stringify({ kind: 'moe.ouom.neriplayer.config', formatVersion: 1,
    settings: { strings: { audio_effects_settings: JSON.stringify(android), cookie_token: 'secret' } },
    neteaseAuth: { cookies: { MUSIC_U: 'secret' } } })
  const summary = r.importAudioEffectsPreset(backup)
  assert.equal(summary.source, 'android_backup')
  assert.equal(summary.routes, 1, 'unknown route keys are dropped like Android does')
  assert.equal(summary.addedPresets, 2)
  const presets = r.settings.audioEffects.userPresets
  assert.deepEqual(presets.map(preset => preset.name), ['手机预设', 'dsp.import.route_preset'])
  assert.equal(presets[1].sound.trebleDb, -2)
  assert.equal(r.audioEffectsProfile.value.enabled, true)
  assert.equal(r.audioEffectsProfile.value.presetId, presets[0].id)
  assert.equal(r.audioEffectsProfile.value.sound.vocal, 0.4)
  assert.equal(r.settings.audioEffects.speaker.size, 'tablet')
  assert.equal(r.settings.audioEffects.powerMode, 'high')
  assert.equal(r.settings.audioEffects.perOutputEnabled, false, 'phone routes never become desktop device profiles')
  assert.equal(JSON.stringify(r.settings.audioEffects).includes('secret'), false)
  r.importAudioEffectsPreset(backup)
  assert.equal(r.settings.audioEffects.userPresets.length, 2, 'importing the same backup twice adds nothing')
  assert.throws(() => r.importAudioEffectsPreset(JSON.stringify({ kind: 'moe.ouom.neriplayer.config', settings: { strings: {} } })), /没有音效设置/)
})

await regression('runtime statistics reflect physical output identity', async r => {
  await r.refreshAudioEffectsStats()
  assert.equal(r.audioEffectsOutputDevice.value, 'USB DAC')
}, { invoke: async command => command === 'get_audio_effects_stats' ? { outputDevice: 'USB DAC' } : undefined })

let releaseNative
const nativeGate = new Promise(resolve => { releaseNative = resolve })
let nativeRequests = 0
await regression('in-flight native edit completes before reset, queued old edits are skipped', async r => {
  r.updateAudioEffectsSound({ bassDb: 8 })
  await r.flush()
  r.updateAudioEffectsSound({ bassDb: 10 })
  await r.flush()
  const reset = r.resetAudioEffects()
  await r.flush()
  assert.equal(r.calls.filter(([command]) => command === 'set_audio_effects').length, 1)
  releaseNative()
  await reset
  await r.flush()
  const native = r.calls.filter(([command]) => command === 'set_audio_effects')
  assert.equal(native.at(-1)[1].settings.main.sound.bassDb, 0)
  const resetIndex = r.calls.findIndex(([command]) => command === 'reset_audio_effects')
  assert.ok(resetIndex > r.calls.findIndex(([command]) => command === 'set_audio_effects'))
}, { invoke: async command => { if (command === 'set_audio_effects' && ++nativeRequests === 1) await nativeGate } })

let releaseStats
const statsGate = new Promise(resolve => { releaseStats = resolve })
let statsRequests = 0
await regression('stale statistics cannot restore the previous output device', async r => {
  const older = r.refreshAudioEffectsStats()
  await r.refreshAudioEffectsStats()
  assert.equal(r.audioEffectsOutputDevice.value, 'New DAC')
  releaseStats()
  await older
  assert.equal(r.audioEffectsOutputDevice.value, 'New DAC')
}, { invoke: async command => {
  if (command !== 'get_audio_effects_stats') return
  if (++statsRequests === 1) { await statsGate; return { outputDevice: 'Old DAC' } }
  return { outputDevice: 'New DAC' }
} })

let releaseVinyl
const vinylGate = new Promise(resolve => { releaseVinyl = resolve })
let vinylRequests = 0
await regression('rapid vinyl toggles finish in the latest state', async r => {
  const older = r.setVinylEffect(true)
  await r.flush()
  const newer = r.setVinylEffect(false)
  await r.flush()
  assert.equal(r.calls.filter(([command]) => command === 'set_vinyl_effect').length, 1)
  releaseVinyl()
  await Promise.all([older, newer])
  const toggles = r.calls.filter(([command]) => command === 'set_vinyl_effect')
  assert.equal(toggles.at(-1)[1].enabled, false)
  assert.equal(r.vinylEffectEnabled.value, false)
}, { invoke: async command => { if (command === 'set_vinyl_effect' && ++vinylRequests === 1) await vinylGate } })

let releaseReset
const resetGate = new Promise(resolve => { releaseReset = resolve })
await regression('edits during an in-flight reset apply after it finishes', async r => {
  const reset = r.resetAudioEffects()
  await r.flush()
  await r.setPitch(1.3)
  await r.setSpeed(1.05)
  const gain = r.setLoudnessGain(650)
  const vinyl = r.setVinylEffect(true)
  await r.flush()
  assert.equal(r.calls.filter(([command]) => command === 'set_vinyl_effect').length, 0)
  releaseReset()
  await Promise.all([reset, vinyl, gain])
  await r.flush()
  assert.equal(r.calls.filter(([command]) => command === 'set_pitch').at(-1)[1].pitch, 1.3)
  assert.equal(r.calls.filter(([command]) => command === 'set_speed').at(-1)[1].speed, 1.05)
  assert.equal(r.calls.filter(([command]) => command === 'set_vinyl_effect').at(-1)[1].enabled, true)
  assert.equal(r.calls.filter(([command]) => command === 'set_loudness_gain').at(-1)[1].gainMb, 650)
}, { invoke: async command => { if (command === 'reset_audio_effects') await resetGate } })

await regression('beginner tuning persists the actual sound with device isolation and headphone correction intact', async r => {
  r.updateAudioEffects({ perOutputEnabled: true, autoHeadroom: false })
  r.importAutoEq('Preamp: -20 dB\nFilter 1: ON PK Fc 1000 Hz Gain -4 dB Q 2')
  r.updateAudioEffectsSound({ reverb: 0.4, limiterEnabled: false })
  r.updateAudioEffectsProfile({ enabled: false })
  const correction = r.audioEffectsProfile.value.sound.parametricBands
  r.setListeningTuning('bass', 50)
  assert.equal(r.audioEffectsProfile.value.enabled, true)
  assert.equal(r.settings.audioEffects.autoHeadroom, true)
  assert.equal(r.audioEffectsProfile.value.sound.limiterEnabled, true)
  assert.equal(r.audioEffectsProfile.value.sound.reverb, 0.4)
  assert.deepEqual(r.audioEffectsProfile.value.sound.parametricBands, correction)
  assert.equal(r.audioEffectsProfile.value.sound.preampDb, -20)
  assert.ok(Math.abs(r.audioEffectsProfile.value.sound.bassDb - 4.2) < 1e-6)
  const before = JSON.stringify(r.settings.audioEffects)
  r.setListeningTuning('__proto__', 50)
  r.setListeningTuning('bass', NaN)
  assert.equal(JSON.stringify(r.settings.audioEffects), before)
  await r.flush()
  assert.ok(Math.abs(r.calls.filter(([command]) => command === 'set_audio_effects').at(-1)[1].settings.outputProfiles.Headphones.sound.bassDb - 4.2) < 1e-6)
  r.audioEffectsOutputDevice.value = 'Speakers'
  assert.equal(r.audioEffectsProfile.value.sound.bassDb, 0)
  r.audioEffectsOutputDevice.value = 'Headphones'
  r.resetListeningTuning()
  assert.equal(r.audioEffectsProfile.value.sound.bassDb, 0)
  assert.equal(r.audioEffectsProfile.value.sound.reverb, 0.4)
  assert.equal(r.playbackSpeed.value, 1)
})

console.log(`Audio effects store regressions: ${passed} passed`)
