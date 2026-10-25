import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import ts from 'typescript'

const sourceRoot = new URL('../src/stores/listenTogether/', import.meta.url)
const indexSource = await readFile(new URL('index.ts', sourceRoot), 'utf8')
const playerSource = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const playerParsed = ts.createSourceFile('player.ts', playerSource, ts.ScriptTarget.ES2022, true)
const playerCommandFunctions = new Map()
let resolvedBiliTargetFunction
function findPlayerCommandFunction(node) {
  if (ts.isFunctionDeclaration(node) && ['markCommandSource', 'isRemoteSyncGuardActive', 'markOptimisticSeek', 'moveQueueItem'].includes(node.name?.text)) {
    playerCommandFunctions.set(node.name.text, node.getText(playerParsed))
  }
  if (ts.isFunctionDeclaration(node) && node.name?.text === 'applyResolvedBiliVideoSkipTarget') {
    resolvedBiliTargetFunction = node.getText(playerParsed)
  }
  ts.forEachChild(node, findPlayerCommandFunction)
}
findPlayerCommandFunction(playerParsed)
assert.equal(playerCommandFunctions.size, 4)
assert.ok(resolvedBiliTargetFunction)
const resolvedBiliTargetCode = ts.transpileModule(resolvedBiliTargetFunction, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const commandSourceCode = ts.transpileModule([...playerCommandFunctions].filter(([name]) => name !== 'moveQueueItem').map(([, code]) => code).join('\n'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const queueReorderCode = ts.transpileModule(playerCommandFunctions.get('moveQueueItem'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
let moduleSequence = 0

async function loadSource(source, dependencies, globals = '') {
  const key = `__neriListenTogetherTest${++moduleSequence}`
  globalThis[key] = dependencies
  const output = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
    reportDiagnostics: true,
  })
  assert.equal(output.diagnostics?.length ?? 0, 0)
  const parsed = ts.createSourceFile('test.mjs', output.outputText, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  let compiled = output.outputText
  for (const statement of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(statement)) continue
    const specifier = statement.moduleSpecifier.text
    assert.ok(specifier in dependencies, `missing dependency ${specifier}`)
    const clause = statement.importClause
    const assignments = []
    if (clause?.name) assignments.push(`const ${clause.name.text} = __deps[${JSON.stringify(specifier)}].default;`)
    if (clause?.namedBindings && ts.isNamedImports(clause.namedBindings)) {
      const names = clause.namedBindings.elements.map(element =>
        element.propertyName ? `${element.propertyName.text}: ${element.name.text}` : element.name.text)
      assignments.push(`const { ${names.join(', ')} } = __deps[${JSON.stringify(specifier)}];`)
    }
    compiled = compiled.slice(0, statement.getStart(parsed)) + assignments.join('\n') + compiled.slice(statement.end)
  }
  try {
    const code = `const __deps = globalThis[${JSON.stringify(key)}];\n${globals}\n${compiled}\n//# sourceURL=listen-together-test-${moduleSequence}.mjs`
    return await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
  } finally {
    delete globalThis[key]
  }
}

function runtimeImports(source) {
  const output = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const parsed = ts.createSourceFile('test.mjs', output, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  return parsed.statements.filter(ts.isImportDeclaration).map(statement => statement.moduleSpecifier.text)
}

const helperModules = new Map()
async function loadHelper(url) {
  if (helperModules.has(url.href)) return helperModules.get(url.href)
  const pending = (async () => {
    const source = await readFile(url, 'utf8')
    const dependencies = {}
    for (const specifier of runtimeImports(source)) {
      assert.ok(specifier.startsWith('.') || specifier.startsWith('@/'), `unsupported helper import ${specifier}`)
      const target = specifier.startsWith('@/')
        ? new URL(`../../${specifier.slice(2)}.ts`, sourceRoot)
        : new URL(`${specifier}.ts`, url)
      dependencies[specifier] = await loadHelper(target)
    }
    return loadSource(source, dependencies)
  })()
  helperModules.set(url.href, pending)
  return pending
}

const helperDependencies = {}
for (const specifier of runtimeImports(indexSource).filter(specifier => specifier.startsWith('.'))) {
  helperDependencies[specifier] = await loadHelper(new URL(`${specifier}.ts`, sourceRoot))
}
const protocol = helperDependencies['./protocol']
const mapper = helperDependencies['./mapper']
const { resolvePlaybackQueueStartIndex } = await loadHelper(new URL('../../modules/playback/playbackQueue.ts', sourceRoot))
const { buildPersistedPlaybackQueue, restorePersistedPlaybackQueue } = await loadHelper(new URL('../../modules/playback/playerState.ts', sourceRoot))

async function flush() {
  for (let i = 0; i < 12; i++) {
    await Promise.resolve()
    await vue.nextTick()
  }
}

function deferred() {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}

function fakeTimers() {
  let now = 100_000
  let sequence = 0
  const pending = new Map()
  const history = []
  const add = (callback, delay, interval) => {
    const id = ++sequence
    pending.set(id, { callback, delay, interval, due: now + delay })
    history.push({ interval, delay })
    return id
  }
  return {
    Date: { now: () => now },
    setTimeout: (callback, delay = 0) => add(callback, delay, false),
    setInterval: (callback, delay = 0) => add(callback, delay, true),
    clearTimeout: id => pending.delete(id),
    clearInterval: id => pending.delete(id),
    history,
    active: () => [...pending.values()],
    async advance(milliseconds) {
      const target = now + milliseconds
      let iterations = 0
      while (true) {
        const next = [...pending.entries()].filter(([, timer]) => timer.due <= target)
          .sort((a, b) => a[1].due - b[1].due || a[0] - b[0])[0]
        if (!next) break
        assert.ok(++iterations < 1000, 'fake timer loop did not converge')
        const [id, timer] = next
        now = timer.due
        if (timer.interval) timer.due += timer.delay
        else pending.delete(id)
        timer.callback()
        await flush()
      }
      now = target
      await flush()
    },
  }
}

function wireTrack(id, extra = {}) {
  return {
    stableKey: `netease:${id}`, channelId: 'netease', audioId: String(id),
    name: `song ${id}`, artist: 'artist', durationMs: 60_000, ...extra,
  }
}

function room(tracks = [wireTrack(1)], extra = {}) {
  const index = extra.currentIndex ?? 0
  return {
    roomId: 'ABC234', version: 1, schemaVersion: 2,
    controllerUserUuid: 'controller', settings: {
      allowMemberControl: true, autoPauseOnMemberChange: true, shareAudioLinks: false,
    }, members: [], queue: tracks, currentIndex: index, track: tracks[index],
    playback: { state: 'paused', basePositionMs: 5000, baseTimestampMs: 100_000,
      playbackRate: 1, repeatMode: 0, shuffleEnabled: false },
    roomStatus: 'active', updatedAt: 100_000, ...extra,
  }
}

async function harness(options = {}) {
  const timers = fakeTimers()
  const events = new Map()
  const commands = []
  const playback = []
  const toasts = []
  const notices = []
  const logs = []
  const playGates = []
  const pauseGates = []
  const seekGates = []
  const lastCommandSource = vue.ref('local')
  const lastCommandSequence = vue.ref(0)
  const lastSeekCommand = vue.ref({ seq: 0, source: 'local', positionMs: 0 })
  const commandSource = new Function('lastCommandSource', 'lastCommandSequence', 'lastSeekCommand', 'setRenderedPosition', 'Date', `
    let _remoteSyncGuardUntil = 0
    let playbackRequestToken = 1, lastSeekedMs = null
    const clearPauseGuard = () => {}, armPendingSeek = () => {}
    ${commandSourceCode}
    return { markCommandSource, isRemoteSyncGuardActive, markOptimisticSeek }
  `)(lastCommandSource, lastCommandSequence, lastSeekCommand,
    value => { player.positionMs = value; return value }, timers.Date)
  const markSource = source => commandSource.markCommandSource(source ?? 'local')
  const clone = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value))
  const player = vue.reactive({
    queue: [], queueIndex: -1, currentTrack: null, hasPlaybackSession: false, isPlaying: false, isLoadingAudio: false,
    positionMs: 0, repeatMode: 'off', shuffleEnabled: false,
    lastCommandSource, lastCommandSequence,
    lastSeekCommand,
    getCurrentStreamUrl() { return player.currentTrack?.audioUrl || '' },
    getCurrentStreamUrls() { return [] },
    async resolveShareableStreamUrls(track) {
      playback.push({ type: 'resolve-shareable', trackId: track.id })
      return options.shareableUrls?.(track) ?? []
    },
    isRemoteSyncGuardActive: commandSource.isRemoteSyncGuardActive,
    setListenTogetherSyncPlaybackRate(rate) { playback.push({ type: 'rate', rate }) },
    applyListenTogetherPlaybackMode(mode) {
      playback.push({ type: 'mode', mode: clone(mode) })
      if (mode.repeatMode != null) player.repeatMode = protocol.wireRepeatToDesktop(mode.repeatMode)
      if (mode.shuffleEnabled != null) player.shuffleEnabled = mode.shuffleEnabled
    },
    async play(track, source, positionMs) {
      markSource(source)
      playback.push({ type: 'play', track: clone(track), source, positionMs })
      player.currentTrack = track
      player.queueIndex = resolvePlaybackQueueStartIndex(player.queue, track.id, track.playlistKey)
      if (positionMs > 1000) {
        commandSource.markOptimisticSeek(positionMs, source ?? 'local', { fromPlaybackStart: true, durationMs: track.durationMs })
      } else if (positionMs !== undefined) player.positionMs = positionMs
      player.hasPlaybackSession = true
      player.isLoadingAudio = true
      const gate = playGates.shift()
      if (gate) await gate.promise
      player.isLoadingAudio = false
      player.isPlaying = true
      player.hasPlaybackSession = true
    },
    async pause(source) {
      markSource(source)
      playback.push({ type: 'pause', source })
      const gate = pauseGates.shift()
      if (gate) await gate.promise
      player.isPlaying = false
    },
    async resume(source) { markSource(source); playback.push({ type: 'resume', source }); player.isPlaying = true },
    async seekTo(positionMs, source) {
      markSource(source)
      playback.push({ type: 'seek', positionMs, source })
      commandSource.markOptimisticSeek(positionMs, source ?? 'local')
      const gate = seekGates.shift()
      if (gate) await gate.promise
    },
  })
  const settings = vue.reactive({
    ltServerUrl: 'https://test.invalid', ltNickname: 'Tester', ltAllowMemberControl: true,
    ltAutoPauseOnMemberChange: true, ltShareAudioLinks: false,
  })
  const emit = (name, payload) => {
    for (const callback of events.get(name) ?? []) callback({ payload })
  }
  const invoke = async (command, args) => {
    commands.push({ command, args: clone(args) })
    if (options.invoke) {
      const handled = options.invoke(command, args)
      if (handled !== undefined) return await handled
    }
    if (command === 'lt_join_room' || command === 'lt_create_room') {
      return { ok: true, roomId: 'ABC234', role: options.role ?? 'listener',
        token: 'test-session-token', state: options.initialState }
    }
    if (command === 'lt_connect_ws') { emit('lt:connected', { connectionId: 'connection-current' }); return }
    if (command === 'lt_send_event') return true
    if (command === 'lt_send_control') return { ok: true }
    if (command === 'lt_get_room_state') return { ok: true, state: options.initialState }
  }
  const listen = async (name, callback) => {
    if (!events.has(name)) events.set(name, new Set())
    events.get(name).add(callback)
    const pending = options.listen?.(name)
    if (pending !== undefined) await pending
    return () => events.get(name).delete(callback)
  }
  const dependencies = {
    pinia: { defineStore: (_id, setup) => setup }, vue,
    '@tauri-apps/api/core': { invoke }, '@tauri-apps/api/event': { listen },
    '@tauri-apps/plugin-clipboard-manager': { readText: async () => '', writeText: async () => {} },
    '@/stores/player': { usePlayerStore: () => player },
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/stores/toast': { useToastStore: () => ({ error: message => toasts.push(message), show: message => notices.push(message), success() {} }) },
    '@/i18n': { default: { global: { t: key => key } } },
    '@/utils/logger': { createLogger: () => Object.fromEntries(['debug', 'warn', 'error'].map(level => [level, (...args) => logs.push({ level, args })])) },
    ...helperDependencies,
    timers, storage: { getItem: () => '11111111-1111-4111-8111-111111111111', setItem() {} },
  }
  const module = await loadSource(indexSource, dependencies,
    'const { Date, setTimeout, clearTimeout, setInterval, clearInterval } = __deps.timers; const localStorage = __deps.storage;')
  const scope = vue.effectScope()
  const store = scope.run(() => module.useListenTogetherStore())
  player.moveQueueItem = new Function(
    'queue', 'queueIndex', 'markCommandSource', 'blockedByListenTogether',
    'useListenTogetherStore', 'savePlayerState', 'maybePrefetchNext',
    `let shuffleBag = [], shuffleHistory = [], shuffleFuture = [];
    ${queueReorderCode}; return moveQueueItem`,
  )(vue.toRef(player, 'queue'), vue.toRef(player, 'queueIndex'), markSource,
    () => !!store.localControlRestriction.value, () => store, () => {}, () => {})
  return {
    store, player, settings, playback, commands, timers, events, toasts, notices, logs, playGates, pauseGates, seekGates,
    emit,
    async join() { await store.joinRoom('ABC234', 'test-invite-secret'); await flush() },
    async create() {
      // 服务端要求建房时已经在放一首可共享的歌
      if (!player.currentTrack) {
        player.queue = [mapper.ltTrackToTrackInfo(wireTrack(1))]
        player.queueIndex = 0
        player.currentTrack = player.queue[0]
      }
      player.hasPlaybackSession = true
      await store.createRoom()
      await flush()
    },
    async message(message) {
      emit('lt:message', { connectionId: 'connection-current', ...message })
      await flush()
    },
    async dispose() { await store.leaveRoom(); scope.stop(); await flush() },
  }
}

const failures = []
let passed = 0
async function test(name, run) {
  let context
  let disposed = false
  try {
    context = await harness(run.options)
    await run(context)
    await context.dispose()
    disposed = true
    assert.equal(context.timers.active().length, 0, 'leaving the room should clear every owned timer')
    passed++
    console.log(`ok ${name}`)
  } catch (error) {
    failures.push(name)
    console.error(`FAIL ${name}: ${error.stack}`)
  } finally {
    if (context && !disposed) await context.dispose()
  }
}

await test('welcome applies playback and accepts position supplements without replacing equal-version state', async h => {
  await h.join()
  const initial = room([wireTrack(1)], { version: 5 })
  await h.message({ type: 'welcome', state: initial, expectedPositionMs: 6000, role: 'listener' })
  assert.equal(h.playback.find(entry => entry.type === 'play')?.positionMs, 6000)
  assert.equal(h.player.isPlaying, false)
  assert.equal(h.store.roomState.value.version, 5)
  h.player.positionMs = 0
  await h.message({ type: 'welcome', state: room([wireTrack(2)], { version: 5 }), expectedPositionMs: 7000 })
  assert.equal(h.store.roomState.value.track.stableKey, 'netease:1')
  assert.equal(h.player.currentTrack.id, 'netease:1')
  assert.ok(h.playback.some(entry => entry.type === 'seek' && entry.positionMs === 7000))
  const count = h.playback.length
  await h.message({ type: 'room_state_updated', state: room([wireTrack(2)], { version: 4 }) })
  await h.message({ type: 'room_state_updated', state: room([wireTrack(2)], { roomId: 'DEF234', version: 8 }) })
  assert.equal(h.playback.length, count)
  assert.equal(h.store.roomState.value.version, 5)
})

await test('late WebSocket messages and disconnects cannot override a newer connection', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  const count = h.playback.length
  await h.message({ connectionId: 'connection-old', type: 'room_state_updated', state: room([wireTrack(2)], { version: 2 }) })
  h.emit('lt:disconnected', { connectionId: 'connection-old', code: 1006, reason: 'old' })
  await flush()
  assert.equal(h.playback.length, count)
  assert.equal(h.store.connectionState.value, 'connected')
  assert.equal(h.store.roomState.value.version, 1)
})

await test('slow remote loading applies the latest pause after loading instead of a fixed timer', async h => {
  await h.join()
  const gate = deferred()
  h.playGates.push(gate)
  const initial = room([wireTrack(1)], { playback: { ...room().playback, state: 'playing' } })
  await h.message({ type: 'welcome', state: initial })
  assert.equal(h.player.isLoadingAudio, true)
  await h.message({ type: 'room_state_updated', state: room([wireTrack(1)], { version: 2 }), causedBy: { type: 'PAUSE' } })
  await h.timers.advance(4000)
  assert.equal(h.playback.filter(entry => entry.type === 'pause').length, 0)
  assert.ok(!h.timers.history.some(timer => !timer.interval && timer.delay === 300))
  gate.resolve()
  await flush()
  assert.equal(h.playback.filter(entry => entry.type === 'pause').length, 1)
  assert.equal(h.player.isPlaying, false)
  assert.equal(h.commands.filter(entry => entry.command === 'lt_send_event').length, 0)
})

await test('a local track selection inside the remote watch release window is reported immediately', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room([wireTrack(1), wireTrack(2)]) })
  const baseline = h.commands.length
  await h.player.play(h.player.queue[1], 'local', 0)
  await flush()
  const request = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_SET_TRACK')?.args.event
  assert.equal(request?.track.stableKey, 'netease:2')
})

