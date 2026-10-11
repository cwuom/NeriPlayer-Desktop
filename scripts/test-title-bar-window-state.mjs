import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/TitleBar.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, { id: 'title-bar-state-test', genDefaultAs: 'component' })
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
const flush = async () => { for (let index = 0; index < 12; index++) await Promise.resolve(); await vue.nextTick() }

function mount({ mac = true, fullscreen = false, register, query } = {}) {
  const classes = new Set(), timers = new Map(), listeners = new Set()
  let timerId = 0, state, fullscreenValue = fullscreen, fullscreenQueries = 0, releases = 0
  const nativeWindow = {
    isMaximized: () => Promise.resolve(false),
    isFullscreen: () => { fullscreenQueries++; return query ? query() : Promise.resolve(fullscreenValue) },
    onResized: async listener => {
      listeners.add(listener)
      const release = () => { releases++; listeners.delete(listener) }
      return register ? register(release) : release
    },
  }
  const document = { documentElement: { classList: {
    toggle(name, enabled) { if (enabled) classes.add(name); else classes.delete(name) },
    remove(name) { classes.delete(name) },
  } } }
  const dependencies = {
    vue, 'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@tauri-apps/api/window': { getCurrentWindow: () => nativeWindow },
    '@/modules/shortcuts/platform': { isMacPlatform: mac },
  }
  const component = new Function('require', 'exports', 'document', 'setTimeout', 'clearTimeout', `${compiled}\nreturn component`)(
    name => { assert.ok(name in dependencies, `unexpected dependency: ${name}`); return dependencies[name] },
    {}, document, callback => { timers.set(++timerId, callback); return timerId }, id => timers.delete(id),
  )
  const setup = component.setup
  component.setup = (props, context) => { state = setup(props, context); return () => null }
  const app = renderer.createApp(component)
  app.mount({ children: [] })
  return {
    state, classes, listeners, timers, queries: () => fullscreenQueries, releases: () => releases,
    async resize(value) {
      fullscreenValue = value
      for (const listener of listeners) listener()
      for (const [id, timer] of [...timers]) { timers.delete(id); timer() }
      await flush()
    },
    stop: () => app.unmount(),
  }
}

let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('macOS fullscreen at startup uses the compact title bar', async () => {
  const window = mount({ fullscreen: true })
  try {
    await flush()
    assert.equal(window.classes.has('window-fullscreen'), true)
    assert.equal(window.state.isFullscreen.value, true)
  } finally { window.stop() }
})

await test('native fullscreen entry and exit update the title bar and restore its traffic-light space', async () => {
  const window = mount()
  try {
    await flush()
    assert.equal(window.classes.has('window-fullscreen'), false)
    await window.resize(true)
    assert.equal(window.classes.has('window-fullscreen'), true)
    assert.equal(window.state.isFullscreen.value, true)
    await window.resize(false)
    assert.equal(window.classes.has('window-fullscreen'), false)
    assert.equal(window.state.isFullscreen.value, false)
  } finally { window.stop() }
  assert.equal(window.listeners.size, 0)
  assert.equal(window.timers.size, 0)
})

await test('other platforms retain their existing title-bar behavior', async () => {
  const window = mount({ mac: false, fullscreen: true })
  try {
    await flush()
    await window.resize(true)
    assert.equal(window.queries(), 0)
    assert.equal(window.classes.has('window-fullscreen'), false)
    assert.equal(window.state.isMaximized.value, false)
  } finally { window.stop() }
})

await test('unmount before native listener registration completes still releases the listener', async () => {
  let finishRegistration
  const window = mount({ register: release => new Promise(resolve => { finishRegistration = () => resolve(release) }) })
  await flush()
  window.stop()
  finishRegistration()
  await flush()
  assert.equal(window.listeners.size, 0)
  assert.equal(window.releases(), 1)
  assert.equal(window.classes.has('window-fullscreen'), false)
})

await test('a fullscreen query completing after unmount cannot restore fullscreen layout', async () => {
  let finishQuery
  const window = mount({ query: () => new Promise(resolve => { finishQuery = resolve }) })
  await flush()
  try {
    assert.equal(typeof finishQuery, 'function', 'macOS must query native fullscreen state')
    window.stop()
    finishQuery(true)
    await flush()
    assert.equal(window.classes.has('window-fullscreen'), false)
    assert.equal(window.listeners.size, 0)
  } finally { window.stop() }
})

console.log(`Title bar window state: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
