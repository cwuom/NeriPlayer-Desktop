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
const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === 'pinia') return pinia
  if (name === 'vue') return vue
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
  await store.applySyncPayload({ entries: payload().entries, deletions: [] }, () => false)
  assertPreserved(store, [candidate.id], [], savedWrites, savedEvents)
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
    await pending
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
    if (completionOrder === 'new-first') {
      newDigest.release()
      await fresh
      oldDigest.release()
      await old
    } else {
      oldDigest.release()
      await old
      newDigest.release()
      await fresh
    }
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

console.log(`History sync regressions: ${cases - failures.length}/${cases} passed`)
assert.equal(failures.length, 0, failures.join(', '))
