import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { createRenderer, h, nextTick, reactive, ref } from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const source = await readFile(new URL('../src/components/ui/CustomSelect.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, { id: 'select-test', genDefaultAs: 'component' })
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const renderer = createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText() {}, setElementText() {}, patchProp() {},
  parentNode: node => node.parent, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { node.parent.children = node.parent.children.filter(child => child !== node) },
})
const options = [{ value: '', label: 'System default' }, { value: 'headphones', label: 'Headphones' }, { value: 'speakers', label: 'Speakers' }]

function mount(overrides = {}, rect = { left: 100, top: 100, bottom: 140, width: 220 }) {
  const listeners = new Map()
  const document = {
    addEventListener(type, listener) { listeners.set(`document:${type}`, listener) },
    removeEventListener(type) { listeners.delete(`document:${type}`) },
  }
  const window = {
    innerWidth: 640, innerHeight: 420,
    addEventListener(type, listener) { listeners.set(`window:${type}`, listener) },
    removeEventListener(type) { listeners.delete(`window:${type}`) },
  }
  const component = new Function('require', 'exports', 'window', 'document', `${compiled}\nreturn component`)(require, {}, window, document)
  const setup = component.setup
  let bindings, props, focusCount = 0
  const updates = [], opens = []
  component.setup = (input, context) => { props = input; bindings = setup(input, context); return () => null }
  const input = reactive({
    modelValue: '', options, ...overrides,
    'onUpdate:modelValue': value => updates.push(value), onOpen: () => opens.push(true),
  })
  const app = renderer.createApp({ setup: () => () => h(component, input) })
  app.mount({ children: [] })
  bindings.triggerRef.value = {
    getBoundingClientRect: () => rect, contains: target => target === 'trigger',
    focus: () => { focusCount++ },
  }
  bindings.menuRef.value = { contains: target => target === 'menu', querySelector: () => ({ scrollIntoView() {} }) }
  return { bindings, props, input, updates, opens, listeners, window, focusCount: () => focusCount, stop: () => app.unmount() }
}
function key(select, value) {
  let prevented = false
  select.bindings.handleKeydown({ key: value, preventDefault: () => { prevented = true }, stopPropagation() {} })
  return prevented
}
let failed = 0
let total = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

await test('disabled selectors cannot open or emit a selection', async () => {
  const select = mount({ disabled: true })
  try {
    select.bindings.toggle()
    assert.equal(select.bindings.isOpen.value, false)
    select.bindings.select('speakers')
    assert.deepEqual(select.updates, [])
  } finally { select.stop() }
})
await test('arrows navigate without changing the setting until Enter confirms', async () => {
  const select = mount()
  try {
    assert.equal(key(select, 'ArrowDown'), true)
    await nextTick()
    key(select, 'ArrowDown')
    assert.deepEqual(select.updates, [])
    key(select, 'Enter')
    assert.deepEqual(select.updates, ['headphones'])
    assert.equal(select.bindings.isOpen.value, false)
    assert.equal(select.focusCount(), 1)
    assert.equal(select.opens.length, 1)
  } finally { select.stop() }
})
await test('Home and End navigate while Escape and Tab cancel without a value change', async () => {
  const select = mount({ modelValue: 'headphones' })
  try {
    key(select, 'End'); key(select, 'Enter')
    assert.deepEqual(select.updates, ['speakers'])
    key(select, 'Home'); key(select, 'Escape')
    assert.equal(select.bindings.isOpen.value, false)
    key(select, 'ArrowUp')
    assert.equal(key(select, 'Tab'), false, 'Tab must keep normal focus traversal')
    assert.equal(select.bindings.isOpen.value, false)
    assert.deepEqual(select.updates, ['speakers'])
  } finally { select.stop() }
})
await test('right and bottom edge menus stay within the viewport and open upwards', async () => {
  const select = mount({}, { left: 580, top: 350, bottom: 390, width: 240 })
  try {
    select.bindings.toggle(); await nextTick()
    const style = select.bindings.menuStyle.value
    assert.ok(parseFloat(style.left) >= 8)
    assert.ok(parseFloat(style.left) + parseFloat(style.width ?? style.minWidth) <= 632)
    assert.equal(style.bottom, '74px')
    assert.ok(parseFloat(style.maxHeight) > 0)
  } finally { select.stop() }
})
await test('clicking the menu preserves it while outside clicks and parent scrolling close it', async () => {
  const select = mount()
  try {
    select.bindings.toggle(); await nextTick()
    select.listeners.get('document:pointerdown')({ target: 'menu' })
    assert.equal(select.bindings.isOpen.value, true)
    select.listeners.get('document:pointerdown')({ target: 'outside' })
    assert.equal(select.bindings.isOpen.value, false)
    select.bindings.toggle(); await nextTick()
    select.listeners.get('window:scroll')({ target: 'menu' })
    assert.equal(select.bindings.isOpen.value, true)
    select.listeners.get('window:scroll')({ target: 'settings-content' })
    assert.equal(select.bindings.isOpen.value, false)
  } finally { select.stop() }
  assert.equal(select.listeners.size, 0, 'unmount must release all document and window listeners')
})
await test('reselecting the current option preserves existing EQ preset activation', async () => {
  const select = mount({ modelValue: 'headphones' })
  try {
    select.bindings.toggle(); await nextTick()
    select.bindings.select('headphones')
    assert.equal(select.bindings.isOpen.value, false)
    assert.deepEqual(select.updates, ['headphones'])
  } finally { select.stop() }
})
await test('focused selectors isolate player shortcuts without blocking normal Tab navigation', async () => {
  const select = mount()
  try {
    for (const value of ['m', 's', 'r', 'ArrowLeft', 'ArrowRight']) {
      let stopped = false
      select.bindings.handleKeydown({ key: value, preventDefault() {}, stopPropagation: () => { stopped = true } })
      assert.equal(stopped, true, `${value} must not reach the global player shortcut handler`)
    }
    assert.deepEqual(select.updates, [])
  } finally { select.stop() }
})
await test('a pending device switch closes an open menu when the selector becomes disabled', async () => {
  const select = mount()
  try {
    select.bindings.toggle(); await nextTick()
    select.input.disabled = true; await nextTick()
    assert.equal(select.bindings.isOpen.value, false)
    select.bindings.select('speakers')
    assert.deepEqual(select.updates, [])
  } finally { select.stop() }
})
await test('device refresh preserves the highlighted value and empty results close the menu', async () => {
  const select = mount({ modelValue: 'headphones' })
  try {
    select.bindings.toggle(); await nextTick()
    select.input.options = [{ value: 'new', label: 'New output' }, ...options]
    await nextTick()
    key(select, 'Enter')
    assert.deepEqual(select.updates, ['headphones'])
    select.bindings.toggle(); await nextTick()
    select.input.options = []; await nextTick()
    assert.equal(select.bindings.isOpen.value, false)
  } finally { select.stop() }
})

