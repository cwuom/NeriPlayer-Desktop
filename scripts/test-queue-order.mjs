import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { computed, ref, watch } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const names = [
  'next', 'previous', 'resume', 'nextPrefetchTracks', 'rebuildShuffleBag', 'pendingReplayIndex',
  'addToQueueNext', 'addToQueueEnd', 'shiftShuffleIndicesForPlayNext', 'shiftShuffleIndicesForRemove',
  'removeFromQueue', 'moveQueueItem', 'clearQueue', 'playQueueItem', 'playAll', 'shufflePlay', 'applyListenTogetherPlaybackMode',
]
const functions = new Map(), declarations = new Map()
function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name) functions.set(node.name.text, node.getText(parsed))
  if (ts.isVariableDeclaration(node)) declarations.set(node.name.getText(parsed), 'const ' + node.getText(parsed))
  ts.forEachChild(node, visit)
}
visit(parsed)
for (const name of names) assert.ok(functions.has(name), 'missing queue behavior: ' + name)
const displayNames = ['queueDisplayOrder', 'queueDisplayIndices', 'queueDisplayCurrentIndex']
for (const name of displayNames) assert.ok(declarations.has(name), 'missing queue display state: ' + name)
const compile = text => ts.transpileModule(text, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const compiled = compile([
  ...names.map(name => functions.get(name)), ...displayNames.map(name => declarations.get(name)),
].join('\n'))
async function importSource(path) {
  const text = await readFile(new URL(path, import.meta.url), 'utf8')
  const output = ts.transpileModule(text, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
  }).outputText
  return import('data:text/javascript;base64,' + Buffer.from(output).toString('base64'))
}
const { buildQueueDisplayOrder } = await importSource('../src/modules/playback/queueDisplayOrder.ts')
const { resolvePlaybackQueueStartIndex } = await importSource('../src/modules/playback/playbackQueue.ts')
const commitTrack = compile(functions.get('commitTrack'))
const track = (id, title = id) => ({ id, title, artist: 'artist', album: '', durationMs: 180000, source: 'netease' })

function fixture({ tracks = ['a', 'b', 'c', 'd', 'e'].map(id => track(id)), current = 0,
  bag = [1, 2, 3, 4], history = [], future = [], shuffle = true, blocked = false,
  source = 'netease', needsReload = false } = {}) {
  const played = [], saved = [], seeks = []
  const queue = ref(tracks), queueIndex = ref(current), currentTrack = ref(tracks[current] ?? null)
  const shuffleBag = ref([...bag]), shuffleHistory = ref([...history]), shuffleFuture = ref([...future])
  const context = {
    ref, computed, buildQueueDisplayOrder, queue, queueIndex, currentTrack,
    shuffleBag, shuffleHistory, shuffleFuture, shuffleEnabled: ref(shuffle), repeatMode: ref('off'),
    positionMs: ref(0), log: { info() {} }, Math: Object.assign(Object.create(Math), { random: () => 0.99 }),
    blockedByListenTogether: () => blocked, blocksLocalSongInRoom: () => false, markCommandSource() {},
    isRemotePlaybackTrack: () => true, getPlaybackSourceKindForPrefetch: () => source,
    savePlayerState: () => saved.push([...queue.value]), maybePrefetchNext() {},
    useListenTogetherStore: () => ({ reportQueueReorder() {} }),
    pause: async () => {}, seekTo: position => seeks.push(position),
    currentAudioFileMutation: () => null, isLoadingAudio: ref(false), isPlaying: ref(false),
    shouldDeferPlaybackSeek: () => false, shouldRefreshUrlBeforeResume: () => false,
    currentStreamUrl: ref(null), getPlaybackSourceKind: () => source,
    startInterpolationFromRenderedPosition() {}, useSettingsStore: () => ({ fadeIn: false }),
    setLocalPlaylistSource() {}, tracePlaybackUi() {},
    resolvePlaybackQueueStartIndex, invoke: async () => {},
    play: async (track, commandSource, start, force, remembered, requestedQueueIndex, shuffleNavigation = false) => {
      played.push({ track, index: requestedQueueIndex })
      const apply = new Function(...Object.keys(context), 'track', 'requestedQueueIndex', 'shuffleNavigation',
        'let trackCommitted = false\n' + commitTrack + '\ncommitTrack()')
      apply(...Object.values(context), track, requestedQueueIndex, shuffleNavigation)
    },
  }
  const methods = new Function(...Object.keys(context),
    'let pendingReplayTrackId = null, _isAutoSkipping = false\n'
    + 'let consecutivePlayFailures = 0, consecutiveLoginSkips = 0\n'
    + 'let _needsReload = ' + JSON.stringify(needsReload) + ', playbackRequestToken = 1, loadedPlaybackRequestToken = 1\n'
    + compiled + '\nreturn { ' + [...names, ...displayNames].join(', ') + ' }'
  )(...Object.values(context))
  return { ...methods, ...context, played, saved, seeks }
}

