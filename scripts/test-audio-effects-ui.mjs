import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
function loadModule(source, dependencies = {}) {
  const exports = {}
  new Function('require', 'exports', ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText)(name => { assert.ok(name in dependencies, `unexpected dependency: ${name}`); return dependencies[name] }, exports)
  return exports
}
const model = loadModule(await read('../src/modules/audioEffects/model.ts'))
const listening = loadModule(await read('../src/modules/audioEffects/listening.ts'), { './model': model })
const initial = () => model.normalizeAudioEffectsSettings(undefined)
const calls = []
let statsCalls = 0
const player = vue.reactive({
  playbackSpeed: 1, playbackPitch: 1, loudnessGainMb: 0, vinylEffectEnabled: false,
  audioEffects: initial(), audioEffectsOutputDevice: 'USB DAC',
  get audioEffectsProfile() { return model.audioEffectsProfileFor(this.audioEffects, this.audioEffectsOutputDevice) },
  get listeningTuning() { return listening.listeningTuningValues(this.audioEffectsProfile.sound) },
  setListeningTuning(key, value) { this.updateAudioEffects({ autoHeadroom: true }); this.updateAudioEffectsProfile({ enabled: true, presetId: 'custom', sound: listening.applyListeningTuning(this.audioEffectsProfile.sound, key, value) }) },
  resetListeningTuning() { this.updateAudioEffectsProfile({ sound: listening.resetListeningTuning(this.audioEffectsProfile.sound) }) },
  setSpeed(value) { this.playbackSpeed = value },
  setPitch(value) { this.playbackPitch = value },
  setVinylEffect(value) { this.vinylEffectEnabled = value },
  effectivePlaybackPitch() { return this.vinylEffectEnabled ? this.playbackSpeed : this.playbackPitch },
  setLoudnessGain(value) { this.loudnessGainMb = value },
  updateAudioEffects(patch) { calls.push(['settings', patch]); Object.assign(this.audioEffects, patch) },
  updateAudioEffectsProfile(patch) { calls.push(['profile', patch]); this.audioEffects = model.replaceAudioEffectsProfile(this.audioEffects, this.audioEffectsOutputDevice, { ...this.audioEffectsProfile, ...patch }) },
  updateAudioEffectsSound(patch) { calls.push(['sound', patch]); this.updateAudioEffectsProfile({ sound: { ...this.audioEffectsProfile.sound, ...patch }, presetId: 'custom' }) },
  setAudioEffectsPreset(id) {
    calls.push(['preset', id]); this.updateAudioEffectsProfile(model.applyAudioEffectPreset(this.audioEffectsProfile, id, this.audioEffects.userPresets))
  },
  resetAudioEffectsSection(section) {
    calls.push(['reset-section', section]);
    if (section === 'speaker') this.audioEffects.speaker = initial().speaker
    else this.updateAudioEffectsProfile({ sound: model.resetAudioEffectsSoundSection(this.audioEffectsProfile.sound, section) })
  },
  saveAudioEffectsPreset(name) { calls.push(['save', name]); this.audioEffects.userPresets.push({ id: 'user_test', name, sound: model.normalizeAudioEffectsSound(this.audioEffectsProfile.sound) }) },
  deleteAudioEffectsPreset(id) { calls.push(['delete', id]); this.audioEffects.userPresets = this.audioEffects.userPresets.filter(preset => preset.id !== id) },
  exportAudioEffectsPreset(id) { calls.push(['export', id]); return model.exportAudioEffectsProfile(this.audioEffectsProfile, 'Exported') },
  importAudioEffectsPreset(text) { calls.push(['import', text]); return model.importAudioEffectsProfile(text) },
  importAutoEq(text) { calls.push(['autoeq', text]); const result = model.importAutoEq(text); this.updateAudioEffectsSound(result.sound); return result },
  async refreshAudioEffectsStats() { statsCalls++; return { active: true, cpuLoadPercent: 2.5, limiterReductionDb: 1, compressorReductionDb: 3, sampleRate: 48000, nativeAvailable: true, inactiveReason: null, outputDevice: 'USB DAC' } },
  resetAudioEffects() { this.playbackSpeed = 1; this.playbackPitch = 1; this.vinylEffectEnabled = false; this.loudnessGainMb = 0; this.audioEffects = initial() },
})
const select = {
  props: ['modelValue', 'selectedLabel', 'options', 'label', 'surface'], emits: ['update:modelValue'],
  setup: (props, { emit }) => () => vue.h('preset-select', { ...props, onChoose: value => emit('update:modelValue', value) }),
}
const translate = (key, args) => args?.frequency != null ? `${key}:${args.frequency}` : args?.number != null ? `${key}:${args.number}` : key
const components = {}
let clipboardWrite = ''
async function component(path, dependencyOverrides = {}) {
  const script = compileScript(parse(await read(path)).descriptor, { id: path, genDefaultAs: 'component', inlineTemplate: true })
  const dependencies = {
    vue, 'vue-i18n': { useI18n: () => ({ t: translate }) },
    '@tauri-apps/api/core': { isTauri: () => true },
    '@/stores/player': { usePlayerStore: () => player },
    '@/modules/audioEffects/model': model,
    '@/modules/audioEffects/listening': listening,
    '@/utils/clipboard': { writeClipboardText: async text => { clipboardWrite = text } },
    '@/utils/editableRange': loadModule(await read('../src/utils/editableRange.ts')),
    './ui/CustomSelect.vue': { default: select }, '@/components/ui/CustomSelect.vue': { default: select },
    ...components, ...dependencyOverrides,
  }
  return new Function('require', 'exports', `${ts.transpileModule(script.content, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText}\nreturn component`)(name => { assert.ok(name in dependencies, `unexpected dependency: ${name}`); return dependencies[name] }, {})
}

