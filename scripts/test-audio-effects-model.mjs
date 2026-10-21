import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/audioEffects/model.ts', import.meta.url), 'utf8')
  .catch(error => { if (error.code === 'ENOENT') return ''; throw error })
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
})
const m = {}
new Function('exports', `${outputText}\n//# sourceURL=audio-effects-model.js`)(m)
assert.equal(typeof m.normalizeAudioEffectsSound, 'function', 'audio effects model must normalize real imported settings')

let passed = 0
function regression(name, run) {
  run()
  passed++
  console.log(`ok - ${name}`)
}
const sound = changes => m.normalizeAudioEffectsSound(changes)
const profile = changes => m.normalizeAudioEffectsProfile({ enabled: true, presetId: 'custom', sound: changes })

regression('invalid numeric controls fall back safely while finite imports retain the full native ranges', () => {
  const normalized = sound({ preampDb: -20, bassDb: Infinity, outputGainDb: NaN, reverb: -2,
    equalizerBandsDb: [99, -99, '6', NaN], parametricBands: [{ frequencyHz: Infinity, gainDb: 24, q: 20 }],
    mono: 'true', toneEnabled: false, accountToken: 'not audio data' })
  assert.equal(normalized.preampDb, -20)
  assert.equal(normalized.bassDb, 0)
  assert.equal(normalized.outputGainDb, 0)
  assert.equal(normalized.reverb, 0)
  assert.deepEqual(normalized.equalizerBandsDb, [12, -12, 0, 0, 0, 0, 0, 0, 0, 0])
  assert.equal(normalized.parametricBands[0].frequencyHz, 1000)
  assert.equal(normalized.parametricBands[0].gainDb, 24)
  assert.equal(normalized.parametricBands[0].q, 20)
  assert.equal(normalized.mono, false)
  assert.equal(normalized.toneEnabled, false)
  assert.equal(Object.hasOwn(normalized, 'accountToken'), false)
})

regression('normalization copies nested arrays and supplies independent default state', () => {
  const input = { equalizerBandsDb: [2], parametricBands: [{ gainDb: 3 }] }
  const first = sound(input), second = sound(undefined)
  first.equalizerBandsDb[0] = 9
  first.parametricBands[0].gainDb = 8
  assert.equal(input.equalizerBandsDb[0], 2)
  assert.equal(input.parametricBands[0].gainDb, 3)
  assert.equal(second.equalizerBandsDb[0], 0)
  assert.equal(m.DEFAULT_AUDIO_EFFECTS_SOUND.equalizerBandsDb[0], 0)
})

regression('all numeric editor controls use their declared safety bounds', () => {
  for (const control of m.SOUND_CONTROLS) {
    const lower = sound({ [control.key]: -Number.MAX_VALUE })[control.key]
    const upper = sound({ [control.key]: Number.MAX_VALUE })[control.key]
    assert.equal(lower, control.min, control.key)
    assert.equal(upper, control.max, control.key)
    assert.equal(sound({ [control.key]: NaN })[control.key], control.defaultValue, control.key)
  }
  assert.equal(m.SOUND_CONTROLS.find(control => control.key === 'stereoWidth').unit, 'ratio')
})

regression('builtin presets retain headphone correction and apply their actual sound differences', () => {
  const original = profile({ parametricEnabled: true, parametricBands: [{ frequencyHz: 450, gainDb: -3, q: 2 }] })
  const pop = m.applyAudioEffectPreset(original, 'pop')
  assert.deepEqual(pop.sound.equalizerBandsDb, [0, 1, 2, 1, 0, -0.5, 1, 2, 2.5, 2])
  assert.equal(pop.sound.vocal, 0.2)
  assert.deepEqual(pop.sound.parametricBands, original.sound.parametricBands)
  assert.equal(pop.sound.parametricEnabled, true)
  assert.equal(m.applyAudioEffectPreset(original, 'concert_hall').sound.reverbPreDelayMs, 40)
  assert.equal(m.applyAudioEffectPreset(original, 'surround_3d').sound.surround, 0.8)
  assert.equal(m.applyAudioEffectPreset(original, 'flat').sound.bassDb, 0)
  assert.equal(m.AUDIO_EFFECT_PRESETS.length, 29)
  assert.equal(new Set(m.AUDIO_EFFECT_PRESETS.map(preset => preset.id)).size, 29)
  assert.equal(m.AUDIO_EFFECT_PRESETS.some(preset => preset.id === 'custom'), false)
})