let passed = 0
async function test(name, run) {
  await run()
  passed++
  console.log('PASS ' + name)
}

await test('next consumes the displayed pre-shuffled slot and the prefetched track', async () => {
  const state = fixture()
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 3, 4])
  assert.equal(state.nextPrefetchTracks()[0].id, 'b')
  await state.next()
  assert.equal(state.currentTrack.value.id, 'b')
  assert.equal(state.queueIndex.value, 1)
  assert.equal(state.queueDisplayCurrentIndex.value, 1)
  await state.next()
  assert.equal(state.currentTrack.value.id, 'c')
})

await test('previous backtracking and future next preserve the visible order', async () => {
  const state = fixture({ current: 2, history: [0, 1], future: [4], bag: [3] })
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 4, 3])
  await state.previous()
  assert.equal(state.queueIndex.value, 1)
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 4, 3])
  assert.equal(state.nextPrefetchTracks()[0].id, 'c')
  await state.next()
  assert.equal(state.queueIndex.value, 2)
  assert.equal(state.queueDisplayCurrentIndex.value, 2)
})

await test('direct play consumes the selected shuffle slot and records its predecessor', async () => {
  const state = fixture()
  await state.play(state.queue.value[1])
  assert.equal(state.nextPrefetchTracks()[0].id, 'c')
  assert.deepEqual(state.shuffleHistory.value, [0])
  await state.next()
  assert.equal(state.currentTrack.value.id, 'c')
  assert.deepEqual(state.shuffleHistory.value, [0, 1], 'navigation must not record history twice')
  await state.previous()
  assert.equal(state.currentTrack.value.id, 'b')
  assert.deepEqual(state.shuffleHistory.value, [0], 'previous must only pop its original history entry')
})

await test('restoring the current track preserves pending order without adding history', async () => {
  const state = fixture({ current: 2, history: [0], bag: [1, 3, 4], needsReload: true })
  state.positionMs.value = 43000
  await state.resume()
  assert.equal(state.queueIndex.value, 2)
  assert.deepEqual(state.shuffleHistory.value, [0])
  assert.deepEqual(state.shuffleBag.value, [1, 3, 4])
  assert.equal(state.nextPrefetchTracks()[0].id, 'b')
})

await test('remote direct play consumes the bag slot while preserving every explicit future replay', async () => {
  const state = fixture({ future: [1, 1] })
  await state.play(state.queue.value[1], 'remote_sync')
  assert.deepEqual(state.shuffleBag.value, [2, 3, 4])
  assert.deepEqual(state.shuffleFuture.value, [1, 1])
  assert.deepEqual(state.shuffleHistory.value, [0])
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 1, 1, 2, 3, 4])
  await state.next()
  await state.next()
  assert.equal(state.queueIndex.value, 1)
  assert.equal(state.nextPrefetchTracks()[0].id, 'c')
})

await test('restoring a previous-cycle current slot preserves its separate upcoming bag visit', async () => {
  const state = fixture({ current: 1, history: [0], future: [0, 2], bag: [1], needsReload: true })
  await state.resume()
  assert.deepEqual(state.shuffleHistory.value, [0])
  assert.deepEqual(state.shuffleFuture.value, [0, 2])
  assert.deepEqual(state.shuffleBag.value, [1])
  assert.deepEqual(state.queueDisplayIndices.value, [3, 4, 1, 2, 0, 1])
})

await test('metadata clones on both current track and queue preserve traversal when resuming', async () => {
  const state = fixture({ current: 1, history: [0], future: [0, 2], bag: [1], needsReload: true })
  state.currentTrack.value = { ...state.currentTrack.value, title: 'edited' }
  state.queue.value = state.queue.value.map(item => ({ ...item }))
  await state.resume()
  assert.equal(state.queueIndex.value, 1)
  assert.deepEqual(state.shuffleHistory.value, [0])
  assert.deepEqual(state.shuffleFuture.value, [0, 2])
  assert.deepEqual(state.shuffleBag.value, [1])
})

