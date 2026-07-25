import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { createPinia, defineStore, setActivePinia } from 'pinia'
import ts from 'typescript'
import { computed, reactive, ref, watch } from 'vue'

const storeUrl = new URL('../src/stores/listenTogether/index.ts', import.meta.url)
const protocolUrl = new URL('../src/stores/listenTogether/protocol.ts', import.meta.url)
const mapperUrl = new URL('../src/stores/listenTogether/mapper.ts', import.meta.url)
const backendCommandSource = await readFile(
  new URL('../src-tauri/src/commands/listen_together_cmd.rs', import.meta.url),
  'utf8',
)
const connectCommandSource = backendCommandSource.slice(
  backendCommandSource.indexOf('pub async fn lt_connect_ws('),
  backendCommandSource.indexOf('pub async fn lt_disconnect_ws('),
)
const disconnectCommandSource = backendCommandSource.slice(
  backendCommandSource.indexOf('pub async fn lt_disconnect_ws('),
  backendCommandSource.indexOf('pub async fn lt_send_event('),
)
assert.match(disconnectCommandSource, /session_id:\s*u64/)
assert.match(
  connectCommandSource,
  /session_id < ws_generation\.load\(Ordering::Acquire\)/,
  'a late connect from an older frontend generation must not replace the current backend session',
)
assert.match(
  disconnectCommandSource,
  /fetch_max\(session_id\.saturating_add\(1\), Ordering::AcqRel\)/,
  'an explicit disconnect must fence out later connect commands from the departed generation',
)
assert.match(
  disconnectCommandSource,
  /ws_generation\.load\(Ordering::Acquire\) > session_id/,
  'a stale frontend disconnect must not close a replacement backend session',
)
assert.match(
  disconnectCommandSource,
  /active_session_id > session_id/,
  'leaving a superseded attempt must still close an older active backend session',
)

