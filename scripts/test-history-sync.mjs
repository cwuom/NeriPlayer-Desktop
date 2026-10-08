import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

const source = await readFile(new URL('../src/stores/history.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const persisted = []
let preloaded = null
const userData = {
  LEGACY_HISTORY_KEY: 'neri:play-history',
  LEGACY_HISTORY_DELETIONS_KEY: 'neri:play-history-deletions',
  preloadedUserData: () => preloaded,
  // Tauri invoke 以 JSON 序列化参数，响应式代理同样能序列化
  persistUserData: async (command, args) => { persisted.push([command, JSON.parse(JSON.stringify(args))]) },
}
const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === 'pinia') return pinia
  if (name === 'vue') return vue
  if (name === '@/modules/persistence/userData') return userData
  if (name === '@/utils/logger') return { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) }
  throw new Error(`Unexpected history dependency: ${name}`)
}, exports)

const storage = new Map()
let writes = 0
const events = []
let now = 1_000
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => { writes++; storage.set(key, value) },
}
globalThis.window = new EventTarget()
if (!globalThis.CustomEvent) {
  globalThis.CustomEvent = class extends Event {
    constructor(type, options) { super(type); this.detail = options.detail }
  }
}
window.addEventListener(exports.HISTORY_CHANGED_EVENT, event => events.push(event.detail.type))
Date.now = () => now

const digestQueue = []
function digestBytes(value) {
  const bytes = createHash('sha256').update(value).digest()
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
}
Object.defineProperty(globalThis, 'crypto', {
  configurable: true,
  value: {
    subtle: {
      async digest(algorithm, bytes) {
        assert.equal(algorithm, 'SHA-256')
        const digest = digestBytes(bytes)
        const waiting = digestQueue.shift()
        if (!waiting) return digest
        waiting.used = true
        waiting.digest = digest
        return waiting.promise
      },
    },
  },
})
function delayDigest() {
  let resolve
  const waiting = { used: false, promise: new Promise(accept => { resolve = accept }) }
  waiting.release = () => resolve(waiting.digest)
  digestQueue.push(waiting)
  return waiting
}

function track(id, source = 'netease') {
  return {
    id, source, title: id, artist: 'fixture', album: source === 'youtube' ? 'YouTube' : 'netease',
    durationMs: 1_000, coverUrl: '', audioUrl: '', addedAt: 0,
  }
}
const videoId = 'abcdefghijk'
const candidate = track(`youtube:${videoId}`, 'youtube')
const signedId = new DataView(digestBytes(videoId)).getBigInt64(0)
const deletion = {
  songId: String(signedId === 0n ? 1n : signedId), album: 'YouTube',
  mediaUri: `ytmusic://video/${videoId}`, deletedAt: 2_000,
}
function payload(id = 'netease:old-cloud') {
  return { entries: [{ track: track(id), playedAt: 1_500 }], deletions: [deletion] }
}
function storedIds(key) {
  return JSON.parse(storage.get(key)).map(item => item.track.id)
}
function assertPreserved(store, entryIds, deletionIds, savedWrites, savedEvents) {
  assert.deepEqual(store.entries.map(item => item.track.id), entryIds)
  assert.deepEqual(store.deletions.map(item => item.track.id), deletionIds)
  assert.deepEqual(storedIds('neri:play-history'), entryIds)
  assert.deepEqual(storedIds('neri:play-history-deletions'), deletionIds)
  assert.equal(writes, savedWrites, 'stale payload must not save')
  assert.deepEqual(events, savedEvents, 'stale payload must not dispatch sync')
}

const failures = []
let cases = 0
async function regression(name, run) {
  cases++
  storage.clear()
  writes = 0
  events.length = 0
  digestQueue.length = 0
  now = 1_000
  pinia.setActivePinia(pinia.createPinia())
  const store = exports.useHistoryStore()
  store.record(candidate)
  try {
    await run(store)
    await vue.nextTick()
  } catch (error) {
    failures.push(name)
    console.error(`FAIL ${name}: ${error.message}`)
  } finally {
    store.$dispose()
  }
}