let focusedInput = null
class TestElement {
  constructor(type) { this.type = type; this.tagName = type.toUpperCase(); this.children = []; this.props = {}; this.style = { display: '' }; this.listeners = new Map() }
  get parentElement() { return this.parent instanceof TestElement ? this.parent : null }
  matches(selector) {
    if (selector.startsWith('.')) return (this.props.class ?? '').split(/\s+/).includes(selector.slice(1))
    if (selector === 'input[type="range"]') return this.type === 'input' && this.props.type === 'range'
    return this.type === selector
  }
  closest(selector) {
    for (let node = this; node; node = node.parentElement) if (selector.split(',').some(part => node.matches(part.trim()))) return node
    return null
  }
  querySelectorAll(selector) { return findAll(this, node => node instanceof TestElement && node.matches(selector)) }
  getRootNode() { return globalThis.document }
  addEventListener(name, callback) { const callbacks = this.listeners.get(name) ?? new Set(); callbacks.add(callback); this.listeners.set(name, callbacks) }
  removeEventListener(name, callback) { this.listeners.get(name)?.delete(callback) }
  focus() { focusedInput = this }
  select() {}
}
class TestInputElement extends TestElement {}
globalThis.Element = TestElement
globalThis.HTMLInputElement = TestInputElement
globalThis.Document = class { activeElement = null }
globalThis.ShadowRoot = class {}
globalThis.document = new Document()
const renderer = vue.createRenderer({
  createElement: type => type === 'input' ? new TestInputElement(type) : new TestElement(type),
  createText: text => ({ text }), createComment: text => ({ text }),
  setText(node, text) { node.text = text }, setElementText(node, text) { node.text = text; node.children = [] },
  patchProp(node, key, _, value) { node.props[key] = value },
  parentNode: node => node.parent ?? null,
  nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  insert(node, parent, anchor = null) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent; const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node); else parent.children.splice(index, 0, node)
  },
  remove(node) { node.parent.children.splice(node.parent.children.indexOf(node), 1) },
})
function findAll(tree, predicate) { return [...(predicate(tree) ? [tree] : []), ...(tree.children ?? []).flatMap(child => findAll(child, predicate))] }
const ranges = tree => findAll(tree, node => node.type === 'input' && node.props.type === 'range')
const byLabel = (tree, label) => findAll(tree, node => node.props?.['aria-label'] === label)[0]
const rangeByLabel = (tree, label) => ranges(tree).find(node => node.props['aria-label'] === label)
const button = (tree, text) => findAll(tree, node => node.type === 'button' && node.text === text)[0]
const click = target => {
  const event = { target, preventDefault() {}, stopPropagation() {} }
  for (let node = target; node; node = node.parentElement) for (const listener of node.listeners.get('click') ?? []) listener(event)
}
const intervals = new Map()
const originalSetInterval = globalThis.setInterval
const originalClearInterval = globalThis.clearInterval
globalThis.setInterval = (callback, milliseconds) => { const id = Symbol(); intervals.set(id, { callback, milliseconds }); return id }
globalThis.clearInterval = id => intervals.delete(id)
const editable = await component('../src/components/ui/EditableRangeValue.vue')
components['./ui/EditableRangeValue.vue'] = { default: editable }
components['@/components/ui/EditableRangeValue.vue'] = { default: editable }
components['./DspNumberControl.vue'] = { default: await component('../src/components/settings/DspNumberControl.vue') }
components['./DspFrequencyGraph.vue'] = { default: await component('../src/components/settings/DspFrequencyGraph.vue') }