await test('a local replay of the same song inside the remote watch release window is reported', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room() })
  const baseline = h.commands.length
  await h.player.play(h.player.currentTrack, 'local', 0)
  await flush()
  assert.ok(h.commands.slice(baseline).some(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_PLAY'))
})

await test('a local pause inside the remote watch release window is reported', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room([wireTrack(1)], {
    playback: { ...room().playback, state: 'playing' },
  }) })
  const baseline = h.commands.length
  await h.player.pause('local')
  await flush()
  assert.ok(h.commands.slice(baseline).some(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_PAUSE'))
})

await test('a local track selection supersedes corrections from a pending remote load', async h => {
  await h.join()
  const remoteLoad = deferred()
  h.playGates.push(remoteLoad)
  await h.message({ type: 'welcome', state: room([wireTrack(1), wireTrack(2)]) })
  const baseline = h.commands.length
  await h.player.play(h.player.queue[1], 'local', 0)
  await flush()
  assert.ok(h.commands.slice(baseline).some(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_SET_TRACK'))
  const corrections = h.playback.length
  remoteLoad.resolve()
  await flush()
  assert.equal(h.player.currentTrack.id, 'netease:2')
  assert.ok(!h.playback.slice(corrections).some(entry => ['seek', 'pause', 'resume'].includes(entry.type)),
    'the old remote load cannot seek or pause the new local selection')
})

await test('a local same-song replay with a start position supersedes a pending paused remote load', async h => {
  await h.join()
  const remoteLoad = deferred()
  h.playGates.push(remoteLoad)
  await h.message({ type: 'welcome', state: room() })
  await h.player.play(h.player.currentTrack, 'local', 12_000)
  await flush()
  const corrections = h.playback.length
  remoteLoad.resolve()
  await flush()
  assert.ok(!h.playback.slice(corrections).some(entry => ['seek', 'pause', 'resume'].includes(entry.type)),
    'a playback start position is not an explicit seek in the older remote session')
  assert.equal(h.player.isPlaying, true)
  assert.equal(h.player.positionMs, 12_000)
})

await test('a local same-song replay with a start position reports the new position to a playing room', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room([wireTrack(1)], {
    playback: { ...room().playback, state: 'playing' },
  }) })
  const baseline = h.commands.length
  await h.player.play(h.player.currentTrack, 'local', 12_000)
  await flush()
  await h.timers.advance(450)
  const request = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_SEEK')?.args.event
  assert.equal(request?.positionMs, 12_000,
    'the same track and playing state still need to publish a changed start position')
})

await test('a local seek inside the remote watch release window is reported after debounce', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room() })
  const baseline = h.commands.length
  await h.player.seekTo(12_000, 'local')
  await flush()
  await h.timers.advance(450)
  const request = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_SEEK')?.args.event
  assert.equal(request?.positionMs, 12_000)
})

await test('a local seek supersedes position corrections from a pending remote load', async h => {
  await h.join()
  const remoteLoad = deferred()
  h.playGates.push(remoteLoad)
  await h.message({ type: 'welcome', state: room() })
  const baseline = h.commands.length
  await h.player.seekTo(12_000, 'local')
  await flush()
  await h.timers.advance(450)
  assert.ok(h.commands.slice(baseline).some(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_SEEK'))
  const corrections = h.playback.length
  remoteLoad.resolve()
  await flush()
  assert.ok(!h.playback.slice(corrections).some(entry => ['seek', 'resume'].includes(entry.type)),
    'the old remote load cannot replace the newer local seek')
  assert.equal(h.player.positionMs, 12_000)
  assert.equal(h.player.isPlaying, false, 'seeking during loading still preserves the room pause intent')
})

await test('remote mode updates after a local command do not echo back as member requests', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room() })
  await h.timers.advance(4_000)
  await h.player.play(h.player.currentTrack, 'local', 5000)
  await flush()
  const baseline = h.commands.length
  await h.message({ type: 'room_state_updated', state: room([wireTrack(1)], {
    version: 2, playback: { ...room().playback, state: 'playing', repeatMode: 2, shuffleEnabled: true },
  }), expectedPositionMs: 5000, causedBy: { type: 'PLAYBACK_MODE' } })
  assert.equal(h.player.repeatMode, 'all')
  assert.equal(h.player.shuffleEnabled, true)
  assert.ok(!h.commands.slice(baseline).some(entry => entry.command === 'lt_send_event'),
    'synchronous room queue and mode writes must not be reported as local controls')
})