await regression('already cancelled payload has no history side effects', async store => {
  const savedWrites = writes
  const savedEvents = [...events]
  assert.equal(await store.applySyncPayload({ entries: payload().entries, deletions: [] }, () => false), 'skipped')
  assertPreserved(store, [candidate.id], [], savedWrites, savedEvents)
})

for (const mutation of ['record', 'remove', 'clear']) {
  await regression(`local ${mutation} after the sync snapshot defers the merged payload`, async store => {
    const snapshotEpoch = store.syncSnapshotEpoch()
    now = 3_000
    if (mutation === 'record') store.record(track('netease:new-local'))
    else if (mutation === 'remove') store.remove(candidate.id)
    else store.clear()
    const savedWrites = writes
    const savedEvents = [...events]
    assert.equal(await store.applySyncPayload(payload(), () => true, snapshotEpoch), 'deferred')
    assertPreserved(
      store,
      mutation === 'record' ? ['netease:new-local', candidate.id] : [],
      mutation === 'record' ? [] : [candidate.id],
      savedWrites,
      savedEvents,
    )
  })
}

await regression('an unchanged snapshot epoch applies the merged payload', async store => {
  const snapshotEpoch = store.syncSnapshotEpoch()
  assert.equal(await store.applySyncPayload(payload(), () => true, snapshotEpoch), 'applied')
  assert.deepEqual(store.entries.map(item => item.track.id), ['netease:old-cloud'])
  assert.equal(store.syncSnapshotEpoch(), snapshotEpoch, 'applying a sync result is not a local edit')
})

await regression('provider disconnect during identity matching cannot commit old history', async store => {
  let providerGeneration = 0
  const requestedGeneration = providerGeneration
  const waiting = delayDigest()
  const pending = store.applySyncPayload(payload(), () => providerGeneration === requestedGeneration)
  assert.ok(waiting.used)
  const savedWrites = writes
  const savedEvents = [...events]
  providerGeneration++
  waiting.release()
  await pending
  assertPreserved(store, [candidate.id], [], savedWrites, savedEvents)
})

for (const mutation of ['record', 'remove', 'clear']) {
  await regression(`local ${mutation} during identity matching survives old payload`, async store => {
    const waiting = delayDigest()
    const pending = store.applySyncPayload(payload())
    assert.ok(waiting.used)
    now = 3_000
    if (mutation === 'record') store.record(track('netease:new-local'))
    else if (mutation === 'remove') store.remove(candidate.id)
    else store.clear()
    const savedWrites = writes
    const savedEvents = [...events]
    waiting.release()
    assert.equal(await pending, 'deferred', 'a local edit during matching must ask for a follow-up sync')
    assertPreserved(
      store,
      mutation === 'record' ? ['netease:new-local', candidate.id] : [],
      mutation === 'record' ? [] : [candidate.id],
      savedWrites,
      savedEvents,
    )
  })
}

await regression('clearing empty history invalidates pending incoming entries', async store => {
  store.remove(candidate.id)
  const waiting = delayDigest()
  const pending = store.applySyncPayload(payload())
  assert.ok(waiting.used)
  store.clear()
  const savedWrites = writes
  const savedEvents = [...events]
  waiting.release()
  await pending
  assertPreserved(store, [], [candidate.id], savedWrites, savedEvents)
})

for (const completionOrder of ['new-first', 'old-first']) {
  await regression(`latest payload wins when completion is ${completionOrder}`, async store => {
    const oldDigest = delayDigest()
    const old = store.applySyncPayload(payload('netease:old-cloud'))
    const newDigest = delayDigest()
    const fresh = store.applySyncPayload(payload('netease:new-cloud'))
    assert.ok(oldDigest.used && newDigest.used)
    const outcomes = {}
    if (completionOrder === 'new-first') {
      newDigest.release()
      outcomes.fresh = await fresh
      oldDigest.release()
      outcomes.old = await old
    } else {
      oldDigest.release()
      outcomes.old = await old
      newDigest.release()
      outcomes.fresh = await fresh
    }
    assert.deepEqual(outcomes, { old: 'skipped', fresh: 'applied' }, 'a superseded payload needs no follow-up sync')
    assert.deepEqual(store.entries.map(item => item.track.id), ['netease:new-cloud'])
    assert.deepEqual(store.deletions.map(item => item.track.id), [candidate.id])
    assert.deepEqual(storedIds('neri:play-history'), ['netease:new-cloud'])
    assert.deepEqual(events, ['record', 'sync'])
    assert.equal(writes, 4, 'only the latest payload saves after the initial record')
  })
}