async function loadMapperModule() {
  const protocolSource = await readFile(protocolUrl, 'utf8')
  const mapperSource = await readFile(mapperUrl, 'utf8')
  const protocolConsts = protocolSource
    .split('export interface')[0]
    .replace(/^[\s\S]*?export const LtChannels/, 'const LtChannels')
  const mapperBody = mapperSource
    .replace(/import type \{ TrackInfo \} from ['"]@\/stores\/player['"]\s*/g, '')
    .replace(/import \{ LtChannels, type ListenTogetherTrack \} from ['"]\.\/protocol['"]\s*/g, '')
  const compiled = ts.transpileModule(`${protocolConsts}\n${mapperBody}\nexport { LtChannels }\n`, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
  return import(moduleUrl)
}

const listenTogetherMapper = await loadMapperModule()

class FakeClock {
  #nextId = 1
  #now = 0
  #timers = new Map()

  setTimeout = (callback, delay = 0) => {
    const id = this.#nextId++
    this.#timers.set(id, {
      callback,
      dueAt: this.#now + Number(delay),
    })
    return id
  }

  clearTimeout = id => {
    this.#timers.delete(id)
  }

  async runAll() {
    let executions = 0
    while (this.#timers.size > 0) {
      if (++executions > 100) throw new Error('fake timer loop did not settle')

      const [id, timer] = [...this.#timers.entries()]
        .sort((left, right) => left[1].dueAt - right[1].dueAt)[0]
      this.#timers.delete(id)
      this.#now = timer.dueAt
      await timer.callback()
      await flushPromises()
    }
  }
}

function deferred() {
  let resolve
  let reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

function makeTrack(id) {
  return {
    stableKey: `netease:${id}`,
    channelId: 'netease',
    audioId: id,
    name: `Track ${id}`,
    artist: 'Artist',
    durationMs: 180_000,
  }
}

function makePlayerTrack(id) {
  return {
    id: `netease:${id}`,
    title: `Track ${id}`,
    artist: 'Artist',
    album: 'Album',
    durationMs: 180_000,
    coverUrl: 'https://example.test/cover.jpg',
    audioUrl: '',
    source: 'netease',
  }
}

function makeLocalPlayerTrack(id) {
  return {
    id: `local:${id}`,
    title: `Local ${id}`,
    artist: 'Local artist',
    album: 'Local album',
    durationMs: 180_000,
    coverUrl: '',
    audioUrl: `C:/Music/${id}.mp3`,
    source: 'local',
  }
}

function makeRoomState(
  id,
  { positionMs = 0, version = 1, isPlaying = false, roomId = `room-${id}` } = {},
) {
  const track = makeTrack(id)
  return {
    roomId,
    version,
    updatedAt: version,
    queue: [track],
    currentIndex: 0,
    track,
    playback: {
      state: isPlaying ? 'playing' : 'paused',
      basePositionMs: positionMs,
      baseTimestampMs: 0,
      playbackRate: 1,
      repeatMode: 0,
      shuffleEnabled: false,
    },
    settings: {
      allowMemberControl: true,
      autoPauseOnMemberChange: true,
      shareAudioLinks: true,
    },
    members: [],
    roomStatus: 'active',
  }
}

function joinResponse(roomId, state = undefined) {
  return {
    ok: true,
    roomId,
    role: 'listener',
    token: `token-${roomId}`,
    wsUrl: `ws://${roomId}`,
    state,
  }
}

function createHarness() {
  const commandCalls = []
  const listeners = new Map()
  const listenerHistory = new Map()
  const playerActions = []

  const player = reactive({
    queue: [],
    queueIndex: -1,
    currentTrack: null,
    currentResolvedStreamUrl: null,
    isPlaying: false,
    positionMs: 0,
    repeatMode: 'off',
    shuffleEnabled: false,
    lastSeekCommand: { seq: 0, source: 'local', positionMs: 0 },
    play(track, source) {
      playerActions.push({ type: 'play', trackId: track.id, source })
      this.currentTrack = track
      this.isPlaying = true
      this.positionMs = 0
    },
    seekTo(positionMs, source) {
      playerActions.push({ type: 'seek', positionMs, trackId: this.currentTrack?.id, source })
      this.positionMs = positionMs
    },
    pause(source) {
      playerActions.push({ type: 'pause', trackId: this.currentTrack?.id, source })
      this.isPlaying = false
    },
    resume(source) {
      playerActions.push({ type: 'resume', trackId: this.currentTrack?.id, source })
      this.isPlaying = true
    },
    togglePlayPause(source) {
      playerActions.push({ type: 'toggle', source })
      this.isPlaying = !this.isPlaying
    },
    applyListenTogetherPlaybackMode() {},
    isRemoteSyncGuardActive() { return false },
  })

  const settings = reactive({
    ltServerUrl: 'https://example.test',
    ltNickname: 'Tester',
    ltAllowMemberControl: true,
    ltAutoPauseOnMemberChange: true,
    ltShareAudioLinks: true,
  })

  const harness = {
    commandCalls,
    listeners,
    listenerHistory,
    player,
    playerActions,
    settings,
    sendResult: true,
    backendRoomId: null,
    backendSessionId: 0,
    createHandler: () => ({
      ok: true,
      roomId: 'room-created',
      role: 'controller',
      token: 'token-room-created',
      wsUrl: 'ws://room-created',
    }),
    joinHandler: ({ roomId }) => joinResponse(roomId),
    connectHandler: async () => undefined,
    disconnectHandler: () => undefined,
    roomStateHandler: async () => ({ ok: false }),

    async invoke(command, args = {}) {
      commandCalls.push({ command, args })
      switch (command) {
        case 'lt_join_room':
          return harness.joinHandler(args)
        case 'lt_create_room':
          return harness.createHandler(args)
        case 'lt_connect_ws':
          harness.backendSessionId = args.sessionId
          return harness.connectHandler(args)
        case 'lt_disconnect_ws':
          return harness.disconnectHandler(args)
        case 'lt_send_event':
          return harness.sendResult
        case 'lt_get_room_state':
          return harness.roomStateHandler(args)
        default:
          throw new Error(`unexpected command: ${command}`)
      }
    },

    async listen(eventName, callback) {
      let current = listeners.get(eventName)
      if (!current) {
        current = new Set()
        listeners.set(eventName, current)
      }
      current.add(callback)

      let history = listenerHistory.get(eventName)
      if (!history) {
        history = []
        listenerHistory.set(eventName, history)
      }
      history.push(callback)

      return () => current.delete(callback)
    },

    emit(eventName, payload = {}) {
      const taggedPayload = eventName === 'lt:message'
        ? { sessionId: harness.backendSessionId, envelope: payload }
        : eventName === 'lt:connected' || eventName === 'lt:disconnected'
          ? { sessionId: harness.backendSessionId, ...payload }
          : payload
      for (const callback of [...(listeners.get(eventName) ?? [])]) {
        callback({ payload: taggedPayload })
      }
    },
  }

  return harness
}

async function loadStoreModule() {
  const source = await readFile(storeUrl, 'utf8')
  const sourceFile = ts.createSourceFile(
    storeUrl.pathname,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  )

  const imports = sourceFile.statements
    .filter(ts.isImportDeclaration)
    .map(statement => ({ start: statement.getStart(sourceFile), end: statement.end }))
    .sort((left, right) => right.start - left.start)

  let body = source
  for (const { start, end } of imports) {
    body = body.slice(0, start) + body.slice(end)
  }

  const injectedDependencies = `
    const { defineStore, ref, computed, watch } = globalThis.__ltTestDependencies
    const invoke = (...args) => globalThis.__ltTestHarness.invoke(...args)
    const listen = (...args) => globalThis.__ltTestHarness.listen(...args)
    const readText = async () => ''
    const writeText = async () => undefined
    const usePlayerStore = () => globalThis.__ltTestHarness.player
    const useSettingsStore = () => globalThis.__ltTestHarness.settings
    const useToastStore = () => ({ error() {}, success() {} })
    const i18n = { global: { t: key => key } }
    const {
      LtChannels,
      trackInfoToLtTrack,
      ltTrackToTrackInfo,
      toShareableQueueSnapshot,
    } = globalThis.__ltMapper
    const desktopRepeatToWire = mode => mode === 'one' ? 1 : mode === 'all' ? 2 : 0
    const createLogger = () => ({ debug() {}, error() {} })
  `

  const compiled = ts.transpileModule(`${injectedDependencies}\n${body}`, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
    fileName: storeUrl.pathname,
  }).outputText

  const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
  return import(moduleUrl)
}

async function flushPromises() {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

globalThis.__ltTestDependencies = { defineStore, ref, computed, watch }
globalThis.__ltMapper = listenTogetherMapper
globalThis.localStorage = {
  values: new Map(),
  getItem(key) { return this.values.get(key) ?? null },
  setItem(key, value) { this.values.set(key, value) },
}

const { useListenTogetherStore } = await loadStoreModule()

await test('listen-together frontend lifecycle', async t => {
  await t.test('controller creation snapshot carries the current resolved stream only', async t => {
    const harness = createHarness()
    const first = makePlayerTrack('snapshot-first')
    const current = makePlayerTrack('snapshot-current')
    const streamUrl = 'https://music.126.net/snapshot-current.mp3'
    harness.player.queue = [first, current]
    harness.player.queueIndex = 1
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = streamUrl
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()

    const snapshot = harness.commandCalls
      .find(call => call.command === 'lt_create_room')
      .args.initialSnapshot
    assert.equal(snapshot.currentIndex, 1)
    assert.equal(snapshot.queue[0].streamUrl, undefined)
    assert.equal(snapshot.queue[1].streamUrl, streamUrl)
    assert.equal(snapshot.track.streamUrl, streamUrl)

  })

  await t.test('controller creation omits a local current track from the shared snapshot', async t => {
    const harness = createHarness()
    const before = makePlayerTrack('before-local-current')
    const localCurrent = makeLocalPlayerTrack('private-current')
    const after = makePlayerTrack('after-local-current')
    harness.player.queue = [before, localCurrent, after]
    harness.player.queueIndex = 1
    harness.player.currentTrack = localCurrent
    harness.player.isPlaying = true
    harness.player.positionMs = 12_345
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()

    const snapshot = harness.commandCalls
      .find(call => call.command === 'lt_create_room')
      .args.initialSnapshot
    assert.deepEqual(
      snapshot.queue.map(track => track.stableKey),
      ['netease:before-local-current', 'netease:after-local-current'],
    )
    assert.equal(snapshot.currentIndex, 0)
    assert.equal(snapshot.track, undefined)
    assert.equal(snapshot.isPlaying, false)
    assert.equal(snapshot.positionMs, 0)
    assert.doesNotMatch(JSON.stringify(snapshot), /C:\/Music/)
  })

  await t.test('controller set-track events carry the current resolved stream only', async t => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => ({
      ...joinResponse(roomId),
      role: 'controller',
    })
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.joinRoom('room-set-track-link')
    harness.emit('lt:connected')
    const first = makePlayerTrack('event-first')
    const current = makePlayerTrack('event-current')
    const streamUrl = 'https://music.126.net/event-current.mp3'
    harness.player.queue = [first, current]
    harness.player.queueIndex = 1
    harness.player.currentResolvedStreamUrl = streamUrl
    harness.player.currentTrack = current
    await flushPromises()

    const event = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.event)
      .find(candidate => candidate.type === 'SET_TRACK')
    assert.ok(event)
    assert.equal(event.currentIndex, 1)
    assert.equal(event.queue[0].streamUrl, undefined)
    assert.equal(event.queue[1].streamUrl, streamUrl)
    assert.equal(event.track.streamUrl, streamUrl)

  })

  await t.test('controller does not emit SET_TRACK for a local current track', async t => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => ({
      ...joinResponse(roomId),
      role: 'controller',
    })
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.joinRoom('room-local-set-track')
    harness.emit('lt:connected')
    const before = makePlayerTrack('before-local-set-track')
    const localCurrent = makeLocalPlayerTrack('private-set-track')
    harness.player.queue = [before, localCurrent]
    harness.player.queueIndex = 1
    harness.player.currentTrack = localCurrent
    await flushPromises()

    const setTrackEvents = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.event)
      .filter(event => event.type === 'SET_TRACK')
    assert.deepEqual(setTrackEvents, [])
  })

  await t.test('listener does not request a local current track', async t => {
    const harness = createHarness()
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.joinRoom('room-local-request-set-track')
    harness.emit('lt:connected')
    const before = makePlayerTrack('before-local-request-set-track')
    const localCurrent = makeLocalPlayerTrack('private-request-set-track')
    harness.player.queue = [before, localCurrent]
    harness.player.queueIndex = 1
    harness.player.currentTrack = localCurrent
    await flushPromises()

    const requestEvents = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.event)
      .filter(event => event.type === 'REQUEST_SET_TRACK')
    assert.deepEqual(requestEvents, [])
  })

  await t.test('controller local playback does not control the shared room', async t => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => ({
      ...joinResponse(roomId),
      role: 'controller',
    })
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.joinRoom('room-local-controller-control')
    harness.emit('lt:connected')
    const before = makePlayerTrack('before-local-controller-control')
    const localCurrent = makeLocalPlayerTrack('private-controller-control')
    harness.player.queue = [before, localCurrent]
    harness.player.queueIndex = 1
    harness.player.currentTrack = localCurrent
    await flushPromises()
    harness.player.isPlaying = true
    await flushPromises()
    store.reportSeekEvent(1_234)
    await flushPromises()

    const controlEvents = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.event)
      .filter(event => ['PLAY', 'PAUSE', 'SEEK', 'PLAYBACK_MODE'].includes(event.type))
    assert.deepEqual(controlEvents, [])
  })

  await t.test('listener local playback does not request room controls', async t => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()
      t.after(() => store.leaveRoom())

      await store.joinRoom('room-local-listener-control')
      harness.emit('lt:connected')
      const before = makePlayerTrack('before-local-listener-control')
      const localCurrent = makeLocalPlayerTrack('private-listener-control')
      harness.player.queue = [before, localCurrent]
      harness.player.queueIndex = 1
      harness.player.currentTrack = localCurrent
      await flushPromises()
      harness.player.isPlaying = true
      harness.player.lastSeekCommand = { seq: 1, source: 'local', positionMs: 2_468 }
      await flushPromises()
      await clock.runAll()

      const requestEvents = harness.commandCalls
        .filter(call => call.command === 'lt_send_event')
        .map(call => call.args.event)
        .filter(event => [
          'REQUEST_PLAY',
          'REQUEST_PAUSE',
          'REQUEST_SEEK',
          'REQUEST_PLAYBACK_MODE',
        ].includes(event.type))
      assert.deepEqual(requestEvents, [])
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('controller heartbeat leaves local playback out of the room state', async t => {
    const originalSetInterval = globalThis.setInterval
    const originalClearInterval = globalThis.clearInterval
    let heartbeat = null
    globalThis.setInterval = callback => {
      heartbeat = callback
      return 1
    }
    globalThis.clearInterval = () => undefined

    try {
      const harness = createHarness()
      const before = makePlayerTrack('before-local-heartbeat')
      const localCurrent = makeLocalPlayerTrack('private-heartbeat')
      harness.player.queue = [before, localCurrent]
      harness.player.queueIndex = 1
      harness.player.currentTrack = localCurrent
      harness.player.isPlaying = true
      harness.player.positionMs = 54_321
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()
      t.after(() => store.leaveRoom())

      await store.createRoom()
      harness.emit('lt:connected')
      await flushPromises()
      assert.equal(typeof heartbeat, 'function')
      heartbeat()
      await flushPromises()

      const event = harness.commandCalls
        .filter(call => call.command === 'lt_send_event')
        .map(call => call.args.event)
        .find(candidate => candidate.type === 'HEARTBEAT')
      assert.ok(event)
      assert.equal(event.track, undefined)
      assert.equal(event.queue, undefined)
      assert.equal(event.currentIndex, undefined)
      assert.equal(event.positionMs, undefined)
      assert.equal(event.state, undefined)
      assert.equal(event.repeatMode, undefined)
      assert.equal(event.shuffleEnabled, undefined)
      assert.doesNotMatch(JSON.stringify(event), /C:\/Music/)
    } finally {
      globalThis.setInterval = originalSetInterval
      globalThis.clearInterval = originalClearInterval
    }
  })

  await t.test('controller confirms a remote member track request with the requested queue item', async t => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => ({
      ...joinResponse(roomId),
      role: 'controller',
    })
    const initial = makePlayerTrack('member-request-initial')
    const requested = makePlayerTrack('member-request-target')
    harness.player.queue = [initial, requested]
    harness.player.queueIndex = 1
    harness.player.currentTrack = initial
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.joinRoom('room-member-track-request')
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'member_control_requested',
      causedBy: { type: 'REQUEST_SET_TRACK' },
      currentIndex: 1,
      track: makeTrack('member-request-target'),
    })
    await flushPromises()

    const event = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.event)
      .find(candidate => candidate.type === 'SET_TRACK')
    assert.ok(event)
    assert.equal(event.track.stableKey, 'netease:member-request-target')
    assert.equal(event.currentIndex, 1)
    assert.deepEqual(
      event.queue.map(track => track.stableKey),
      ['netease:member-request-initial', 'netease:member-request-target'],
    )
  })

  await t.test('link requests receive Android-compatible LINK_READY for the current session and track', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('link-current')
    const streamUrl = 'https://music.126.net/link-current.mp3'
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = streamUrl
    harness.player.isPlaying = true
    harness.player.positionMs = 12_345
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
      track: { stableKey: current.id },
    })
    await flushPromises()

    const linkReadyCall = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .find(call => call.args.event.type === 'LINK_READY')
    const connectSessionId = harness.commandCalls
      .find(call => call.command === 'lt_connect_ws')
      .args.sessionId
    assert.ok(linkReadyCall)
    assert.equal(linkReadyCall.args.sessionId, connectSessionId)
    assert.deepEqual(
      {
        type: linkReadyCall.args.event.type,
        requestTrackStableKey: linkReadyCall.args.event.requestTrackStableKey,
        currentIndex: linkReadyCall.args.event.currentIndex,
        positionMs: linkReadyCall.args.event.positionMs,
        state: linkReadyCall.args.event.state,
        trackStreamUrl: linkReadyCall.args.event.track?.streamUrl,
        queueStreamUrl: linkReadyCall.args.event.queue?.[0]?.streamUrl,
      },
      {
        type: 'LINK_READY',
        requestTrackStableKey: current.id,
        currentIndex: 0,
        positionMs: 12_345,
        state: 'playing',
        trackStreamUrl: streamUrl,
        queueStreamUrl: streamUrl,
      },
    )

  })

  await t.test('local current tracks never send LINK_READY', async t => {
    const harness = createHarness()
    const current = makeLocalPlayerTrack('private-link-current')
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = 'https://music.126.net/private-link-current.mp3'
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
      track: { stableKey: current.id },
    })
    await flushPromises()

    const sentEvents = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.event)
    assert.equal(sentEvents.some(event => event.type === 'LINK_READY'), false)
    assert.doesNotMatch(JSON.stringify(sentEvents), /C:\/Music/)
  })

  await t.test('link requests without requestTrackStableKey are rejected', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('missing-request-key')
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = 'https://music.126.net/missing-request-key.mp3'
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      track: { stableKey: current.id },
    })
    await flushPromises()

    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )
  })

  await t.test('link requests wait for the current stream URL to resolve', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('deferred-link')
    const streamUrl = 'https://music.126.net/deferred-link.mp3'
    const resolution = deferred()
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = null
    harness.player.resolveCurrentStreamUrl = () => resolution.promise
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

    harness.player.currentResolvedStreamUrl = streamUrl
    resolution.resolve(streamUrl)
    await flushPromises()

    const linkReadyCall = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .find(call => call.args.event.type === 'LINK_READY')
    assert.ok(linkReadyCall)
    assert.equal(linkReadyCall.args.event.requestTrackStableKey, current.id)
    assert.equal(linkReadyCall.args.event.track.streamUrl, streamUrl)
  })

  await t.test('a pending link resolution cannot publish after the current track changes', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('deferred-stale-track')
    const replacement = makePlayerTrack('deferred-replacement-track')
    const staleStreamUrl = 'https://music.126.net/deferred-stale-track.mp3'
    const resolution = deferred()
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = null
    harness.player.resolveCurrentStreamUrl = () => resolution.promise
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

    harness.player.queue = [replacement]
    harness.player.queueIndex = 0
    harness.player.currentTrack = replacement
    harness.player.currentResolvedStreamUrl = staleStreamUrl
    resolution.resolve(staleStreamUrl)
    await flushPromises()

    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )
  })

  await t.test('a stale same-track stream resolution cannot publish a newer URL', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('deferred-stale-same-track')
    const replacementStreamUrl = 'https://music.126.net/deferred-new-same-track.mp3'
    const resolution = deferred()
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = null
    harness.player.resolveCurrentStreamUrl = () => resolution.promise
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

    harness.player.currentResolvedStreamUrl = replacementStreamUrl
    resolution.resolve(null)
    await flushPromises()

    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )
  })

  await t.test('a pending link resolution cannot publish after leaving and creating a new session', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('deferred-stale-session')
    const replacement = makePlayerTrack('deferred-new-session')
    const staleStreamUrl = 'https://music.126.net/deferred-stale-session.mp3'
    const resolution = deferred()
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = null
    harness.player.resolveCurrentStreamUrl = () => resolution.promise
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

    await store.leaveRoom()
    harness.player.queue = [replacement]
    harness.player.queueIndex = 0
    harness.player.currentTrack = replacement
    harness.player.currentResolvedStreamUrl = null
    await store.createRoom()
    harness.emit('lt:connected')

    resolution.resolve(staleStreamUrl)
    await flushPromises()

    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )
  })

  await t.test('link requests from listeners or disconnected sessions are rejected', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('link-gate')
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = 'https://music.126.net/link-gate.mp3'
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.joinRoom('room-link-gate')
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

    await store.leaveRoom()
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )
  })

  await t.test('non-http stream URLs are never sent in LINK_READY', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('invalid-link-url')
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = 'file:///invalid-link-url.mp3'
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )
  })

  await t.test('disabled audio-link sharing never leaks URLs or sends LINK_READY', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('private-current')
    harness.settings.ltShareAudioLinks = false
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = 'https://music.126.net/private-current.mp3'
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    const snapshot = harness.commandCalls
      .find(call => call.command === 'lt_create_room')
      .args.initialSnapshot
    assert.equal(snapshot.track.streamUrl, undefined)
    assert.equal(snapshot.queue[0].streamUrl, undefined)

    harness.emit('lt:connected')
    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: current.id,
    })
    await flushPromises()
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

  })

  await t.test('link requests for a stale track or session never send LINK_READY', async t => {
    const harness = createHarness()
    const current = makePlayerTrack('current-link-target')
    harness.player.queue = [current]
    harness.player.queueIndex = 0
    harness.player.currentTrack = current
    harness.player.currentResolvedStreamUrl = 'https://music.126.net/current-link-target.mp3'
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()
    t.after(() => store.leaveRoom())

    await store.createRoom()
    harness.emit('lt:connected')
    const oldSessionId = harness.backendSessionId
    const oldMessageListener = harness.listenerHistory.get('lt:message')[0]

    harness.emit('lt:message', {
      type: 'link_requested',
      requestTrackStableKey: 'netease:stale-track',
    })
    await store.createRoom()
    const currentSessionId = harness.backendSessionId
    oldMessageListener({
      payload: {
        sessionId: oldSessionId,
        envelope: {
          type: 'link_requested',
          requestTrackStableKey: current.id,
        },
      },
    })
    await flushPromises()

    assert.notEqual(currentSessionId, oldSessionId)
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_send_event' && call.args.event.type === 'LINK_READY'
      )),
      false,
    )

  })

  await t.test('session generation survives recreating the Pinia store', async () => {
    globalThis.localStorage.values.delete('neri:lt-session-generation')

    const harness = createHarness()
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const firstStore = useListenTogetherStore()

    await firstStore.joinRoom('room-before-store-reload')
    const firstSessionId = harness.commandCalls
      .filter(call => call.command === 'lt_connect_ws')
      .at(-1)
      .args.sessionId

    setActivePinia(createPinia())
    const reloadedStore = useListenTogetherStore()
    await reloadedStore.joinRoom('room-after-store-reload')
    const reloadedSessionId = harness.commandCalls
      .filter(call => call.command === 'lt_connect_ws')
      .at(-1)
      .args.sessionId

    assert.equal(
      reloadedSessionId > firstSessionId,
      true,
      'a reconstructed frontend store must advance beyond the backend session barrier',
    )
  })

  await t.test('a delayed action from an older Pinia store cannot overwrite the current store', async () => {
    const generationKey = 'neri:lt-session-generation'
    globalThis.localStorage.values.delete(generationKey)

    try {
      const staleJoin = deferred()
      const harness = createHarness()
      harness.joinHandler = ({ roomId }) => (
        roomId === 'room-stale-store' ? staleJoin.promise : joinResponse(roomId)
      )
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const staleStore = useListenTogetherStore()
      const pendingStaleJoin = staleStore.joinRoom('room-stale-store')
      await flushPromises()

      setActivePinia(createPinia())
      const currentStore = useListenTogetherStore()
      await currentStore.joinRoom('room-current-store')

      staleJoin.resolve(joinResponse(
        'room-stale-store',
        makeRoomState('stale-store', { roomId: 'room-stale-store' }),
      ))
      await pendingStaleJoin

      assert.equal(currentStore.roomId, 'room-current-store')
      assert.deepEqual(
        harness.commandCalls
          .filter(call => call.command === 'lt_connect_ws')
          .map(call => call.args.wsUrl),
        ['ws://room-current-store'],
      )
      assert.deepEqual(harness.playerActions, [])
    } finally {
      globalThis.localStorage.values.delete(generationKey)
    }
  })

  await t.test('invalid persisted session generations stop before backend work', async () => {
    const invalidValues = [
      '-1',
      '1.5',
      '1.0000000000000000000001',
      '01',
      '+1',
      '0x10',
      '1e3',
      ' 1 ',
      'not-a-number',
      String(Number.MAX_SAFE_INTEGER + 1),
    ]

    for (const [index, invalidValue] of invalidValues.entries()) {
      globalThis.localStorage.values.set('neri:lt-session-generation', invalidValue)
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await assert.rejects(
        store.joinRoom(`room-invalid-generation-${index}`),
        /invalid persisted session generation/,
      )
      assert.deepEqual(harness.commandCalls, [])
      assert.equal(
        globalThis.localStorage.values.get('neri:lt-session-generation'),
        invalidValue,
      )
    }
  })

  await t.test('session generation never crosses the safe integer boundary', async () => {
    const generationKey = 'neri:lt-session-generation'
    globalThis.localStorage.values.set(
      generationKey,
      String(Number.MAX_SAFE_INTEGER - 1),
    )

    try {
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const lastSafeStore = useListenTogetherStore()

      await lastSafeStore.joinRoom('room-last-safe-generation')
      const connectedSessionIds = harness.commandCalls
        .filter(call => call.command === 'lt_connect_ws')
        .map(call => call.args.sessionId)
      assert.deepEqual(connectedSessionIds, [Number.MAX_SAFE_INTEGER])

      await assert.rejects(
        lastSafeStore.leaveRoom(),
        /session generation exhausted/,
      )
      assert.equal(lastSafeStore.roomId, null)
      assert.equal(lastSafeStore.connectionState, 'disconnected')
      assert.equal(
        harness.commandCalls.some(call => (
          call.command === 'lt_disconnect_ws'
          && call.args.sessionId === Number.MAX_SAFE_INTEGER
        )),
        true,
      )

      setActivePinia(createPinia())
      const exhaustedStore = useListenTogetherStore()
      await assert.rejects(
        exhaustedStore.joinRoom('room-generation-overflow'),
        /session generation exhausted/,
      )
      assert.deepEqual(
        harness.commandCalls
          .filter(call => call.command === 'lt_connect_ws')
          .map(call => call.args.sessionId),
        [Number.MAX_SAFE_INTEGER],
      )
    } finally {
      globalThis.localStorage.values.delete(generationKey)
    }
  })

  await t.test('a session does not start when its generation cannot be persisted', async () => {
    const generationKey = 'neri:lt-session-generation'
    const originalSetItem = globalThis.localStorage.setItem
    globalThis.localStorage.values.set(generationKey, '41')
    globalThis.localStorage.setItem = function setItem(key, value) {
      if (key === generationKey) throw new Error('generation storage unavailable')
      return originalSetItem.call(this, key, value)
    }

    try {
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await assert.rejects(
        store.joinRoom('room-without-generation-storage'),
        /generation storage unavailable/,
      )
      assert.deepEqual(harness.commandCalls, [])
      assert.equal(globalThis.localStorage.values.get(generationKey), '41')
    } finally {
      globalThis.localStorage.setItem = originalSetItem
      globalThis.localStorage.values.delete(generationKey)
    }
  })

  await t.test('a session does not start when its generation cannot be read', async () => {
    const generationKey = 'neri:lt-session-generation'
    const originalGetItem = globalThis.localStorage.getItem
    let generationReads = 0
    globalThis.localStorage.values.set(generationKey, '41')
    globalThis.localStorage.getItem = function getItem(key) {
      if (key === generationKey && ++generationReads > 1) {
        throw new Error('generation storage unreadable')
      }
      return originalGetItem.call(this, key)
    }

    try {
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await assert.rejects(
        store.joinRoom('room-without-generation-read'),
        /generation storage unreadable/,
      )
      assert.deepEqual(harness.commandCalls, [])
      assert.equal(globalThis.localStorage.values.get(generationKey), '41')
    } finally {
      globalThis.localStorage.getItem = originalGetItem
      globalThis.localStorage.values.delete(generationKey)
    }
  })

  await t.test('leaving cancels delayed seek and pause from the departed room', async () => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const harness = createHarness()
      harness.joinHandler = ({ roomId }) => joinResponse(
        roomId,
        makeRoomState('old', { positionMs: 5_000 }),
      )
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await store.joinRoom('room-old')
      await store.leaveRoom()
      await clock.runAll()

      assert.deepEqual(
        harness.playerActions.filter(action => action.type === 'seek' || action.type === 'pause'),
        [],
      )
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('a delayed leave disconnect cannot close a newer room session', async () => {
    const harness = createHarness()
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-before-leave')
    const delayedDisconnect = deferred()
    let delayNextDisconnect = true
    harness.disconnectHandler = async ({ sessionId }) => {
      if (delayNextDisconnect) {
        delayNextDisconnect = false
        await delayedDisconnect.promise
      }
      if (typeof sessionId !== 'number' || harness.backendSessionId === sessionId) {
        harness.backendSessionId = 0
      }
    }

    const pendingLeave = store.leaveRoom()
    const pendingJoin = store.joinRoom('room-after-leave')

    for (let attempt = 0; attempt < 20; attempt++) {
      await flushPromises()
      if (harness.commandCalls.some(call => (
        call.command === 'lt_connect_ws' && call.args.wsUrl === 'ws://room-after-leave'
      ))) break
    }
    assert.equal(
      harness.commandCalls.some(call => (
        call.command === 'lt_connect_ws' && call.args.wsUrl === 'ws://room-after-leave'
      )),
      true,
      'the replacement room must connect before the delayed old disconnect is released',
    )

    delayedDisconnect.resolve()
    await Promise.all([pendingLeave, pendingJoin])

    const currentSessionId = harness.commandCalls
      .filter(call => call.command === 'lt_connect_ws')
      .at(-1)
      .args.sessionId
    assert.equal(store.roomId, 'room-after-leave')
    assert.equal(harness.backendSessionId, currentSessionId)
  })

  await t.test('a newer room state supersedes the previous delayed player action', async () => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const harness = createHarness()
      harness.joinHandler = ({ roomId }) => joinResponse(
        roomId,
        makeRoomState('first', { positionMs: 5_000, version: 1, roomId: 'room-shared' }),
      )
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await store.joinRoom('room-shared')
      harness.emit('lt:message', {
        type: 'room_state_updated',
        state: makeRoomState('second', {
          positionMs: 9_000,
          version: 2,
          roomId: 'room-shared',
        }),
      })
      await clock.runAll()

      assert.deepEqual(
        harness.playerActions.filter(action => action.type === 'seek'),
        [{
          type: 'seek',
          positionMs: 9_000,
          trackId: 'netease:second',
          source: 'remote_sync',
        }],
      )
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('a newer empty room state cancels the previous delayed player action', async () => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const harness = createHarness()
      harness.joinHandler = ({ roomId }) => joinResponse(
        roomId,
        makeRoomState('first', { positionMs: 5_000, version: 1, roomId: 'room-empty' }),
      )
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await store.joinRoom('room-empty')
      const emptyState = makeRoomState('unused', { version: 2, roomId: 'room-empty' })
      emptyState.track = undefined
      emptyState.queue = []
      harness.emit('lt:message', {
        type: 'room_state_updated',
        state: emptyState,
      })
      await clock.runAll()

      assert.deepEqual(
        harness.playerActions.filter(action => action.type === 'seek' || action.type === 'pause'),
        [],
      )
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('joining another room cancels the previous room reconnect', async () => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await store.joinRoom('room-a')
      harness.emit('lt:connected')
      harness.emit('lt:disconnected', { code: 1006, reason: 'network' })
      await store.joinRoom('room-b')
      await clock.runAll()

      assert.deepEqual(
        harness.commandCalls
          .filter(call => call.command === 'lt_connect_ws')
          .map(call => call.args.wsUrl),
        ['ws://room-a', 'ws://room-b'],
      )
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('a stalled reconnect does not block leaving and joining a new room', async () => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const reconnectGate = deferred()
      const harness = createHarness()
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await store.joinRoom('room-stalled-reconnect')
      harness.emit('lt:connected')
      let stallNextOldConnect = true
      harness.connectHandler = ({ wsUrl }) => {
        if (wsUrl === 'ws://room-stalled-reconnect' && stallNextOldConnect) {
          stallNextOldConnect = false
          return reconnectGate.promise
        }
        return undefined
      }
      harness.emit('lt:disconnected', { code: 1006, reason: 'network' })
      const pendingReconnect = clock.runAll()

      for (let attempt = 0; attempt < 20; attempt++) {
        await flushPromises()
        if (harness.commandCalls.filter(call => (
          call.command === 'lt_connect_ws'
          && call.args.wsUrl === 'ws://room-stalled-reconnect'
        )).length === 2) break
      }

      const pendingLeave = store.leaveRoom()
      const pendingJoin = store.joinRoom('room-after-stalled-reconnect')
      for (let attempt = 0; attempt < 20; attempt++) {
        await flushPromises()
        if (harness.commandCalls.some(call => (
          call.command === 'lt_connect_ws'
          && call.args.wsUrl === 'ws://room-after-stalled-reconnect'
        ))) break
      }

      assert.equal(
        harness.commandCalls.some(call => (
          call.command === 'lt_connect_ws'
          && call.args.wsUrl === 'ws://room-after-stalled-reconnect'
        )),
        true,
        'a stalled reconnect must not occupy the backend room-operation queue',
      )

      reconnectGate.resolve()
      await Promise.all([pendingReconnect, pendingLeave, pendingJoin])
      assert.equal(store.roomId, 'room-after-stalled-reconnect')
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('a false send result marks the session disconnected and reconnects it', async () => {
    const clock = new FakeClock()
    const originalSetTimeout = globalThis.setTimeout
    const originalClearTimeout = globalThis.clearTimeout
    globalThis.setTimeout = clock.setTimeout
    globalThis.clearTimeout = clock.clearTimeout

    try {
      const harness = createHarness()
      harness.sendResult = false
      globalThis.__ltTestHarness = harness
      setActivePinia(createPinia())
      const store = useListenTogetherStore()

      await store.joinRoom('room-send')
      harness.emit('lt:connected')
      store.reportSeekEvent(1_234)
      await flushPromises()

      assert.equal(store.connectionState, 'disconnected')
      await clock.runAll()
      assert.equal(
        harness.commandCalls.filter(call => call.command === 'lt_connect_ws').length,
        2,
      )
      const reconnectSessions = harness.commandCalls
        .filter(call => call.command === 'lt_connect_ws')
        .map(call => call.args.sessionId)
      assert.equal(reconnectSessions[0] > 0, true)
      assert.deepEqual(reconnectSessions, [reconnectSessions[0], reconnectSessions[0]])
    } finally {
      globalThis.setTimeout = originalSetTimeout
      globalThis.clearTimeout = originalClearTimeout
    }
  })

  await t.test('outbound commands stay bound to the session that created them', async () => {
    const harness = createHarness()
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-send-a')
    harness.emit('lt:connected')
    store.reportSeekEvent(1_000)
    await flushPromises()

    await store.joinRoom('room-send-b')
    harness.emit('lt:connected')
    store.reportSeekEvent(5_000)
    await flushPromises()

    const connectSessions = harness.commandCalls
      .filter(call => call.command === 'lt_connect_ws')
      .map(call => call.args.sessionId)
    const sendSessions = harness.commandCalls
      .filter(call => call.command === 'lt_send_event')
      .map(call => call.args.sessionId)

    assert.deepEqual(sendSessions, connectSessions)
    assert.equal(new Set(sendSessions).size, 2)
  })

  await t.test('an initial websocket failure rolls back the joined room', async () => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => joinResponse(roomId, makeRoomState('failed'))
    harness.connectHandler = async () => { throw new Error('socket refused') }
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-failed')

    assert.equal(store.connectionState, 'disconnected')
    assert.equal(store.roomId, null)
    assert.equal(store.role, null)
    assert.equal(store.roomState, null)
    assert.match(store.sessionError, /socket refused/)
    assert.deepEqual(harness.playerActions, [])
  })

  await t.test('a websocket welcome received during join supersedes the HTTP snapshot', async () => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => joinResponse(
      roomId,
      makeRoomState('http', { version: 1, roomId: 'room-welcome' }),
    )
    harness.connectHandler = async () => {
      harness.emit('lt:message', {
        type: 'welcome',
        role: 'listener',
        state: makeRoomState('welcome', { version: 2, roomId: 'room-welcome' }),
      })
    }
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-welcome')

    assert.equal(store.roomState.track.stableKey, 'netease:welcome')
    assert.deepEqual(
      harness.playerActions.filter(action => action.type === 'play'),
      [{ type: 'play', trackId: 'netease:welcome', source: 'remote_sync' }],
    )
  })

  await t.test('a websocket welcome received after join synchronizes the player', async () => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => joinResponse(
      roomId,
      makeRoomState('http-late', { version: 1, roomId: 'room-welcome-late' }),
    )
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-welcome-late')
    harness.emit('lt:message', {
      type: 'welcome',
      role: 'listener',
      state: makeRoomState('welcome-late', { version: 2, roomId: 'room-welcome-late' }),
    })
    await flushPromises()

    assert.equal(store.roomState.track.stableKey, 'netease:welcome-late')
    assert.equal(harness.player.currentTrack.id, 'netease:welcome-late')
    assert.equal(
      harness.playerActions.filter(action => action.type === 'play').at(-1)?.trackId,
      'netease:welcome-late',
    )
  })

  await t.test('a delayed backend message from the previous session cannot update the new room', async () => {
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => joinResponse(
      roomId,
      makeRoomState(roomId === 'room-old-event' ? 'old-initial' : 'current-initial', {
        version: 1,
        roomId,
      }),
    )
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-old-event')
    const oldSessionId = harness.commandCalls
      .find(call => call.command === 'lt_connect_ws')
      .args.sessionId

    await store.joinRoom('room-current-event')
    const currentSessionId = harness.commandCalls
      .filter(call => call.command === 'lt_connect_ws')
      .at(-1)
      .args.sessionId
    const currentMessageListener = [...harness.listeners.get('lt:message')][0]

    currentMessageListener({
      payload: {
        type: 'room_state_updated',
        state: makeRoomState('queued-old', { version: 99, roomId: 'room-old-event' }),
      },
    })
    assert.equal(store.roomState.track.stableKey, 'netease:current-initial')

    currentMessageListener({
      payload: {
        sessionId: oldSessionId,
        envelope: {
          type: 'room_state_updated',
          state: makeRoomState('tagged-old', { version: 100, roomId: 'room-old-event' }),
        },
      },
    })
    assert.equal(store.roomState.track.stableKey, 'netease:current-initial')

    currentMessageListener({
      payload: {
        sessionId: currentSessionId,
        envelope: {
          type: 'room_state_updated',
          state: makeRoomState('current-update', { version: 2, roomId: 'room-current-event' }),
        },
      },
    })
    assert.equal(store.roomState.track.stableKey, 'netease:current-update')
  })

  await t.test('a superseded join response cannot replace the current room', async () => {
    const firstJoin = deferred()
    const harness = createHarness()
    harness.joinHandler = ({ roomId }) => (
      roomId === 'room-first' ? firstJoin.promise : joinResponse(roomId)
    )
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    const pendingFirst = store.joinRoom('room-first')
    await flushPromises()
    const pendingSecond = store.joinRoom('room-second')
    firstJoin.resolve(joinResponse('room-first'))
    await Promise.all([pendingFirst, pendingSecond])

    assert.equal(store.roomId, 'room-second')
    assert.deepEqual(
      harness.commandCalls
        .filter(call => call.command === 'lt_connect_ws')
        .map(call => call.args.wsUrl),
      ['ws://room-second'],
    )
  })

  await t.test('superseded room commands cannot overwrite the active backend session', async () => {
    const firstJoin = deferred()
    const harness = createHarness()
    harness.joinHandler = async ({ roomId }) => {
      const response = roomId === 'room-first'
        ? await firstJoin.promise
        : joinResponse(roomId)
      harness.backendRoomId = roomId
      return response
    }
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    const pendingFirst = store.joinRoom('room-first')
    await flushPromises()
    const pendingSecond = store.joinRoom('room-second')
    await flushPromises()
    firstJoin.resolve(joinResponse('room-first'))
    await Promise.all([pendingFirst, pendingSecond])

    assert.equal(store.roomId, 'room-second')
    assert.equal(harness.backendRoomId, 'room-second')
  })

  await t.test('a superseded room command waiting in the queue has no backend side effects', async () => {
    const firstJoin = deferred()
    const joinedRooms = []
    const harness = createHarness()
    harness.joinHandler = async ({ roomId }) => {
      joinedRooms.push(roomId)
      if (roomId === 'room-first') await firstJoin.promise
      harness.backendRoomId = roomId
      return joinResponse(roomId)
    }
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    const pendingFirst = store.joinRoom('room-first')
    await flushPromises()
    const pendingSecond = store.joinRoom('room-second')
    await flushPromises()
    const pendingThird = store.joinRoom('room-third')
    await flushPromises()
    firstJoin.resolve()
    await Promise.all([pendingFirst, pendingSecond, pendingThird])

    assert.deepEqual(joinedRooms, ['room-first', 'room-third'])
    assert.equal(store.roomId, 'room-third')
    assert.equal(harness.backendRoomId, 'room-third')
  })

  await t.test('an event callback from a superseded listener cannot connect the new room', async () => {
    const harness = createHarness()
    globalThis.__ltTestHarness = harness
    setActivePinia(createPinia())
    const store = useListenTogetherStore()

    await store.joinRoom('room-old-listener')
    const staleConnected = harness.listenerHistory.get('lt:connected')[0]
    const currentConnect = deferred()
    harness.connectHandler = ({ wsUrl }) => (
      wsUrl === 'ws://room-current' ? currentConnect.promise : undefined
    )
    const pendingCurrentJoin = store.joinRoom('room-current')
    for (let attempts = 0; attempts < 10; attempts++) {
      if (harness.commandCalls.some(call => (
        call.command === 'lt_connect_ws' && call.args.wsUrl === 'ws://room-current'
      ))) break
      await flushPromises()
    }
    staleConnected({ payload: {} })

    assert.equal(store.connectionState, 'connecting')
    currentConnect.resolve()
    await pendingCurrentJoin
  })
})