for (const requestType of ['REQUEST_SET_TRACK', 'REQUEST_SET_QUEUE']) {
  const supersededMemberLoad = async h => {
    await h.create()
    await h.message({ type: 'welcome', role: 'controller', state: room([wireTrack(1), wireTrack(3)]) })
    await h.timers.advance(4_000)
    const remoteLoad = deferred()
    h.playGates.push(remoteLoad)
    await h.message({ type: 'member_control_requested',
      causedBy: { userUuid: 'listener', type: requestType, eventId: 'paused-member-selection' },
      queue: [wireTrack(2), wireTrack(3)], currentIndex: 0, track: wireTrack(2), shouldPlay: false, positionMs: 0,
    })
    assert.equal(h.player.isLoadingAudio, true)
    await h.player.play(h.player.queue[1], 'local', 0)
    await flush()
    const corrections = h.playback.length
    remoteLoad.resolve()
    await flush()
    assert.equal(h.player.currentTrack.id, 'netease:3')
    assert.ok(!h.playback.slice(corrections).some(entry => ['seek', 'pause', 'resume'].includes(entry.type)),
      'the old member request cannot pause or seek the controller newer local selection')
    assert.equal(h.player.isPlaying, true)
  }
  supersededMemberLoad.options = { role: 'controller' }
  await test(`a controller local selection supersedes a pending paused ${requestType}`, supersededMemberLoad)

  const pausedMemberLoad = async h => {
    await h.create()
    await h.message({ type: 'welcome', role: 'controller', state: room() })
    await h.timers.advance(4_000)
    const remoteLoad = deferred()
    h.playGates.push(remoteLoad)
    await h.message({ type: 'member_control_requested',
      causedBy: { userUuid: 'listener', type: requestType, eventId: 'paused-member-selection' },
      queue: [wireTrack(2)], currentIndex: 0, track: wireTrack(2), shouldPlay: false, positionMs: 0,
    })
    remoteLoad.resolve()
    await flush()
    assert.equal(h.player.currentTrack.id, 'netease:2')
    assert.equal(h.player.isPlaying, false, 'an unchanged member pause intent is still applied after loading')
    assert.ok(h.commands.some(entry => entry.command === 'lt_send_event'
      && entry.args.event.type === requestType.replace('REQUEST_', '')))
  }
  pausedMemberLoad.options = { role: 'controller' }
  await test(`a pending ${requestType} still honors a member pause without a newer local command`, pausedMemberLoad)
}

await test('duplicate occurrences preserve the room current index and select another occurrence', async h => {
  await h.join()
  const tracks = [wireTrack(1), wireTrack(2), wireTrack(1)]
  await h.message({ type: 'welcome', state: room(tracks, { currentIndex: 2 }) })
  assert.equal(h.player.queueIndex, 2)
  assert.equal(h.player.currentTrack.playlistKey, JSON.stringify({ stableKey: 'netease:1', occurrence: 1 }))
  const plays = h.playback.filter(entry => entry.type === 'play').length
  await h.message({ type: 'room_state_updated', state: room(tracks, { currentIndex: 0, version: 2 }), causedBy: { type: 'SET_TRACK' } })
  assert.equal(h.player.queueIndex, 0)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, plays + 1)
})

for (const role of ['controller', 'listener']) {
  for (const unchangedKeys of [false, true]) {
    const queueReorder = async h => {
      await h.join()
      const tracks = unchangedKeys
        ? [wireTrack(1), wireTrack(1), wireTrack(2)]
        : [wireTrack(1), wireTrack(2), wireTrack(1)]
      const oldIndex = unchangedKeys ? 1 : 2
      await h.message({ type: 'welcome', role, state: room(tracks, { currentIndex: oldIndex }) })
      await h.timers.advance(4000)
      const baseline = h.commands.length
      const playbackCount = h.playback.length
      const currentTrack = h.player.currentTrack
      const positionMs = h.player.positionMs
      assert.equal(h.player.moveQueueItem(oldIndex, 0), true)
      await flush()
      const events = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
      assert.equal(events.length, 1, 'reordering must publish one queue change without an extra track event')
      assert.equal(events[0].args.event.type, role === 'controller' ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE')
      assert.deepEqual(events[0].args.event.queueMutation.targetCurrent, { stableKey: 'netease:1', occurrence: 1 })
      const applied = helperDependencies['./queue'].applyListenTogetherQueueMutation({
        roomQueue: tracks, roomCurrentIndex: oldIndex, roomVersion: 1,
        mutation: events[0].args.event.queueMutation,
      })
      assert.equal(applied.currentIndex, 0, 'the original playing occurrence must move to the first slot on the peer')
      assert.equal(applied.queue[0], tracks[oldIndex])
      assert.equal(events[0].args.event.positionMs, positionMs)
      assert.equal(h.player.currentTrack, currentTrack)
      assert.equal(h.player.positionMs, positionMs)
      assert.equal(h.playback.length, playbackCount, 'reordering must not replay or seek the loaded song')
      await h.message({ type: 'room_state_updated', state: room(applied.queue, { currentIndex: 0, version: 2 }),
        causedBy: { userUuid: h.store.userUuid.value, eventId: events[0].args.event.eventId, type: events[0].args.event.type } })
      assert.equal(h.player.currentTrack, currentTrack, 'acknowledging the reorder must keep the loaded track')
      assert.equal(h.player.queueIndex, 0)
      const persisted = buildPersistedPlaybackQueue(h.player.queue, h.player.queueIndex, h.player.currentTrack, false)
      assert.equal(persisted.currentTrackPlaylistKey, h.player.queue[0].playlistKey)
      const restored = restorePersistedPlaybackQueue(
        persisted.queue, persisted.queueIndex, persisted.hasPlaybackSession,
        persisted.currentTrackId, persisted.currentTrackPlaylistKey, raw => raw, () => true,
      )
      assert.equal(restored.queueIndex, 0, 'restarting after the acknowledgement must restore the moved occurrence')
      assert.equal(restored.currentTrack, h.player.queue[0])
    }
    queueReorder.options = { role }
    await test(`${role} reorders the current duplicate using a queue event (${unchangedKeys ? 'equal' : 'changed'} stable-key order)`, queueReorder)
  }
}

for (const role of ['controller', 'listener']) {
  for (const desiredState of ['paused', 'playing']) {
    const loadingReorder = async h => {
      await h.join()
      const gate = deferred()
      h.playGates.push(gate)
      const initial = room([wireTrack(1), wireTrack(2)], {
        playback: { ...room().playback, state: desiredState },
      })
      await h.message({ type: 'welcome', role, state: initial })
      assert.equal(h.player.isLoadingAudio, true)
      const baseline = h.commands.length
      const commandSequence = h.player.lastCommandSequence
      assert.equal(h.player.moveQueueItem(0, 1), true)
      await flush()
      const events = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
      assert.equal(events.length, 1)
      assert.equal(events[0].args.event.type, role === 'controller' ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE')
      assert.equal(events[0].args.event.shouldPlay, desiredState === 'playing', 'sorting during loading must preserve room playback intent')
      assert.equal(events[0].args.event.positionMs, 5000)
      assert.equal(h.player.lastCommandSequence, commandSequence, 'sorting must not supersede the in-flight playback command')
      gate.resolve()
      await flush()
      assert.equal(h.player.isPlaying, desiredState === 'playing', 'remote loading must still apply the desired playback state')
      assert.equal(h.player.queueIndex, 1)
      assert.equal(h.playback.filter(entry => entry.type === 'play').length, 1)
      assert.equal(h.playback.filter(entry => entry.type === 'pause').length, desiredState === 'paused' ? 1 : 0)
    }
    loadingReorder.options = { role }
    await test(`${role} preserves ${desiredState} playback while reordering a remote load`, loadingReorder)
  }
}

for (const role of ['controller', 'listener']) {
  const seekThenReorder = async h => {
    await h.join()
    const gate = deferred()
    h.playGates.push(gate)
    await h.message({ type: 'welcome', role, state: room([wireTrack(1), wireTrack(2)]) })
    await h.player.seekTo(19000, 'local')
    await flush()
    await h.timers.advance(450)
    assert.ok(h.commands.some(entry => entry.command === 'lt_send_event'
      && entry.args.event.type === (role === 'controller' ? 'SEEK' : 'REQUEST_SEEK')))
    const baseline = h.commands.length
    assert.equal(h.player.moveQueueItem(0, 1), true)
    await flush()
    const events = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
    assert.equal(events.length, 1)
    assert.equal(events[0].args.event.type, role === 'controller' ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE')
    assert.equal(events[0].args.event.positionMs, 19000, 'sorting must keep a local seek that the room has not acknowledged')
    assert.equal(events[0].args.event.shouldPlay, false)
    gate.resolve()
    await flush()
    assert.equal(h.player.positionMs, 19000)
    assert.equal(h.player.isPlaying, false)
  }
  seekThenReorder.options = { role }
  await test(`${role} preserves a local seek when sorting an in-flight remote load`, seekThenReorder)
}

for (const role of ['controller', 'listener']) {
  const pauseThenReorder = async h => {
    await h.join()
    const playGate = deferred(), pauseGate = deferred()
    h.playGates.push(playGate)
    h.pauseGates.push(pauseGate)
    await h.message({ type: 'welcome', role, state: room([wireTrack(1), wireTrack(2)]) })
    await h.player.seekTo(19000, 'local')
    await flush()
    await h.timers.advance(450)
    playGate.resolve()
    await flush()
    assert.equal(h.player.isLoadingAudio, false)
    assert.equal(h.player.lastCommandSource, 'remote_sync')
    const baseline = h.commands.length
    assert.equal(h.player.moveQueueItem(0, 1), true)
    await flush()
    const event = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event')?.args.event
    assert.equal(event.positionMs, 19000, 'sorting while the native pause is pending must retain the latest seek')
    assert.equal(event.shouldPlay, false)
    pauseGate.resolve()
    await flush()
    assert.equal(h.player.positionMs, 19000)
    assert.equal(h.player.isPlaying, false)
  }
  pauseThenReorder.options = { role }
  await test(`${role} preserves the seek while sorting a pending remote pause continuation`, pauseThenReorder)
}

await test('same desktop id with a different Bilibili subaudio context reloads playback', async h => {
  await h.join()
  const make = sub => wireTrack('BV1TEST', { channelId: 'bilibili', audioId: 'BV1TEST', subAudioId: sub, stableKey: `bilibili:BV1TEST:${sub}` })
  await h.message({ type: 'welcome', state: room([make('11')]) })
  await h.message({ type: 'room_state_updated', state: room([make('22')], { version: 2 }), causedBy: { type: 'SET_TRACK' } })
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
  assert.equal(h.player.currentTrack.id, 'bilibili:BV1TEST')
  assert.equal(h.player.currentTrack.syncPayload.subAudioId, '22')
})

await test('soft drift correction rechecks every 500ms and resets once it converges', async h => {
  await h.join()
  const initial = room([wireTrack(1)], { playback: { ...room().playback, state: 'playing' } })
  await h.message({ type: 'welcome', state: initial })
  await h.timers.advance(1000)
  h.player.positionMs = 5500
  await h.message({ type: 'room_state_updated', state: { ...initial, version: 2 }, expectedPositionMs: 7000, causedBy: { type: 'PLAY' } })
  assert.equal(h.playback.filter(entry => entry.type === 'rate').at(-1)?.rate, 1.05)
  assert.ok(h.timers.active().some(timer => timer.interval && timer.delay === 500))
  h.player.positionMs = 6100
  await h.timers.advance(499)
  assert.equal(h.playback.filter(entry => entry.type === 'rate').at(-1)?.rate, 1.05)
  await h.timers.advance(1)
  assert.equal(h.playback.filter(entry => entry.type === 'rate').at(-1)?.rate, null)
  assert.ok(!h.timers.active().some(timer => timer.interval && timer.delay === 500))
})

const fallback = async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  h.store.reportSeekEvent(9000)
  await flush()
  const websocket = h.commands.find(entry => entry.command === 'lt_send_event')
  const http = h.commands.find(entry => entry.command === 'lt_send_control')
  assert.ok(websocket)
  assert.ok(http)
  assert.deepEqual(http.args.event, websocket.args.event)
  assert.ok(http.args.event.eventId)
  assert.ok(http.args.event.clientInstanceId)
  assert.ok(http.args.event.clientSequence > 0)
}
fallback.options = { invoke: command => command === 'lt_send_event' ? false : undefined }
await test('HTTP fallback preserves the WebSocket event identity and ordering metadata', fallback)

await test('leaving while remote playback is loading cancels deferred room corrections', async h => {
  await h.join()
  const gate = deferred()
  h.playGates.push(gate)
  await h.message({ type: 'welcome', state: room() })
  await h.store.leaveRoom()
  const playbackCount = h.playback.length
  const sends = h.commands.filter(entry => entry.command === 'lt_send_event').length
  gate.resolve()
  await flush()
  await h.timers.advance(30_000)
  assert.equal(h.playback.length, playbackCount)
  assert.equal(h.commands.filter(entry => entry.command === 'lt_send_event').length, sends)
  assert.equal(h.store.roomState.value, null)
  assert.equal(h.timers.active().length, 0)
})

const lateFallback = async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  h.store.reportSeekEvent(9000)
  await flush()
  assert.ok(h.commands.some(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'SEEK'))
  await h.store.leaveRoom()
  lateFallback.gate.resolve(false)
  await flush()
  assert.equal(h.commands.filter(entry => entry.command === 'lt_send_control').length, 0)
  assert.equal(h.store.roomState.value, null)
}
lateFallback.gate = deferred()
lateFallback.options = { invoke: command => command === 'lt_send_event' ? lateFallback.gate.promise : undefined }
await test('a delayed WebSocket failure after leaving does not start HTTP fallback', lateFallback)

const lateHttpResponse = async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  h.store.reportSeekEvent(9000)
  await flush()
  assert.ok(h.commands.some(entry => entry.command === 'lt_send_control'))
  await h.store.leaveRoom()
  const playbackCount = h.playback.length
  lateHttpResponse.gate.resolve({ ok: true, applied: { type: 'SEEK', state: room([wireTrack(2)], { version: 20 }), expectedPositionMs: 9000 } })
  await flush()
  assert.equal(h.store.roomState.value, null)
  assert.equal(h.playback.length, playbackCount)
}
lateHttpResponse.gate = deferred()
lateHttpResponse.options = { invoke: command => command === 'lt_send_event' ? false
  : command === 'lt_send_control' ? lateHttpResponse.gate.promise : undefined }