await test('selecting another track after a metadata clone records the old logical slot', async () => {
  const state = fixture()
  state.currentTrack.value = { ...state.currentTrack.value, title: 'edited' }
  await state.playQueueItem(1)
  assert.deepEqual(state.shuffleHistory.value, [0])
  assert.deepEqual(state.shuffleBag.value, [2, 3, 4])
  await state.previous()
  assert.equal(state.queueIndex.value, 0)
})

await test('restoring cloned duplicate metadata retains the current occurrence without a playlist key', async () => {
  const state = fixture({ tracks: [track('same'), track('b'), track('same')], current: 2,
    history: [0], future: [0], bag: [1, 2], needsReload: true })
  state.currentTrack.value = { ...state.currentTrack.value }
  state.queue.value = state.queue.value.map(item => ({ ...item }))
  await state.resume()
  assert.equal(state.queueIndex.value, 2)
  assert.deepEqual(state.shuffleHistory.value, [0])
  assert.deepEqual(state.shuffleBag.value, [1, 2])
})

await test('replaced logical identity does not invent a predecessor from queue slot zero', async () => {
  const state = fixture()
  state.currentTrack.value = track('removed')
  await state.playQueueItem(1)
  assert.deepEqual(state.shuffleHistory.value, [])
})

await test('playAll starts a fresh traversal even when it reuses the previous current track', () => {
  const state = fixture({ history: [3], future: [4] })
  state.playAll([...state.queue.value], 'b')
  assert.equal(state.queueIndex.value, 1)
  assert.deepEqual(state.shuffleHistory.value, [])
  assert.deepEqual(state.shuffleFuture.value, [])
  assert.deepEqual(state.shuffleBag.value, [0, 2, 3, 4])
})

await test('shufflePlay keeps the rebuilt bag and empty history when restarting the same queue', () => {
  const state = fixture({ current: 1, history: [0], future: [4], bag: [2, 3] })
  state.shufflePlay([...state.queue.value])
  assert.equal(state.queueIndex.value, 0)
  assert.deepEqual(state.shuffleHistory.value, [])
  assert.deepEqual(state.shuffleFuture.value, [])
  assert.deepEqual(state.shuffleBag.value, [1, 2, 3, 4])
})

await test('cross-cycle previous keeps both future visits and the next cycle bag visit', async () => {
  const state = fixture({ tracks: ['a', 'b', 'c'].map(id => track(id)), bag: [1, 2] })
  await state.next()
  await state.next()
  await state.next(true)
  await state.previous()
  await state.previous()
  await state.previous()
  assert.equal(state.queueIndex.value, 0)
  assert.deepEqual(state.shuffleFuture.value, [0, 2, 1])
  assert.deepEqual(state.shuffleBag.value, [1])
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 0, 1])
  const following = []
  for (let count = 0; count < 4; count++) {
    await state.next()
    following.push(state.queueIndex.value)
  }
  assert.deepEqual(following, [1, 2, 0, 1])
})

await test('YouTube prefetch follows both future stack entries before bag entries', () => {
  const state = fixture({ current: 2, history: [0], future: [1, 3], bag: [4], source: 'youtube' })
  assert.deepEqual(state.nextPrefetchTracks().map(track => track.id), ['d', 'b'])
  assert.deepEqual(state.queueDisplayIndices.value, [0, 2, 3, 1, 4])
})

await test('Play Next promotion updates displayed next and prefetch synchronously', async () => {
  const state = fixture()
  state.addToQueueNext(track('d'))
  assert.deepEqual(state.queueDisplayIndices.value.map(index => state.queue.value[index].id), ['a', 'd', 'b', 'c', 'e'])
  assert.equal(state.nextPrefetchTracks()[0].id, 'd')
  await state.next()
  assert.equal(state.currentTrack.value.id, 'd')
})

await test('current-track replay displays a separate future row with one active current row', async () => {
  const state = fixture()
  const observations = []
  const stop = watch(state.queueDisplayIndices, indices => observations.push([...indices]), { flush: 'sync' })
  state.addToQueueNext(state.queue.value[0])
  assert.deepEqual(state.queueDisplayIndices.value, [0, 0, 1, 2, 3, 4])
  assert.equal(state.queueDisplayCurrentIndex.value, 0)
  assert.ok(observations.some(indices => indices.length === 6), 'future-only mutation must notify the UI')
  stop()
  await state.next()
  assert.equal(state.queueIndex.value, 0)
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 3, 4])
})

await test('history and repeated cycles list every physical slot once', () => {
  const state = fixture({ current: 2, history: [0, 1, 0, 4], future: [3], bag: [4] })
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 3, 4])
  assert.equal(state.queueDisplayCurrentIndex.value, 2)
})