async function audioSettings() {
  const source = await readFile(new URL('../src/views/SettingsView.vue', import.meta.url), 'utf8')
  const script = ts.createSourceFile('settings.ts', parse(source).descriptor.scriptSetup.content, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const functions = script.statements.filter(node => ts.isFunctionDeclaration(node) && ['changeAudioOutputDevice', 'loadAudioOutputDevices'].includes(node.name?.text))
  const compiled = ts.transpileModule(functions.map(node => ts.createPrinter().printNode(ts.EmitHint.Unspecified, node, script)).join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  const calls = [], errors = []
  let resolve, reject
  const pending = new Promise((accept, fail) => { resolve = accept; reject = fail })
  const dependencies = {
    audioOutputDevice: ref('headphones'), audioOutputDevices: ref([]), audioOutputSwitching: ref(false),
    invoke: (command, input) => { calls.push({ command, input }); return command === 'set_audio_output_device' ? pending : Promise.resolve([]) },
    t: key => key, log: { warn() {} }, toast: { error: message => errors.push(message) },
  }
  const change = new Function(...Object.keys(dependencies), `${compiled}\nreturn changeAudioOutputDevice`)(...Object.values(dependencies))
  return { ...dependencies, change, calls, errors, resolve, reject }
}
await test('device selection saves only after backend confirmation and ignores duplicate or overlapping requests', async () => {
  const settings = await audioSettings()
  await settings.change('headphones')
  assert.equal(settings.calls.length, 0)
  const change = settings.change('speakers')
  assert.equal(settings.audioOutputSwitching.value, true)
  assert.equal(settings.audioOutputDevice.value, 'headphones')
  await settings.change('other')
  assert.deepEqual(settings.calls, [{ command: 'set_audio_output_device', input: { name: 'speakers' } }])
  settings.resolve(); await change
  assert.equal(settings.audioOutputDevice.value, 'speakers')
  assert.equal(settings.audioOutputSwitching.value, false)
  assert.equal(settings.calls[1].command, 'list_audio_output_devices')
})
await test('failed system-default selection sends null and retains the last confirmed device', async () => {
  const settings = await audioSettings()
  const change = settings.change('')
  assert.deepEqual(settings.calls[0], { command: 'set_audio_output_device', input: { name: null } })
  settings.reject(new Error('device unavailable')); await change
  assert.equal(settings.audioOutputDevice.value, 'headphones')
  assert.equal(settings.audioOutputSwitching.value, false)
  assert.deepEqual(settings.errors, ['settings.audio_output_failed'])
  assert.equal(settings.calls[1].command, 'list_audio_output_devices')
})
console.log(`Custom select regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
