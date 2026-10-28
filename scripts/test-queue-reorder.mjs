import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { ref } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const names = ['moveQueueItem', 'persistedPlayerState']
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

const playerStateSource = await readFile(new URL('../src/modules/playback/playerState.ts', import.meta.url), 'utf8')
const playerStateCompiled = ts.transpileModule(playerStateSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
}).outputText
const { buildPersistedPlaybackQueue, restorePersistedPlaybackQueue } = await import(
  `data:text/javascript;base64,${Buffer.from(playerStateCompiled).toString('base64')}`
)

const track = (id, title = id, playlistKey) => ({
  id, title, artist: 'artist', album: 'album', durationMs: 180000,
  source: 'netease', coverUrl: '', audioUrl: '', ...(playlistKey ? { playlistKey } : {}),
})

function fixture({
  tracks = [track('a'), track('b'), track('c'), track('d'), track('e')],
  currentIndex = 2, playing = true, shuffle = true, blocked = false,
  bag = [4, 0, 2], history = [1, 3, 1], future = [0, 4],
} = {}) {
  const calls = [], saved = [], reports = []
  const queue = ref(tracks)
  const currentTrack = ref(currentIndex >= 0 ? queue.value[currentIndex] : null)
  const queueIndex = ref(currentIndex)
  const positionMs = ref(42424)
  const isPlaying = ref(playing)
  const isLoadingAudio = ref(false)
  const hasPlaybackSession = ref(currentIndex >= 0)
  const lastCommandSource = ref('remote_sync')
  const lastCommandSequence = ref(9)
  const lastSeekCommand = ref({ seq: 3, positionMs: 42424, source: 'remote_sync', requestGeneration: 8 })
  let methods
  const context = {
    queue, queueIndex, currentTrack, positionMs, isPlaying, isLoadingAudio, hasPlaybackSession,
    lastCommandSource, lastCommandSequence, lastSeekCommand,
    shuffleEnabled: ref(shuffle), repeatMode: ref('all'), volume: ref(0.6), durationMs: ref(180000),
    buildPersistedPlaybackQueue,
    useSettingsStore: () => ({ keepProgress: true, keepPlaybackMode: true }),
    currentRenderedPosition: () => positionMs.value,
    blockedByListenTogether(source) { calls.push(['blockedByListenTogether', source]); return blocked },
    useListenTogetherStore: () => ({
      reportQueueReorder() {
        calls.push(['reportQueueReorder'])
        reports.push({
          queue: queue.value.map(item => item.title), queueIndex: queueIndex.value,
          currentTrack: currentTrack.value, source: lastCommandSource.value,
          commandSequence: lastCommandSequence.value,
        })
      },
    }),
    markCommandSource(source) {
      calls.push(['markCommandSource', source])
      lastCommandSource.value = source
      lastCommandSequence.value++
    },
    savePlayerState() {
      calls.push(['savePlayerState'])
      saved.push(JSON.parse(JSON.stringify(methods.persistedPlayerState())))
    },
    maybePrefetchNext() {
      calls.push(['maybePrefetchNext'])
      assert.equal(saved.length, 1, 'the updated queue must be saved before prefetching')
    },
    play() { throw new Error('reordering must not play a track') },
    pause() { throw new Error('reordering must not pause playback') },
    seek() { throw new Error('reordering must not seek playback') },
    invoke() { throw new Error('reordering must not invoke native playback') },
    rebuildShuffleBag() { throw new Error('reordering must preserve the existing shuffle traversal') },
  }
  methods = new Function(...Object.keys(context), 'ref', 'initialBag', 'initialHistory', 'initialFuture', `
    const shuffleBag = ref([...initialBag]), shuffleHistory = ref([...initialHistory]), shuffleFuture = ref([...initialFuture])
    ${compiled}
    return {
      moveQueueItem, persistedPlayerState,
      shuffleState() { return { bag: [...shuffleBag.value], history: [...shuffleHistory.value], future: [...shuffleFuture.value] } },
    }
  `)(...Object.values(context), ref, bag, history, future)
  return { ...methods, ...context, calls, saved, reports }
}