let details = 0
const panelTree = { children: [] }
const panelApp = renderer.createApp(await component('../src/components/AudioEffectsPanel.vue'), { onDetails: () => details++ })
panelApp.mount(panelTree)
try {
  assert.equal(ranges(panelTree).length, 2)
  const quickSpeeds = findAll(panelTree, node => node.type === 'button' && node.closest('.audiofx-speed-grid'))
  assert.deepEqual(quickSpeeds.map(node => Number(node.text.slice(0, -1))), [0.5, 0.75, 0.95, 1, 1.05, 1.25, 1.5, 2])
  assert.ok(byLabel(panelTree, 'player.vinyl_effect'))
  const quickPreset = findAll(panelTree, node => node.type === 'preset-select')[0]
  assert.equal(quickPreset.props.options.length, 29)
  assert.ok(quickPreset.props.options.every(option => option.value !== 'custom'))
  rangeByLabel(panelTree, 'player.pitch').props.onInput({ target: { value: '0.85' } })
  rangeByLabel(panelTree, 'player.playback_speed').props.onInput({ target: { value: '1.05' } })
  byLabel(panelTree, 'player.vinyl_effect').props.onChange({ target: { checked: true } })
  await vue.nextTick()
  assert.equal(ranges(panelTree).length, 1, 'manual pitch is hidden while vinyl follows speed')
  assert.equal(player.effectivePlaybackPitch(), 1.05)
  assert.equal(player.playbackPitch, 0.85, 'vinyl keeps the independent pitch value')
  byLabel(panelTree, 'player.vinyl_effect').props.onChange({ target: { checked: false } })
  await vue.nextTick()
  assert.equal(rangeByLabel(panelTree, 'player.pitch').props.value, 0.85)
  quickPreset.props.onChoose('rock')
  await vue.nextTick()
  assert.equal(player.audioEffectsProfile.enabled, true)
  assert.equal(player.audioEffectsProfile.presetId, 'rock')
  player.updateAudioEffectsSound({ bassDb: 4 })
  await vue.nextTick()
  assert.equal(quickPreset.props.selectedLabel, 'dsp.manual')
  player.updateAudioEffectsProfile({ presetId: 'user_deleted_on_another_output' })
  await vue.nextTick()
  assert.equal(quickPreset.props.selectedLabel, 'dsp.manual', 'missing or deleted preset IDs display the saved sound as manual')
  button(panelTree, 'player.audio_effects_details').props.onClick(); assert.equal(details, 1)
  button(panelTree, 'player.reset_effects').props.onClick(); assert.equal(player.playbackSpeed, 1)
  console.log('PASS quick speed steps, vinyl pitch restore, preset activation and manual label')
} finally { panelApp.unmount() }