regression('user presets replace the complete correction without sharing editable nested data', () => {
  const saved = { id: 'user_voice', name: 'voice', sound: sound({ vocal: 0.7 }) }
  const result = m.applyAudioEffectPreset(profile({ parametricEnabled: true,
    parametricBands: [{ gainDb: 2 }] }), saved.id, [saved])
  assert.equal(result.enabled, true)
  assert.equal(result.sound.vocal, 0.7)
  assert.equal(result.sound.parametricEnabled, false)
  assert.deepEqual(result.sound.parametricBands, [])
  result.sound.equalizerBandsDb[0] = 5
  assert.equal(saved.sound.equalizerBandsDb[0], 0)
})

regression('output profiles preserve device isolation and immutable fallback updates', () => {
  const main = profile({ bassDb: 2 }), dac = profile({ bassDb: -3 })
  const original = m.normalizeAudioEffectsSettings({ main, perOutputEnabled: true,
    outputProfiles: { 'DAC A': dac } })
  assert.equal(m.audioEffectsProfileFor(original, 'DAC A').sound.bassDb, -3)
  assert.equal(m.audioEffectsProfileFor(original, 'DAC B').sound.bassDb, 2)
  const updated = m.replaceAudioEffectsProfile(original, '', profile({ bassDb: 6 }))
  assert.equal(m.audioEffectsProfileFor(updated, '').sound.bassDb, 6)
  assert.equal(Object.hasOwn(original.outputProfiles, 'system_default'), false)
  assert.equal(updated.outputProfiles['DAC A'].sound.bassDb, -3)
  const global = m.replaceAudioEffectsProfile({ ...original, perOutputEnabled: false }, 'DAC A', profile({ bassDb: 4 }))
  assert.equal(global.main.sound.bassDb, 4)
  assert.equal(global.outputProfiles['DAC A'].sound.bassDb, -3)
})

regression('unseen named outputs use the system profile before the global fallback', () => {
  const settings = m.normalizeAudioEffectsSettings({ perOutputEnabled: true,
    main: profile({ outputGainDb: -3 }), outputProfiles: {
      system_default: profile({ outputGainDb: -6 }), Headphones: profile({ outputGainDb: -9 }),
    } })
  assert.equal(m.audioEffectsProfileFor(settings, 'Headphones').sound.outputGainDb, -9)
  assert.equal(m.audioEffectsProfileFor(settings, 'Speakers').sound.outputGainDb, -6)
  assert.equal(m.audioEffectsProfileFor(settings, '').sound.outputGainDb, -6)
  assert.equal(m.audioEffectsProfileFor({ ...settings, perOutputEnabled: false }, 'Speakers').sound.outputGainDb, -3)
})

regression('emoji device names preserve the same 256 Unicode character boundary as native lookup', () => {
  const device = '🎧'.repeat(200) + 'a'.repeat(56)
  const settings = m.normalizeAudioEffectsSettings({ perOutputEnabled: true,
    outputProfiles: { [device]: profile({ bassDb: 6 }) } })
  assert.equal(Object.keys(settings.outputProfiles)[0], device)
  assert.equal(m.audioEffectsProfileFor(settings, device).sound.bassDb, 6)
  const updated = m.replaceAudioEffectsProfile(settings, `${device}overflow`, profile({ bassDb: 3 }))
  assert.deepEqual(Object.keys(updated.outputProfiles), [device])
  assert.equal(m.audioEffectsProfileFor(updated, `${device}overflow`).sound.bassDb, 3)
})