function snapshot(state) {
  return {
    queue: [...state.queue.value], queueIndex: state.queueIndex.value,
    currentTrack: state.currentTrack.value, positionMs: state.positionMs.value,
    isPlaying: state.isPlaying.value, isLoadingAudio: state.isLoadingAudio.value,
    hasPlaybackSession: state.hasPlaybackSession.value, durationMs: state.durationMs.value,
    repeatMode: state.repeatMode.value, shuffleEnabled: state.shuffleEnabled.value,
    lastSeekCommand: state.lastSeekCommand.value,
    lastCommandSource: state.lastCommandSource.value, lastCommandSequence: state.lastCommandSequence.value,
    shuffle: state.shuffleState(),
  }
}

function assertPlaybackUnchanged(state, before) {
  assert.equal(state.currentTrack.value, before.currentTrack, 'the loaded track object must remain unchanged')
  for (const name of ['positionMs', 'isPlaying', 'isLoadingAudio', 'hasPlaybackSession', 'durationMs',
    'repeatMode', 'shuffleEnabled', 'lastSeekCommand', 'lastCommandSource', 'lastCommandSequence']) {
    assert.equal(state[name].value, before[name], `${name} must remain unchanged`)
  }
}

function assertRejected(state, before) {
  assert.deepEqual(snapshot(state), before, 'a rejected move must leave all queue and playback state unchanged')
  state.queue.value.forEach((item, index) => assert.equal(item, before.queue[index]))
  assertPlaybackUnchanged(state, before)
  assert.deepEqual(state.saved, [])
  assert.deepEqual(state.reports, [])
  assert.deepEqual(state.calls.filter(([name]) => name !== 'blockedByListenTogether'), [])
  for (const [, source] of state.calls) assert.equal(source, 'local')
}

function assertAccepted(state, before, expectedOrder, expectedIndex, expectedShuffle) {
  assert.deepEqual(state.queue.value.map(item => item.title), expectedOrder)
  assert.equal(state.queueIndex.value, expectedIndex)
  assert.deepEqual(state.shuffleState(), expectedShuffle)
  assertPlaybackUnchanged(state, before)
  assert.equal(state.lastCommandSource.value, 'remote_sync')
  assert.equal(state.lastCommandSequence.value, 9)
  assert.deepEqual(state.calls.filter(([name]) => name !== 'reportQueueReorder'), [
    ['blockedByListenTogether', 'local'],
    ['savePlayerState'], ['maybePrefetchNext'],
  ])
  assert.equal(state.reports.length, 1)
  assert.deepEqual(state.reports[0].queue, expectedOrder)
  assert.equal(state.reports[0].queueIndex, expectedIndex)
  assert.equal(state.reports[0].currentTrack, before.currentTrack)
  assert.equal(state.reports[0].source, 'remote_sync')
  assert.equal(state.reports[0].commandSequence, 9)
  assert.equal(state.saved.length, 1)
  const persisted = state.saved[0]
  assert.deepEqual(persisted.queue.map(item => item.title), expectedOrder)
  assert.equal(persisted.queueIndex, expectedIndex)
  assert.equal(persisted.positionMs, 42424)
  assert.equal(persisted.shuffleEnabled, before.shuffleEnabled)
  assert.equal(persisted.repeatMode, 'all')
  assert.equal(persisted.hasPlaybackSession, before.hasPlaybackSession)
}

function restoreSaved(state) {
  const persisted = state.saved[0]
  return restorePersistedPlaybackQueue(
    persisted.queue, persisted.queueIndex, persisted.hasPlaybackSession,
    persisted.currentTrackId, persisted.currentTrackPlaylistKey,
    raw => {
      if (!raw || typeof raw !== 'object') throw new Error('invalid persisted track')
      return raw
    },
    item => !!item.id && (!!item.audioUrl || !item.id.startsWith('local:')),
  )
}

