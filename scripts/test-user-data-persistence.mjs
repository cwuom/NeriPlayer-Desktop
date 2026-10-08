import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/persistence/userData.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText

function loadModule(invoke) {
  const exports = {}
  new Function('require', 'exports', compiled)((name) => {
    if (name === '@tauri-apps/api/core') return { invoke }
    throw new Error(`Unexpected dependency: ${name}`)
  }, exports)
  return exports
}

function memoryStorage(entries = {}) {
  const map = new Map(Object.entries(entries))
  return {
    map,
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    removeItem: (key) => { map.delete(key) },
  }
}

const emptySnapshot = (migrated) => ({
  playbackState: null,
  history: { entries: [], deletions: [] },
  lyricOffsets: {},
  migrated,
})

{
  const { normalizeLegacyPlaybackState } = loadModule(async () => undefined)
  assert.equal(normalizeLegacyPlaybackState(null), null)
  assert.equal(normalizeLegacyPlaybackState([]), null)
  assert.deepEqual(
    normalizeLegacyPlaybackState({ queue: [{ id: 'a' }], queueIndex: -1 }),
    { queue: [{ id: 'a' }], queueIndex: -1, hasPlaybackSession: false },
    'a legacy snapshot without identity and with a negative index had no session',
  )
  assert.equal(normalizeLegacyPlaybackState({ queue: [{ id: 'a' }], queueIndex: 0 }).hasPlaybackSession, true)
  assert.equal(
    normalizeLegacyPlaybackState({ queue: [{ id: 'a' }], queueIndex: -1, currentTrackId: 'a' }).hasPlaybackSession,
    true,
  )
  assert.deepEqual(
    normalizeLegacyPlaybackState({
      queue: 'broken', queueIndex: 1.6, hasPlaybackSession: true, volume: 0.4, positionMs: 12.5,
      repeatMode: 'one', shuffleEnabled: true, currentTrackPlaylistKey: 'k',
    }),
    {
      queue: [], queueIndex: 2, hasPlaybackSession: true, currentTrackPlaylistKey: 'k',
      volume: 0.4, positionMs: 12.5, repeatMode: 'one', shuffleEnabled: true,
    },
  )
}

{
  const { readLegacyUserData } = loadModule(async () => undefined)
  const legacy = readLegacyUserData(memoryStorage({
    'neri:play-history': JSON.stringify([
      { track: { id: 'netease:1' }, playedAt: 1000.4 },
      { track: { id: 'netease:2' }, playedAt: 0 },
      { playedAt: 5 },
    ]),
    'neri.lyric-user-offsets': JSON.stringify({ 'netease:1': 250.7, 'qq:1': 0, bad: 'x' }),
  }))
  assert.equal(legacy.playbackState, null)
  assert.deepEqual(legacy.history, { entries: [{ track: { id: 'netease:1' }, playedAt: 1000 }], deletions: [] })
  assert.deepEqual(legacy.lyricOffsets, { 'netease:1': 251 })
  const nothing = readLegacyUserData(memoryStorage())
  assert.deepEqual(nothing, { playbackState: null, history: null, lyricOffsets: null })
}

{
  const calls = []
  const storage = memoryStorage({
    'neri:player-state': JSON.stringify({ queue: [{ id: 'a' }], queueIndex: 0, hasPlaybackSession: true }),
    'neri:play-history': JSON.stringify([{ track: { id: 'netease:1' }, playedAt: 10 }]),
    'neri:play-history-deletions': '[]',
    'neri.lyric-user-offsets': '{}',
  })
  const migrated = { playbackState: true, history: true, lyricOffsets: true }
  const module = loadModule(async (command, args) => {
    calls.push([command, args])
    if (command === 'load_user_data_snapshot') {
      return emptySnapshot({ playbackState: false, history: false, lyricOffsets: false })
    }
    if (command === 'import_legacy_user_data') return { ...emptySnapshot(migrated), playbackState: args.playbackState }
    throw new Error(`unexpected command ${command}`)
  })
  assert.equal(await module.preloadUserData(storage), true)
  assert.deepEqual(calls.map(([command]) => command), ['load_user_data_snapshot', 'import_legacy_user_data'])
  assert.equal(calls[1][1].history.entries[0].track.id, 'netease:1')
  assert.equal(module.preloadedUserData().playbackState.queueIndex, 0)
  assert.equal(storage.getItem('neri:play-history'), null, 'migrated history keys are removed')
  assert.equal(storage.getItem('neri.lyric-user-offsets'), null)
  assert.notEqual(storage.getItem('neri:player-state'), null, 'the player snapshot waits for the first database write')
  module.finishLegacyPlayerStateCleanup(storage)
  assert.equal(storage.getItem('neri:player-state'), null)
}

{
  const calls = []
  const module = loadModule(async (command, args) => {
    calls.push([command, args])
    return emptySnapshot({ playbackState: true, history: true, lyricOffsets: true })
  })
  const storage = memoryStorage({ 'neri:play-history': '[]' })
  assert.equal(await module.preloadUserData(storage), true)
  assert.deepEqual(calls.map(([command]) => command), ['load_user_data_snapshot'], 'migrated domains never re-import')
  assert.equal(storage.getItem('neri:play-history'), null, 'stale legacy keys are cleaned up')
}

{
  const module = loadModule(async () => { throw new Error('no tauri bridge') })
  const originalWarn = console.warn
  console.warn = () => {}
  try {
    assert.equal(await module.preloadUserData(memoryStorage()), false)
  } finally {
    console.warn = originalWarn
  }
  assert.equal(module.preloadedUserData(), null)
  assert.equal(module.hasUserDataBackend(), false)
}

{
  const order = []
  const releases = []
  const module = loadModule((command) => new Promise((resolve, reject) => {
    releases.push(() => {
      order.push(command)
      if (command === 'fails') reject(new Error('fixture'))
      else resolve(command)
    })
  }))
  const first = module.persistUserData('first', {})
  const failing = module.persistUserData('fails', {}).catch(() => 'caught')
  const third = module.persistUserData('third', {})
  await Promise.resolve()
  assert.equal(releases.length, 1, 'later writes wait for earlier ones')
  releases[0]()
  assert.equal(await first, 'first')
  await new Promise(resolve => setTimeout(resolve, 0))
  releases[1]()
  assert.equal(await failing, 'caught')
  await new Promise(resolve => setTimeout(resolve, 0))
  releases[2]()
  assert.equal(await third, 'third', 'a failed write does not block the queue')
  assert.deepEqual(order, ['first', 'fails', 'third'])
}

console.log('user data persistence tests passed')
