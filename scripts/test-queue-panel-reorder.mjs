import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { computed, ref } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/QueuePanel.vue', import.meta.url), 'utf8')
const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const parsed = ts.createSourceFile('QueuePanel.ts', script, ts.ScriptTarget.ES2022, true)
const names = [
  'canReorderQueue', 'queueDragIndex', 'queueDropIndex', 'queueDragRowStyle',
  'queueDragPointerId', 'queueDragHandle', 'queueDragRow', 'queueDragSnapshot', 'queueDragStartY',
  'queueDragPointerY', 'queueDragStartScrollTop', 'queueDragFrame',
  'cancelQueueDrag', 'updateQueueDragTarget', 'tickQueueDrag',
  'startQueueDrag', 'moveQueueDrag', 'finishQueueDrag', 'moveQueueByKeyboard',
]
const declarations = names.map(name => {
  const node = parsed.statements.find(item =>
    ts.isFunctionDeclaration(item) ? item.name?.text === name :
    ts.isVariableStatement(item) && item.declarationList.declarations.some(decl => decl.name.getText(parsed) === name))
  assert.ok(node, `missing queue drag behavior: ${name}`)
  return node.getText(parsed)
}).join('\n')
const compiled = ts.transpileModule(declarations, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

function fixture(count = 4) {
  const player = { queue: Array.from({ length: count }, (_, index) => ({ id: `track:${index}` })) }
  const listenTogether = { localControlRestriction: null }
  const calls = [], ensured = [], focused = [], listeners = new Map(), frames = new Map()
  let frameId = 0, menuClosed = 0, expansions = 0
  const list = {
    scrollTop: 0, scrollHeight: count * 50, clientHeight: Math.min(count * 50, 200),
    getBoundingClientRect: () => ({ top: 100, bottom: 300 }),
    querySelectorAll: selector => player.queue.map((_, index) => ({
      getBoundingClientRect: () => ({ top: 100 + index * 50 - list.scrollTop, height: 50 }),
      ...(selector === '.qi-drag-handle' ? { focus: () => focused.push(index) } : {}),
    })),
  }
  const captured = new Set()
  // The drag offset is written to a CSS variable on the dragged row, not to reactive state
  const rowStyle = new Map()
  const row = {
    style: {
      setProperty: (name, value) => rowStyle.set(name, value),
      removeProperty: name => rowStyle.delete(name),
    },
  }
  const handle = {
    setPointerCapture: id => captured.add(id), hasPointerCapture: id => captured.has(id),
    releasePointerCapture: id => captured.delete(id),
    closest: () => row,
  }
  player.moveQueueItem = (from, to, snapshot) => { calls.push({ from, to, snapshot }); return true }
  const context = {
    computed, ref, player, listenTogether, queueListRef: ref(list), isPlaybackOrderView: ref(false),
    window: {
      addEventListener: (type, fn) => listeners.set(type, fn),
      removeEventListener: type => listeners.delete(type),
    },
    requestAnimationFrame: fn => { frames.set(++frameId, fn); return frameId },
    cancelAnimationFrame: id => frames.delete(id), nextTick: async () => {},
    closeQueueContextMenu: () => { menuClosed++ },
    expand: () => { expansions++ }, ensureIndex: index => ensured.push(index),
  }
  const methods = new Function(...Object.keys(context), `${compiled}\nreturn {
    startQueueDrag, moveQueueDrag, finishQueueDrag, cancelQueueDrag, moveQueueByKeyboard,
    queueDragIndex, queueDropIndex, queueDragRowStyle, canReorderQueue,
  }`)(...Object.values(context))
  const event = (y, extra = {}) => ({
    pointerId: 7, pointerType: 'mouse', button: 0, clientY: y, currentTarget: handle,
    preventDefault() { this.prevented = true }, stopPropagation() {}, ...extra,
  })
  return {
    ...methods, player, listenTogether, list, calls, ensured, focused, listeners, frames, captured, event, rowStyle,
    get menuClosed() { return menuClosed }, get expansions() { return expansions },
    tick() { const [id, fn] = frames.entries().next().value; frames.delete(id); fn() },
  }
}

let passed = 0
async function test(name, run) {
  await run()
  passed++
  console.log(`PASS ${name}`)
}

await test('dragging down uses the final queue index and never plays a song', async () => {
  const state = fixture()
  state.startQueueDrag(state.event(125), 0)
  state.moveQueueDrag(state.event(280))
  assert.equal(state.queueDropIndex.value, 3)
  // The offset goes into a CSS variable on the dragged row: pointermove must not write
  // reactive state, which re-rendered the whole panel on every move
  assert.equal(state.rowStyle.get('--queue-drag-offset'), '155px')
  const original = [...state.player.queue]
  await state.finishQueueDrag(state.event(280))
  assert.deepEqual(state.calls, [{ from: 0, to: 3, snapshot: original }])
  assert.equal(state.rowStyle.has('--queue-drag-offset'), false)
  assert.equal(state.queueDragIndex.value, -1)
  assert.equal(state.listeners.size, 0)
  assert.equal(state.frames.size, 0)
  assert.equal(state.captured.size, 0)
  assert.deepEqual(state.ensured, [3])
})

await test('dragging up preserves the exact duplicate occurrence in the snapshot', async () => {
  const state = fixture()
  state.player.queue[2] = state.player.queue[0]
  state.startQueueDrag(state.event(225), 2)
  state.moveQueueDrag(state.event(110))
  await state.finishQueueDrag(state.event(110))
  assert.equal(state.calls[0].from, 2)
  assert.equal(state.calls[0].to, 0)
  assert.equal(state.calls[0].snapshot[2], state.player.queue[0])
})

await test('an unrelated pointer cannot move or finish an active drag', async () => {
  const state = fixture()
  state.startQueueDrag(state.event(175), 1)
  state.moveQueueDrag(state.event(290, { pointerId: 8 }))
  await state.finishQueueDrag(state.event(290, { pointerId: 8 }))
  assert.equal(state.queueDropIndex.value, 1)
  assert.equal(state.queueDragIndex.value, 1)
  assert.equal(state.calls.length, 0)
  state.cancelQueueDrag()
})

await test('cancellation removes all drag listeners, animation frames and pointer capture', () => {
  const state = fixture()
  state.startQueueDrag(state.event(175), 1)
  state.cancelQueueDrag()
  assert.equal(state.calls.length, 0)
  assert.equal(state.queueDragIndex.value, -1)
  assert.equal(state.listeners.size, 0)
  assert.equal(state.frames.size, 0)
  assert.equal(state.captured.size, 0)
})

await test('revoked room control and right clicks cannot start sorting', () => {
  const state = fixture()
  state.listenTogether.localControlRestriction = 'member_control_disabled'
  state.startQueueDrag(state.event(175), 1)
  assert.equal(state.queueDragIndex.value, -1)
  state.listenTogether.localControlRestriction = null
  state.startQueueDrag(state.event(175, { button: 2 }), 1)
  assert.equal(state.queueDragIndex.value, -1)
})

await test('a stationary pointer at the lower edge keeps scrolling and expands long queues', () => {
  const state = fixture(100)
  state.list.scrollTop = 4800
  state.startQueueDrag(state.event(275), 98)
  state.moveQueueDrag(state.event(299))
  state.tick()
  const firstScroll = state.list.scrollTop
  assert.ok(firstScroll > 4800)
  state.tick()
  assert.ok(state.list.scrollTop > firstScroll)
  assert.ok(state.expansions > 0)
  state.cancelQueueDrag()
  assert.equal(state.frames.size, 0)
})

await test('arrow keys adjust order without forwarding a playback click', async () => {
  const state = fixture()
  await state.moveQueueByKeyboard(state.event(175, { key: 'ArrowDown' }), 1)
  assert.deepEqual(state.calls.map(({ from, to }) => [from, to]), [[1, 2]])
  assert.deepEqual(state.focused, [2])
  await state.moveQueueByKeyboard(state.event(175, { key: 'Enter' }), 1)
  assert.equal(state.calls.length, 1)
})

console.log(`Queue panel reorder: ${passed} passed`)