await regression('normal payload resolves real hashed identity and saves both sections', async store => {
  await store.applySyncPayload({
    entries: [
      { track: track('netease:older'), playedAt: 1_500 },
      { track: track('netease:newer'), played_at: 2_500 },
    ],
    deletions: [deletion],
  })
  assert.deepEqual(store.entries.map(item => [item.track.id, item.playedAt]), [
    ['netease:newer', 2_500], ['netease:older', 1_500],
  ])
  assert.deepEqual(store.deletions.map(item => [item.track.id, item.deletedAt]), [[candidate.id, 2_000]])
  assert.deepEqual(storedIds('neri:play-history'), ['netease:newer', 'netease:older'])
  assert.deepEqual(storedIds('neri:play-history-deletions'), [candidate.id])
  assert.equal(writes, 4)
  assert.deepEqual(events, ['record', 'sync'])
})

async function databaseRegression(name, run) {
  cases++
  storage.clear()
  writes = 0
  events.length = 0
  persisted.length = 0
  digestQueue.length = 0
  now = 1_000
  preloaded = {
    history: { entries: [{ track: track('netease:stored'), playedAt: 500 }], deletions: [] },
  }
  pinia.setActivePinia(pinia.createPinia())
  const store = exports.useHistoryStore()
  try {
    await run(store)
  } catch (error) {
    failures.push(name)
    console.error(`FAIL ${name}: ${error.message}`)
  } finally {
    store.$dispose()
    preloaded = null
  }
}

await databaseRegression('database mode restores the preloaded history and persists each mutation', async store => {
  assert.deepEqual(store.entries.map(item => item.track.id), ['netease:stored'])
  store.record(candidate)
  now = 2_000
  store.remove('netease:stored')
  now = 3_000
  store.clear()
  assert.equal(writes, 0, 'database mode must not write localStorage')
  assert.deepEqual(persisted.map(([command]) => command), [
    'record_play_history', 'remove_play_history', 'clear_play_history',
  ])
  assert.equal(persisted[0][1].track.id, candidate.id)
  assert.equal(persisted[0][1].playedAt, 1_000)
  assert.deepEqual(persisted[1][1], { identityKey: 'netease:stored', deletedAt: 2_000 })
  assert.deepEqual(persisted[2][1], { deletedAt: 3_000 })
})

function page(cid) {
  return { ...track('bilibili:BV1xx', 'bilibili'), album: `Bilibili|${cid}`, title: `P${cid}` }
}

await databaseRegression('bilibili pages of one video are separate history entries', async store => {
  now = 2_000
  store.record(page(1))
  now = 3_000
  store.record(page(2))
  now = 4_000
  store.record(page(1))
  assert.deepEqual(store.entries.map(item => item.track.title), ['P1', 'P2', 'netease:stored'])
  assert.equal(exports.historyEntryKey(page(2)), 'bilibili:BV1xx#2')
  assert.equal(exports.historyEntryKey({ ...page(0), album: '', syncPayload: { subAudioId: 9 } }), 'bilibili:BV1xx#9')
  assert.equal(exports.historyEntryKey({ ...track('netease:1'), album: 'Bilibili|5' }), 'netease:1')
  persisted.length = 0
  store.remove(exports.historyEntryKey(page(2)))
  assert.deepEqual(store.entries.map(item => item.track.title), ['P1', 'netease:stored'])
  assert.deepEqual(store.deletions.map(item => item.track.title), ['P2'])
  assert.deepEqual(persisted, [['remove_play_history', { identityKey: 'bilibili:BV1xx#2', deletedAt: 4_000 }]])
})

