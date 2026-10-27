import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { ref } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const names = ['pause', 'handleTrackEnded']
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

function fixture({ inRoom = true, controller = false, last = false, pendingReplay = -1 } = {}) {
  let releasePause
  const pauseAck = new Promise(resolve => { releasePause = resolve })
  const calls = [], reports = []
  const currentTrack = ref({ id: 'netease:1', durationMs: 180000 })
  const lt = {
    roomId: inRoom ? 'room-1' : null, isController: controller,
    reportTrackFinished: id => reports.push(id),
  }
  const context = {
    currentTrack, durationMs: ref(180000), isPlaying: ref(true), isLoadingAudio: ref(false),
    hasPlaybackSession: ref(true), queue: ref(last ? [currentTrack.value] : [currentTrack.value, { id: 'netease:2' }]),
    queueIndex: ref(0), shuffleEnabled: ref(false), repeatMode: ref('off'), sleepTimerMode: ref(null),
    pendingReplayIndex: () => pendingReplay,
    useListenTogetherStore: () => lt,
    blockedByListenTogether: () => false, currentAudioFileMutation: () => null,
    markCommandSource() {}, freezeRenderedPosition() {}, persistCurrentLongFormProgress() {},
    persistLongFormProgress() {}, savePlayerState() {}, cancelSleepTimer() {},
    playbackStartupWatchdog: { cancel() {} },
    useSettingsStore: () => ({ fadeIn: false }),
    invoke: async command => { calls.push(command); await pauseAck },
    next: async () => { calls.push('next') }, play: async () => { calls.push('play') },
  }
  const methods = new Function(...Object.keys(context), `
    let playbackRequestToken = 1, loadedPlaybackRequestToken = 1
    let _interpIsPlaying = true, lastTrackEndedId = null, lastTrackEndedTime = 0, lastTrackEndedRequestToken = -1
    ${compiled}
    return {
      handleTrackEnded,
      replaceTrack(id) { playbackRequestToken++; currentTrack.value = { id, durationMs: 180000 } },
    }
  `)(...Object.values(context))
  return { ...methods, currentTrack, lt, calls, reports, releasePause }
}

let passed = 0, failed = 0
async function test(name, run) {
  try { await run(); passed++; console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

await test('a listener reports the finished song after the native pause acknowledgment', async () => {
  const state = fixture()
  const ended = state.handleTrackEnded()
  assert.deepEqual(state.calls, ['pause'])
  assert.deepEqual(state.reports, [])
  state.releasePause()
  await ended
  assert.deepEqual(state.reports, ['netease:1'])
})

for (const id of ['netease:2', 'netease:1']) {
  await test(`a superseded playback session cannot report a finish after pause (${id})`, async () => {
    const state = fixture()
    const ended = state.handleTrackEnded()
    state.replaceTrack(id)
    state.releasePause()
    await ended
    assert.deepEqual(state.reports, [], 'an old finish must not advance the replacement playback session')
  })
}

await test('leaving and joining another room during pause cannot report an old finish to the new room', async () => {
  const state = fixture()
  const ended = state.handleTrackEnded()
  state.lt.roomId = 'room-2'
  state.releasePause()
  await ended
  assert.deepEqual(state.reports, [])
})

await test('duplicate native finish notifications do not report twice', async () => {
  const state = fixture()
  const first = state.handleTrackEnded()
  await state.handleTrackEnded()
  state.releasePause()
  await first
  assert.deepEqual(state.calls, ['pause'])
  assert.deepEqual(state.reports, ['netease:1'])
})

for (const config of [{ inRoom: false }, { controller: true }]) {
  await test(`local playback and the room controller keep normal queue advancement (${JSON.stringify(config)})`, async () => {
    const state = fixture(config)
    await state.handleTrackEnded()
    assert.deepEqual(state.calls, ['next'])
    assert.deepEqual(state.reports, [])
  })
}

await test('a pending replay advances a last track with repeat off', async () => {
  const state = fixture({ inRoom: false, last: true, pendingReplay: 0 })
  await state.handleTrackEnded()
  assert.deepEqual(state.calls, ['next'], 'the queued replay must be consumed instead of stopping at the queue end')
})

console.log(`playback ended tests: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
