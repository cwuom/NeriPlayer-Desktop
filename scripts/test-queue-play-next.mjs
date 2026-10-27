import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { ref } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const names = [
  'addToQueueNext',
  'shiftShuffleIndicesForPlayNext',
  'shiftShuffleIndicesForRemove',
]
const functions = new Map()
function visit(node) {
  if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text)) {
    functions.set(node.name.text, node.getText(parsed))
  }
  ts.forEachChild(node, visit)
}
visit(parsed)
for (const name of names) assert.ok(functions.has(name), `missing playback operation ${name}`)
const compiled = ts.transpileModule(names.map(name => functions.get(name)).join('\n'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

const track = id => ({ id, title: id, artist: 'artist', album: 'album', durationMs: 180000, source: 'netease' })

function fixture({
  tracks = [track('a'), track('b'), track('c'), track('d'), track('e')],
  currentIndex = 2, shuffle = true, bag = [4, 0, 2], history = [1, 3, 1], future = [0, 4],
} = {}) {
  const saved = []
  const queue = ref(tracks)
  const queueIndex = ref(currentIndex)
  const context = {
    queue, queueIndex,
    shuffleEnabled: ref(shuffle),
    savePlayerState() { saved.push(queue.value.map(item => item.title)) },
  }
  const methods = new Function(
    ...Object.keys(context), 'initialBag', 'initialHistory', 'initialFuture', `
    let shuffleBag = [...initialBag], shuffleHistory = [...initialHistory], shuffleFuture = [...initialFuture]
    let pendingReplayTrackId = null
    ${compiled}
    return {
      addToQueueNext,
      shuffleState() { return { bag: [...shuffleBag], history: [...shuffleHistory], future: [...shuffleFuture] } },
      pendingReplay() { return pendingReplayTrackId },
    }
  `)(...Object.values(context), bag, history, future)
  return { ...methods, ...context, saved }
}

/** Index next() picks in shuffle mode: pop the future stack first, then draw from the bag */
function nextIndex(state) {
  const { future, bag } = state.shuffleState()
  if (future.length > 0) return future[future.length - 1]
  return bag[0]
}

let passed = 0, failed = 0
function test(name, run) {
  try { run(); passed++; console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

test('shuffle: a new track is inserted after the current one and becomes the next track', () => {
  const state = fixture()
  state.addToQueueNext(track('x'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['a', 'b', 'c', 'x', 'd', 'e'])
  assert.equal(state.queueIndex.value, 2)
  assert.equal(nextIndex(state), 3, 'shuffle must play the inserted slot next')
  assert.deepEqual(state.shuffleState(), { bag: [5, 0, 2], history: [1, 4, 1], future: [0, 5, 3] })
  assert.deepEqual(state.saved, [['a', 'b', 'c', 'x', 'd', 'e']])
})

test('shuffle: the inserted slot is popped before the random bag', () => {
  const state = fixture({ bag: [4, 0], history: [], future: [] })
  state.addToQueueNext(track('x'))
  const { future } = state.shuffleState()
  assert.deepEqual(future, [3])
  assert.equal(future.pop(), 3)
})

test('shuffle: an already queued track is relocated and still becomes the next track', () => {
  const state = fixture()
  state.addToQueueNext(track('a'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['b', 'c', 'a', 'd', 'e'])
  assert.equal(state.queueIndex.value, 1)
  assert.equal(nextIndex(state), 2)
  assert.equal(state.queue.value[nextIndex(state)].title, 'a')
  assert.deepEqual(state.shuffleState(), { bag: [4, 1], history: [0, 3, 0], future: [4, 2] })
})

test('shuffle: a track already waiting in the pre-queue stack is promoted, not duplicated', () => {
  const state = fixture({ future: [3] })
  state.addToQueueNext(track('d'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['a', 'b', 'c', 'd', 'e'])
  assert.equal(state.queueIndex.value, 2)
  assert.deepEqual(state.shuffleState(), { bag: [4, 0, 2], history: [1, 1], future: [3] })
  assert.equal(nextIndex(state), 3)
})

test('shuffle: adding the current track keeps its slot and queue cursor', () => {
  const state = fixture()
  state.addToQueueNext(track('c'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['a', 'b', 'c', 'd', 'e'])
  assert.equal(state.queueIndex.value, 2, 'the playing cursor must not move off the current track')
  assert.deepEqual(state.shuffleState(), { bag: [4, 0, 2], history: [1, 3, 1], future: [0, 4, 2] })
  assert.equal(nextIndex(state), 2)
})

test('shuffle: a relocated track is dropped from the pre-queue stack before re-promotion', () => {
  const state = fixture({ future: [4] })
  state.addToQueueNext(track('e'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['a', 'b', 'c', 'e', 'd'])
  assert.deepEqual(state.shuffleState(), { bag: [0, 2], history: [1, 4, 1], future: [3] })
  assert.equal(state.queue.value[nextIndex(state)].title, 'e')
})

test('shuffle: insertion at the head of an empty session waits for the whole queue', () => {
  const state = fixture({ tracks: [track('a'), track('b')], currentIndex: -1, bag: [], history: [], future: [] })
  state.addToQueueNext(track('x'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['x', 'a', 'b'])
  assert.equal(state.queueIndex.value, -1)
  assert.deepEqual(state.shuffleState().future, [0])
})

test('non-shuffle: only the physical queue changes', () => {
  const state = fixture({ shuffle: false })
  state.addToQueueNext(track('x'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['a', 'b', 'c', 'x', 'd', 'e'])
  assert.equal(state.queueIndex.value, 2)
  assert.deepEqual(state.shuffleState(), { bag: [4, 0, 2], history: [1, 3, 1], future: [0, 4] })
})

test('non-shuffle: adding a track before the cursor keeps the cursor on the same track', () => {
  const state = fixture({ shuffle: false })
  state.addToQueueNext(track('a'))
  assert.deepEqual(state.queue.value.map(item => item.title), ['b', 'c', 'a', 'd', 'e'])
  assert.equal(state.queueIndex.value, 1)
  assert.equal(state.queue.value[state.queueIndex.value].title, 'c')
})

test('non-shuffle: a different Play Next request clears a pending current-track replay', () => {
  const state = fixture({ shuffle: false })
  state.addToQueueNext(track('c'))
  assert.equal(state.pendingReplay(), 'c')
  state.addToQueueNext(track('d'))
  assert.equal(state.pendingReplay(), null, 'a later different request supersedes the replay')
})

console.log(`queue play-next tests: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