await test('an HTTP fallback response arriving after leaving cannot restore the old room', lateHttpResponse)

for (const operation of ['join', 'create']) {
  const delayedMembership = async h => {
    const request = operation === 'join'
      ? h.store.joinRoom('ABC234', 'test-invite-secret') : h.store.createRoom()
    await flush()
    await h.store.leaveRoom()
    const commandCount = h.commands.length
    delayedMembership.gate.resolve({ ok: true, roomId: 'ABC234', token: 'test-session-token',
      role: operation === 'join' ? 'listener' : 'controller', state: room() })
    await request
    await flush()
    assert.equal(h.store.roomId.value, null)
    assert.equal(h.store.roomState.value, null)
    assert.equal(h.store.connectionState.value, 'disconnected')
    assert.ok(!h.commands.slice(commandCount).some(entry => entry.command === 'lt_connect_ws'))
  }
  delayedMembership.gate = deferred()
  delayedMembership.options = { invoke: command => command === `lt_${operation}_room` ? delayedMembership.gate.promise : undefined }
  await test(`leaving during an unfinished ${operation} request ignores the late membership response`, delayedMembership)
}

for (const eventName of ['lt:message', 'lt:connected', 'lt:disconnected']) {
  const interruptedRegistration = async h => {
    const joining = h.store.joinRoom('ABC234', 'test-invite-secret')
    await flush()
    assert.equal(h.events.get(eventName)?.size, 1)
    assert.ok(!h.commands.some(entry => entry.command === 'lt_connect_ws'))
    await h.store.leaveRoom()
    const commandCount = h.commands.length
    interruptedRegistration.gate.resolve()
    await joining
    await flush()
    assert.ok(!h.commands.slice(commandCount).some(entry => entry.command === 'lt_connect_ws'))
    assert.equal([...h.events.values()].reduce((count, handlers) => count + handlers.size, 0), 0)
    assert.equal(h.store.roomId.value, null)
    assert.equal(h.store.connectionState.value, 'disconnected')
    assert.equal(h.timers.active().length, 0)
  }
  interruptedRegistration.gate = deferred()
  interruptedRegistration.options = { listen: name => name === eventName ? interruptedRegistration.gate.promise : undefined }
  await test(`leaving while ${eventName} registration is pending disposes every late listener`, interruptedRegistration)
}

await test('schema 2 authoritative queue updates reorder without restarting the current song', async h => {
  await h.join()
  const a = wireTrack(1), b = wireTrack(2), c = wireTrack(3)
  await h.message({ type: 'welcome', state: room([a, b, c], { currentIndex: 1 }) })
  const plays = h.playback.filter(entry => entry.type === 'play').length
  const seeks = h.playback.filter(entry => entry.type === 'seek').length
  await h.message({ type: 'room_state_updated', state: room([c, a, b], { currentIndex: 2, version: 2 }), causedBy: { type: 'SET_QUEUE' } })
  assert.equal(h.player.queueIndex, 2)
  assert.deepEqual(h.player.queue.map(track => track.id), ['netease:3', 'netease:1', 'netease:2'])
  assert.equal(h.player.currentTrack.id, 'netease:2')
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, plays)
  assert.equal(h.playback.filter(entry => entry.type === 'seek').length, seeks)
})

await test('heartbeats keep a successful fallback stream while it remains in the shared candidates', async h => {
  await h.join()
  const primary = 'https://music.126.net/primary.mp3'
  const fallback = 'https://music.126.net/fallback.mp3'
  const track = wireTrack(1, { streamUrl: primary, streamUrls: [primary, fallback] })
  const initial = room([track], { settings: { ...room().settings, shareAudioLinks: true },
    playback: { ...room().playback, state: 'playing' } })
  await h.message({ type: 'welcome', state: initial })
  h.player.currentTrack.audioUrl = fallback
  await h.timers.advance(4000)
  const plays = h.playback.filter(entry => entry.type === 'play').length
  for (const version of [2, 3, 4]) {
    await h.message({ type: 'room_state_updated', state: { ...initial, version }, causedBy: { type: 'HEARTBEAT' } })
  }
  assert.equal(h.player.getCurrentStreamUrl(), fallback)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, plays)
})

async function metadataEnrichingBiliRoom(h, queue) {
  const helpers = await loadHelper(new URL('../../modules/playback/biliVideoSkip.ts', sourceRoot))
  const runtimeTarget = vue.ref(null)
  const applyResolvedTarget = new Function('currentTrack', 'currentBiliVideoSkipTarget',
    'normalizeBiliVideoSkipTarget', 'isBiliVideoSkipTrack', 'useListenTogetherStore', 'savePlayerState', `
      const playbackRequestToken = 1, biliVideoSkipRequestToken = 1
      const biliVideoSkipTracker = { reset() {} }
      ${resolvedBiliTargetCode}
      return applyResolvedBiliVideoSkipTarget
    `)(vue.toRef(h.player, 'currentTrack'), runtimeTarget,
      helpers.normalizeBiliVideoSkipTarget, helpers.isBiliVideoSkipTrack,
      () => vue.proxyRefs(h.store), () => {})
  const play = h.player.play
  h.player.play = async (track, ...args) => {
    await play(track, ...args)
    const subAudioId = track.syncPayload?.subAudioId
    const cid = /^\d+$/.test(subAudioId ?? '') ? Number(subAudioId) : 123456
    applyResolvedTarget(h.player.currentTrack, { bvid: 'BV1ZJ4m1b7Zz', cid }, 1)
  }
  h.player.getCurrentStreamUrl = () => 'https://a.bilivideo.com/fixture.m4a'
  h.player.getCurrentStreamUrls = () => [h.player.getCurrentStreamUrl()]
  await h.join()
  const initial = room(queue, { settings: { ...room().settings, shareAudioLinks: true },
    playback: { ...room().playback, state: 'playing' } })
  await h.message({ type: 'welcome', state: initial, role: 'listener' })
  await flush()
  return { initial, runtimeTarget }
}

const biliMetadataCases = [
  { name: 'a missing room CID', audioId: 'BV1ZJ4m1b7Zz', album: 'Bilibili' },
  { name: 'a three-part local album', audioId: 'BV1ZJ4m1b7Zz', album: 'Bilibili|123456|BV1ZJ4m1b7Zz' },
  { name: 'a numeric room AV identity', audioId: '12345', album: 'Bilibili' },
]
for (const { name, audioId, album } of biliMetadataCases) {
  await test(`resolved Bilibili metadata preserves ${name} through heartbeat and watchdog checks`, async h => {
    const track = mapper.trackInfoToLtTrack({ id: `bilibili:${audioId}`, source: 'bilibili',
      title: 'fixture', artist: 'artist', album, durationMs: 272_000, audioUrl: '' })
    const { initial, runtimeTarget } = await metadataEnrichingBiliRoom(h, [track])
    assert.deepEqual(runtimeTarget.value, { bvid: 'BV1ZJ4m1b7Zz', cid: 123456 })
    const plays = () => h.playback.filter(entry => entry.type === 'play').length
    for (let seconds = 8; seconds <= 40; seconds += 8) {
      h.player.positionMs = 5000 + seconds * 1000
      await h.timers.advance(8000)
      await h.message({ type: 'room_state_updated', state: { ...initial, version: seconds },
        causedBy: { type: 'HEARTBEAT' } })
    }
    assert.equal(plays(), 1, 'runtime CID/BV enrichment must not create a new room playback identity')
    assert.equal(h.player.currentTrack.id, `bilibili:${audioId}`)
    assert.equal(h.player.currentTrack.audioUrl, '')
    assert.ok(h.player.currentTrack.syncPayload)
    assert.equal(mapper.trackInfoToLtTrack(h.player.currentTrack).stableKey, track.stableKey)
  })
}

await test('resolved Bilibili metadata still allows real part changes and duplicate occurrence selection', async h => {
  const part = cid => wireTrack('BV1ZJ4m1b7Zz', { channelId: 'bilibili', audioId: 'BV1ZJ4m1b7Zz',
    subAudioId: String(cid), stableKey: `bilibili:BV1ZJ4m1b7Zz:${cid}`, durationMs: 272_000 })
  const queue = [part(123456), part(123457), part(123457)]
  const { initial, runtimeTarget } = await metadataEnrichingBiliRoom(h, queue)
  await h.message({ type: 'room_state_updated', state: { ...initial, version: 2,
    currentIndex: 1, track: queue[1] }, causedBy: { type: 'SET_TRACK' } })
  await flush()
  assert.equal(runtimeTarget.value.cid, 123457)
  assert.equal(h.player.queueIndex, 1)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
  await h.message({ type: 'room_state_updated', state: { ...initial, version: 3,
    currentIndex: 2, track: queue[2] }, causedBy: { type: 'SET_TRACK' } })
  await flush()
  assert.equal(h.player.queueIndex, 2)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 3)
})