regression('emoji preset names and IDs keep complete characters at the native 24 and 128 limits', () => {
  const name = 'a'.repeat(23) + '🎧'
  const id = 'p'.repeat(127) + '🎧'
  const settings = m.normalizeAudioEffectsSettings({ userPresets: [{ id: `${id}extra`, name: `${name}extra`, sound: {} }] })
  assert.equal(settings.userPresets[0].name, name)
  assert.equal(settings.userPresets[0].id, id)
  const exported = m.exportAudioEffectsProfile({ enabled: true, presetId: `${id}extra`, sound: sound({}) }, `${name}extra`)
  assert.equal(exported.includes('\\ud83c'), false)
  const imported = m.importAudioEffectsProfile(exported)
  assert.equal(imported.name, name)
  assert.equal(imported.profile.presetId, id)
})

regression('prototype keys and oversized profile collections cannot enter persistent sound settings', () => {
  const data = JSON.parse('{"outputProfiles":{"__proto__":{"enabled":true},"constructor":{},"DAC":{"enabled":true}},"powerMode":"other"}')
  const normalized = m.normalizeAudioEffectsSettings(data)
  assert.equal(Object.hasOwn(normalized.outputProfiles, '__proto__'), false)
  assert.equal(Object.hasOwn(normalized.outputProfiles, 'constructor'), false)
  assert.equal(normalized.outputProfiles.DAC.enabled, true)
  assert.equal(normalized.powerMode, 'balanced')
  const bounded = m.normalizeAudioEffectsSettings({ outputProfiles: Object.fromEntries(
    Array.from({ length: 90 }, (_, i) => [`DAC${i}`, {}])), userPresets: Array.from({ length: 80 }, (_, i) => (
      { id: `user_${i}`, name: 'n'.repeat(80), sound: {} })) })
  assert.equal(Object.keys(bounded.outputProfiles).length, 64)
  assert.equal(bounded.userPresets.length, 50)
  assert.equal(bounded.userPresets[0].name.length, 24)
  assert.equal(m.normalizeAudioEffectsSettings({ outputProfiles: { ['x'.repeat(300)]: {} } }).outputProfiles['x'.repeat(256)].enabled, false)
})

regression('resetting sound sections restores their complete controls while retaining unrelated tuning', () => {
  const original = sound({ bassDb: 6, vocalRemoval: 0.7, reverb: 0.4, compressorEnabled: true,
    limiterEnabled: false, preampDb: -10, equalizerBandsDb: [2], parametricEnabled: true,
    parametricBands: [{ gainDb: 3 }], toneEnabled: false })
  const tone = m.resetAudioEffectsSoundSection(original, 'tone')
  assert.equal(tone.toneEnabled, true)
  assert.equal(tone.bassDb, 0)
  assert.equal(tone.vocalRemoval, 0)
  assert.equal(tone.reverb, 0.4)
  const dynamics = m.resetAudioEffectsSoundSection(original, 'dynamics')
  assert.equal(dynamics.compressorEnabled, false)
  assert.equal(dynamics.limiterEnabled, true)
  assert.equal(dynamics.bassDb, 6)
  const eq = m.resetAudioEffectsSoundSection(original, 'equalizer')
  assert.equal(eq.preampDb, 0)
  assert.equal(eq.parametricEnabled, false)
  assert.equal(eq.parametricBands.length, 0)
  assert.equal(m.resetAudioEffectsSoundSection(original, 'space').reverb, 0)
})

