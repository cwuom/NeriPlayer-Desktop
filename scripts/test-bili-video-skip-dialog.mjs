import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const source = await readFile(new URL('../src/components/bilibili/BiliVideoSkipDialog.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, { id: 'bili-skip-test', genDefaultAs: 'component' })
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const policySource = await readFile(new URL('../src/modules/playback/biliVideoSkip.ts', import.meta.url), 'utf8')
const policyCode = ts.transpileModule(policySource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const policy = {}
new Function('require', 'exports', policyCode)(require, policy)
const renderer = vue.createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText() {}, setElementText() {}, patchProp() {},
  parentNode: node => node.parent, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { node.parent.children = node.parent.children.filter(child => child !== node) },
})
const part = (cid, durationMs = 60_000) => ({ bvid: 'BV1fixture', cid, label: `P${cid}`, durationMs })
const track = (id = 'bilibili:BV1fixture:11') => ({
  id, title: 'fixture', artist: 'artist', album: '哔哩哔哩', source: 'bilibili', durationMs: 60_000,
  coverUrl: '', audioUrl: '', syncPayload: { audioId: 'BV1fixture', subAudioId: '11' },
})
const deferred = () => {
  let resolve, reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  return { promise, resolve, reject }
}
async function flush() { for (let i = 0; i < 8; i++) { await Promise.resolve(); await vue.nextTick() } }

function mount({ load = async () => [part(11), part(22)], save, saved = [], currentTrack = track(), storage = new Map(), storageFails = false } = {}) {
  const calls = [], updates = [], notices = [], seeks = []
  const player = vue.reactive({
    currentTrack, currentBiliVideoSkipTarget: currentTrack ? { bvid: 'BV1fixture', cid: 11 } : null,
    interpolatedPositionMs: 12_999, positionMs: 12_999, durationMs: 60_000,
    isPlaying: true, isLoadingAudio: false,
    seekTo: async value => { seeks.push(value) },
    togglePlayPause: async () => { player.isPlaying = !player.isPlaying },
  })
  const lt = vue.reactive({ localControlRestriction: null })
  const store = vue.reactive({
    rules: saved,
    ruleFor: target => store.rules.find(rule => rule.bvid === target.bvid && rule.cid === target.cid) ?? null,
    setIntervals: async (target, intervals, durationMs) => {
      calls.push({ target: { ...target }, intervals: structuredClone(vue.toRaw(intervals)), durationMs })
      if (save) return save(target, intervals, durationMs)
      return { ...target, intervals, modifiedAt: 1, isDeleted: !intervals.length }
    },
  })
  const dependencies = {
    vue,
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@tauri-apps/api/core': { invoke: (command, args) => {
      assert.equal(command, 'get_bili_video_skip_targets'); return load(args)
    } },
    '@/stores/player': { usePlayerStore: () => player },
    '@/stores/biliVideoSkip': { useBiliVideoSkipStore: () => store },
    '@/stores/listenTogether': { useListenTogetherStore: () => lt },
    '@/stores/toast': { useToastStore: () => ({ success: value => notices.push(value) }) },
    '@/modules/playback/biliVideoSkip': policy,
  }
  const localStorage = {
    getItem: key => { if (storageFails) throw new Error('storage unavailable'); return storage.get(key) ?? null },
    setItem: (key, value) => { if (storageFails) throw new Error('storage unavailable'); storage.set(key, value) },
  }
  const component = new Function('require', 'exports', 'localStorage', `${compiled}\nreturn component`)(
    name => name.endsWith('.vue') ? {} : dependencies[name] ?? require(name), {}, localStorage,
  )
  let bindings
  const setup = component.setup
  component.setup = (props, context) => { bindings = setup(props, context); return () => null }
  const input = vue.reactive({ open: true, track: track(), 'onUpdate:open': value => updates.push(value) })
  const app = renderer.createApp({ setup: () => () => vue.h(component, input) })
  app.mount({ children: [] })
  return { bindings, input, store, player, lt, calls, updates, notices, seeks, storage, stop: () => app.unmount() }
}
let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('opening selects the explicit part and adding merges touching intervals without persisting', async () => {
  const r = mount()
  try {
    await flush()
    assert.equal(r.bindings.selectedKey.value, 'BV1fixture|11')
    r.bindings.startText.value = '10'; r.bindings.endText.value = '20'; r.bindings.addInterval()
    r.bindings.startText.value = '00:20'; r.bindings.endText.value = '00:25'; r.bindings.addInterval()
    assert.deepEqual(r.bindings.draftIntervals.value, [{ startMs: 10_000, endMs: 25_000 }])
    assert.equal(r.calls.length, 0)
    r.bindings.close()
    assert.deepEqual(r.updates, [false]); assert.equal(r.calls.length, 0)
  } finally { r.stop() }
})

