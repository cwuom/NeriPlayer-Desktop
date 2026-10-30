import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/settings/ConfigTransferDialogs.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, { id: 'config-transfer-test', genDefaultAs: 'component', inlineTemplate: true })
const compiled = ts.transpileModule(script.content, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const dialog = {}
const calls = []
const queue = []
const store = {
  async exportConfig(options) { calls.push(['export', options]); return queue.shift() ?? { success: true } },
  async importConfig(options) { calls.push(['import', options]); return queue.shift() ?? { success: false } },
}
const mocks = {
  vue,
  'vue-i18n': { useI18n: () => ({ t: key => key }) },
  '@/stores/sync': { useSyncStore: () => store },
  '@/components/ui/M3Dialog.vue': { default: {
    props: ['open', 'title', 'confirmText', 'confirmDisabled'], emits: ['update:open', 'confirm'],
    setup(props, { emit, slots }) {
      dialog.props = props
      dialog.confirm = () => emit('confirm')
      dialog.close = () => emit('update:open', false)
      return () => props.open ? vue.h('dialog', {}, slots.default?.()) : null
    },
  } },
  '@/components/ui/M3Input.vue': { default: {
    props: ['modelValue', 'label', 'type', 'disabled', 'autocomplete', 'maxlength'],
    emits: ['update:modelValue', 'enter'],
    setup(props, { emit }) {
      return () => vue.h('input', {
        type: props.type, value: props.modelValue, disabled: props.disabled,
        autocomplete: props.autocomplete, maxlength: props.maxlength, 'aria-label': props.label,
        onInput: event => emit('update:modelValue', event.target.value),
      })
    },
  } },
}
const component = new Function('require', 'exports', `${compiled}\nreturn component`)(name => {
  assert.ok(name in mocks, `Unexpected dependency: ${name}`)
  return mocks[name]
}, {})
const renderer = vue.createRenderer({
  createElement: type => ({ type, children: [], props: {} }), createText: text => ({ text }), createComment: text => ({ text }),
  setText(node, text) { node.text = text }, setElementText(node, text) { node.text = text; node.children = [] },
  patchProp(node, key, _, value) { node.props[key] = value }, parentNode: node => node.parent ?? null,
  nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  insert(node, parent, anchor = null) {
    node.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
  },
  remove(node) { node.parent.children.splice(node.parent.children.indexOf(node), 1) },
})
const mode = vue.ref(null)
const imported = []
const tree = { children: [] }
const app = renderer.createApp({ setup: () => () => vue.h(component, {
  mode: mode.value, 'onUpdate:mode': value => { mode.value = value }, onImported: () => imported.push(true),
}) })
app.mount(tree)
const flush = async () => { await vue.nextTick(); await Promise.resolve(); await vue.nextTick() }
function findAll(predicate, node = tree) {
  return [...(predicate(node) ? [node] : []), ...(node.children ?? []).flatMap(child => findAll(predicate, child))]
}
const input = (label, value) => {
  const node = findAll(node => node.type === 'input' && node.props['aria-label'] === label)[0]
  assert.ok(node, `Missing input ${label}`)
  node.props.onInput({ target: { value } })
}
try {
  mode.value = 'export'
  await flush()
  assert.equal(dialog.props.open, true)
  assert.equal(dialog.props.confirmDisabled, false, 'export defaults to no encryption and no credentials')
  assert.equal(findAll(node => node.type === 'input' && node.props.type === 'password').length, 0)
  assert.equal(findAll(node => node.text === 'settings.export_config_warning_desc').length, 0)
  assert.equal(findAll(node => node.text === 'settings.config_plaintext_warning').length, 0)
  assert.equal(findAll(node => node.props?.class === 'm3-switch').length, 2, 'options reuse the application switches')
  dialog.confirm()
  await flush()
  assert.deepEqual(calls.at(-1), ['export', { password: undefined, includeLoginState: false }])
  assert.equal(mode.value, null)

  mode.value = 'export'
  await flush()
  let encrypted = findAll(node => node.type === 'input' && node.props.type === 'checkbox')[0]
  encrypted.props.onChange({ target: { checked: true } })
  await flush()
  assert.equal(dialog.props.confirmDisabled, true, 'opting into encryption requires a password')
  assert.equal(findAll(node => node.text === 'settings.export_config_warning_desc').length, 0)
  input('settings.config_password', ' 密码 🔑 ')
  input('settings.config_password_confirm', 'different')
  await flush()
  assert.equal(dialog.props.confirmDisabled, true, 'mismatched passwords must not export')
  input('settings.config_password_confirm', ' 密码 🔑 ')
  await flush()
  assert.equal(dialog.props.confirmDisabled, false)
  dialog.confirm()
  await flush()
  assert.deepEqual(calls.at(-1), ['export', { password: ' 密码 🔑 ', includeLoginState: false }])
  assert.equal(mode.value, null)

  mode.value = 'export'
  await flush()
  assert.equal(dialog.props.confirmDisabled, false, 'reopening resets encryption to off')
  encrypted = findAll(node => node.type === 'input' && node.props.type === 'checkbox')[0]
  encrypted.props.onChange({ target: { checked: true } })
  await flush()
  assert.equal(dialog.props.confirmDisabled, true, 'passwords must clear after closing')
  encrypted.props.onChange({ target: { checked: false } })
  await flush()
  assert.equal(dialog.props.confirmDisabled, false, 'plaintext export does not require a password')
  const credentials = findAll(node => node.type === 'input' && node.props.type === 'checkbox')[1]
  credentials.props.onChange({ target: { checked: true } })
  await flush()
  assert.equal(findAll(node => node.text === 'settings.export_config_warning_desc').length, 1, 'credentials show exactly one warning')
  assert.equal(findAll(node => node.text === 'settings.config_plaintext_warning').length, 0, 'no duplicate plaintext warning')
  encrypted.props.onChange({ target: { checked: true } })
  await flush()
  assert.equal(findAll(node => node.text === 'settings.export_config_warning_desc').length, 1, 'credential warning also applies to encrypted exports')
  credentials.props.onChange({ target: { checked: false } })
  await flush()
  assert.equal(findAll(node => node.text === 'settings.export_config_warning_desc').length, 0, 'credential warning disappears when the option is off')
  encrypted.props.onChange({ target: { checked: false } })
  credentials.props.onChange({ target: { checked: true } })
  await flush()
  dialog.confirm()
  await flush()
  assert.deepEqual(calls.at(-1), ['export', { password: undefined, includeLoginState: true }])

  queue.push({ success: false, requiresPassword: true, filePath: 'android.json' })
  mode.value = 'import'
  await flush()
  assert.equal(dialog.props.title, 'settings.config_password_required')
  assert.equal(imported.length, 0)
  input('settings.config_password', 'wrong')
  queue.push({ success: false, error: 'settings.config_decryption_failed' })
  dialog.confirm()
  await flush()
  assert.equal(mode.value, 'import', 'wrong password must allow retry with the same file')
  assert.equal(dialog.props.title, 'settings.config_password_required')
  input('settings.config_password', ' correct password ')
  queue.push({ success: false, requiresConfirmation: true, filePath: 'android.json', platform: 'android' })
  dialog.confirm()
  await flush()
  assert.equal(dialog.props.title, 'settings.config_android_warning_title')
  assert.equal(imported.length, 0, 'warning must appear before applying Android settings')
  assert.ok(findAll(node => node.text === 'settings.config_android_warning_desc').length)
  queue.push({ success: true })
  dialog.confirm()
  await flush()
  assert.deepEqual(calls.at(-1), ['import', { filePath: 'android.json', password: ' correct password ', confirmedAndroid: true }])
  assert.equal(imported.length, 1)
  assert.equal(mode.value, null)

  queue.push({ success: false, requiresConfirmation: true, filePath: 'plain-android.json' })
  mode.value = 'import'
  await flush()
  const beforeCancel = calls.length
  dialog.close()
  await flush()
  assert.equal(calls.length, beforeCancel, 'cancelling Android warning must not apply the backup')
  assert.equal(imported.length, 1)
  dialog.confirm()
  await flush()
  assert.equal(calls.length, beforeCancel, 'a closing dialog must not start a new import from a stale click')

  let finish
  queue.push(new Promise(resolve => { finish = resolve }))
  mode.value = 'import'
  await flush()
  const beforeDouble = calls.length
  dialog.confirm()
  await flush()
  assert.equal(calls.length, beforeDouble, 'pending operations must not duplicate file reads or writes')
  finish({ success: false })
  await flush()
  assert.equal(mode.value, null)
  console.log('PASS encrypted/plain export, password clearing/retry, Android preflight/cancel, and duplicate-submit protection')
} finally { app.unmount() }

const keys = [...source.matchAll(/\bt\('([^']+)'/g)].map(match => match[1])
for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) {
  const translations = JSON.parse(await readFile(new URL(`../src/i18n/${locale}.json`, import.meta.url), 'utf8'))
  for (const key of [...keys, 'settings.cookie_login', 'settings.cookie_login_submit', 'settings.cookie_login_title']) {
    const value = key.split('.').reduce((current, part) => current?.[part], translations)
    assert.equal(typeof value, 'string', `Missing ${locale} translation: ${key}`)
    assert.ok(value.trim())
  }
}
console.log('PASS configuration and cookie dialog translations in all four locales')
