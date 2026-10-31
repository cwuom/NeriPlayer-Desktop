import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const componentUrl = new URL('../src/components/settings/CookieLoginDialog.vue', import.meta.url)
assert.ok(existsSync(componentUrl), 'Cookie login dialog must exist before it can handle account input')

const listeners = new Set()
globalThis.Document = class Document {}
globalThis.ShadowRoot = class ShadowRoot {}
globalThis.document = Object.assign(new Document(), {
  activeElement: null,
  addEventListener(type, listener) { assert.equal(type, 'keydown'); listeners.add(listener) },
  removeEventListener(type, listener) { assert.equal(type, 'keydown'); listeners.delete(listener) },
})
const forbiddenStorage = new Proxy({}, { get() { throw new Error('Cookie drafts must not access local storage') } })
const slotWrapper = vue.defineComponent({ inheritAttrs: false, setup: (_, { slots }) => () => slots.default?.() })
const runtime = { ...vue, Transition: slotWrapper }
const translate = (key, params) => params?.platform ? `${key}: ${params.platform}` : key

function transpile(source) {
  return ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
}
const escapeSource = await readFile(new URL('../src/composables/useEscapeClose.ts', import.meta.url), 'utf8')
const escapeExports = {}
new Function('require', 'exports', transpile(escapeSource))(require, escapeExports)

async function loadComponent(url, dependencies) {
  const source = await readFile(url, 'utf8')
  const { descriptor, errors } = parse(source)
  assert.deepEqual(errors, [])
  const script = compileScript(descriptor, { id: 'cookie-login-test', genDefaultAs: 'component', inlineTemplate: true })
  return new Function('require', 'exports', 'localStorage', `${transpile(script.content)}\nreturn component`)(
    name => dependencies[name] ?? require(name), {}, forbiddenStorage,
  )
}
const dialog = await loadComponent(new URL('../src/components/ui/M3Dialog.vue', import.meta.url), {
  vue: runtime,
  'vue-i18n': { useI18n: () => ({ t: translate }) },
  '@/composables/useEscapeClose': escapeExports,
})

function node(type, text = '') {
  return {
    type, tagName: type.toUpperCase(), text, value: '', props: {}, children: [], handlers: new Map(),
    getRootNode() { return globalThis.document },
    addEventListener(name, handler) { this.handlers.set(name, handler) },
    removeEventListener(name) { this.handlers.delete(name) },
    dispatchEvent(event) { this.handlers.get(event.type)?.(event) },
  }
}
let teleportTarget
const renderer = vue.createRenderer({
  createElement: type => node(type), createText: text => node('#text', text),
  createComment: text => node('#comment', text),
  setText(target, text) { target.text = text },
  setElementText(target, text) { target.text = text; target.children = [] },
  patchProp(target, key, _, value) { target.props[key] = value },
  parentNode: target => target.parent ?? null,
  nextSibling: target => target.parent?.children[target.parent.children.indexOf(target) + 1] ?? null,
  insert(target, parent, anchor) {
    if (target.parent) target.parent.children = target.parent.children.filter(child => child !== target)
    target.parent = parent
    const index = parent.children.indexOf(anchor)
    if (index < 0) parent.children.push(target)
    else parent.children.splice(index, 0, target)
  },
  remove(target) {
    if (target.parent) target.parent.children = target.parent.children.filter(child => child !== target)
    target.parent = null
  },
  querySelector: () => teleportTarget,
})
const deferred = () => {
  let resolve, reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  return { promise, resolve, reject }
}
async function flush() {
  for (let index = 0; index < 6; index++) { await Promise.resolve(); await vue.nextTick() }
}
function allNodes(root) {
  return [root, ...root.children.flatMap(allNodes)]
}
function pressEscape() {
  const event = { key: 'Escape', defaultPrevented: false, preventDefault() { this.defaultPrevented = true } }
  for (const listener of listeners) listener(event)
}
async function mount({ platform = 'netease', login = async () => true } = {}) {
  const calls = [], updates = []
  const component = await loadComponent(componentUrl, {
    vue: runtime,
    'vue-i18n': { useI18n: () => ({ t: translate }) },
    '@/stores/auth': { useAuthStore: () => ({ loginWithCookies: (...args) => { calls.push(args); return login(...args) } }) },
    '@/components/ui/M3Dialog.vue': { default: dialog },
  })
  const props = vue.reactive({
    open: true, platform,
    'onUpdate:open': value => { updates.push(value); props.open = value },
  })
  const root = node('body')
  teleportTarget = root
  const app = renderer.createApp({ setup: () => () => vue.h(component, props) })
  app.mount(node('root'))
  const find = predicate => allNodes(root).find(predicate)
  const textarea = () => find(target => target.type === 'textarea')
  const button = kind => find(target => target.type === 'button' && String(target.props.class).includes(kind))
  const input = async value => {
    const target = textarea()
    assert.ok(target, 'Open cookie dialog must render a textarea')
    target.value = value
    target.dispatchEvent({ type: 'input', target })
    await flush()
  }
  const submit = async () => { button('filled').props.onClick(); await flush() }
  const cancel = async () => { button('text').props.onClick(); await flush() }
  return { calls, updates, props, root, textarea, button, input, submit, cancel, stop: () => app.unmount() }
}