await test('invalid formats and durations keep the input for correction', async () => {
  const r = mount()
  try {
    await flush()
    for (const [start, end, error] of [
      ['1.5', '20', 'player.bili_skip_invalid_time'],
      ['20', '10', 'player.bili_skip_invalid_range'],
      ['10', '61', 'player.bili_skip_exceeds_duration'],
    ]) {
      r.bindings.startText.value = start; r.bindings.endText.value = end; r.bindings.addInterval()
      assert.equal(r.bindings.inputError.value, error)
      assert.equal(r.bindings.startText.value, start); assert.equal(r.bindings.endText.value, end)
      assert.equal(r.bindings.draftIntervals.value.length, 0)
    }
  } finally { r.stop() }
})

await test('part switching keeps independent unsaved interval and input drafts', async () => {
  const r = mount()
  try {
    await flush()
    r.bindings.startText.value = '10'; r.bindings.endText.value = '20'; r.bindings.addInterval()
    r.bindings.startText.value = '30'
    r.bindings.selectedKey.value = 'BV1fixture|22'; await flush()
    assert.deepEqual(r.bindings.draftIntervals.value, [])
    r.bindings.startText.value = '2'; r.bindings.endText.value = '4'; r.bindings.addInterval()
    r.bindings.selectedKey.value = 'BV1fixture|11'; await flush()
    assert.equal(r.bindings.startText.value, '30')
    assert.deepEqual(r.bindings.draftIntervals.value, [{ startMs: 10_000, endMs: 20_000 }])
    assert.equal(r.calls.length, 0)
  } finally { r.stop() }
})

await test('cancel and reopen restore input text locally while unsaved intervals are discarded', async () => {
  const storage = new Map()
  const r = mount({ storage })
  try {
    await flush()
    r.bindings.startText.value = '10'; r.bindings.endText.value = '20'; r.bindings.addInterval()
    r.bindings.startText.value = ' 30 '; r.bindings.endText.value = '40'
    r.bindings.close(); r.input.open = false; await flush()
    assert.equal(r.calls.length, 0)
    const document = JSON.parse([...storage.values()][0])
    assert.equal(document.drafts[0].startText, '30'); assert.equal(document.drafts[0].endText, '40')
    assert.equal('intervals' in document.drafts[0], false)
    r.input.open = true; await flush()
    assert.equal(r.bindings.startText.value, '30'); assert.equal(r.bindings.endText.value, '40')
    assert.deepEqual(r.bindings.draftIntervals.value, [])
    r.bindings.startText.value = '1'.repeat(150); await flush()
    assert.equal(JSON.parse([...storage.values()][0]).drafts[0].startText.length, 128)
  } finally { r.stop() }
})

await test('local storage failures never block editing or saving rules', async () => {
  const r = mount({ storageFails: true })
  try {
    await flush()
    r.bindings.startText.value = '10'; r.bindings.endText.value = '20'; await flush(); r.bindings.addInterval()
    await r.bindings.saveIntervals()
    assert.equal(r.calls.length, 1); assert.deepEqual(r.updates, [false])
  } finally { r.stop() }
})

await test('missing Bilibili identity reports an error without requesting or guessing a part', async () => {
  const r = mount()
  try {
    await flush()
    r.input.track = { ...track('local:1'), source: 'local', album: '', syncPayload: {} }; await flush()
    assert.equal(r.bindings.loadError.value, 'player.bili_skip_identity_missing')
    assert.equal(r.bindings.loading.value, false); assert.equal(r.bindings.selectedTarget.value, null)
  } finally { r.stop() }
})

await test('sync changes cannot overwrite a locally edited draft', async () => {
  const r = mount()
  try {
    await flush()
    r.store.rules = [{ bvid: 'BV1fixture', cid: 11, intervals: [{ startMs: 2_000, endMs: 3_000 }] }]
    await flush()
    assert.deepEqual(r.bindings.draftIntervals.value, [{ startMs: 2_000, endMs: 3_000 }])
    r.bindings.startText.value = '10'; r.bindings.endText.value = '20'; r.bindings.addInterval()
    r.store.rules = [{ bvid: 'BV1fixture', cid: 11, intervals: [{ startMs: 4_000, endMs: 5_000 }] }]
    await flush()
    assert.deepEqual(r.bindings.draftIntervals.value, [{ startMs: 2_000, endMs: 3_000 }, { startMs: 10_000, endMs: 20_000 }])
  } finally { r.stop() }
})