const detailTree = { children: [] }
const detailApp = renderer.createApp(await component('../src/components/settings/AudioEffectsSettings.vue'))
detailApp.mount(detailTree)
try {
  await Promise.resolve(); await vue.nextTick()
  assert.equal(ranges(detailTree).length, 53, 'five listening controls and all 48 professional controls are wired')
  const professional = findAll(detailTree, node => node instanceof TestElement && node.matches('.effects-professional'))[0]
  assert.equal(professional.style.display, 'none', 'technical controls start hidden in simple mode')
  assert.equal(findAll(detailTree, node => node.type === 'preset-select' && node.props.label === 'player.eq_preset').length, 0,
    'detailed presets are direct cards, never a selection box')
  assert.equal(findAll(detailTree, node => node.type === 'button' && node.matches('.effects-preset-card')).length, 8)
  button(detailTree, 'dsp.categories.all').props.onClick(); await vue.nextTick()
  assert.equal(findAll(detailTree, node => node.type === 'button' && node.matches('.effects-preset-card')).length, 29)
  byLabel(detailTree, 'dsp.presets.warm').props.onClick(); await vue.nextTick()
  assert.equal(player.audioEffectsProfile.presetId, 'warm')
  assert.equal(byLabel(detailTree, 'dsp.presets.warm').props['aria-pressed'], true)
  rangeByLabel(detailTree, 'dsp.listening.bass.title').props.onInput({ target: { value: '50' } }); await vue.nextTick()
  assert.ok(player.audioEffectsProfile.sound.bassDb > 0 && player.audioEffectsProfile.sound.bassDb < 6)
  assert.equal(player.audioEffects.autoHeadroom, true)
  assert.equal(player.audioEffectsProfile.sound.limiterEnabled, true)
  const calibration = [...player.audioEffectsProfile.sound.equalizerBandsDb]
  button(detailTree, 'dsp.listening.reset').props.onClick(); await vue.nextTick()
  assert.equal(player.audioEffectsProfile.sound.bassDb, 0)
  assert.deepEqual(player.audioEffectsProfile.sound.equalizerBandsDb, calibration)
  button(detailTree, 'dsp.expert_mode').props.onClick(); await vue.nextTick()
  assert.equal(professional.style.display, '', 'advanced mode reveals the full controls')
  for (const range of ranges(detailTree)) assert.ok(range.props['aria-label'])
  assert.equal(intervals.size, 1); assert.equal([...intervals.values()][0].milliseconds, 1000); assert.equal(statsCalls, 1)
  rangeByLabel(detailTree, 'settings.audio_effects_band:125').props.onInput({ target: { value: '-3.5' } })
  assert.deepEqual([...player.audioEffectsProfile.sound.equalizerBandsDb], [0, 0, -3.5, 0, 0, 0, 0, 0, 0, 0])
  byLabel(detailTree, 'dsp.graphic_eq').props.onChange({ target: { checked: false } })
  assert.equal(player.audioEffectsProfile.sound.equalizerBandsDb[2], -3.5, 'graphic EQ disable preserves saved gains')
  byLabel(detailTree, 'dsp.graphic_eq').props.onChange({ target: { checked: true } })
  rangeByLabel(detailTree, 'dsp.controls.preampDb').props.onInput({ target: { value: '-24' } })
  assert.equal(player.audioEffectsProfile.sound.preampDb, -24)
  rangeByLabel(detailTree, 'dsp.controls.virtualBass').props.onInput({ target: { value: '0.35' } })
  assert.equal(player.audioEffectsProfile.sound.virtualBass, 0.35)
  byLabel(detailTree, 'dsp.tone_section').props.onChange({ target: { checked: false } })
  assert.equal(player.audioEffectsProfile.sound.virtualBass, 0.35, 'turning a section off preserves its parameters')
  byLabel(detailTree, 'dsp.tone_section').props.onChange({ target: { checked: true } })
  assert.equal(player.audioEffectsProfile.enabled, true)
  await vue.nextTick()
  button(detailTree, 'dsp.compare_original').props.onClick()
  assert.equal(player.audioEffectsProfile.enabled, false, 'A/B comparison changes the master bypass')
  assert.equal(player.audioEffectsProfile.sound.virtualBass, 0.35)
  const addFilter = findAll(detailTree, node => node.type === 'button' && node.matches('.effects-add'))[0]
  assert.ok(addFilter)
  addFilter.props.onClick(); await vue.nextTick()
  assert.equal(player.audioEffectsProfile.enabled, true, 'adding a PEQ filter activates the DSP master')
  byLabel(detailTree, 'dsp.graphic_eq').props.onChange({ target: { checked: false } })
  byLabel(detailTree, 'dsp.parametric_eq').props.onChange({ target: { checked: true } })
  assert.equal(player.audioEffectsProfile.sound.equalizerEnabled, true, 'enabling PEQ also enables its containing EQ section')
  rangeByLabel(detailTree, 'dsp.filter_frequency:1').props.onInput({ target: { value: '60' } })
  rangeByLabel(detailTree, 'dsp.filter_gain:1').props.onInput({ target: { value: '4.5' } })
  rangeByLabel(detailTree, 'dsp.filter_q:1').props.onInput({ target: { value: '0.8' } })
  assert.deepEqual(vue.toRaw(player.audioEffectsProfile.sound.parametricBands[0]), { enabled: true, type: 'peak', frequencyHz: 60, gainDb: 4.5, q: 0.8 })
  const filterType = findAll(detailTree, node => node.type === 'preset-select' && node.props.label === 'dsp.filter_type')[0]
  assert.equal(filterType.props.options.length, 7)
  filterType.props.onChoose('high_pass'); await vue.nextTick()
  assert.equal(rangeByLabel(detailTree, 'dsp.filter_gain:1'), undefined, 'gain-free filter types hide irrelevant gain')
  assert.equal(player.audioEffectsProfile.sound.parametricBands[0].gainDb, 4.5, 'filter changes preserve stored gain')
  byLabel(detailTree, 'dsp.remove_filter:1').props.onClick(); await vue.nextTick()
  assert.equal(player.audioEffectsProfile.sound.parametricBands.length, 0)
  byLabel(detailTree, 'dsp.autoeq_text').props['onUpdate:modelValue']('Preamp: -4 dB\nFilter 1: ON PK Fc 100 Hz Gain 3 dB Q 1')
  await vue.nextTick(); button(detailTree, 'dsp.apply_autoeq').props.onClick(); await vue.nextTick()
  assert.equal(player.audioEffectsProfile.sound.preampDb, -4)
  assert.equal(player.audioEffectsProfile.sound.parametricBands[0].frequencyHz, 100)
  byLabel(detailTree, 'dsp.preset_name').props['onUpdate:modelValue'](' My headphones ')
  await vue.nextTick(); button(detailTree, 'dsp.save_preset').props.onClick(); await vue.nextTick()
  assert.equal(player.audioEffects.userPresets[0].name, 'My headphones')
  assert.equal(findAll(detailTree, node => node.type === 'button' && node.matches('.effects-preset-card')).length, 1,
    'saving shows the named preset immediately')
  button(detailTree, 'dsp.categories.all').props.onClick(); await vue.nextTick()
  assert.equal(findAll(detailTree, node => node.type === 'button' && node.matches('.effects-preset-card')).length, 30)
  assert.equal(findAll(detailTree, node => node.text === 'dsp.user_preset_note').length, 0, 'user preset cards carry no filler description')
  byLabel(detailTree, 'My headphones').props.onClick(); await vue.nextTick()
  const transferButton = text => findAll(detailTree, node => node.type === 'button' && node.closest('.effects-transfer-actions')
    && findAll(node, child => child.text === text).length > 0)[0]
  await transferButton('dsp.transfer.copy').props.onClick(); await vue.nextTick()
  const shared = JSON.parse(clipboardWrite)
  assert.equal(shared.name, 'Exported')
  assert.ok(shared.id && shared.sound, 'share codes carry the top-level fields Android requires')
  assert.ok(transferButton('dsp.transfer.copied'), 'the copy button confirms in place instead of a toast')
  assert.equal(findAll(detailTree, node => node instanceof TestElement && node.matches('.effects-paste')).length, 0)
  transferButton('dsp.transfer.paste').props.onClick(); await vue.nextTick()
  byLabel(detailTree, 'dsp.transfer.paste_placeholder').props['onUpdate:modelValue'](clipboardWrite); await vue.nextTick()
  button(detailTree, 'dsp.transfer.import').props.onClick(); await vue.nextTick()
  assert.ok(calls.some(call => call[0] === 'import'))
  assert.equal(findAll(detailTree, node => node instanceof TestElement && node.matches('.effects-paste')).length, 0, 'a successful paste import closes the box')
  assert.equal(findAll(detailTree, node => node instanceof TestElement && node.matches('.effects-status'))[0].text, 'dsp.transfer.imported_profile')
  assert.equal(findAll(detailTree, node => node instanceof TestElement && node.matches('.effects-feedback')).length, 0, 'no floating toast')
  byLabel(detailTree, 'dsp.delete_preset My headphones').props.onClick(); await vue.nextTick(); assert.equal(player.audioEffects.userPresets.length, 0)
  byLabel(detailTree, 'dsp.per_output').props.onChange({ target: { checked: true } }); assert.equal(player.audioEffects.perOutputEnabled, true)
  byLabel(detailTree, 'dsp.auto_headroom').props.onChange({ target: { checked: false } }); assert.equal(player.audioEffects.autoHeadroom, false)
  rangeByLabel(detailTree, 'dsp.speaker_controls.protectionHz').props.onInput({ target: { value: '140' } }); assert.equal(player.audioEffects.speaker.protectionHz, 140)
  player.updateAudioEffectsProfile({ enabled: false })
  byLabel(detailTree, 'dsp.speaker_section').props.onChange({ target: { checked: true } })
  assert.equal(player.audioEffects.speaker.enabled, true)
  assert.equal(player.audioEffectsProfile.enabled, true, 'speaker enable activates the DSP master')
  const eqBands = findAll(detailTree, node => node instanceof TestElement && node.matches('.effects-band'))
  assert.equal(eqBands.length, 10)
  for (const card of findAll(detailTree, node => node instanceof TestElement && node.matches('.setting-card'))) {
    assert.ok(findAll(card, node => node instanceof TestElement && node.matches('.editable-range-value')).length <= 1,
      'every clickable setting card has at most one numeric editor')
  }
  for (const band of eqBands) {
    click(findAll(band, node => node.props?.class === 'setting-title')[0]); await vue.nextTick()
    const editors = findAll(detailTree, node => node instanceof TestElement && node.matches('.editable-range-input'))
    assert.equal(editors.length, 1, 'a label opens only its own number editor')
    assert.equal(editors[0].closest('.setting-card'), band)
    assert.equal(vue.toRaw(focusedInput), editors[0])
    editors[0].props.onKeydown({ key: 'Escape', preventDefault() {}, stopPropagation() {} }); await vue.nextTick()
  }
  console.log('PASS full DSP parameter wiring, A/B, PEQ, AutoEQ, named presets, per-output settings and independent real editors')
} finally { detailApp.unmount(); assert.equal(intervals.size, 0, 'detail unmount clears stats polling'); globalThis.setInterval = originalSetInterval; globalThis.clearInterval = originalClearInterval }