let failed = 0, total = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('platform dialogs use a short title, platform identity and compact cookie format hints', async () => {
  for (const [platform, keys] of [
    ['netease', ['MUSIC_U']],
    ['bilibili', ['SESSDATA']],
    ['youtube', ['SAPISID', '__Secure-1PAPISID', '__Secure-3PAPISID']],
  ]) {
    const r = await mount({ platform })
    try {
      const nodes = allNodes(r.root)
      assert.equal(nodes.find(target => target.props.class === 'dialog-title')?.text, 'settings.cookie_login')
      assert.equal(nodes.find(target => target.props.class === 'cookie-platform-name')?.text, `settings.${platform}_account`)
      assert.equal(nodes.find(target => target.props.class === 'cookie-platform-icon')?.props.style.maskImage, `url(/icons/ic_${platform}.svg)`)
      const format = nodes.find(target => target.props.class === 'cookie-format-card')
      assert.ok(format, 'format instructions use a separate information panel')
      assert.deepEqual(allNodes(format).filter(target => target.type === 'code').map(target => target.text), keys)
      assert.ok(nodes.some(target => target.text === 'settings.cookie_login_private'))
      assert.equal(r.textarea().props['aria-label'], 'settings.cookie_login_content')
    } finally { r.stop() }
  }
})

await test('empty input cannot submit even when confirm is invoked directly', async () => {
  const r = await mount()
  try {
    await r.submit()
    await r.input(' \n\t ')
    await r.submit()
    assert.equal(r.calls.length, 0)
    assert.match(String(r.button('filled').props.class), /disabled/)
    assert.deepEqual(r.updates, [])
  } finally { r.stop() }
})

await test('each platform submits its raw cookie input and closes only after success', async () => {
  for (const [platform, key] of [['netease', 'MUSIC_U'], ['bilibili', 'SESSDATA'], ['youtube', 'SAPISID']]) {
    const pending = deferred()
    const r = await mount({ platform, login: () => pending.promise })
    try {
      const value = ` Cookie: ${key}=synthetic-cookie; other=a=b\n `
      await r.input(value)
      await r.submit()
      assert.deepEqual(r.calls, [[platform, value]])
      assert.equal(r.props.open, true)
      pending.resolve(true); await flush()
      assert.deepEqual(r.updates, [false])
      r.props.open = true; await flush()
      assert.equal(r.textarea().value, '')
    } finally { r.stop() }
  }
})

await test('rejected login preserves input and allows a corrected retry', async () => {
  let accepted = false
  const r = await mount({ login: async () => accepted })
  try {
    await r.input('MUSIC_U=synthetic-rejected')
    await r.submit()
    assert.equal(r.props.open, true)
    assert.equal(r.textarea().value, 'MUSIC_U=synthetic-rejected')
    assert.deepEqual(r.updates, [])
    accepted = true
    await r.input('MUSIC_U=synthetic-corrected')
    await r.submit()
    assert.deepEqual(r.calls, [['netease', 'MUSIC_U=synthetic-rejected'], ['netease', 'MUSIC_U=synthetic-corrected']])
    assert.equal(r.props.open, false)
  } finally { r.stop() }
})

await test('pending login prevents repeat submissions and cancel, overlay or Escape dismissal', async () => {
  const pending = deferred()
  const r = await mount({ login: () => pending.promise })
  try {
    await r.input('MUSIC_U=synthetic-pending')
    await r.submit(); await r.submit(); await r.cancel()
    const overlay = allNodes(r.root).find(target => target.props.class === 'm3-dialog-overlay')
    overlay.props.onClick({ target: overlay }); pressEscape(); await flush()
    assert.equal(r.calls.length, 1)
    assert.equal(r.props.open, true)
    assert.equal(r.textarea().props.disabled, true)
    assert.deepEqual(r.updates, [])
    pending.resolve(false); await flush()
    assert.equal(r.textarea().props.disabled, false)
    await r.cancel()
    assert.equal(r.props.open, false)
  } finally { pending.resolve(false); r.stop() }
})

await test('cancel and parent closure clear cookies before the dialog is reopened', async () => {
  const r = await mount()
  try {
    await r.input('MUSIC_U=synthetic-cancelled'); await r.cancel()
    r.props.open = true; await flush()
    assert.equal(r.textarea().value, '')
    await r.input('MUSIC_U=synthetic-external-close')
    r.props.open = false; await flush(); r.props.open = true; await flush()
    assert.equal(r.textarea().value, '')
    assert.equal(r.calls.length, 0)
  } finally { r.stop() }
})

await test('switching platforms clears drafts and stale success cannot dismiss a new dialog', async () => {
  const pending = deferred()
  const r = await mount({ login: () => pending.promise })
  try {
    await r.input('MUSIC_U=synthetic-old-account'); await r.submit()
    r.props.open = false; await flush(); r.props.platform = 'bilibili'; r.props.open = true; await flush()
    assert.equal(r.textarea().value, '')
    pending.resolve(true); await flush()
    assert.equal(r.props.open, true)
    assert.deepEqual(r.updates, [])
    await r.input('SESSDATA=synthetic-draft')
    r.props.platform = 'youtube'; await flush()
    assert.equal(r.textarea().value, '')
  } finally { pending.resolve(false); r.stop() }
})

assert.equal(listeners.size, 0, 'Dialog unmount must remove Escape listeners')
console.log(`Cookie login dialog tests: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