await test('save failures keep edited intervals and only confirmed save closes the dialog', async () => {
  let fails = true
  const r = mount({ save: async () => { if (fails) throw new Error('disk'); return null } })
  try {
    await flush()
    r.bindings.startText.value = '10'; r.bindings.endText.value = '20'; r.bindings.addInterval()
    await r.bindings.saveIntervals()
    assert.equal(r.bindings.inputError.value, 'player.bili_skip_save_failed')
    assert.deepEqual(r.updates, [])
    assert.deepEqual(r.bindings.draftIntervals.value, [{ startMs: 10_000, endMs: 20_000 }])
    fails = false; await r.bindings.saveIntervals()
    assert.deepEqual(r.updates, [false]); assert.equal(r.calls[1].target.cid, 11)
  } finally { r.stop() }
})

await test('delete and clear stay local until saving the empty list', async () => {
  const r = mount({ saved: [{ bvid: 'BV1fixture', cid: 11, intervals: [{ startMs: 10_000, endMs: 20_000 }] }] })
  try {
    await flush()
    r.bindings.pendingDeletion.value = 0; r.bindings.confirmDelete()
    assert.deepEqual(r.bindings.draftIntervals.value, []); assert.equal(r.calls.length, 0)
    await r.bindings.saveIntervals()
    assert.deepEqual(r.calls[0].intervals, [])
  } finally { r.stop() }
})

await test('playback helpers require the exact active part and respect room control restrictions', async () => {
  const r = mount()
  try {
    await flush()
    r.bindings.setCurrentTime('start'); assert.equal(r.bindings.startText.value, '00:12')
    await r.bindings.movePlayback(-5_000); assert.deepEqual(r.seeks, [7_999])
    r.lt.localControlRestriction = 'member_control_disabled'
    await r.bindings.movePlayback(1_000); await r.bindings.togglePlayback()
    assert.equal(r.seeks.length, 1); assert.equal(r.player.isPlaying, true)
    r.lt.localControlRestriction = null
    r.bindings.selectedKey.value = 'BV1fixture|22'; await flush()
    r.bindings.setCurrentTime('end'); await r.bindings.movePlayback(1_000)
    assert.equal(r.bindings.endText.value, ''); assert.equal(r.seeks.length, 1)
  } finally { r.stop() }
})

await test('load failure uses an explicit target but never invents a CID', async () => {
  const r = mount({ load: async () => { throw new Error('offline') } })
  try {
    await flush()
    assert.equal(r.bindings.selectedTarget.value.cid, 11)
    assert.equal(r.bindings.loadError.value, 'player.bili_skip_load_failed')
    r.input.open = false; await flush()
    r.input.track = { ...track('bilibili:BV1fixture'), syncPayload: { audioId: 'BV1fixture' } }
    r.player.currentTrack = null; r.player.currentBiliVideoSkipTarget = null
    r.input.open = true; await flush()
    assert.equal(r.bindings.selectedTarget.value, null)
    await r.bindings.saveIntervals(); assert.equal(r.calls.length, 0)
  } finally { r.stop() }
})

await test('older asynchronous results cannot replace a newer track or reopen a closed dialog', async () => {
  const first = deferred(), second = deferred()
  let requests = 0
  const r = mount({ load: () => ++requests === 1 ? first.promise : second.promise })
  try {
    await flush()
    r.input.track = { ...track('bilibili:BV2fixture:33'), syncPayload: { audioId: 'BV2fixture', subAudioId: '33' } }
    await flush()
    second.resolve([{ bvid: 'BV2fixture', cid: 33, label: 'P1', durationMs: 30_000 }]); await flush()
    first.resolve([part(11)]); await flush()
    assert.equal(r.bindings.selectedTarget.value.bvid, 'BV2fixture')
    r.input.open = false; await flush()
    assert.equal(r.bindings.selectedTarget.value, null)
  } finally { r.stop() }
})

for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) {
  await test(`${locale} translates every skip dialog message`, async () => {
    const messages = JSON.parse(await readFile(new URL(`../src/i18n/${locale}.json`, import.meta.url), 'utf8'))
    for (const [, key] of source.matchAll(/['"]player\.(bili_skip_[a-z_]+)['"]/g)) {
      assert.equal(typeof messages.player[key], 'string', `${locale}.${key}`)
    }
  })
}
console.log(`Bili video skip dialog regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
