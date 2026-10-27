import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { ref } from 'vue'

const source = readFileSync(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
function declaration(name) {
  let found
  function visit(node) { if (ts.isFunctionDeclaration(node) && node.name?.text === name) found = node; ts.forEachChild(node, visit) }
  visit(parsed)
  assert.ok(found, `player implements ${name}`)
  return found.getText(parsed)
}
const compiled = ts.transpileModule([
  declaration('prepareBiliVideoSkipTrack'), declaration('applyResolvedBiliVideoSkipTarget'), declaration('maybeAutoSkipBiliVideoInterval'),
].join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
const policy = {}
new Function('exports', ts.transpileModule(readFileSync(new URL('../src/modules/playback/biliVideoSkip.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(policy)
const track = { id: 'bilibili:BVtest', album: 'Bilibili|101', syncPayload: { originalName: 'Song' } }
function runtime({ roomId = null, playing = true, loading = false, generation = 7, loadedGeneration = 7, fixtureTrack = track } = {}) {
  const calls = [], state = { roomId }
  const currentTrack = ref({ ...fixtureTrack, syncPayload: { ...fixtureTrack.syncPayload } })
  const currentBiliVideoSkipTarget = ref(null), rules = ref([{ bvid: 'BVtest', cid: 101, intervals: [{ startMs: 1000, endMs: 5000 }], modifiedAt: 100, isDeleted: false }])
  const context = {
    ...policy, currentTrack, currentBiliVideoSkipTarget, biliVideoSkipTracker: new policy.BiliVideoSkipTracker(),
    isPlaying: ref(playing), isLoadingAudio: ref(loading),
    pendingReplayIndex: () => -1,
    useListenTogetherStore: () => state, useBiliVideoSkipStore: () => ({ rules: rules.value }),
    seekTo: async (ms, commandSource) => { calls.push({ ms, commandSource }) },
    handleTrackEnded: async () => { calls.push({ ended: true }) },
    invoke: async command => { assert.equal(command, 'get_bili_video_skip_targets'); return [] },
    savePlayerState: () => {}, log: { warn: () => {} },
  }
  const methods = new Function(...Object.keys(context), `let playbackRequestToken=${generation}, loadedPlaybackRequestToken=${loadedGeneration}, biliVideoSkipRequestToken=0, _needsReload=false, pendingSeek=null;\n${compiled}\nreturn {prepareBiliVideoSkipTrack, applyResolvedBiliVideoSkipTarget, maybeAutoSkipBiliVideoInterval, changeGeneration: () => playbackRequestToken++, setPendingSeek: value => pendingSeek = value };`)(...Object.values(context))
  methods.prepareBiliVideoSkipTrack(currentTrack.value, generation)
  return { ...methods, ...context, state, rules, calls }
}
let cases = 0
function regression(name, run) { run(); cases++; console.log(`ok - ${name}`) }
regression('playing exact page seeks to its interval end through local safety', () => {
  const r = runtime()
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), true)
  assert.deepEqual(r.calls, [{ ms: 5000, commandSource: 'local_safety' }])
  assert.equal(r.maybeAutoSkipBiliVideoInterval(2000, 10000), false)
})
regression('all active room roles disable automatic interval skips', () => {
  const r = runtime({ roomId: 'room' })
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
  assert.deepEqual(r.calls, [])
  r.state.roomId = null
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), true)
})
regression('paused, loading, pending seek and obsolete sessions do not seek', () => {
  for (const options of [{ playing: false }, { loading: true }, { loadedGeneration: 6 }]) {
    const r = runtime(options)
    assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
    assert.deepEqual(r.calls, [])
  }
  const r = runtime(); r.setPendingSeek({ targetMs: 1000 })
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
  r.setPendingSeek(null); r.changeGeneration()
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
})
regression('resolved targets replace pages and persist offline origin identity', () => {
  const r = runtime()
  r.applyResolvedBiliVideoSkipTarget(r.currentTrack.value, { bvid: 'BVtest', cid: 102 }, 7)
  assert.deepEqual(r.currentBiliVideoSkipTarget.value, { bvid: 'BVtest', cid: 102 })
  assert.equal(r.currentTrack.value.syncPayload.originalName, 'Song')
  assert.equal(r.currentTrack.value.syncPayload.subAudioId, '102')
  assert.equal(r.currentTrack.value.syncPayload.album, 'Bilibili|102|BVtest')
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
})
regression('active room playback retains its received source identity while resolving the runtime target', () => {
  for (const fixtureTrack of [
    { id: 'bilibili:BVtest', album: '', syncPayload: { channelId: 'bilibili', audioId: 'BVtest' } },
    { id: 'bilibili:12345', album: '', syncPayload: { channelId: 'bilibili', audioId: '12345' } },
    { id: 'bilibili:BVtest', album: 'Bilibili|101', syncPayload: { channelId: 'bilibili', audioId: 'BVtest', subAudioId: '101' } },
  ]) {
    const r = runtime({ roomId: 'room', fixtureTrack })
    const received = JSON.parse(JSON.stringify(r.currentTrack.value))
    r.applyResolvedBiliVideoSkipTarget(r.currentTrack.value, { bvid: 'BVtest', cid: 102 }, 7)
    assert.deepEqual(r.currentBiliVideoSkipTarget.value, { bvid: 'BVtest', cid: 102 })
    assert.deepEqual(r.currentTrack.value, received, 'resolving a stream cannot rewrite the room track or its album CID')
    assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
  }
})
regression('leaving a room restores local metadata completion despite a retained queue reference', () => {
  const r = runtime({ roomId: 'room', fixtureTrack: {
    id: 'bilibili:12345', album: '',
    playlistKey: JSON.stringify({ stableKey: 'bilibili:12345', occurrence: 0 }),
    syncPayload: { channelId: 'bilibili', audioId: '12345', originalName: 'Song' },
  } })
  r.applyResolvedBiliVideoSkipTarget(r.currentTrack.value, { bvid: 'BVtest', cid: 102 }, 7)
  assert.equal(r.currentTrack.value.album, '')
  assert.equal(r.currentTrack.value.syncPayload.audioId, '12345')
  r.state.roomId = null
  r.applyResolvedBiliVideoSkipTarget(r.currentTrack.value, { bvid: 'BVtest', cid: 102 }, 7)
  assert.equal(r.currentTrack.value.album, 'Bilibili|102|BVtest')
  assert.equal(r.currentTrack.value.syncPayload.audioId, 'BVtest')
  assert.equal(r.currentTrack.value.syncPayload.subAudioId, '102')
  assert.equal(r.currentTrack.value.syncPayload.originalName, 'Song')
})
regression('late resolutions cannot change another playback request', () => {
  const r = runtime(); r.changeGeneration()
  r.applyResolvedBiliVideoSkipTarget(r.currentTrack.value, { bvid: 'BVother', cid: 102 }, 7)
  assert.deepEqual(r.currentBiliVideoSkipTarget.value, { bvid: 'BVtest', cid: 101 })
})
regression('a repeated playback request resets the interval tracker', () => {
  const r = runtime()
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), true)
  r.prepareBiliVideoSkipTrack(r.currentTrack.value, 7)
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), true)
})
regression('tail interval advances through the existing ended handler without decoder EOF seeks', () => {
  const r = runtime(); r.rules.value[0].intervals = [{ startMs: 1000, endMs: 20000 }]
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), true)
  assert.deepEqual(r.calls, [{ ended: true }])
  assert.equal(r.maybeAutoSkipBiliVideoInterval(2000, 10000), false)
})
regression('an active room cannot advance locally through a tail interval', () => {
  const r = runtime({ roomId: 'room' }); r.rules.value[0].intervals = [{ startMs: 1000, endMs: 20000 }]
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 10000), false)
  assert.deepEqual(r.calls, [])
})
const endedCompiled = ts.transpileModule([
  declaration('prepareBiliVideoSkipTrack'), declaration('maybeAutoSkipBiliVideoInterval'), declaration('handleTrackEnded'),
].join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
function tailLoopRuntime({ repeat = 'one' } = {}) {
  const calls = [], durationMs = ref(1800000)
  const currentTrack = ref({ ...track, durationMs: durationMs.value })
  const context = {
    ...policy, calls, currentTrack, durationMs, currentBiliVideoSkipTarget: ref(null),
    biliVideoSkipTracker: new policy.BiliVideoSkipTracker(), isPlaying: ref(true), isLoadingAudio: ref(false),
    pendingReplayIndex: () => -1,
    useListenTogetherStore: () => ({ roomId: null }),
    useBiliVideoSkipStore: () => ({ rules: [{ bvid: 'BVtest', cid: 101, intervals: [{ startMs: 1000, endMs: 1800000 }], modifiedAt: 100, isDeleted: false }] }),
    queue: ref([currentTrack.value, { id: 'netease:next' }]), queueIndex: ref(0),
    repeatMode: ref(repeat), shuffleEnabled: ref(false), sleepTimerMode: ref(null),
    persistLongFormProgress() {}, cancelSleepTimer() {},
    next: async () => { calls.push('next') }, pause: async () => { calls.push('pause') },
    seekTo: async () => { calls.push('seek') }, log: { warn: () => {} },
  }
  const methods = new Function(...Object.keys(context), `
    let playbackRequestToken=7, loadedPlaybackRequestToken=7, biliVideoSkipRequestToken=0, _needsReload=false, pendingSeek=null;
    let lastTrackEndedId=null, lastTrackEndedTime=0, lastTrackEndedRequestToken=-1;
    async function play(track) {
      calls.push('repeat'); playbackRequestToken++; loadedPlaybackRequestToken=playbackRequestToken;
      prepareBiliVideoSkipTrack(track, playbackRequestToken);
    }
    ${endedCompiled}
    prepareBiliVideoSkipTrack(currentTrack.value, playbackRequestToken);
    return { maybeAutoSkipBiliVideoInterval, handleTrackEnded };
  `)(...Object.values(context))
  return { ...methods, calls }
}
regression('each new repeat-one generation can skip the same tail within two seconds', () => {
  const r = tailLoopRuntime()
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 1800000), true)
  assert.equal(r.maybeAutoSkipBiliVideoInterval(1000, 1800000), true)
  assert.deepEqual(r.calls, ['repeat', 'repeat'])
})
regression('duplicate finish notifications within the same generation remain deduplicated', () => {
  const r = tailLoopRuntime({ repeat: 'off' })
  void r.handleTrackEnded()
  void r.handleTrackEnded()
  assert.deepEqual(r.calls, ['next'])
})
assert.match(source, /commitBackendPosition\(normalizedPositionMs, e\.payload\.durationMs\)[\s\S]*?maybeAutoSkipBiliVideoInterval\(normalizedPositionMs, e\.payload\.durationMs\)/, 'accepted backend progress drives auto skip')
console.log(`Bilibili skip interval runtime: ${cases} passed`)