async function sharedNeteaseFallback(h, tracks = [wireTrack(1)]) {
  const failedUrl = 'https://music.126.net/unavailable.mp3'
  const biliUrl = 'https://a.bilivideo.com/BVfallback.m4a'
  const settings = { neteaseQuality: 'exhigh', qqMusicQuality: 'high', biliQuality: 'high',
    youtubeQuality: 'high', neteaseAutoSourceSwitch: true }
  const sourceCommands = []
  const source = await readFile(new URL('../../modules/playback/playbackSource.ts', sourceRoot), 'utf8')
  const { PlaybackUrlResolver } = await loadSource(source, {
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      sourceCommands.push({ command, args })
      if (command === 'get_netease_song_url') {
        return { url: null, bitrate: 0, format: 'mp3', unavailable_reason: 'no_permission' }
      }
      if (command === 'find_netease_bili_sources') {
        return [{ id: 'bilibili:BVfallback', title: args.title, artist: args.artist,
          album: 'Bilibili|12', url: '', duration_ms: 60_000 }]
      }
      assert.equal(command, 'get_bili_audio_url')
      return { url: biliUrl, bandwidth: 128_000, codecs: 'mp4a.40.2', quality_key: 'high',
        candidates: [], bvid: 'BVfallback', cid: 12 }
    } },
    '@/stores/listenTogether/mapper': mapper,
    './playbackFailure': await loadHelper(new URL('../../modules/playback/playbackFailure.ts', sourceRoot)),
  })
  const resolver = new PlaybackUrlResolver()
  const play = h.player.play
  const nativeStarts = []
  let currentUrl = null
  h.player.getCurrentStreamUrl = id => !id || id === h.player.currentTrack?.id ? currentUrl : null
  h.player.getCurrentStreamUrls = () => currentUrl ? [currentUrl] : []
  h.player.play = async (track, ...args) => {
    await play(track, ...args)
    let resolved = await resolver.resolve(track, settings)
    assert.equal(resolved.type, 'success')
    nativeStarts.push(resolved.url)
    // 原生直链失败后，播放器强制重新解析，并允许网易云自动换源
    if (resolved.url === failedUrl) {
      resolver.invalidate(track, settings)
      resolved = await resolver.resolve(track, settings, { forceRefresh: true })
      assert.equal(resolved.type, 'success')
      nativeStarts.push(resolved.url)
    }
    currentUrl = resolved.url
    h.player.audioInfo = { source: resolved.source }
  }
  await h.join()
  const queue = tracks.map(track => ({ ...track, streamUrl: failedUrl, streamUrls: [failedUrl] }))
  const initial = room(queue, { settings: { ...room().settings, shareAudioLinks: true },
    playback: { ...room().playback, state: 'playing' } })
  await h.message({ type: 'welcome', state: initial, role: 'listener' })
  await flush()
  assert.equal(h.player.currentTrack.id, 'netease:1')
  assert.equal(h.player.currentTrack.source, 'netease')
  assert.equal(h.player.currentTrack.syncPayload.channelId, 'netease')
  assert.equal(h.player.audioInfo.source, 'bilibili')
  assert.equal(h.player.getCurrentStreamUrl(), biliUrl)
  assert.deepEqual(nativeStarts, [failedUrl, biliUrl])
  return { initial, sourceCommands, nativeStarts, failedUrl, biliUrl }
}

await test('heartbeats preserve a Bilibili fallback after the shared Netease stream fails', async h => {
  const { initial, sourceCommands, nativeStarts } = await sharedNeteaseFallback(h)
  const calls = sourceCommands.length
  for (const version of [2, 3, 4]) {
    await h.message({ type: 'room_state_updated', state: { ...initial, version }, causedBy: { type: 'HEARTBEAT' } })
    await flush()
  }
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 1,
    'an unchanged shared stream must not restart a successful automatic fallback')
  assert.equal(sourceCommands.length, calls, 'heartbeats must not repeat the restricted-source lookup')
  assert.equal(nativeStarts.length, 2)
})

for (const missingUrl of [true, false]) {
  await test(`a failed shared-stream session reloads unchanged URLs with ${missingUrl ? 'no current URL' : 'a stale current URL'}`, async h => {
    const { initial, failedUrl } = await sharedNeteaseFallback(h)
    h.player.getCurrentStreamUrl = () => missingUrl ? null : failedUrl
    h.player.hasPlaybackSession = false
    h.player.isPlaying = false
    h.player.isLoadingAudio = false
    await h.message({ type: 'room_state_updated', state: { ...initial, version: 2 },
      causedBy: { type: 'HEARTBEAT' } })
    await flush()
    assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2,
      'a failed pipeline needs a new load even when the advertised URL set did not change')
    assert.equal(h.playback.filter(entry => entry.type === 'resume').length, 0,
      'resuming an empty backend cannot recreate the failed playback session')
  })
}

await test('a paused successful fallback retains its session without reloading shared URLs', async h => {
  const { initial } = await sharedNeteaseFallback(h)
  await h.player.pause('remote_sync')
  assert.equal(h.player.hasPlaybackSession, true)
  await h.message({ type: 'room_state_updated', state: { ...initial, version: 2,
    playback: { ...initial.playback, state: 'paused' } }, causedBy: { type: 'HEARTBEAT' } })
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 1)
  assert.equal(h.player.isPlaying, false)
})

await test('an ongoing shared-stream load is not restarted by unchanged heartbeats', async h => {
  const { initial } = await sharedNeteaseFallback(h)
  const gate = deferred()
  h.playGates.push(gate)
  const pending = h.player.play(h.player.currentTrack, 'remote_sync', h.player.positionMs)
  try {
    h.player.getCurrentStreamUrl = () => null
    assert.equal(h.player.isLoadingAudio, true)
    for (const version of [2, 3, 4]) {
      await h.message({ type: 'room_state_updated', state: { ...initial, version },
        causedBy: { type: 'HEARTBEAT' } })
    }
    assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
  } finally {
    gate.resolve()
    await pending
  }
})

await test('new shared URLs replace a Bilibili fallback and duplicate occurrences still reload', async h => {
  const { initial } = await sharedNeteaseFallback(h, [wireTrack(1), wireTrack(1)])
  const refreshedUrl = 'https://music.126.net/refreshed.mp3'
  const refreshed = { ...initial, version: 2, queue: initial.queue.map(track => ({ ...track,
    streamUrl: refreshedUrl, streamUrls: [refreshedUrl] })), track: { ...initial.track,
    streamUrl: refreshedUrl, streamUrls: [refreshedUrl] } }
  await h.message({ type: 'room_state_updated', state: refreshed, causedBy: { type: 'LINK_READY' } })
  await flush()
  assert.equal(h.player.getCurrentStreamUrl(), refreshedUrl, 'a genuinely refreshed shared URL is tried')
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
  await h.message({ type: 'room_state_updated', state: { ...refreshed, version: 3, currentIndex: 1 },
    causedBy: { type: 'SET_TRACK' } })
  await flush()
  assert.equal(h.player.queueIndex, 1)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 3,
    'selecting another occurrence is a new playback context even with the same shared URL')
})

await test('removing an earlier duplicate keeps the current Bilibili fallback playing', async h => {
  const { initial } = await sharedNeteaseFallback(h, [wireTrack(1), wireTrack(1)])
  const selected = { ...initial, version: 2, currentIndex: 1 }
  await h.message({ type: 'room_state_updated', state: selected, causedBy: { type: 'SET_TRACK' } })
  await flush()
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
  await h.message({ type: 'room_state_updated', state: { ...selected, version: 3,
    queue: selected.queue.slice(1), currentIndex: 0 }, causedBy: { type: 'SET_QUEUE' } })
  await flush()
  assert.equal(h.player.queueIndex, 0)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2,
    'rebasing the current occurrence after a queue edit does not select another song')
})

await test('the watchdog can force-reload a stalled automatic fallback with unchanged shared URLs', async h => {
  await sharedNeteaseFallback(h)
  h.player.isPlaying = false
  h.player.isLoadingAudio = true
  await h.timers.advance(16_000)
  await flush()
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
})

await test('reconnecting retries the shared Netease stream after a Bilibili fallback', async h => {
  const { initial, nativeStarts } = await sharedNeteaseFallback(h)
  h.emit('lt:disconnected', { connectionId: 'connection-current', code: 1006, reason: '' })
  await flush()
  await h.timers.advance(2000)
  await h.message({ type: 'welcome', state: initial, role: 'listener' })
  await flush()
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2)
  assert.equal(nativeStarts.length, 4)
})

for (const role of ['controller', 'listener']) {
  const coalescedQueue = async h => {
    if (role === 'controller') await h.create()
    else await h.join()
    const a = wireTrack(1), b = wireTrack(2), c = wireTrack(3)
    await h.message({ type: 'welcome', role, state: room([a]) })
    await h.timers.advance(4000)
    const baseline = h.commands.length
    h.player.queue.push(mapper.ltTrackToTrackInfo(b))
    await flush()
    h.player.queue.push(mapper.ltTrackToTrackInfo(c))
    await flush()
    const queueEvents = () => h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
    assert.equal(queueEvents().length, 1)
    const first = queueEvents()[0].args.event
    assert.equal(first.queueMutation.baseRoomVersion, 1)
    assert.deepEqual(first.queueMutation.operations.filter(operation => operation.type === 'insert').map(operation => operation.track.stableKey), ['netease:2'])
    await h.message({ type: 'control_result', result: { ok: true, applied: {
      type: first.type, state: room([a, b], { version: 2 }), expectedPositionMs: 5000,
      causedBy: { userUuid: h.store.userUuid.value, eventId: first.eventId, type: first.type },
    } } })
    assert.equal(queueEvents().length, 2)
    const second = queueEvents()[1].args.event
    assert.equal(second.queueMutation.baseRoomVersion, 2)
    assert.deepEqual(second.queueMutation.operations.filter(operation => operation.type === 'insert').map(operation => operation.track.stableKey), ['netease:3'])
    assert.notEqual(second.eventId, first.eventId)
  }
  coalescedQueue.options = { role }
  await test(`${role} queues the latest local snapshot until the first mutation is acknowledged`, coalescedQueue)
}

await test('a broadcast queue acknowledgement preserves the listener pending current track', async h => {
  await h.join()
  const a = wireTrack(1), b = wireTrack(2), c = wireTrack(3)
  await h.message({ type: 'welcome', state: room([a]) })
  await h.timers.advance(4000)
  const baseline = h.commands.length
  const plays = h.playback.filter(entry => entry.type === 'play').length
  h.player.queue.push(mapper.ltTrackToTrackInfo(b))
  await flush()
  const first = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event')?.args.event
  assert.equal(first.type, 'REQUEST_SET_QUEUE')
  h.player.queue.push(mapper.ltTrackToTrackInfo(c))
  h.player.queueIndex = 2
  h.player.currentTrack = h.player.queue[2]
  await flush()
  assert.equal(h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event').length, 1)
  await h.message({ type: 'room_state_updated', state: room([a, b], { version: 2 }), expectedPositionMs: 5000,
    causedBy: { userUuid: h.store.userUuid.value, eventId: first.eventId, type: first.type } })
  assert.equal(h.player.currentTrack.id, 'netease:3')
  assert.equal(h.player.queueIndex, 2)
  assert.deepEqual(h.player.queue.map(track => track.id), ['netease:1', 'netease:2', 'netease:3'])
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, plays)
  const outbound = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
  assert.equal(outbound.length, 2)
  const second = outbound[1].args.event
  assert.equal(second.type, 'REQUEST_SET_TRACK')
  assert.equal(second.track.stableKey, 'netease:3')
  assert.equal(second.queueMutation.baseRoomVersion, 2)
  assert.deepEqual(second.queueMutation.operations.filter(operation => operation.type === 'insert').map(operation => operation.track.stableKey), ['netease:3'])
})

