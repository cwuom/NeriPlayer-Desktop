// 鼠标快捷键回归：组合键解析、鼠标键识别、全局加速器豁免与应用内触发
//
// 与 test-shortcut-settings.mjs 同构：直接编译模块源码，用伪造的键盘 / 鼠标事件驱动，
// 不依赖浏览器环境。

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
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
const globalShortcutsSource = await read('../src/modules/shortcuts/globalShortcuts.ts')

function mouseEvent(overrides = {}) {
  return {
    button: 1, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false,
    defaultPrevented: false,
    target: null,
    preventDefault() { this.defaultPrevented = true },
    ...overrides,
  }
}

let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('mouse buttons map to shortcut tokens and left/right buttons are ignored', () => {
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 1 })), 'MouseMiddle')
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 3 })), 'MouseBack')
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 4 })), 'MouseForward')
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 0 })), null)
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 2 })), null)
})

await test('mouse combinations keep modifiers in the platform order', () => {
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 3, ctrlKey: true })), 'Mod+MouseBack')
  assert.equal(bindings.comboFromMouseEvent(mouseEvent({ button: 3, shiftKey: true, ctrlKey: true })), 'Mod+Shift+MouseBack')
  const mac = loadBindings(true)
  assert.equal(mac.comboFromMouseEvent(mouseEvent({ button: 1, metaKey: true })), 'Mod+MouseMiddle')
})

await test('mouse tokens survive normalization and are reported as mouse combos', () => {
  const normalized = bindings.normalizeShortcutBindings({
    local: { toggle_play: 'mouseback' },
    global: { toggle_play: 'MouseForward' },
  })
  assert.deepEqual(normalized.local, { toggle_play: 'MouseBack' })
  assert.deepEqual(normalized.global, { toggle_play: 'MouseForward' })
  assert.equal(bindings.isMouseCombo('MouseBack'), true)
  assert.equal(bindings.isMouseCombo('Mod+MouseBack'), true)
  assert.equal(bindings.isMouseCombo('Mod+Space'), false)
  assert.equal(bindings.mouseKeyOfCombo('Mod+MouseForward'), 'MouseForward')
  assert.equal(bindings.mouseKeyOfCombo('Mod+Space'), null)
})

await test('mouse combos never convert to a system shortcut accelerator', () => {
  assert.equal(bindings.toAccelerator('MouseBack'), null)
  assert.equal(bindings.toAccelerator('Mod+MouseMiddle'), null)
  assert.equal(bindings.toAccelerator('Mod+Alt+Space'), 'CommandOrControl+Alt+Space')
})

await test('mouse keys are displayed with localized labels', () => {
  // 不传翻译函数时回落到键名本身
  assert.deepEqual(bindings.comboLabelParts('MouseBack'), ['MouseBack'])
  assert.deepEqual(bindings.comboLabelParts('Mod+MouseForward'), ['Ctrl', 'MouseForward'])
  // 传入翻译函数时走 i18n key，标签由语言包决定
  const translate = key => `t:${key}`
  assert.deepEqual(bindings.comboLabelParts('MouseMiddle', translate), ['t:shortcuts.mouse_middle'])
  assert.deepEqual(bindings.comboLabelParts('MouseBack', translate), ['t:shortcuts.mouse_back'])
  assert.deepEqual(
    bindings.comboLabelParts('Mod+MouseForward', translate),
    ['Ctrl', 't:shortcuts.mouse_forward'],
  )
})

await test('a globally bound mouse key does not also fire in-app', async () => {
  const listeners = new Set()
  const window = {
    addEventListener(type, handler) { listeners.add([type, handler]) },
    removeEventListener(type, handler) {
      for (const entry of [...listeners]) {
        if (entry[0] === type && entry[1] === handler) listeners.delete(entry)
      }
    },
  }
  const exports = {}
  new Function('require', 'exports', 'window', compile(globalShortcutsSource))(name => {
    if (name === './bindings') return bindings
    // Windows 有原生鼠标钩子，全局绑定存在时应用内让位
    if (name === './platform') return { isEditableTarget: () => false, hasNativeMouseHook: true }
    if (name === './recording') return { shortcutRecording: { value: false } }
    throw new Error(`unexpected dependency: ${name}`)
  }, exports, window)

  const triggered = []
  const actions = new Proxy({}, {
    get: (_target, name) => {
      if (name === 'isOverlayOpen') return () => false
      return () => triggered.push(name)
    },
  })
  const uninstall = exports.installGlobalShortcuts(actions, () => ({
    local: { toggle_play: 'MouseBack' },
    global: { toggle_play: 'MouseBack' },
  }), () => true)

  const mouseHandler = [...listeners].find(([type]) => type === 'mousedown')?.[1]
  assert.ok(mouseHandler, 'the mouse listener is installed')

  // 全局绑定存在时由原生钩子负责，应用内不再重复触发
  mouseHandler(mouseEvent({ button: 3 }))
  assert.deepEqual(triggered, [])

  uninstall()
})

