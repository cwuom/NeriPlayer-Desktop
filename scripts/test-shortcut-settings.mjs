import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
const compile = source => ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const bindingSource = await read('../src/modules/shortcuts/bindings.ts')
function loadBindings(isMacPlatform = false) {
  const exports = {}
  new Function('require', 'exports', compile(bindingSource))(name => {
    assert.equal(name, './platform')
    return { isMacPlatform }
  }, exports)
  return exports
}
const bindings = loadBindings()
const source = await read('../src/components/settings/ShortcutSettings.vue')
const script = compileScript(parse(source).descriptor, { id: 'shortcut-settings-test', genDefaultAs: 'component' })
const compiled = compile(script.content)
const renderer = vue.createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText() {}, setElementText() {}, patchProp() {},
  parentNode: node => node.parent, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { node.parent.children = node.parent.children.filter(child => child !== node) },
})

function keyboardEvent(overrides = {}) {
  return {
    key: ' ', code: 'Space', ctrlKey: false, metaKey: false, altKey: false, shiftKey: false,
    repeat: false, isComposing: false, defaultPrevented: false, stopped: false,
    preventDefault() { this.defaultPrevented = true },
    stopImmediatePropagation() { this.stopped = true },
    ...overrides,
  }
}

function mount() {
  const settings = vue.reactive({ shortcutBindings: { local: {}, global: {} }, globalShortcutsEnabled: true })
  const shortcutRecording = vue.ref(false)
  const listeners = new Set()
  const notices = []
  const document = {
    addEventListener(type, listener, capture) {
      assert.ok(['keydown', 'mousedown'].includes(type), `unexpected event type: ${type}`)
      assert.equal(capture, true)
      listeners.add(listener)
    },
    removeEventListener(type, listener, capture) {
      assert.ok(['keydown', 'mousedown'].includes(type), `unexpected event type: ${type}`)
      assert.equal(capture, true)
      listeners.delete(listener)
    },
  }
  const dependencies = {
    vue,
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/stores/toast': { useToastStore: () => ({ show: message => notices.push(message) }) },
    '@/modules/shortcuts/bindings': bindings,
    '@/modules/shortcuts/recording': { shortcutRecording },
    '@/modules/shortcuts/systemShortcuts': { systemShortcutFailures: vue.ref([]) },
    '@/utils/platform': { detectPlatform: () => 'windows' },
  }
  const component = new Function('require', 'exports', 'document', `${compiled}\nreturn component`)(name => {
    assert.ok(name in dependencies, `unexpected dependency: ${name}`)
    return dependencies[name]
  }, {}, document)
  let state
  const setup = component.setup
  component.setup = (props, context) => { state = setup(props, context); return () => null }
  const app = renderer.createApp(component)
  app.mount({ children: [] })
  return {
    state, settings, shortcutRecording, listeners, notices,
    press(overrides) {
      const event = keyboardEvent(overrides)
      for (const listener of [...listeners]) listener(event)
      return event
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

await test('Ctrl+Space normalizes to a supported global accelerator for standard and legacy key values', () => {
  for (const event of [{}, { key: 'Spacebar' }, { code: '' }]) {
    const combo = bindings.comboFromEvent(keyboardEvent({ ctrlKey: true, ...event }))
    assert.equal(combo, 'Mod+Space')
    assert.equal(bindings.toAccelerator(combo), 'CommandOrControl+Space')
  }
})

await test('physical Space survives IME Process and unidentified key values without losing modifiers', () => {
  for (const key of ['Process', 'Unidentified', '', '\u3000']) {
    const combo = bindings.comboFromEvent(keyboardEvent({ key, ctrlKey: true, isComposing: true }))
    assert.equal(combo, 'Mod+Space')
    assert.equal(bindings.toAccelerator(combo), 'CommandOrControl+Space')
  }
  const mac = loadBindings(true)
  assert.equal(mac.comboFromEvent(keyboardEvent({ key: 'Process', metaKey: true, ctrlKey: true, altKey: true, shiftKey: true })), 'Mod+Ctrl+Alt+Shift+Space')
})

for (const key of [' ', 'Process']) {
  await test(`global recording saves Ctrl+Space when key is ${JSON.stringify(key)}`, () => {
    const r = mount()
    try {
      r.state.startRecording('toggle_play', 'global')
      const event = r.press({ key, ctrlKey: true, isComposing: key === 'Process' })
      assert.deepEqual(vue.toRaw(r.settings.shortcutBindings), { local: {}, global: { toggle_play: 'Mod+Space' } })
      assert.deepEqual(bindings.normalizeShortcutBindings(JSON.parse(JSON.stringify(r.settings.shortcutBindings))), { local: {}, global: { toggle_play: 'Mod+Space' } })
      assert.equal(r.state.recordError.value, '')
      assert.equal(r.shortcutRecording.value, false)
      assert.equal(r.listeners.size, 0)
      assert.equal(event.defaultPrevented, true)
      assert.equal(event.stopped, true)
    } finally { r.stop() }
  })
}

await test('Space still requires a modifier for global recording even when the IME changes its key value', () => {
  const r = mount()
  try {
    r.state.startRecording('toggle_play', 'global')
    r.press({ key: 'Process', isComposing: true })
    assert.deepEqual(vue.toRaw(r.settings.shortcutBindings), { local: {}, global: {} })
    assert.equal(r.state.recordError.value, 'shortcuts.global_needs_modifier')
    assert.equal(r.shortcutRecording.value, true)
  } finally { r.stop() }
  assert.equal(r.shortcutRecording.value, false)
  assert.equal(r.listeners.size, 0)
})

await test('modifier-only and repeated events do not record, and Escape cancels recording', () => {
  const r = mount()
  try {
    r.state.startRecording('toggle_play', 'global')
    r.press({ key: 'Control', code: 'ControlLeft', ctrlKey: true })
    r.press({ key: 'Process', ctrlKey: true, repeat: true })
    assert.equal(r.shortcutRecording.value, true)
    assert.deepEqual(vue.toRaw(r.settings.shortcutBindings), { local: {}, global: {} })
    r.press({ key: 'Escape', code: 'Escape' })
    assert.equal(r.shortcutRecording.value, false)
    assert.equal(r.listeners.size, 0)
  } finally { r.stop() }
})

await test('physical letter and digit shortcuts and reserved keys keep their existing behavior', () => {
  const mac = loadBindings(true)
  assert.equal(mac.comboFromEvent(keyboardEvent({ key: '\u03c0', code: 'KeyP', altKey: true })), 'Alt+P')
  assert.equal(bindings.comboFromEvent(keyboardEvent({ key: '!', code: 'Digit1', ctrlKey: true, shiftKey: true })), 'Mod+Shift+1')
  assert.equal(bindings.comboFromEvent(keyboardEvent({ key: 'Tab', code: 'Tab', ctrlKey: true })), null)
  assert.equal(bindings.comboFromEvent(keyboardEvent({ key: 'Escape', code: 'Escape' })), null)
})

console.log(`Shortcut settings regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