const forwardedQueue = async h => {
  const a = wireTrack(1), b = wireTrack(2), c = wireTrack(3)
  h.player.queue.push(mapper.ltTrackToTrackInfo(a), mapper.ltTrackToTrackInfo(b))
  h.player.queueIndex = 0
  h.player.currentTrack = h.player.queue[0]
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: room([a, b]) })
  await h.message({ type: 'member_control_requested', causedBy: { userUuid: 'listener', type: 'REQUEST_SET_QUEUE', eventId: 'queue-1' },
    requestSequence: 1, queueMutation: { baseRoomVersion: 1, operations: [{ type: 'insert', track: c, placement: 'append' }] } })
  assert.deepEqual(h.player.queue.map(track => track.id), ['netease:1', 'netease:2', 'netease:3'])
  const event = h.commands.filter(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'SET_QUEUE').at(-1)?.args.event
  assert.ok(event)
  assert.ok(event.queueMutation, 'schema 2 queue request should be committed as a mutation')
  assert.equal(event.queueMutation.baseRoomVersion, 1)
  const commitCount = h.commands.filter(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'SET_QUEUE').length
  await h.message({ type: 'member_control_requested', causedBy: { userUuid: 'listener', type: 'REQUEST_SET_QUEUE', eventId: 'queue-duplicate' },
    requestSequence: 1, queueMutation: { baseRoomVersion: 1, operations: [{ type: 'remove', target: { stableKey: 'netease:2', occurrence: 0 } }] } })
  assert.equal(h.commands.filter(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'SET_QUEUE').length, commitCount)
}
forwardedQueue.options = { role: 'controller' }
await test('schema 2 forwarded queue requests apply mutations and suppress repeated requester sequence', forwardedQueue)

for (const role of ['controller', 'listener']) {
  const localQueueAndTrack = async h => {
    if (role === 'controller') await h.create()
    else await h.join()
    await h.message({ type: 'welcome', role, state: room([wireTrack(1)]) })
    await h.timers.advance(4000)
    const commandCount = h.commands.length
    h.player.queue.push(mapper.ltTrackToTrackInfo(wireTrack(2)))
    h.player.queueIndex = 1
    h.player.currentTrack = h.player.queue[1]
    await flush()
    const outbound = h.commands.slice(commandCount).filter(entry => entry.command === 'lt_send_event')
    const expectedType = role === 'controller' ? 'SET_TRACK' : 'REQUEST_SET_TRACK'
    assert.deepEqual(outbound.map(entry => entry.args.event.type), [expectedType])
    const event = outbound[0].args.event
    assert.ok(event.queueMutation)
    assert.equal(event.queueMutation.baseRoomVersion, 1)
    assert.ok(event.queueMutation.operations.some(operation => operation.type === 'insert' && operation.track.stableKey === 'netease:2'))
    assert.equal(event.track.stableKey, 'netease:2')
    assert.equal(event.currentIndex, 1)
  }
  localQueueAndTrack.options = { role }
  await test(`${role} queue growth with a track change publishes one mutation-bound track event`, localQueueAndTrack)
}

for (const role of ['controller', 'listener']) {
  const insertedDuplicateSelection = async h => {
    if (role === 'controller') await h.create()
    else await h.join()
    const a = wireTrack(1)
    await h.message({ type: 'welcome', role, state: room([a]) })
    await h.timers.advance(4000)
    const baseline = h.commands.length
    h.player.queue.push(mapper.ltTrackToTrackInfo(a))
    h.player.queueIndex = 1
    h.player.currentTrack = h.player.queue[1]
    await flush()
    const outbound = () => h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
    assert.equal(outbound().length, 1)
    const insertion = outbound()[0].args.event
    assert.equal(insertion.type, role === 'controller' ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE')
    assert.deepEqual(insertion.queueMutation.operations.filter(operation => operation.type === 'insert').map(operation => operation.track.stableKey), ['netease:1'])
    await h.message({ type: 'control_result', result: { ok: true, applied: {
      type: insertion.type, state: room([a, wireTrack(1)], { version: 2 }), expectedPositionMs: 5000,
      causedBy: { userUuid: h.store.userUuid.value, eventId: insertion.eventId, type: insertion.type },
    } } })
    assert.equal(outbound().length, 2)
    const selection = outbound()[1].args.event
    assert.equal(selection.type, role === 'controller' ? 'SET_TRACK' : 'REQUEST_SET_TRACK')
    assert.equal(selection.queueMutation.baseRoomVersion, 2)
    assert.deepEqual(selection.queueMutation.operations, [])
    assert.deepEqual(selection.queueMutation.targetCurrent, { stableKey: 'netease:1', occurrence: 1 })
    assert.equal(selection.currentIndex, 1)
  }
  insertedDuplicateSelection.options = { role }
  await test(`${role} selects a newly inserted duplicate only after its occurrence exists in the room`, insertedDuplicateSelection)
}

await test('legacy queue compatibility failure retries a snapshot once with a new event id', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room([wireTrack(1)]) })
  await h.timers.advance(4000)
  const baseline = h.commands.length
  h.player.queue.push(mapper.ltTrackToTrackInfo(wireTrack(2)))
  await flush()
  const outbound = () => h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
  const original = outbound()[0]?.args.event
  assert.ok(original?.queueMutation)
  await h.message({ type: 'error', message: 'queue mutation event type unsupported' })
  assert.equal(outbound().length, 2)
  const snapshot = outbound()[1].args.event
  assert.notEqual(snapshot.eventId, original.eventId)
  assert.equal(snapshot.queueMutation, undefined)
  assert.deepEqual(snapshot.queue.map(track => track.stableKey), ['netease:1', 'netease:2'])
  await h.message({ type: 'error', message: 'queue mutation event type unsupported' })
  assert.equal(outbound().length, 2)
  await h.timers.advance(3000)
  assert.equal(outbound().length, 2)
  assert.equal(h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_control').length, 0)
})

const acknowledgedOccurrence = async h => {
  await h.create()
  const a = wireTrack(1), b = wireTrack(2), c = wireTrack(3)
  await h.message({ type: 'welcome', role: 'controller', state: room([a, wireTrack(1), b], { currentIndex: 2 }) })
  await h.timers.advance(4000)
  const baseline = h.commands.length
  h.player.queue.splice(0, 1)
  h.player.queueIndex = 1
  await flush()
  const first = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event')?.args.event
  assert.equal(first.type, 'SET_QUEUE')
  assert.deepEqual(first.queueMutation.operations, [{ type: 'remove', target: { stableKey: 'netease:1', occurrence: 0 } }])
  const committed = room([a, b], { version: 2, currentIndex: 1 })
  const cause = { userUuid: h.store.userUuid.value, eventId: first.eventId, type: first.type }
  await h.message({ type: 'control_result', result: { ok: true, applied: { type: first.type, state: committed, causedBy: cause } } })
  await h.message({ type: 'room_state_updated', state: committed, causedBy: cause })
  assert.equal(h.player.queue[0].playlistKey, JSON.stringify({ stableKey: 'netease:1', occurrence: 0 }))
  h.player.queue.push(mapper.ltTrackToTrackInfo(c))
  await flush()
  const outbound = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event')
  assert.equal(outbound.length, 2)
  const second = outbound[1].args.event
  assert.equal(second.queueMutation.baseRoomVersion, 2)
  assert.deepEqual(second.queueMutation.operations.map(operation => operation.type), ['insert'])
  assert.equal(second.queueMutation.operations[0].track.stableKey, 'netease:3')
}
acknowledgedOccurrence.options = { role: 'controller' }
await test('acknowledging a duplicate removal rebases local occurrence references before another insertion', acknowledgedOccurrence)

const disablePendingLinks = async h => {
  await h.create()
  const withLinks = id => wireTrack(id, { streamUrl: `https://music.126.net/${id}.mp3`,
    streamUrls: [`https://music.126.net/${id}.mp3`, `https://music.126.net/${id}-backup.mp3`] })
  const a = withLinks(1), b = withLinks(2), c = withLinks(3)
  await h.message({ type: 'welcome', role: 'controller', state: room([a], {
    settings: { ...room().settings, shareAudioLinks: true },
  }) })
  await h.timers.advance(4000)
  const baseline = h.commands.length
  h.player.queue.push(mapper.ltTrackToTrackInfo(b))
  h.player.queueIndex = 1
  h.player.currentTrack = h.player.queue[1]
  await flush()
  const first = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event')?.args.event
  assert.ok(first.track.streamUrl)
  assert.ok(first.track.streamUrls.length > 0)
  assert.ok(first.queueMutation.operations.some(operation => operation.track?.streamUrl))
  h.player.queue.push(mapper.ltTrackToTrackInfo(c))
  h.player.queueIndex = 2
  h.player.currentTrack = h.player.queue[2]
  await flush()
  assert.equal(h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event').length, 1)
  await h.store.updateRoomSettings({ shareAudioLinks: false })
  const disabledAt = h.commands.length
  await h.message({ type: 'control_result', result: { ok: true, applied: {
    type: first.type, state: room([wireTrack(1), wireTrack(2)], { version: 2, currentIndex: 1,
      settings: { ...room().settings, shareAudioLinks: false } }),
    causedBy: { userUuid: h.store.userUuid.value, eventId: first.eventId, type: first.type },
  } } })
  const outbound = h.commands.slice(disabledAt).filter(entry => ['lt_send_event', 'lt_send_control'].includes(entry.command))
  assert.equal(outbound.filter(entry => entry.command === 'lt_send_event').length, 1)
  assert.equal(outbound.filter(entry => entry.command === 'lt_send_control').length, 1)
  for (const command of outbound) {
    const event = command.args.event
    assert.equal(event.track.stableKey, 'netease:3')
    assert.equal(event.queueMutation.baseRoomVersion, 2)
    const tracks = [event.track, ...(event.queue ?? []), ...event.queueMutation.operations.flatMap(operation => operation.track ? [operation.track] : [])]
    for (const track of tracks) {
      assert.ok(!track.streamUrl, 'disabled sharing must redact the legacy stream field')
      assert.equal(track.streamUrls?.length ?? 0, 0, 'disabled sharing must redact every shared candidate')
    }
  }
  assert.equal(outbound[0].args.event.eventId, outbound[1].args.event.eventId)
}
disablePendingLinks.options = { role: 'controller', invoke: (command, args) =>
  command === 'lt_send_event' && args?.event?.track?.stableKey === 'netease:3' ? false : undefined }
await test('disabling sharing redacts queued snapshots and their HTTP fallback candidates', disablePendingLinks)

await test('room state from a joined room does not rewrite local room defaults', async h => {
  await h.join()
  await h.message({ type: 'welcome', role: 'listener', state: room([wireTrack(1)], {
    settings: { allowMemberControl: false, autoPauseOnMemberChange: false, shareAudioLinks: true },
  }) })
  assert.equal(h.store.roomSettings.value.allowMemberControl, false)
  assert.equal(h.store.roomSettings.value.shareAudioLinks, true)
  assert.deepEqual(
    [h.settings.ltAllowMemberControl, h.settings.ltAutoPauseOnMemberChange, h.settings.ltShareAudioLinks],
    [true, true, false],
  )
  // 听众修改的是自己的默认值，不能改房间设置
  await h.store.updateRoomSettings({ allowMemberControl: true })
  await flush()
  assert.equal(h.settings.ltAllowMemberControl, true)
  assert.equal(h.store.roomSettings.value.allowMemberControl, false)
  assert.ok(!h.commands.some(entry => entry.args?.event?.type === 'UPDATE_SETTINGS'))
  await h.store.leaveRoom()
  assert.equal(h.store.roomSettings.value.shareAudioLinks, false, 'outside a room the defaults apply again')
})

const hostSettings = async h => {
  await h.create()
  await h.store.updateRoomSettings({ shareAudioLinks: true })
  await flush()
  assert.equal(h.settings.ltShareAudioLinks, true)
  assert.equal(h.store.roomSettings.value.shareAudioLinks, true)
  const update = h.commands.find(entry => entry.args?.event?.type === 'UPDATE_SETTINGS')
  assert.ok(update, 'the host sends the change to the room')
  assert.equal(update.args.event.roomSettings.shareAudioLinks, true)
  assert.equal(update.args.event.roomSettings.allowMemberControl, true)
}
hostSettings.options = { role: 'controller', initialState: room([wireTrack(1)], { controllerUserUuid: '11111111-1111-4111-8111-111111111111' }) }
await test('settings changed while hosting are sent to the room', hostSettings)

await test('identity reset is immediate and refused inside a room', async h => {
  const before = h.store.userUuid.value
  assert.equal(h.store.resetIdentity(), true)
  assert.notEqual(h.store.userUuid.value, before)
  assert.match(h.store.userUuid.value, /^[0-9a-f-]{36}$/)
  await h.join()
  const inRoom = h.store.userUuid.value
  assert.equal(h.store.resetIdentity(), false)
  assert.equal(h.store.userUuid.value, inRoom)
})

const closedWhileAway = async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  h.emit('lt:disconnected', { connectionId: 'connection-current', code: 1006, reason: '' })
  await flush()
  await h.timers.advance(2000)
  assert.equal(h.store.roomId.value, null, 'a closed room ends the session instead of reconnecting forever')
  assert.ok(!h.commands.some(entry => entry.command === 'lt_leave_room'), 'a closed room is not asked to let us leave')
  assert.match(h.store.sessionError.value, /410/)
  const connects = h.commands.filter(entry => entry.command === 'lt_connect_ws').length
  await h.timers.advance(120_000)
  assert.equal(h.commands.filter(entry => entry.command === 'lt_connect_ws').length, connects)
}
closedWhileAway.connects = 0
closedWhileAway.options = {
  invoke: command => command === 'lt_connect_ws' && ++closedWhileAway.connects > 1
    ? Promise.reject(new Error('WebSocket connect failed: HTTP error: 410 Gone'))
    : undefined,
}
await test('a room closed while disconnected is closed locally without reconnecting', closedWhileAway)