await test('without a native mouse hook the in-app mouse binding still fires', async () => {
  const listeners = new Set()
  const window = {
    addEventListener(type, handler) { listeners.add([type, handler]) },
    removeEventListener(type, handler) {
      for (const entry of [...listeners]) {
        if (entry[0] === type && entry[1] === handler) listeners.delete(entry)
      }
    },
  }
  const exports = {}
  new Function('require', 'exports', 'window', compile(globalShortcutsSource))(name => {
    if (name === './bindings') return bindings
    // macOS / Linux 没有原生鼠标钩子：全局鼠标键只在应用内生效
    if (name === './platform') return { isEditableTarget: () => false, hasNativeMouseHook: false }
    if (name === './recording') return { shortcutRecording: { value: false } }
    throw new Error(`unexpected dependency: ${name}`)
  }, exports, window)

  const triggered = []
  const actions = new Proxy({}, {
    get: (_target, name) => {
      if (name === 'isOverlayOpen') return () => false
      return () => triggered.push(name)
    },
  })
  const uninstall = exports.installGlobalShortcuts(actions, () => ({
    local: { toggle_play: 'MouseBack' },
    global: { toggle_play: 'MouseBack' },
  }), () => true)

  const mouseHandler = [...listeners].find(([type]) => type === 'mousedown')?.[1]
  assert.ok(mouseHandler, 'the mouse listener is installed')

  // 没有原生钩子接管，全局绑定不该让应用内的动作失效
  mouseHandler(mouseEvent({ button: 3 }))
  assert.deepEqual(triggered, ['togglePlay'])

  uninstall()
})

await test('an in-app mouse binding triggers its action and left/right clicks do not', async () => {
  const listeners = new Set()
  const window = {
    addEventListener(type, handler) { listeners.add([type, handler]) },
    removeEventListener(type, handler) {
      for (const entry of [...listeners]) {
        if (entry[0] === type && entry[1] === handler) listeners.delete(entry)
      }
    },
  }
  const exports = {}
  new Function('require', 'exports', 'window', compile(globalShortcutsSource))(name => {
    if (name === './bindings') return bindings
    if (name === './platform') return { isEditableTarget: target => target instanceof Object && target?.tagName === 'INPUT', hasNativeMouseHook: true }
    if (name === './recording') return { shortcutRecording: { value: false } }
    throw new Error(`unexpected dependency: ${name}`)
  }, exports, window)

  const triggered = []
  const actions = new Proxy({}, {
    get: (_target, name) => {
      // isOverlayOpen 是查询而不是动作，不计入触发记录
      if (name === 'isOverlayOpen') return () => false
      return () => triggered.push(name)
    },
  })
  const uninstall = exports.installGlobalShortcuts(actions, () => ({
    local: { toggle_play: 'MouseBack' },
    global: {},
  }))

  const mouseHandler = [...listeners].find(([type]) => type === 'mousedown')?.[1]
  assert.ok(mouseHandler, 'the mouse listener is installed')

  const event = mouseEvent({ button: 3 })
  mouseHandler(event)
  assert.deepEqual(triggered, ['togglePlay'])
  assert.equal(event.defaultPrevented, true)

  // 左右键不是快捷键：不应触发任何动作
  triggered.length = 0
  const left = mouseEvent({ button: 0 })
  mouseHandler(left)
  assert.deepEqual(triggered, [])
  assert.equal(left.defaultPrevented, false)

  uninstall()
  assert.equal(listeners.size, 0)
})

console.log(`Mouse shortcut regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
