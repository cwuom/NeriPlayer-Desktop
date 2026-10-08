import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { createRenderer, defineComponent, h, KeepAlive, nextTick, ref } from 'vue'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const source = await readFile(new URL('../src/composables/useEscapeClose.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

const listeners = []
globalThis.document = {
  addEventListener(type, listener, capture) {
    assert.equal(type, 'keydown')
    assert.equal(capture, true)
    listeners.push(listener)
  },
  removeEventListener(type, listener) {
    const index = listeners.indexOf(listener)
    if (index >= 0) listeners.splice(index, 1)
  },
}
const exports = {}
new Function('require', 'exports', compiled)(require, exports)
const { useEscapeClose } = exports

function pressEscape({ prevented = false, key = 'Escape' } = {}) {
  const event = {
    key,
    defaultPrevented: prevented,
    preventDefault() { this.defaultPrevented = true },
  }
  for (const listener of [...listeners]) listener(event)
  return event
}

const renderer = createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText() {}, setElementText() {}, patchProp() {},
  parentNode: node => node.parent ?? null, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { if (node.parent) node.parent.children = node.parent.children.filter(child => child !== node) },
})

function overlay(name, open) {
  return defineComponent({
    name,
    setup() {
      useEscapeClose(() => open.value, () => { open.value = false })
      return () => h('div')
    },
  })
}

const lower = ref(false)
const upper = ref(false)
const Lower = overlay('Lower', lower)
const Upper = overlay('Upper', upper)
const app = renderer.createApp({ setup: () => () => h('div', [h(Upper), h(Lower)]) })
app.mount({ children: [] })

// 无弹层时 Escape 不被消费，交给全局快捷键（例如关闭播放页）
assert.equal(pressEscape().defaultPrevented, false)

// 打开顺序决定关闭顺序，与组件挂载顺序无关
lower.value = true
await nextTick()
upper.value = true
await nextTick()
const first = pressEscape()
assert.equal(first.defaultPrevented, true)
assert.equal(upper.value, false)
assert.equal(lower.value, true)
pressEscape()
assert.equal(lower.value, false)

// 其他处理者已消费的 Escape、以及非 Escape 键都不处理
lower.value = true
await nextTick()
pressEscape({ prevented: true })
pressEscape({ key: 'ArrowLeft' })
assert.equal(lower.value, true)
lower.value = false
await nextTick()

// KeepAlive 停用的页面不可见：其弹层让出栈顶，重新激活后恢复
const pageOpen = ref(false)
const Page = overlay('Page', pageOpen)
const Other = defineComponent({ name: 'Other', setup: () => () => h('div') })
const showPage = ref(true)
const cached = renderer.createApp({
  setup: () => () => h(KeepAlive, null, [showPage.value ? h(Page) : h(Other)]),
})
cached.mount({ children: [] })
pageOpen.value = true
await nextTick()
showPage.value = false
await nextTick()
assert.equal(pressEscape().defaultPrevented, false)
assert.equal(pageOpen.value, true)
showPage.value = true
await nextTick()
pressEscape()
assert.equal(pageOpen.value, false)

// 卸载后移除监听
cached.unmount()
app.unmount()
assert.equal(listeners.length, 0)
console.log('escape close stack tests passed')
