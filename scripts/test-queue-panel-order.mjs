import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { computed, effectScope, nextTick, reactive, ref, watch } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/QueuePanel.vue', import.meta.url), 'utf8')
const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const parsed = ts.createSourceFile('QueuePanel.ts', script, ts.ScriptTarget.ES2022, true)
const names = [
  'showPlaybackOrder', 'isPlaybackOrderView', 'queueEntryCache', 'displayQueue', 'currentDisplayIndex',
  'queueListRef', 'queuePanelUnmounted', 'locateCurrentTrack', 'canReorderQueue',
  'playFromQueue', 'removeFromQueue', 'openQueueContextMenu', 'handleQueueContextMenuClick',
]
const declarations = names.map(name => {
  const node = parsed.statements.find(item =>
    ts.isFunctionDeclaration(item) ? item.name?.text === name :
    ts.isVariableStatement(item) && item.declarationList.declarations.some(decl => decl.name.getText(parsed) === name))
  assert.ok(node, `missing queue order behavior: ${name}`)
  return node.getText(parsed)
})
const hooks = parsed.statements.filter(item => ts.isExpressionStatement(item)
  && /^(onMounted|onUnmounted|watch)\(/.test(item.getText(parsed))
  && /locateCurrentTrack|queuePanelUnmounted/.test(item.getText(parsed)))
assert.ok(hooks.length >= 3, 'opening, following and unmount cleanup must be wired')
const compiled = ts.transpileModule([...declarations, ...hooks.map(item => item.getText(parsed))].join('\n'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

const incrementalSource = await readFile(new URL('../src/composables/useIncrementalList.ts', import.meta.url), 'utf8')
const incrementalCompiled = ts.transpileModule(incrementalSource.replace(/^import .*$/m, '').replace('export function', 'function'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const useIncrementalList = new Function('computed', 'ref', 'watch', `${incrementalCompiled}\nreturn useIncrementalList`)(computed, ref, watch)

function fixture({ count = 300, current = 240, shuffle = false, order, orderCurrent = 0, reduced = false } = {}) {
  const player = reactive({
    queue: Array.from({ length: count }, (_, index) => ({ id: `track:${index}`, title: `Track ${index}` })),
    queueIndex: current, shuffleEnabled: shuffle,
    queueDisplayIndices: order ?? Array.from({ length: count }, (_, index) => index),
    queueDisplayCurrentIndex: orderCurrent,
    playQueueItem: index => calls.push(['play', index]),
    removeFromQueue: index => calls.push(['remove', index]),
    addToQueueNext: track => calls.push(['next', track.id]),
    addToQueueEnd: track => calls.push(['end', track.id]),
  })
  const calls = [], scrolls = [], mounted = [], unmounted = [], ensured = []
  const queueDragIndex = ref(-1), queueContextMenuOpen = ref(false), queueContextMenuIndex = ref(-1)
  const list = {
    querySelectorAll: () => Array.from({ length: incremental.visibleItems.value.length }, (_, index) => ({
      scrollIntoView: options => scrolls.push({ index, ...options }),
    })),
  }
  let incremental, methods
  const scope = effectScope()
  const context = {
    computed, ref, watch, nextTick, player, queueDragIndex, queueContextMenuOpen, queueContextMenuIndex,
    queueContextMenuPosition: ref({ x: 0, y: 0 }), listenTogether: reactive({ localControlRestriction: null }),
    onMounted: fn => mounted.push(fn), onUnmounted: fn => unmounted.push(fn),
    window: { matchMedia: () => ({ matches: reduced }) }, HTMLElement: class {},
    ensureIndex: index => { ensured.push(index); return incremental.ensureIndex(index) },
    downloadFromMenu: () => calls.push(['download']),
  }
  scope.run(() => {
    methods = new Function(...Object.keys(context), `${compiled}\nreturn {
      showPlaybackOrder, isPlaybackOrderView, displayQueue, currentDisplayIndex, canReorderQueue,
      queueListRef, locateCurrentTrack, playFromQueue, removeFromQueue, openQueueContextMenu, handleQueueContextMenuClick,
    }`)(...Object.values(context))
    incremental = useIncrementalList(() => methods.displayQueue.value)
    methods.queueListRef.value = list
  })
  return {
    ...methods, player, calls, scrolls, ensured, queueDragIndex, queueContextMenuIndex, queueContextMenuOpen,
    listenTogether: context.listenTogether, incremental,
    mount: () => mounted.forEach(fn => fn()),
    dispose: () => { unmounted.forEach(fn => fn()); scope.stop() },
  }
}

let passed = 0
async function test(name, run) {
  await run()
  passed++
  console.log(`PASS ${name}`)
}
async function settle() { await nextTick(); await nextTick(); await nextTick() }

await test('opening expands past the first chunk and locates the current queue slot', async () => {
  const state = fixture()
  assert.equal(state.incremental.visibleItems.value.length, 100)
  state.mount()
  await settle()
  assert.deepEqual(state.scrolls, [{ index: 240, block: 'center', behavior: 'auto' }])
  assert.ok(state.incremental.visibleItems.value.length > 240)
  assert.deepEqual(state.calls, [])
  state.dispose()
})

await test('manual locate is smooth and does not trigger playback', async () => {
  const state = fixture()
  await state.locateCurrentTrack()
  assert.deepEqual(state.scrolls, [{ index: 240, block: 'center', behavior: 'smooth' }])
  assert.deepEqual(state.calls, [])
  state.dispose()
})

await test('shuffle projection preserves original slot indices and only marks the current occurrence', async () => {
  const state = fixture({ count: 4, current: 2, shuffle: true, order: [0, 2, 2, 3, 1], orderCurrent: 1 })
  state.player.queue[2].id = state.player.queue[0].id
  assert.deepEqual(state.displayQueue.value.map(item => item.queueIndex), [0, 2, 2, 3, 1])
  assert.equal(state.currentDisplayIndex.value, 1)
  state.mount()
  await settle()
  assert.equal(state.scrolls.at(-1).index, 1)
  state.playFromQueue(state.displayQueue.value[4].queueIndex)
  state.removeFromQueue(state.displayQueue.value[3].queueIndex)
  state.openQueueContextMenu({ type: 'contextmenu', clientX: 12, clientY: 34 }, state.displayQueue.value[0].queueIndex)
  state.handleQueueContextMenuClick({ id: 'play' })
  state.handleQueueContextMenuClick({ id: 'play-next' })
  state.handleQueueContextMenuClick({ id: 'queue-end' })
  assert.deepEqual(state.calls, [['play', 1], ['remove', 3], ['play', 0], ['next', 'track:0'], ['end', 'track:0']])
  state.dispose()
})

await test('switching to original order restores sorting and relocates the same current slot', async () => {
  const state = fixture({ count: 4, current: 2, shuffle: true, order: [2, 3, 0, 1], orderCurrent: 0 })
  assert.equal(state.canReorderQueue.value, false)
  state.showPlaybackOrder.value = false
  await settle()
  assert.deepEqual(state.displayQueue.value.map(item => item.queueIndex), [0, 1, 2, 3])
  assert.equal(state.currentDisplayIndex.value, 2)
  assert.equal(state.canReorderQueue.value, true)
  assert.equal(state.scrolls.at(-1).index, 2)
  state.listenTogether.localControlRestriction = 'member_control_disabled'
  assert.equal(state.canReorderQueue.value, false)
  state.dispose()
})

await test('following a new current song expands the list but does not interrupt a drag', async () => {
  const state = fixture({ current: 0 })
  state.player.queueIndex = 205
  await settle()
  assert.equal(state.scrolls.at(-1).index, 205)
  state.queueDragIndex.value = 4
  state.player.queueIndex = 290
  await settle()
  assert.equal(state.scrolls.at(-1).index, 205)
  state.dispose()
})

await test('updating current metadata preserves the expanded window and visible current row', async () => {
  const state = fixture()
  state.mount()
  await settle()
  assert.ok(state.incremental.visibleItems.value.length > 240)
  state.player.queue = state.player.queue.map((track, index) => index === 240
    ? { ...track, title: 'Updated current track' } : track)
  await settle()
  assert.ok(state.incremental.visibleItems.value.length > 240, 'metadata updates must not hide the current row')
  assert.equal(state.incremental.visibleItems.value[240].track.title, 'Updated current track')
  state.player.queue = state.player.queue.map((track, index) => index === 0
    ? { ...track, title: 'Updated first track' } : track)
  await settle()
  assert.ok(state.incremental.visibleItems.value.length > 240, 'first-row metadata must preserve the expanded window too')
  assert.equal(state.incremental.visibleItems.value[0].track.title, 'Updated first track')
  assert.equal(state.scrolls.length, 1, 'metadata changes should preserve the user scroll position')
  state.dispose()
})

await test('empty, invalid and unmounted queues never scroll', async () => {
  for (const options of [{ count: 0, current: -1 }, { count: 2, current: -1 }, { count: 2, current: 2 }]) {
    const state = fixture(options)
    await state.locateCurrentTrack()
    assert.deepEqual(state.scrolls, [])
    state.dispose()
  }
  const state = fixture()
  const pending = state.locateCurrentTrack()
  state.dispose()
  await pending
  assert.deepEqual(state.scrolls, [])
})

await test('reduced motion skips smooth scrolling', async () => {
  const state = fixture({ reduced: true })
  await state.locateCurrentTrack()
  assert.equal(state.scrolls.at(-1).behavior, 'auto')
  state.dispose()
})

assert.match(source, /class="queue-locate"[\s\S]*?@click="locateCurrentTrack\(\)"/)
assert.match(source, /:aria-pressed="isPlaybackOrderView"/)
assert.match(source, /active: index === currentDisplayIndex/)
assert.match(source, /@click="playFromQueue\(entry.queueIndex\)"/)
console.log(`Queue panel order and locate: ${passed} passed`)