regression('real APO headphone correction imports full gain and Q with retained negative headroom', () => {
  const imported = m.importAutoEq('# AutoEq\nPreamp: -20 dB\nFilter 1: ON PK Fc 1000 Hz Gain -3.5 dB Q 4.1\nFilter 2: OFF HSC Fc 7000 Hz Gain 18 dB Q 12\nFilter 3: ON HPQ Fc 30 Hz Q 0.707')
  assert.equal(imported.imported, 3)
  assert.equal(imported.skipped, 0)
  assert.equal(imported.sound.preampDb, -20)
  assert.equal(imported.sound.parametricEnabled, true)
  assert.deepEqual(imported.sound.parametricBands[1], { enabled: false, type: 'high_shelf',
    frequencyHz: 7000, gainDb: 18, q: 12 })
  assert.equal(imported.sound.parametricBands[2].type, 'high_pass')
})

regression('GraphicEQ resamples by log frequency instead of treating points as ten fixed bands', () => {
  const imported = m.importAutoEq('Preamp: -3 dB\nGraphicEQ: 31.25 0; 125 12; 16000 0')
  assert.equal(imported.sound.equalizerEnabled, true)
  assert.equal(imported.sound.preampDb, -3)
  assert.equal(imported.sound.equalizerBandsDb.length, 10)
  assert.equal(imported.sound.equalizerBandsDb[0], 0)
  assert.ok(Math.abs(imported.sound.equalizerBandsDb[1] - 6) < 1e-9)
  assert.equal(imported.sound.equalizerBandsDb[2], 12)
  assert.equal(imported.sound.equalizerBandsDb[9], 0)
})

regression('APO accepts compact filter numbering and harmless trailing GraphicEQ delimiters', () => {
  assert.equal(m.importAutoEq('Filter1: ON PK Fc 500 Hz Gain 2 dB Q 1').sound.parametricBands[0].frequencyHz, 500)
  const graphic = m.importAutoEq('GraphicEQ: 31.25 0; 125 12; 16000 0;')
  assert.equal(graphic.sound.equalizerBandsDb[2], 12)
})

regression('AutoEQ skips unsupported or excess filters explicitly and warns about clamped imported ranges', () => {
  const lines = Array.from({ length: 12 }, (_, i) => `Filter ${i + 1}: ON PK Fc 1000 Hz Gain 1 dB Q 1`)
  const result = m.importAutoEq(`Filter 0: ON XYZ Fc 1000 Hz Gain 1 dB Q 1\n${lines.join('\n')}`)
  assert.equal(result.imported, 10)
  assert.equal(result.skipped, 3)
  assert.ok(result.warnings.length >= 1)
  const clamped = m.importAutoEq('Preamp: -30 dB\nFilter 1: ON PK Fc 30000 Hz Gain 30 dB Q 30')
  assert.equal(clamped.sound.preampDb, -24)
  assert.equal(clamped.sound.parametricBands[0].frequencyHz, 20000)
  assert.equal(clamped.sound.parametricBands[0].gainDb, 24)
  assert.equal(clamped.sound.parametricBands[0].q, 20)
  assert.ok(clamped.warnings.length >= 1)
})

regression('invalid AutoEQ cannot silently replace the active correction with neutral sound', () => {
  for (const text of ['', 'not an EQ', 'Filter 1: ON PK Fc 1000 Hz Q 1',
    'Preamp: NaN dB\nFilter 1: ON PK Fc 1000 Hz Gain 2 dB Q 1',
    'Filter 1: ON PK Fc 1000 Hz Gain 1e999 dB Q 1', 'GraphicEQ: 1000 2; 1000 3',
    'x'.repeat(131073)]) assert.throws(() => m.importAutoEq(text), undefined, text.slice(0, 60))
})

regression('exported profiles round trip without unrelated account data', () => {
  const original = { ...profile({ bassDb: 4, reverb: 0.3 }), token: 'account-secret',
    sound: { ...sound({ bassDb: 4, reverb: 0.3 }), password: 'account-secret' } }
  const exported = m.exportAudioEffectsProfile(original, 'my room')
  assert.equal(exported.includes('account-secret'), false)
  const restored = m.importAudioEffectsProfile(exported)
  assert.equal(restored.name, 'my room')
  assert.equal(restored.profile.enabled, true)
  assert.equal(restored.profile.sound.bassDb, 4)
  assert.equal(restored.profile.sound.reverb, 0.3)
})