const exhausted = async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  exhausted.failing = true
  h.emit('lt:disconnected', { connectionId: 'connection-current', code: 1006, reason: '' })
  await flush()
  await h.timers.advance(240_000)
  assert.equal(h.store.roomId.value, null)
  assert.equal(h.store.sessionError.value, 'reconnect_max_attempts_exceeded')
  assert.equal(h.commands.filter(entry => entry.command === 'lt_connect_ws').length, 1 + 15)
  const rejoins = h.commands.filter(entry => entry.command === 'lt_join_room').length
  assert.equal(rejoins, 1 + 15, 'a listener rejoins before every reconnect attempt')
}
exhausted.options = {
  invoke: command => exhausted.failing && command === 'lt_connect_ws'
    ? Promise.reject(new Error('connection reset by peer'))
    : undefined,
}
await test('reconnecting gives up after fifteen attempts', exhausted)

const unrelatedRejection = async h => {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: room([wireTrack(1)]) })
  await h.timers.advance(4000)
  const baseline = h.commands.length
  h.player.queue.push(mapper.ltTrackToTrackInfo(wireTrack(2)))
  await flush()
  const queueEvent = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event')?.args.event
  assert.equal(queueEvent?.type, 'SET_QUEUE')
  await h.message({
    type: 'control_result', causedBy: { eventId: 'unrelated', type: 'REQUEST_LINK' },
    result: { ok: false, error: 'link request throttled' },
  })
  await h.timers.advance(3000)
  const fallback = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_control')
  assert.equal(fallback?.args.event.eventId, queueEvent.eventId, 'the unacknowledged queue change is still delivered')
  assert.deepEqual(h.toasts, [], 'rejections are recorded on the session instead of toasted')
  assert.equal(h.store.sessionError.value, 'link request throttled')
}
unrelatedRejection.options = { role: 'controller' }
await test('a rejection for another event does not fail the queue change in flight', unrelatedRejection)

await test('an np_ping rejection switches to the legacy ping', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  await h.message({ type: 'error', message: 'unsupported event type: np_ping' })
  await h.timers.advance(20_000)
  const ping = h.commands.filter(entry => entry.command === 'lt_send_ping').at(-1)
  assert.equal(ping?.args.legacy, true)
  assert.equal(h.store.sessionError.value, null)
})

await test('a room-closed rejection ends the session locally', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  await h.message({ type: 'control_result', result: { ok: false, error: 'room closed' } })
  assert.equal(h.store.roomId.value, null)
  assert.ok(!h.commands.some(entry => entry.command === 'lt_leave_room'))
})

async function hostPlaying(h) {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: room([wireTrack(1)]) })
  // 等远端同步保护期过去，之后的播放才算房主自己的操作
  await h.timers.advance(4000)
  h.player.isPlaying = true
  await flush()
}

const offlineHost = async h => {
  await hostPlaying(h)
  offlineHost.offline = true
  h.emit('lt:disconnected', { connectionId: 'connection-current', code: 1006, reason: '' })
  await flush()
  const baseline = h.commands.length
  h.player.isPlaying = false
  await flush()
  const http = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_control')
  assert.equal(http?.args.event.type, 'PAUSE', 'a pause made while the socket is down still reaches the room over HTTP')
  offlineHost.offline = false
  const playbackCount = h.playback.length
  h.emit('lt:connected', { connectionId: 'connection-current' })
  await h.message({ type: 'welcome', role: 'controller', state: room([wireTrack(1)], {
    version: 2, playback: { ...room().playback, state: 'playing' },
  }) })
  assert.ok(!h.playback.slice(playbackCount).some(entry => entry.type === 'play' || entry.type === 'resume'),
    'the reconnect welcome must not undo what the host did while offline')
}
offlineHost.options = { role: 'controller', invoke: command => offlineHost.offline && command === 'lt_send_event' ? false : undefined }
await test('host controls made while disconnected are sent and survive the reconnect welcome', offlineHost)

const outboxReplay = async h => {
  await hostPlaying(h)
  outboxReplay.offline = true
  h.emit('lt:disconnected', { connectionId: 'connection-current', code: 1006, reason: '' })
  await flush()
  h.player.isPlaying = false
  await flush()
  h.player.repeatMode = 'all'
  await flush()
  h.player.isPlaying = true
  await flush()
  assert.deepEqual(h.toasts, [], 'controls that cannot be sent wait for the connection instead of failing')
  const baseline = h.commands.length
  outboxReplay.offline = false
  h.emit('lt:connected', { connectionId: 'connection-current' })
  await flush()
  const replayed = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event').map(entry => entry.args.event)
  assert.deepEqual(replayed.map(event => event.type), ['PLAYBACK_MODE', 'PLAY'],
    'the pause is superseded by the later play, so only the newest transport control is replayed')
  assert.equal(new Set(replayed.map(event => event.eventId)).size, 2)
}
outboxReplay.options = {
  role: 'controller',
  invoke: command => {
    if (!outboxReplay.offline) return undefined
    if (command === 'lt_send_event') return false
    if (command === 'lt_send_control') return Promise.reject(new Error('network unreachable'))
    return undefined
  },
}
await test('controls that fail on both channels are replayed once after reconnecting', outboxReplay)

const loadingTrackChange = async h => {
  await hostPlaying(h)
  const baseline = h.commands.length
  h.player.queue.push(mapper.ltTrackToTrackInfo(wireTrack(2)))
  h.player.queueIndex = 1
  h.player.currentTrack = h.player.queue[1]
  h.player.isLoadingAudio = true
  h.player.isPlaying = false
  await flush()
  const sent = () => h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event').map(entry => entry.args.event)
  assert.equal(sent()[0]?.shouldPlay, true, 'a track that is still loading is committed as playing')
  h.player.isLoadingAudio = false
  h.player.isPlaying = true
  await flush()
  assert.ok(!sent().some(event => event.type === 'PAUSE' || event.type === 'PLAY'),
    'loading a new track is not reported as a pause followed by a play')
}
loadingTrackChange.options = { role: 'controller' }
await test('a host track change commits as playing without a loading pause', loadingTrackChange)

await test('TRACK_FINISHED reports where the finished track ended', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room([wireTrack(1), wireTrack(2)]), role: 'listener' })
  h.player.positionMs = 59_400
  h.store.reportTrackFinished('netease:1')
  await flush()
  const finished = h.commands.find(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'TRACK_FINISHED')
  assert.equal(finished?.args.event.finishedTrackStableKey, 'netease:1')
  assert.equal(finished?.args.event.positionMs, 60_000, 'the finish position is at least the track duration')
})

await test('a late track finish does not mark the newly selected room track as finished', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room([wireTrack(1), wireTrack(2)], { currentIndex: 1 }), role: 'listener' })
  const baseline = h.commands.length
  h.store.reportTrackFinished('netease:1')
  h.store.reportTrackFinished(null)
  await flush()
  assert.ok(!h.commands.slice(baseline).some(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'TRACK_FINISHED'), 'only the current finished track can advance the room')
})

const sharingRoom = () => room([wireTrack(1)], { settings: { ...room().settings, shareAudioLinks: true } })

const unavailableLink = async h => {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: sharingRoom() })
  const baseline = h.commands.length
  await h.message({ type: 'link_requested', requestTrackStableKey: 'netease:1' })
  await h.timers.advance(14_000)
  const sent = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event').map(entry => entry.args.event)
  assert.deepEqual(sent.filter(event => event.type.startsWith('LINK_')).map(event => [event.type, event.requestTrackStableKey]),
    [['LINK_UNAVAILABLE', 'netease:1']], 'a host without a shareable link says so instead of staying silent')
  assert.equal(h.playback.filter(entry => entry.type === 'resolve-shareable').length, 3)
}
unavailableLink.options = { role: 'controller' }
await test('a host that cannot share a link answers LINK_UNAVAILABLE', unavailableLink)

const resolvedLink = async h => {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: sharingRoom() })
  const baseline = h.commands.length
  await h.message({ type: 'link_requested', requestTrackStableKey: 'netease:1' })
  await h.message({ type: 'link_requested', requestTrackStableKey: 'netease:1' })
  await h.timers.advance(14_000)
  const links = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event'
    && entry.args.event.type.startsWith('LINK_')).map(entry => entry.args.event)
  assert.equal(links.length, 1, 'a repeated request for the same track is answered once')
  assert.equal(links[0].type, 'LINK_READY')
  assert.equal(links[0].track.streamUrl, 'https://m701.music.126.net/shared.mp3')
  assert.equal(links[0].queue, undefined, 'LINK_READY carries only the requested track')
}
resolvedLink.attempts = 0
resolvedLink.options = {
  role: 'controller',
  shareableUrls: () => ++resolvedLink.attempts >= 2 ? ['https://m701.music.126.net/shared.mp3'] : [],
}
await test('a host resolves a shareable link when it has none to hand', resolvedLink)

await test('a closed room ends the session locally with its reason', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  await h.message({ type: 'room_closed', state: room([wireTrack(1)], { version: 2, roomStatus: 'closed', closedReason: 'controller_left' }) })
  assert.equal(h.store.roomId.value, null)
  assert.equal(h.store.sessionError.value, 'controller_left')
  assert.ok(!h.commands.some(entry => entry.command === 'lt_leave_room'), 'a closed room is not asked to let us leave')
  assert.ok(h.playback.some(entry => entry.type === 'pause'), 'a room closed while paused pauses the listener')
})

