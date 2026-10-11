import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/LyricsView.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, {
  id: 'lyrics-foreground-test', genDefaultAs: 'component',
})
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const renderer = vue.createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText() {}, setElementText() {}, patchProp() {},
  parentNode: node => node.parent, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { node.parent.children = node.parent.children.filter(child => child !== node) },
})

class ListenerTarget {
  listeners = new Map()
  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set())
    this.listeners.get(type).add(listener)
  }
  removeEventListener(type, listener) { this.listeners.get(type)?.delete(listener) }
  dispatch(type) {
    for (const listener of [...(this.listeners.get(type) || [])]) listener({ type })
  }
  listenerCount() { return [...this.listeners.values()].reduce((count, set) => count + set.size, 0) }
}

async function mount(overrides = {}) {
  let now = 1000, frameId = 0
  const frames = new Map()
  const document = Object.assign(new ListenerTarget(), {
    visibilityState: 'visible', fonts: { status: 'loaded' },
  })
  Object.defineProperty(document, 'hidden', { get: () => document.visibilityState === 'hidden' })
  const window = new ListenerTarget()
  const host = Object.assign(new ListenerTarget(), {
    clientWidth: 700, clientHeight: 600, children: [],
    appendChild(node) { this.children.push(node); node.parentElement = this },
  })
  const players = []
  class FakeDomLyricPlayer extends ListenerTarget {
    size = [0, 0]
    currentLyricGroups = []
    lyricGroupSize = new WeakMap()
    times = []
    resumes = 0
    pauses = 0
    updates = 0
    element = { clientWidth: 700, clientHeight: 600 }
    constructor() { super(); players.push(this) }
    setOptimizeOptions() {}
    setLyricLines(lines) { this.lines = lines }
    getLyricLines() { return this.lines }
    getElement() { return this.element }
    setEnableBlur() {}
    setBlurAmount() {}
    setWordFadeWidth() {}
    setCurrentTime(time, seek = false) { this.times.push([time, seek]) }
    update() { this.updates++ }
    calcLayout() {}
    resetScroll() {}
    resume() { this.resumes++ }
    pause() { this.pauses++ }
    dispose() { this.disposed = true }
  }
  const settings = vue.reactive({
    advancedLyrics: true, showTranslation: true, showRomanization: true,
    lyricBlur: true, lyricBlurAmount: 1, lyricFontScale: 1,
  })
  const dependencies = {
    vue,
    '@amll-core/lyric-player/dom/index.ts': { DomLyricPlayer: FakeDomLyricPlayer },
    '@/stores/settings': { useSettingsStore: () => settings },
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@tauri-apps/plugin-clipboard-manager': { writeText: async () => {} },
    '@/components/ui/ContextMenu.vue': { default: {} },
    '@/utils/contextMenu': { createContextMenuItem: (label, options) => ({ label, ...options }) },
  }
  const component = new Function(
    'require', 'exports', 'document', 'window', 'performance', 'requestAnimationFrame',
    'cancelAnimationFrame', 'ResizeObserver', `${compiled}\nreturn component`,
  )(name => {
    assert.ok(name in dependencies, `unexpected dependency: ${name}`)
    return dependencies[name]
  }, {}, document, window, { now: () => now }, callback => {
    const id = ++frameId
    frames.set(id, callback)
    return id
  }, id => frames.delete(id), undefined)
  const setup = component.setup
  component.setup = (props, context) => {
    const state = setup(props, context)
    state.hostRef.value = host
    return () => null
  }
  const props = vue.reactive({
    lyrics: [{ startMs: 0, text: 'A long line', words: [{ startMs: 0, durationMs: 30000, text: 'A long line' }] }],
    currentTimeMs: 1000, isPlaying: true, lyricOffsetMs: 0, ...overrides,
  })
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...props }) })
  app.mount({ children: [] })
  const frame = (elapsed = 16) => {
    now += elapsed
    const pending = [...frames]
    for (const [id, callback] of pending) {
      if (!frames.delete(id)) continue
      callback(now)
    }
  }
  await vue.nextTick()
  for (let index = 0; index < 6; index++) frame()
  await vue.nextTick()
  assert.equal(players.length, 1)
  const player = players[0]
  const clearCalls = () => { player.times = []; player.resumes = 0; player.pauses = 0 }
  clearCalls()
  return {
    props, player, document, window, host, frames, frame, clearCalls,
    queueFrame(callback) { const id = ++frameId; frames.set(id, callback); return id },
    async setProps(patch) { now += 100; Object.assign(props, patch); await vue.nextTick() },
    async emit(target, type) { target.dispatch(type); await vue.nextTick(); frame(); await vue.nextTick() },
    stop: () => app.unmount(),
  }
}