const nowPlaying = await read('../src/components/NowPlaying.vue')
const functionSource = nowPlaying.match(/function openAudioEffectsSettings\(\)[^{]*\{([\s\S]*?)\n\}/)?.[1]
const pushed = [], emitted = [], popover = { value: true }, sheet = { value: true }
await new Function('showAudioFxPanel', 'showMoreSheet', 'router', 'emit', functionSource)(popover, sheet, { push: path => { pushed.push(path); return Promise.resolve() } }, event => emitted.push(event))
await Promise.resolve()
assert.equal(popover.value, false); assert.equal(sheet.value, false); assert.deepEqual(pushed, ['/settings?section=audio_effects']); assert.deepEqual(emitted, ['collapse'])
const index = await read('../src/modules/settings/searchIndex.ts')
for (const key of ['player.playback_speed', 'player.pitch', 'player.vinyl_effect', 'dsp.autoeq', 'dsp.per_output', ...model.SOUND_CONTROLS.map(control => `dsp.controls.${control.key}`)]) assert.ok(index.includes(`title: '${key}'`), `${key} is searchable`)
for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) {
  const messages = JSON.parse(await read(`../src/i18n/${locale}.json`))
  assert.ok(messages.dsp.simple_mode && messages.dsp.expert_mode)
  assert.ok(Object.values(messages.dsp.speaker_sizes).every(value => !/[\/]/.test(value)))
  for (const control of listening.LISTENING_CONTROLS) for (const key of ['title', 'help', 'left', 'right']) assert.ok(messages.dsp.listening[control.key][key])
  for (const preset of model.AUDIO_EFFECT_PRESETS) assert.ok(messages.dsp.preset_notes[preset.id])
  assert.ok(messages.player.vinyl_effect)
  assert.ok(messages.dsp.preset_limit)
  for (const preset of model.AUDIO_EFFECT_PRESETS) assert.ok(messages.dsp.presets[preset.id], `${locale} preset ${preset.id}`)
  for (const control of model.SOUND_CONTROLS) { assert.ok(messages.dsp.controls[control.key]); assert.ok(messages.dsp.help[control.key]) }
  for (const type of ['peak', 'low_shelf', 'high_shelf', 'low_pass', 'high_pass', 'notch', 'band_pass']) assert.ok(messages.dsp.filters[type])
}
console.log('PASS settings navigation, full parameter search and four-language DSP coverage')