const hostSuspended = async h => {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: room() })
  await h.timers.advance(4000)
  const playbackCount = h.playback.length
  await h.message({ type: 'room_suspended', state: room([wireTrack(1)], { version: 2, roomStatus: 'controller_offline' }) })
  assert.equal(h.playback.length, playbackCount, 'a suspension does not touch the player')
  assert.deepEqual(h.toasts, [])
  assert.deepEqual(h.notices, [], 'the host is not told that it is offline')
}
hostSuspended.options = { role: 'controller' }
await test('a suspended room is recorded without driving the host player', hostSuspended)

await test('an invite server is used for that join only', async h => {
  await h.store.joinRoom('ABC234', 'test-invite-secret', 'https://friend.invalid')
  await flush()
  const join = h.commands.find(entry => entry.command === 'lt_join_room')
  assert.equal(join.args.baseUrl, 'https://friend.invalid')
  assert.equal(h.settings.ltServerUrl, 'https://test.invalid', 'the saved server is not rewritten by an invite')
})

const leanControls = async h => {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: room([wireTrack(1), wireTrack(2), wireTrack(3)]) })
  await h.timers.advance(4000)
  const baseline = h.commands.length
  h.player.isPlaying = true
  await flush()
  const play = h.commands.slice(baseline).find(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'PLAY')
  assert.ok(play)
  assert.equal(play.args.event.queue, undefined, 'schema 2 transport events bind the track without the queue')
  assert.equal(play.args.event.track.stableKey, 'netease:1')
}
leanControls.options = { role: 'controller' }
await test('transport events on schema 2 carry only the track binding', leanControls)

const localTrack = () => ({
  id: 'local:C:/Music/a.flac', title: 'a', artist: '', album: '', durationMs: 60_000,
  coverUrl: '', audioUrl: 'C:/Music/a.flac', source: 'local', addedAt: 0,
})

await test('creating a room with an unshareable track is refused before contacting the server', async h => {
  h.player.queue = [localTrack()]
  h.player.queueIndex = 0
  h.player.currentTrack = h.player.queue[0]
  await h.store.createRoom()
  await flush()
  assert.ok(!h.commands.some(entry => entry.command === 'lt_create_room'))
  assert.equal(h.store.roomId.value, null)
  assert.equal(h.store.sessionError.value, 'listen_together.create_requires_shareable_track')
})

const localHost = async h => {
  await h.create()
  await h.message({ type: 'welcome', role: 'controller', state: room() })
  await h.timers.advance(4000)
  h.player.queue = [localTrack()]
  h.player.queueIndex = 0
  h.player.currentTrack = h.player.queue[0]
  await flush()
  const baseline = h.commands.length
  h.player.isPlaying = true
  await flush()
  h.player.isPlaying = false
  await flush()
  await h.timers.advance(26_000)
  const sent = h.commands.slice(baseline).filter(entry => entry.command === 'lt_send_event').map(entry => entry.args.event)
  assert.ok(!sent.some(event => event.type === 'PLAY' || event.type === 'PAUSE'), 'a local track does not drive the room')
  const heartbeat = sent.find(event => event.type === 'HEARTBEAT')
  assert.ok(heartbeat, 'the host still keeps the room alive')
  assert.equal(heartbeat.positionMs, undefined)
  assert.equal(heartbeat.state, undefined)
}
localHost.options = { role: 'controller' }
await test('a host on an unshareable track only keeps the room alive', localHost)

await test('leaving pauses local playback before telling the server', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  await h.timers.advance(4000)
  h.player.isPlaying = true
  await flush()
  const gate = deferred()
  h.pauseGates.push(gate)
  const leaving = h.store.leaveRoom()
  await flush()
  assert.ok(h.playback.some(entry => entry.type === 'pause'))
  assert.ok(!h.commands.some(entry => entry.command === 'lt_leave_room'), 'the pause happens first')
  gate.resolve()
  await leaving
  assert.ok(h.commands.some(entry => entry.command === 'lt_leave_room'))
})

// 房间从 100_000 起以位置 5000 播放；harness 的播放器不会自己走进度，要按预期位置手动设好
const playingRoom = (extra = {}) => room([wireTrack(1)], { playback: { ...room().playback, state: 'playing' }, ...extra })
const expectedAt = h => 5000 + (h.timers.Date.now() - 100_000)

async function requestListenerTrack(h) {
  await h.join()
  const initial = playingRoom({ queue: [wireTrack(1), wireTrack(2)] })
  await h.message({ type: 'welcome', state: initial, role: 'listener' })
  await h.timers.advance(4_000)
  h.player.queueIndex = 1
  h.player.currentTrack = h.player.queue[1]
  h.player.positionMs = 0
  await flush()
  const request = h.commands.find(entry => entry.command === 'lt_send_event'
    && entry.args.event.type === 'REQUEST_SET_TRACK')?.args.event
  assert.ok(request, 'the local selection is waiting for the host to confirm it')
  return { initial, request }
}

await test('the listener watchdog preserves a track selection while its queue request is awaiting acknowledgement', async h => {
  await requestListenerTrack(h)
  const plays = h.playback.filter(entry => entry.type === 'play').length
  await h.timers.advance(4_000)
  assert.equal(h.player.currentTrack.id, 'netease:2')
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, plays,
    'a watchdog tick cannot roll an unanswered track request back to the old room track')
})

await test('an unanswered listener queue request eventually allows the watchdog to restore authoritative playback', async h => {
  await requestListenerTrack(h)
  await h.timers.advance(20_000)
  assert.equal(h.player.currentTrack.id, 'netease:1',
    'a missing acknowledgement cannot suppress room recovery indefinitely')
})

await test('a heartbeat preserves a pending listener selection until its authoritative acknowledgement', async h => {
  const { initial, request } = await requestListenerTrack(h)
  const plays = h.playback.filter(entry => entry.type === 'play').length
  await h.message({ type: 'room_state_updated', state: { ...initial, version: 2 }, causedBy: { type: 'HEARTBEAT' } })
  assert.equal(h.store.roomState.value.version, 2, 'room metadata is still kept current')
  assert.equal(h.player.currentTrack.id, 'netease:2')
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, plays)
  await h.message({ type: 'room_state_updated', state: room(initial.queue, {
    version: 3, currentIndex: 1, playback: { ...initial.playback, basePositionMs: 0 },
  }), causedBy: { type: 'REQUEST_SET_TRACK', eventId: request.eventId } })
  assert.equal(h.player.currentTrack.id, 'netease:2')
  assert.equal(h.store.roomState.value.currentIndex, 1)
})

await test('an authoritative track change is applied while a listener queue request is pending', async h => {
  await requestListenerTrack(h)
  await h.message({ type: 'room_state_updated', state: room([wireTrack(3)], { version: 2 }),
    causedBy: { type: 'SET_TRACK', eventId: 'host-selection' } })
  assert.equal(h.player.currentTrack.id, 'netease:3')
})

await test('the listener watchdog leaves a synced player alone and corrects a drifted one', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: playingRoom(), role: 'listener' })
  await h.timers.advance(7_000)
  h.player.positionMs = expectedAt(h) + 1_000
  const synced = h.playback.length
  await h.timers.advance(1_000)
  assert.deepEqual(h.playback.slice(synced), [], 'a player that matches the room is not touched')
  await h.timers.advance(7_000)
  h.player.positionMs = expectedAt(h) + 1_000 + 10_000
  const drifted = h.playback.length
  await h.timers.advance(1_000)
  const seek = h.playback.slice(drifted).find(entry => entry.type === 'seek')
  assert.ok(seek, 'a 10 second drift is corrected without waiting for the host heartbeat')
  assert.ok(Math.abs(seek.positionMs - expectedAt(h)) <= 1_000)
})

await test('a room that has gone silent is refreshed over HTTP', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  await h.timers.advance(40_000)
  assert.ok(!h.commands.some(entry => entry.command === 'lt_get_room_state'))
  await h.timers.advance(8_000)
  assert.ok(h.commands.some(entry => entry.command === 'lt_get_room_state'), 'more than 45 seconds of silence triggers a refresh')
})

await test('an unreadable message is reported and repaired at the next watchdog tick', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  h.emit('lt:protocol_error', { connectionId: 'connection-current', message: 'Protocol: invalid type: string "x"' })
  await flush()
  assert.equal(h.store.sessionError.value, 'Protocol: invalid type: string "x"')
  await h.timers.advance(8_000)
  assert.ok(h.commands.some(entry => entry.command === 'lt_get_room_state'),
    'the possibly missed room update is fetched without waiting for 45 seconds of silence')
})

await test('a pending member request is retried and not undone by the watchdog', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: playingRoom(), role: 'listener' })
  await h.timers.advance(4_000)
  h.player.positionMs = expectedAt(h)
  h.player.isPlaying = false
  await flush()
  const requests = () => h.commands.filter(entry => entry.command === 'lt_send_event' && entry.args.event.type === 'REQUEST_PAUSE')
  assert.equal(requests().length, 1)
  const playbackCount = h.playback.length
  await h.timers.advance(4_000)
  assert.equal(requests().length, 2, 'an unanswered request is sent again')
  assert.equal(requests()[1].args.event.eventId, requests()[0].args.event.eventId)
  assert.ok(!h.playback.slice(playbackCount).some(entry => entry.type === 'resume' || entry.type === 'play'),
    'the user pause is not undone while the host has not answered')
  await h.message({ type: 'room_state_updated', state: room([wireTrack(1)], { version: 2 }) })
  await h.timers.advance(16_000)
  assert.equal(requests().length, 2, 'a request the room already reflects is not retried')
})

await test('a listener stuck loading the room track is reloaded', async h => {
  await h.join()
  const stuck = deferred()
  h.playGates.push(stuck)
  await h.message({ type: 'welcome', state: playingRoom(), role: 'listener' })
  await h.timers.advance(8_000)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 1, 'the first tick only starts the stall clock')
  await h.timers.advance(8_000)
  assert.equal(h.playback.filter(entry => entry.type === 'play').length, 2, 'a load stuck for 8 seconds is replaced')
  stuck.resolve()
  await flush()
})

const connectCount = h => h.commands.filter(entry => entry.command === 'lt_connect_ws').length

await test('a socket that stops answering pings is reconnected', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  const before = connectCount(h)
  await h.timers.advance(40_000)
  assert.equal(connectCount(h), before, 'an unanswered ping is given 35 seconds')
  assert.equal(h.commands.filter(entry => entry.command === 'lt_send_ping').length, 1, 'no new ping is sent until the last one is answered')
  await h.timers.advance(22_000)
  assert.equal(connectCount(h), before + 1, 'a half-open socket is replaced instead of waiting for TCP to give up')
})

await test('a socket that answers pings stays connected', async h => {
  await h.join()
  await h.message({ type: 'welcome', state: room(), role: 'listener' })
  const before = connectCount(h)
  await h.timers.advance(30_000)
  await h.message({ type: 'pong' })
  await h.timers.advance(32_000)
  assert.equal(connectCount(h), before)
})

if (failures.length) {
  console.error(`${passed} store tests passed; ${failures.length} failed: ${failures.join('; ')}`)
  process.exitCode = 1
} else {
  console.log(`listen together store tests passed (${passed} scenarios)`)
}