let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('visible restoration seeks even when the background watcher already consumed the current time', async () => {
  const r = await mount()
  try {
    r.document.visibilityState = 'hidden'
    await r.emit(r.document, 'visibilitychange')
    await r.setProps({ currentTimeMs: 1400 })
    r.clearCalls()
    r.document.visibilityState = 'visible'
    await r.emit(r.document, 'visibilitychange')
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[1400, true]])
  } finally { r.stop() }
})

await test('focus restoration seeks without a visibility event', async () => {
  const r = await mount()
  try {
    await r.emit(r.window, 'blur')
    await r.setProps({ currentTimeMs: 1400 })
    r.clearCalls()
    await r.emit(r.window, 'focus')
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[1400, true]])
  } finally { r.stop() }
})

await test('restoration reads the latest function clock and coalesces visibility and focus events', async () => {
  const clock = vue.ref(1000)
  const r = await mount({ currentTimeMs: () => clock.value })
  try {
    r.document.visibilityState = 'hidden'
    await r.emit(r.document, 'visibilitychange')
    clock.value = 1200
    await vue.nextTick()
    r.clearCalls()
    r.document.visibilityState = 'visible'
    r.document.dispatch('visibilitychange')
    r.window.dispatch('focus')
    clock.value = 1400
    await vue.nextTick()
    r.frame()
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[1400, true]])
    assert.equal(r.player.resumes, 1)
  } finally { r.stop() }
})

await test('focus restoration waits for a playback clock frame queued after the existing lyric frame', async () => {
  const clock = vue.ref(1000)
  const r = await mount({ currentTimeMs: () => clock.value })
  try {
    r.queueFrame(() => { clock.value = 1400 })
    r.window.dispatch('focus')
    r.frame()
    await vue.nextTick()
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[1400, true]])
  } finally { r.stop() }
})

await test('restoring a paused player aligns lyrics without resuming playback', async () => {
  const r = await mount({ isPlaying: false, currentTimeMs: 5000 })
  try {
    r.document.visibilityState = 'hidden'
    await r.emit(r.document, 'visibilitychange')
    r.clearCalls()
    r.document.visibilityState = 'visible'
    await r.emit(r.document, 'visibilitychange')
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[5000, true]])
    assert.equal(r.player.resumes, 0)
  } finally { r.stop() }
})

await test('restoration wakes a paused player after its idle frame loop has stopped', async () => {
  const r = await mount({ isPlaying: false, currentTimeMs: 5000 })
  try {
    r.frame(3000)
    assert.equal(r.frames.size, 0)
    r.clearCalls()
    await r.emit(r.window, 'focus')
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[5000, true]])
    assert.equal(r.player.resumes, 0)
    assert.ok(r.frames.size > 0)
  } finally { r.stop() }
})

for (const [name, overrides, expected] of [
  ['source offset', { lyricOffsetMs: 700 }, 1700],
  ['preview time and offset', { previewTimeMs: 2500, lyricOffsetMs: 700 }, 3200],
]) {
  await test(`foreground synchronization uses ${name}`, async () => {
    const r = await mount(overrides)
    try {
      await r.emit(r.window, 'focus')
      assert.deepEqual(r.player.times.filter(([, seek]) => seek), [[expected, true]])
    } finally { r.stop() }
  })
}

await test('focus while hidden does not restore lyric animations', async () => {
  const r = await mount()
  try {
    r.document.visibilityState = 'hidden'
    await r.emit(r.document, 'visibilitychange')
    r.clearCalls()
    await r.emit(r.window, 'focus')
    assert.deepEqual(r.player.times, [])
    assert.equal(r.player.resumes, 0)
  } finally { r.stop() }
})

await test('normal playback updates do not force seek on every frame', async () => {
  const r = await mount()
  try {
    for (let time = 1040; time <= 2000; time += 40) {
      await r.setProps({ currentTimeMs: time })
      r.frame()
    }
    assert.ok(r.player.times.some(([, seek]) => !seek))
    assert.deepEqual(r.player.times.filter(([, seek]) => seek), [])
  } finally { r.stop() }
})

await test('unmount removes foreground listeners and scheduled frames', async () => {
  const r = await mount()
  r.stop()
  r.clearCalls()
  assert.equal(r.window.listenerCount(), 0)
  assert.equal(r.document.listenerCount(), 0)
  assert.equal(r.host.listenerCount(), 0)
  assert.equal(r.frames.size, 0)
  r.window.dispatch('focus')
  r.document.dispatch('visibilitychange')
  assert.deepEqual(r.player.times, [])
})

console.log(`Lyrics foreground regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