let passed = 0, failed = 0
async function test(name, run) {
  try { await run(); passed++; console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

const forwardShuffle = { bag: [4, 0, 1], history: [3, 2, 3], future: [0, 4] }
const backwardShuffle = { bag: [4, 0, 3], history: [2, 1, 2], future: [0, 4] }
for (const scenario of [
  { name: 'moves the playing item forward to its final index', from: 1, to: 3, current: 1,
    order: ['a', 'c', 'd', 'b', 'e'], index: 3, shuffle: forwardShuffle },
  { name: 'moves the paused item backward without resuming playback', from: 3, to: 1, current: 3, playing: false,
    order: ['a', 'd', 'b', 'c', 'e'], index: 1, shuffle: backwardShuffle },
  { name: 'shifts the current index left when another item crosses it forward', from: 1, to: 3, current: 2,
    order: ['a', 'c', 'd', 'b', 'e'], index: 1, shuffle: forwardShuffle },
  { name: 'shifts the current index right when another item crosses it backward', from: 3, to: 1, current: 2,
    order: ['a', 'd', 'b', 'c', 'e'], index: 3, shuffle: backwardShuffle },
  { name: 'preserves a current item before the affected range', from: 1, to: 3, current: 0,
    order: ['a', 'c', 'd', 'b', 'e'], index: 0, shuffle: forwardShuffle },
  { name: 'preserves a current item after the affected range', from: 3, to: 1, current: 4,
    order: ['a', 'd', 'b', 'c', 'e'], index: 4, shuffle: backwardShuffle },
  { name: 'reorders a queued list without creating a playback session', from: 1, to: 3, current: -1, playing: false,
    order: ['a', 'c', 'd', 'b', 'e'], index: -1, shuffle: forwardShuffle },
]) {
  await test(scenario.name, () => {
    const state = fixture({ currentIndex: scenario.current, playing: scenario.playing })
    const before = snapshot(state)
    assert.equal(state.moveQueueItem(scenario.from, scenario.to, [...state.queue.value]), true)
    assertAccepted(state, before, scenario.order, scenario.index, scenario.shuffle)
    const restored = restoreSaved(state)
    assert.deepEqual(restored.queue.map(item => item.title), scenario.order)
    assert.equal(restored.queueIndex, scenario.index)
    assert.equal(restored.currentTrack?.title ?? null, before.currentTrack?.title ?? null)
    assert.equal(restored.hasPlaybackSession, before.hasPlaybackSession)
  })
}

for (const scenario of [
  { name: 'moves the first item to the last slot', from: 0, to: 4,
    order: ['b', 'c', 'd', 'e', 'a'], index: 1,
    shuffle: { bag: [2, 3, 0], history: [3, 4, 1, 4], future: [2, 0] } },
  { name: 'moves the last item to the first slot', from: 4, to: 0,
    order: ['e', 'a', 'b', 'c', 'd'], index: 3,
    shuffle: { bag: [4, 0, 2], history: [0, 1, 3, 1], future: [4, 2] } },
]) {
  await test(scenario.name, () => {
    const state = fixture({ bag: [3, 4, 1], history: [4, 0, 2, 0], future: [3, 1] })
    const before = snapshot(state)
    assert.equal(state.moveQueueItem(scenario.from, scenario.to), true)
    assertAccepted(state, before, scenario.order, scenario.index, scenario.shuffle)
  })
}

await test('keeps positional identity for duplicate song ids and restores the selected playlist occurrence', () => {
  const state = fixture({
    tracks: [track('dup', 'first', 'list:first'), track('middle'), track('dup', 'selected', 'list:selected'), track('last')],
    currentIndex: 2, bag: [0, 3], history: [2, 1, 0], future: [1, 0],
  })
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(0, 3, [...state.queue.value]), true)
  assertAccepted(state, before, ['middle', 'selected', 'last', 'first'], 1,
    { bag: [3, 2], history: [1, 0, 3], future: [0, 3] })
  assert.equal(state.queue.value[1], before.queue[2])
  const restored = restoreSaved(state)
  assert.deepEqual(restored.queue.map(item => item.title), ['middle', 'selected', 'last', 'first'])
  assert.equal(restored.queueIndex, 1)
  assert.equal(restored.currentTrack.playlistKey, 'list:selected')
})

await test('tracks separate slots when the identical object appears twice', () => {
  const repeated = track('dup', 'repeated')
  const state = fixture({ tracks: [repeated, track('middle'), repeated, track('last')],
    currentIndex: 2, bag: [0, 3], history: [2, 1, 0], future: [1, 0] })
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(0, 3), true)
  assertAccepted(state, before, ['middle', 'repeated', 'last', 'repeated'], 1,
    { bag: [3, 2], history: [1, 0, 3], future: [0, 3] })
  assert.equal(state.queue.value[1], before.queue[2])
  assert.equal(state.queue.value[3], before.queue[0])
})

