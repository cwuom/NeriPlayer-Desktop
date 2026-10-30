import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/ui/M3Input.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, { id: 'm3-input-test', genDefaultAs: 'component', inlineTemplate: true })
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const component = new Function('require', 'exports', `${compiled}\nreturn component`)(name => {
  assert.equal(name, 'vue')
  return vue
}, {})
const originalDocument = Object.getOwnPropertyDescriptor(globalThis, 'document')
globalThis.document = { activeElement: null }
function node(type) {
  return {
    type, tagName: type.toUpperCase(), value: '', children: [], props: {}, handlers: new Map(), focused: 0,
    getRootNode: () => globalThis.document,
    addEventListener(name, listener) { this.handlers.set(name, listener) },
    dispatchEvent(event) { this.handlers.get(event.type)?.(event) },
    focus() { this.focused++ },
  }
}
const renderer = vue.createRenderer({
  createElement: node, createText: text => ({ text }), createComment: text => ({ text }),
  setText(target, text) { target.text = text }, setElementText(target, text) { target.text = text },
  patchProp(target, key, _, value) { target.props[key] = value },
  parentNode: target => target.parent ?? null, nextSibling: () => null,
  insert(target, parent) { target.parent = parent; parent.children.push(target) },
  remove(target) { target.parent.children = target.parent.children.filter(item => item !== target) },
})
const all = target => [target, ...(target.children ?? []).flatMap(all)]
const value = vue.ref('')
const props = vue.reactive({ label: 'Password', type: 'password', disabled: false, autocomplete: 'new-password', maxlength: 1024 })
let enters = 0
const root = node('root')
const app = renderer.createApp({ setup: () => () => vue.h(component, {
  ...props, modelValue: value.value, 'onUpdate:modelValue': next => { value.value = next }, onEnter: () => enters++,
}) })
app.mount(root)
const input = () => all(root).find(target => target.type === 'input')
try {
  assert.equal(input().props.type, 'password', 'password type must reach the actual input')
  assert.equal(input().props.autocomplete, 'new-password')
  assert.equal(input().props.maxlength, 1024)
  assert.equal(input().props['aria-label'], 'Password')
  input().value = ' 密码 🔑 '
  input().dispatchEvent({ type: 'input', target: input() })
  await vue.nextTick()
  assert.equal(value.value, ' 密码 🔑 ', 'v-model preserves the password exactly')
  input().props.onKeydown({ key: 'Enter' })
  assert.equal(enters, 1)
  props.disabled = true
  await vue.nextTick()
  assert.equal(input().props.disabled, true)
  input().props.onKeydown({ key: 'Enter' })
  assert.equal(enters, 1, 'disabled input cannot submit even through a synthetic key event')
  const wrapper = all(root).find(target => target.type === 'div' && String(target.props.class).includes('m3-input'))
  wrapper.props.onClick()
  assert.equal(input().focused, 0, 'disabled field cannot receive focus through its wrapper')
  delete props.type
  delete props.disabled
  delete props.autocomplete
  await vue.nextTick()
  assert.equal(input().props.type, 'text', 'existing input defaults remain plain text')
  assert.equal(input().props.disabled, false)
  assert.equal(input().props.autocomplete, undefined)
  assert.equal(value.value, ' 密码 🔑 ')
  console.log('PASS M3Input password props, unmodified values, disabled handling and existing defaults')
} finally {
  app.unmount()
  if (originalDocument) Object.defineProperty(globalThis, 'document', originalDocument)
  else delete globalThis.document
}