await databaseRegression('history is not capped and clearing keeps every tombstone', async store => {
  for (let index = 0; index < 1_502; index++) {
    now = 2_000 + index
    store.record(track(`netease:${index}`))
  }
  assert.equal(store.entries.length, 1_503)
  store.clear()
  assert.equal(store.entries.length, 0)
  assert.equal(store.deletions.length, 1_503)
})

await databaseRegression('a sync result keeps one entry per local identity', async store => {
  await store.applySyncPayload({
    entries: [
      { track: { ...track('netease:dup'), album: 'old album' }, playedAt: 1_000 },
      { track: track('netease:dup'), playedAt: 2_000 },
      { track: page(1), playedAt: 1_500 },
      { track: page(2), playedAt: 1_200 },
    ],
    deletions: [],
  })
  assert.deepEqual(store.entries.map(item => [item.track.id, item.track.album]), [
    ['netease:dup', 'netease'], ['bilibili:BV1xx', 'Bilibili|1'], ['bilibili:BV1xx', 'Bilibili|2'],
  ])
  const [command, args] = persisted.at(-1)
  assert.equal(command, 'replace_play_history')
  assert.equal(args.history.entries.length, 3)
})

await databaseRegression('database mode replaces history only for the current sync payload', async store => {
  await store.applySyncPayload(payload(), () => false)
  assert.equal(persisted.length, 0, 'a cancelled payload must not persist')
  await store.applySyncPayload({ entries: [{ track: track('netease:cloud'), playedAt: 1_500.4 }], deletions: [] })
  assert.equal(persisted.length, 1)
  const [command, args] = persisted[0]
  assert.equal(command, 'replace_play_history')
  assert.deepEqual(args.history.entries.map(item => [item.track.id, item.playedAt]), [['netease:cloud', 1_500]])
})

await databaseRegression('long-form positions survive replays and only change when they move', async store => {
  const episode = track('netease:episode')
  now = 2_000
  store.updateResumePosition(episode, 600_000)
  assert.equal(store.rememberedPosition(episode), 600_000)
  assert.equal(store.entries[0].track.id, 'netease:episode')
  assert.ok(events.includes('progress'))
  now = 3_000
  store.record(track('netease:other'))
  now = 4_000
  store.record(episode)
  assert.equal(store.rememberedPosition(episode), 600_000, 'replaying keeps the remembered position')
  assert.equal(store.entries[0].playedAt, 4_000)
  persisted.length = 0
  store.updateResumePosition(episode, 600_000)
  assert.equal(persisted.length, 0, 'an unchanged position is not rewritten')
  now = 5_000
  store.updateResumePosition(episode, 0)
  assert.deepEqual(persisted, [['record_play_history', { track: episode, playedAt: 5_000, resumePositionMs: 0 }]])
  store.updateResumePosition(track('netease:never-played'), 0)
  assert.ok(!store.entries.some(item => item.track.id === 'netease:never-played'), 'clearing a position never adds an entry')
  const snapshot = store.getSyncSnapshot()
  assert.equal(snapshot.entries.find(item => item.track.id === 'netease:stored').resumePositionMs, null,
    'an entry from before the upgrade stays unknown so sync keeps the archived position')
  assert.equal(snapshot.entries.find(item => item.track.id === 'netease:episode').resumePositionMs, 0)
})

await databaseRegression('sync results carry the merged resume position', async store => {
  await store.applySyncPayload({
    entries: [{ track: track('netease:episode'), playedAt: 1_500, resumePositionMs: 1_234 }],
    deletions: [],
  })
  assert.equal(store.rememberedPosition(track('netease:episode')), 1_234)
  const [, args] = persisted.at(-1)
  assert.equal(args.history.entries[0].resumePositionMs, 1_234)
})

console.log(`History sync regressions: ${cases - failures.length}/${cases} passed`)
assert.equal(failures.length, 0, failures.join(', '))