await test('future keeps repeated replay occurrences after previous moves to a different song', async () => {
  const state = fixture({ current: 1, history: [0], future: [0, 0], bag: [2, 3, 4] })
  assert.deepEqual(state.queueDisplayIndices.value, [1, 0, 0, 2, 3, 4])
  assert.equal(state.queueDisplayCurrentIndex.value, 0)
  await state.next()
  assert.equal(state.queueIndex.value, 0)
  assert.equal(state.nextPrefetchTracks()[0].id, 'a')
  await state.next()
  assert.equal(state.queueIndex.value, 0)
})

await test('missing or invalid traversal slots still display the complete physical queue', () => {
  assert.deepEqual(buildQueueDisplayOrder(5, 2, true, [0, -1], [9], [4]), {
    indices: [0, 1, 3, 2, 4], currentIndex: 3,
  })
  assert.deepEqual(buildQueueDisplayOrder(3, -1, true, [], [], [1, 2, 0]), {
    indices: [1, 2, 0], currentIndex: -1,
  })
})

await test('deletion remaps future and bag rows to their remaining physical slots', async () => {
  const state = fixture({ current: 2, history: [0], future: [3], bag: [1, 4] })
  state.removeFromQueue(1)
  assert.deepEqual(state.queueDisplayIndices.value.map(index => state.queue.value[index].id), ['a', 'c', 'd', 'e'])
  await state.next()
  assert.equal(state.currentTrack.value.id, 'd')
})

await test('deleting the current slot consumes the replacement slot from the unplayed bag', async () => {
  const state = fixture()
  state.removeFromQueue(0)
  assert.equal(state.currentTrack.value.id, 'b')
  assert.deepEqual(state.queueDisplayIndices.value.map(index => state.queue.value[index].id), ['b', 'c', 'd', 'e'])
  assert.equal(state.nextPrefetchTracks()[0].id, 'c')
  await state.next()
  assert.equal(state.currentTrack.value.id, 'c')
})

await test('physical reordering preserves actual playback order and current position', () => {
  const state = fixture({ current: 2, history: [0], future: [3], bag: [1, 4] })
  const before = state.queueDisplayIndices.value.map(index => state.queue.value[index].id)
  assert.equal(state.moveQueueItem(3, 0, [...state.queue.value]), true)
  assert.deepEqual(state.queueDisplayIndices.value.map(index => state.queue.value[index].id), before)
  assert.equal(state.queueIndex.value, 3)
})

await test('explicit queue selection preserves the chosen duplicate slot and consumes its bag visit', async () => {
  const duplicate = track('same')
  const state = fixture({ tracks: [duplicate, track('b'), duplicate], bag: [2, 1] })
  await state.playQueueItem(2)
  assert.equal(state.queueIndex.value, 2)
  assert.equal(state.queueDisplayCurrentIndex.value, 1)
  assert.deepEqual(state.queueDisplayIndices.value, [0, 2, 1])
  await state.next()
  assert.equal(state.queueIndex.value, 1)
  await state.previous()
  assert.equal(state.queueIndex.value, 2)
})

await test('shuffle next and previous preserve two identical object occurrences', async () => {
  const duplicate = track('same')
  const state = fixture({ tracks: [duplicate, track('b'), duplicate], bag: [2, 1] })
  await state.next()
  assert.equal(state.queueIndex.value, 2)
  await state.previous()
  assert.equal(state.queueIndex.value, 0)
  await state.next()
  assert.equal(state.queueIndex.value, 2)
})

await test('appending, remote mode changes, and clearing update the exposed order', () => {
  const state = fixture({ shuffle: false, current: 2, bag: [] })
  assert.deepEqual(state.queueDisplayIndices.value, [0, 1, 2, 3, 4])
  state.applyListenTogetherPlaybackMode({ shuffleEnabled: true })
  assert.equal(state.queueDisplayCurrentIndex.value, 0)
  state.addToQueueEnd(track('f'))
  assert.equal(state.queueDisplayIndices.value.at(-1), 5)
  state.clearQueue()
  assert.deepEqual(state.queueDisplayIndices.value, [])
  assert.equal(state.queueDisplayCurrentIndex.value, -1)
})

await test('room restrictions reject explicit selection without modifying traversal', async () => {
  const state = fixture({ blocked: true })
  const before = [...state.queueDisplayIndices.value]
  await state.playQueueItem(2)
  assert.deepEqual(state.queueDisplayIndices.value, before)
  assert.deepEqual(state.played, [])
})

console.log('queue playback order: ' + passed + ' passed')