regression('Android user-preset and settings JSON import into the same desktop sound model', () => {
  const user = m.importAudioEffectsProfile('{"id":"user_android","name":"Android","sound":{"bassDb":3,"parametricBands":[{"type":"notch","frequencyHz":900,"q":11}]}}')
  assert.equal(user.name, 'Android')
  assert.equal(user.profile.presetId, 'user_android')
  assert.equal(user.profile.enabled, true)
  assert.equal(user.profile.sound.parametricBands[0].q, 11)
  const settings = m.importAudioEffectsProfile('{"version":2,"main":{"enabled":true,"presetId":"pop","sound":{"vocal":0.2}}}')
  assert.equal(settings.profile.sound.vocal, 0.2)
})

regression('malformed, nonfinite, prototype and oversized JSON imports are rejected clearly', () => {
  for (const text of ['{}', '[]', '{"sound":{"bassDb":1e999}}',
    '{"sound":{"bassDb":"NaN"}}', '{"sound":{"parametricBands":[{"type":"unknown"}]}}',
    '{"sound":{"equalizerBandsDb":["3"]}}',
    '{"__proto__":{},"sound":{}}', '{"format":"neriplayer-audio-effects-profile","version":99,"profile":{"sound":{}}}',
    'x'.repeat(131073)]) assert.throws(() => m.importAudioEffectsProfile(text))
})

regression('import size limits count UTF-8 bytes before accepting otherwise valid JSON', () => {
  assert.throws(() => m.importAudioEffectsProfile(JSON.stringify({ name: '文'.repeat(44000), sound: {} })), /128 KiB/)
})

regression('frequency response is an actual RBJ cascade with preamp and section enable state', () => {
  const flat = m.audioEffectsFrequencyResponse(sound({ preampDb: -4 }))
  assert.equal(flat.length, 96)
  assert.ok(Math.abs(flat[0].frequencyHz - 20) < 1e-9)
  assert.ok(Math.abs(flat.at(-1).frequencyHz - 20000) < 1e-8)
  assert.ok(flat.every(point => Math.abs(point.gainDb + 4) < 1e-9))
  const peak = m.audioEffectsFrequencyResponse(sound({ parametricEnabled: true,
    parametricBands: [{ frequencyHz: 20, gainDb: 6, q: 1 }] }))
  assert.ok(Math.abs(peak[0].gainDb - 6) < 1e-6)
  assert.ok(Math.abs(peak.at(-1).gainDb) < 0.001)
  const disabled = m.audioEffectsFrequencyResponse(sound({ bassDb: 12, toneEnabled: false }))
  assert.ok(disabled.every(point => Math.abs(point.gainDb) < 1e-9))
})

regression('bass response shows Android subsonic protection only for positive gain', () => {
  const boosted = m.audioEffectsFrequencyResponse(sound({ bassDb: 6, bassFrequencyHz: 100 }), 1, 1000)
  const at = hz => boosted.reduce((closest, point) =>
    Math.abs(point.frequencyHz - hz) < Math.abs(closest.frequencyHz - hz) ? point : closest)
  assert.ok(boosted[0].gainDb < 2, `20 Hz gain ${boosted[0].gainDb}`)
  assert.ok(Math.abs(at(40).gainDb - 6) < 1, `40 Hz gain ${at(40).gainDb}`)
  assert.ok(Math.abs(at(2000).gainDb) < 0.1)
  const cut = m.audioEffectsFrequencyResponse(sound({ bassDb: -6, bassFrequencyHz: 100 }))
  assert.ok(Math.abs(cut[0].gainDb + 6) < 0.2)
  const disabled = m.audioEffectsFrequencyResponse(sound({ bassDb: 6, toneEnabled: false }))
  assert.ok(disabled.every(point => Math.abs(point.gainDb) < 1e-9))
})