await test('restores the selected duplicate id slot without a playlist key after reordering', () => {
  const state = fixture({
    tracks: [track('dup', 'selected'), track('middle'), track('dup', 'other'), track('last')],
    currentIndex: 0, bag: [2, 3], history: [0, 1, 2], future: [1, 2],
  })
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(2, 0), true)
  assertAccepted(state, before, ['other', 'selected', 'middle', 'last'], 1,
    { bag: [0, 3], history: [1, 2, 0], future: [2, 0] })
  const restored = restoreSaved(state)
  assert.deepEqual(restored.queue.map(item => item.title), ['other', 'selected', 'middle', 'last'])
  assert.equal(restored.queueIndex, 1)
  assert.equal(restored.currentTrack.title, 'selected')
})

await test('remaps populated shuffle stacks even when shuffle mode is disabled', () => {
  const state = fixture({ shuffle: false })
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(1, 3), true)
  assertAccepted(state, before, ['a', 'c', 'd', 'b', 'e'], 1, forwardShuffle)
})

await test('preserves empty shuffle stacks without rebuilding random traversal', () => {
  const state = fixture({ bag: [], history: [], future: [] })
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(3, 1), true)
  assertAccepted(state, before, ['a', 'd', 'b', 'c', 'e'], 3, { bag: [], history: [], future: [] })
})

for (const [from, to] of [
  [-1, 1], [1, -1], [5, 1], [1, 5], [1.5, 3], [1, 3.5],
  [NaN, 1], [1, NaN], [Infinity, 1], [1, Infinity], ['1', 3], [1, '3'],
  [null, 1], [1, undefined], [2, 2],
]) {
  await test(`rejects invalid or unchanged indices (${String(from)} -> ${String(to)})`, () => {
    const state = fixture()
    const before = snapshot(state)
    assert.equal(state.moveQueueItem(from, to, [...state.queue.value]), false)
    assertRejected(state, before)
  })
}

for (const tracks of [[], [track('only')]]) {
  await test(`rejects a move in a queue of length ${tracks.length}`, () => {
    const state = fixture({ tracks, currentIndex: tracks.length ? 0 : -1, bag: [], history: [], future: [] })
    const before = snapshot(state)
    assert.equal(state.moveQueueItem(0, 1), false)
    assertRejected(state, before)
  })
}

for (const staleSnapshot of [
  { name: 'a shortened queue', create: items => items.slice(0, 4) },
  { name: 'an appended queue', create: items => [...items, track('extra')] },
  { name: 'a replaced object with the same song identity', create: items => [items[0], { ...items[1] }, ...items.slice(2)] },
  { name: 'the same objects in a different order', create: items => [items[1], items[0], ...items.slice(2)] },
]) {
  await test(`rejects an outdated drag snapshot containing ${staleSnapshot.name}`, () => {
    const state = fixture()
    const expectedQueue = staleSnapshot.create([...state.queue.value])
    const before = snapshot(state)
    assert.equal(state.moveQueueItem(1, 3, expectedQueue), false)
    assertRejected(state, before)
  })
}

await test('allows a new queue array whose positional object references match the drag snapshot', () => {
  const state = fixture()
  const expectedQueue = [...state.queue.value]
  state.queue.value = [...state.queue.value]
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(1, 3, expectedQueue), true)
  assertAccepted(state, before, ['a', 'c', 'd', 'b', 'e'], 1, forwardShuffle)
})

await test('blocks local reordering when Listen Together denies local control', () => {
  const state = fixture({ blocked: true })
  const before = snapshot(state)
  assert.equal(state.moveQueueItem(1, 3, [...state.queue.value]), false)
  assertRejected(state, before)
  assert.deepEqual(state.calls, [['blockedByListenTogether', 'local']])
})

console.log(`queue reorder tests: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