regression('bass presets match Android punch tuning without changing saved custom sounds', () => {
  const original = profile({ bassDb: 4, virtualBass: 0.8 })
  const boost = m.applyAudioEffectPreset(original, 'bass_boost').sound
  assert.deepEqual(boost.equalizerBandsDb, [1, 1, 0.5, 0, 0, 0, 0, 0, 0, 0])
  assert.equal(boost.bassDb, 7)
  assert.equal(boost.virtualBass, 0.25)
  const deep = m.applyAudioEffectPreset(original, 'deep_bass').sound
  assert.deepEqual(deep.equalizerBandsDb, [2, 1.5, 0.5, 0, -0.5, 0, 0, 0, 0, 0])
  assert.equal(deep.bassDb, 8)
  assert.equal(deep.bassFrequencyHz, 80)
  assert.equal(deep.virtualBass, 0.45)
  assert.equal(deep.monoBassHz, 120)
  assert.equal(original.sound.bassDb, 4)
  assert.equal(original.sound.virtualBass, 0.8)
})

regression('legacy five-band migration preserves the existing Q and headroom instead of reinterpreting gains as ten bands', () => {
  const settings = m.migrateLegacyAudioEffects({ equalizerEnabled: true, equalizerPresetId: 'custom',
    equalizerBands: [300, 100, -100, 100, 300], loudnessGainMb: 1200 })
  assert.equal(settings.main.enabled, true)
  assert.equal(settings.autoHeadroom, false)
  assert.equal(settings.main.sound.parametricEnabled, true)
  assert.deepEqual(settings.main.sound.parametricBands.map(band => band.frequencyHz), [60, 230, 910, 3600, 14000])
  assert.deepEqual(settings.main.sound.parametricBands.map(band => band.gainDb), [0, -2, -4, -2, 0])
  assert.ok(settings.main.sound.parametricBands.every(band => band.q === 1))
  assert.deepEqual(settings.main.sound.equalizerBandsDb, Array(10).fill(0))
  assert.equal(settings.main.sound.outputGainDb, 0)
})

regression('legacy migration resolves known preset values and never enables EQ for gain-only settings', () => {
  const rock = m.migrateLegacyAudioEffects({ equalizerEnabled: true, equalizerPresetId: 'rock', equalizerBands: [] })
  assert.deepEqual(rock.main.sound.parametricBands.map(band => band.gainDb), [0, -3, -7, -3, 0])
  const disabled = m.migrateLegacyAudioEffects({ equalizerEnabled: false, equalizerPresetId: 'rock',
    equalizerBands: [600, 300, -100, 300, 600], loudnessGainMb: 1200 })
  assert.equal(disabled.main.enabled, false)
  assert.equal(disabled.main.sound.outputGainDb, 0)
})

regression('extreme legacy cuts retain minus thirty decibels without clipping the migrated gain', () => {
  const migrated = m.migrateLegacyAudioEffects({ equalizerEnabled: true, equalizerPresetId: 'custom',
    equalizerBands: [1500, -1500, 0, -1500, 1500] })
  assert.deepEqual(migrated.main.sound.parametricBands.map(band => band.gainDb), [0, -30, -15, -30, 0])
  const imported = m.importAutoEq('Filter 1: ON PK Fc 1000 Hz Gain -30 dB Q 1')
  assert.equal(imported.sound.parametricBands[0].gainDb, -30)
  const response = m.audioEffectsFrequencyResponse(sound({ parametricEnabled: true,
    parametricBands: [{ frequencyHz: 20, gainDb: -30, q: 1 }] }))
  assert.ok(Math.abs(response[0].gainDb + 30) < 1e-5)
})

console.log(`audio effects model tests passed (${passed})`)
