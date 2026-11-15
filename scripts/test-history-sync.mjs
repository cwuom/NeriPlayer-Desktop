import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

async function loadModule(path, resolve = name => { throw new Error(`Unexpected dependency: ${name}`) }) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const result = {}
  new Function('require', 'exports', compiled)(resolve, result)
  return result
}

const persisted = []
const detailsRequests = []
let preloaded = null
const userData = {
  LEGACY_HISTORY_KEY: 'neri:play-history',
  LEGACY_HISTORY_DELETIONS_KEY: 'neri:play-history-deletions',
  preloadedUserData: () => preloaded,
  // Tauri invoke 以 JSON 序列化参数，响应式代理同样能序列化
  persistUserData: async (command, args) => { persisted.push([command, JSON.parse(JSON.stringify(args))]) },
}
const biliVideoSkip = await loadModule('../src/modules/playback/biliVideoSkip.ts')
const coverCache = await loadModule('../src/utils/bilibiliCoverCache.ts')
const logger = { createLogger: () => ({ info() {}, warn() {}, error() {}, debug() {} }) }
const biliVideoParts = await loadModule('../src/modules/playback/biliVideoParts.ts', name => {
  if (name.endsWith('biliVideoSkip')) return biliVideoSkip
  if (name.endsWith('bilibiliCoverCache')) return coverCache
  if (name === '@/utils/logger') return logger
  throw new Error(`Unexpected Bilibili parts dependency: ${name}`)
})
const exports = await loadModule('../src/stores/history.ts', name => {
  if (name === 'pinia') return pinia
  if (name === 'vue') return vue
  if (name === '@tauri-apps/api/core') return { invoke: (command, args) => new Promise((resolve, reject) => {
    detailsRequests.push({ command, args, resolve, reject })
  }) }
  if (name === '@/modules/persistence/userData') return userData
  if (name === '@/modules/playback/biliVideoParts') return biliVideoParts
  if (name === '@/utils/bilibiliCoverCache') return coverCache
  if (name === '@/utils/logger') return logger
  throw new Error(`Unexpected history dependency: ${name}`)
})

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
let digestCalls = 0
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
        digestCalls++
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
  digestCalls = 0
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
  detailsRequests.length = 0
  digestQueue.length = 0
  digestCalls = 0
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

const latestBiliDetail = {
  bvid: 'BV14Dn16tELC', aid: 114, title: '音乐现场', owner: '原上传者', cover: 'https://i0.hdslb.com/bfs/archive/parent.jpg',
  cid: 1001, duration: 86,
  pages: [
    { cid: 1001, page: 1, part: '01.曲目一 - 歌手一', duration: 51 },
    { cid: 1002, page: 2, part: '02.曲目二', duration: 35 },
  ],
}

function missingBiliPage(cid, overrides = {}) {
  return {
    id: 'bilibili:114', title: '', artist: '', album: `Bilibili|${cid}`, durationMs: 0, coverUrl: '', audioUrl: '',
    source: 'bilibili', syncPayload: { channelId: 'bilibili', audioId: '114', subAudioId: String(cid) },
    ...overrides,
  }
}

await databaseRegression('old Bilibili history restores each exact part using one lookup and keeps edits and progress', async store => {
  await store.applySyncPayload({ entries: [
    { track: missingBiliPage(1001), playedAt: 90, resumePositionMs: 3100 },
    { track: missingBiliPage(1002, { title: '自己的标题', album: 'Bilibili|1002|BV14Dn16tELC' }), playedAt: 80 },
    { track: missingBiliPage(1003), playedAt: 70 },
  ] })
  persisted.length = 0

  const first = store.restoreBiliMetadata(store.entries.slice(0, 1))
  const rest = store.restoreBiliMetadata(store.entries.slice(1))
  await Promise.resolve()
  assert.equal(detailsRequests.length, 1)
  assert.equal(detailsRequests[0].command, 'get_bili_video_info')
  assert.deepEqual(detailsRequests[0].args, { bvid: null, avid: 114 })
  detailsRequests[0].resolve(latestBiliDetail)
  await first
  // 给未去重版本的串行请求返回数据，确保断言命中重复请求而不是卡在等待
  detailsRequests[1]?.resolve(latestBiliDetail)
  await rest
  assert.equal(detailsRequests.length, 1)

  assert.deepEqual(store.entries.map(({ track, playedAt, resumePositionMs }) => [
    track.id, track.title, track.artist, track.coverUrl, track.durationMs, track.album, playedAt, resumePositionMs,
  ]), [
    ['bilibili:114', '曲目一', '歌手一', latestBiliDetail.cover, 51_000, 'Bilibili|1001', 90, 3100],
    ['bilibili:114', '自己的标题', '原上传者', latestBiliDetail.cover, 35_000, 'Bilibili|1002|BV14Dn16tELC', 80, undefined],
    ['bilibili:114', '', '', '', 0, 'Bilibili|1003', 70, undefined],
  ])
  assert.ok(persisted.every(([command]) => command === 'update_play_history_metadata'))
  assert.deepEqual(persisted.flatMap(([, args]) => args.entries.map(entry => entry.playedAt)), [90, 80])

  await store.applySyncPayload({ entries: [
    { track: missingBiliPage(1001, { title: '远端的新标题' }), playedAt: 100 },
  ] })
  assert.equal(store.entries[0].track.title, '远端的新标题')
  assert.equal(store.entries[0].track.artist, '歌手一')
  assert.equal(store.entries[0].track.coverUrl, latestBiliDetail.cover)
  assert.equal(store.entries[0].track.durationMs, 51_000)
})

await databaseRegression('Bilibili history restoration follows the saved artist-first title order', async store => {
  const key = `neri:bili-part-name-order:v1:${latestBiliDetail.bvid}`
  storage.set(key, 'artist-first')
  try {
    store.record(missingBiliPage(1001))
    const repair = store.restoreBiliMetadata(store.entries)
    await Promise.resolve()
    detailsRequests[0].resolve({
      ...latestBiliDetail,
      pages: [{ ...latestBiliDetail.pages[0], part: '01. 歌手一 - 曲目一' }, latestBiliDetail.pages[1]],
    })
    await repair
    assert.equal(store.entries[0].track.title, '曲目一')
    assert.equal(store.entries[0].track.artist, '歌手一')
  } finally {
    storage.delete(key)
  }
})

for (const album of ['Bilibili|1002', 'bilibili']) {
  await databaseRegression(`Bilibili part tombstones with album ${album} match BV-enriched history`, async store => {
    const otherPage = missingBiliPage(1001)
    const deletedPage = missingBiliPage(1002, {
      id: 'bilibili:114:1002',
      album: 'Bilibili|1002|BV14Dn16tELC',
    })
    const [deleted, survivor] = [deletedPage, otherPage].map(track => ({ track, playedAt: 100 }))
    store.entries = [deleted, survivor]
    const signed = new DataView(digestBytes('bilibili|114|1002')).getBigInt64(0)
    await store.applySyncPayload({
      entries: [survivor],
      deletions: [{ songId: String(signed), album, deletedAt: 200 }],
    })
    assert.equal(store.deletions[0]?.track.album, 'Bilibili|1002|BV14Dn16tELC')
    assert.deepEqual(store.entries.map(entry => entry.track.album), ['Bilibili|1001'])
  })
}

await databaseRegression('late Bilibili metadata does not recreate a deleted history entry', async store => {
  now = 2_000
  store.record(missingBiliPage(1001))
  const repair = store.restoreBiliMetadata(store.entries)
  await Promise.resolve()
  store.remove('bilibili:114#1001')
  persisted.length = 0
  detailsRequests[0].resolve(latestBiliDetail)
  await repair
  assert.deepEqual(store.entries.map(entry => entry.track.id), ['netease:stored'])
  assert.equal(store.deletions.length, 1)
  assert.equal(persisted.length, 0)
})

await databaseRegression('opening history after a failed Bilibili lookup can retry recovery', async store => {
  store.record(missingBiliPage(1002))
  const first = store.restoreBiliMetadata(store.entries)
  await Promise.resolve()
  detailsRequests[0].reject(new Error('temporarily unavailable'))
  await first
  const retry = store.restoreBiliMetadata(store.entries)
  await Promise.resolve()
  assert.equal(detailsRequests.length, 2)
  detailsRequests[1].resolve(latestBiliDetail)
  await retry
  assert.equal(store.entries[0].track.title, '曲目二')
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

await databaseRegression('many remote tombstones hash each candidate once despite repeated snapshot tracks', async store => {
  const tracks = [...Array.from({ length: 24 }, (_, index) => track(`youtube:fixture-${index}`, 'youtube')), candidate]
  store.entries = tracks.map(item => ({ track: item, playedAt: 1_000 }))
  const tombstones = Array.from({ length: 50 }, (_, index) => ({ songId: `unknown-${index}`, deletedAt: 3_000 }))
  assert.equal(await store.applySyncPayload({
    entries: tracks.map(item => ({ track: item, playedAt: 1_500 })),
    deletions: [...tombstones, deletion],
  }), 'applied')
  assert.deepEqual(store.entries.map(entry => entry.track.id), tracks.slice(0, -1).map(item => item.id))
  assert.deepEqual(store.deletions.map(item => [item.track.id, item.deletedAt]), [[candidate.id, 2_000]])
  assert.ok(digestCalls <= tracks.length, `expected at most ${tracks.length} hashes, received ${digestCalls}`)
})

await databaseRegression('a tombstone matching the first candidate does not hash later tracks', async store => {
  store.entries = [candidate, ...Array.from({ length: 24 }, (_, index) => track(`youtube:later-${index}`, 'youtube'))]
    .map(item => ({ track: item, playedAt: 1_000 }))
  await store.applySyncPayload({ entries: [], deletions: [deletion] })
  assert.deepEqual(store.deletions.map(item => item.track.id), [candidate.id])
  assert.equal(digestCalls, 1, 'early matches keep their existing fast path')
})

for (const [album, title] of [['', 'first album'], ['first album', 'first album'], ['second album', 'second album']]) {
  await databaseRegression(`indexed tombstones preserve candidate priority for album ${album || 'unspecified'}`, async store => {
    store.entries = ['first album', 'second album'].map(name => ({
      track: { ...candidate, album: name, title: name }, playedAt: 1_000,
    }))
    await store.applySyncPayload({
      entries: [{ track: { ...candidate, album: 'first album', title: 'incoming title' }, playedAt: 1_500 }],
      deletions: [{ ...deletion, album }],
    })
    assert.deepEqual(store.entries, [])
    assert.deepEqual(store.deletions.map(item => item.track.title), [title])
  })
}

for (const [mediaUri, source] of [['', 'netease'], [deletion.mediaUri, 'youtube']]) {
  await databaseRegression(`indexed tombstones preserve optional media URI matching for ${source}`, async store => {
    store.entries = [
      { track: { ...track(`netease:${deletion.songId}`), album: candidate.album }, playedAt: 1_000 },
      { track: candidate, playedAt: 1_000 },
    ]
    await store.applySyncPayload({ entries: [], deletions: [{ ...deletion, mediaUri }] })
    assert.deepEqual(store.deletions.map(item => item.track.source), [source])
  })
}

await databaseRegression('indexed tombstones retain QQ identity and Bilibili part boundaries', async store => {
  const songs = [track('qq:qq-song', 'qq'), page(1), page(2)]
  store.entries = songs.map(item => ({ track: item, playedAt: 1_000 }))
  await store.applySyncPayload({
    entries: songs.map(item => ({ track: item, playedAt: 1_500 })),
    deletions: [
      { songId: '-6323550006513068467', deletedAt: 3_000 },
      { songId: '3409221603982616090', album: 'Bilibili|2', deletedAt: 2_000 },
    ],
  })
  assert.deepEqual(store.entries.map(item => item.track.title), ['P1'])
  assert.deepEqual(store.deletions.map(item => [item.track.id, item.track.album]), [
    ['qq:qq-song', 'netease'], ['bilibili:BV1xx', 'Bilibili|2'],
  ])
  assert.equal(digestCalls, 3)
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

await databaseRegression('sync snapshots exclude local entries and tombstones but keep downloaded online tracks', async store => {
  const downloaded = { ...track('netease:downloaded'), audioUrl: 'E:/Music/downloaded.flac' }
  store.record(downloaded)
  store.record(track('local:file', 'local'))
  const legacyLocal = { ...track('legacy-file'), source: undefined }
  store.record(legacyLocal)
  now = 2_000
  store.remove(legacyLocal.id)
  store.record(track('local:deleted', 'local'))
  store.remove('local:deleted')
  store.record(track('netease:deleted'))
  store.remove('netease:deleted')
  const snapshot = store.getSyncSnapshot()
  assert.deepEqual(snapshot.entries.map(entry => entry.track.id), ['netease:downloaded', 'netease:stored'])
  assert.deepEqual(snapshot.deletions.map(entry => entry.track.id), ['netease:deleted'])
  assert.ok(store.entries.some(entry => entry.track.id === 'local:file'), 'filtering a snapshot keeps local history')
  assert.equal(store.deletions.length, 3, 'local tombstones remain in the local database')
})

await databaseRegression('sync restore preserves local positions and tombstones and ignores incoming local files', async store => {
  const local = track('local:episode', 'local')
  now = 2_000
  store.updateResumePosition(local, 42_000)
  now = 3_000
  store.record(track('local:removed', 'local'))
  store.remove('local:removed')
  persisted.length = 0
  assert.equal(await store.applySyncPayload({
    entries: [
      { track: track('netease:remote-new'), playedAt: 4_000 },
      { track: track('netease:remote-old'), playedAt: 1_000 },
      { track: { ...local, audioUrl: 'E:/OtherDevice/episode.flac' }, playedAt: 9_000, resumePositionMs: 1 },
      { track: track('local:removed', 'local'), playedAt: 8_000 },
      { track: { ...track('foreign-file'), source: undefined }, playedAt: 7_000 },
    ],
    deletions: [],
  }), 'applied')
  assert.deepEqual(store.entries.map(entry => [entry.track.id, entry.playedAt]), [
    ['netease:remote-new', 4_000], ['local:episode', 2_000], ['netease:remote-old', 1_000],
  ])
  assert.equal(store.rememberedPosition(local), 42_000)
  assert.deepEqual(store.deletions.map(entry => [entry.track.id, entry.deletedAt]), [['local:removed', 3_000]])
  assert.equal(persisted.length, 1)
  assert.equal(persisted[0][0], 'replace_play_history')
  assert.deepEqual(persisted[0][1].history.entries.map(entry => entry.track.id), store.entries.map(entry => entry.track.id))
  assert.deepEqual(persisted[0][1].history.deletions.map(entry => entry.track.id), ['local:removed'])
  assert.equal(store.getSyncSnapshot().entries.length, 2)
  assert.deepEqual(store.getSyncSnapshot().deletions, [])
})

await databaseRegression('remote tombstones cannot match a local file with the same numeric identity', async store => {
  const local = { ...track('local:7', 'local'), album: 'shared album' }
  const online = { ...track('netease:7'), album: 'shared album' }
  now = 2_000
  store.record(local)
  await store.applySyncPayload({
    entries: [{ track: online, playedAt: 1_500 }],
    deletions: [{ songId: '7', album: 'shared album', deletedAt: 3_000 }],
  })
  assert.deepEqual(store.entries.map(entry => entry.track.id), ['local:7'])
  assert.deepEqual(store.deletions.map(entry => entry.track.id), ['netease:7'])
})

await databaseRegression('clear keeps local tombstones without uploading or resurrecting local history', async store => {
  const local = track('local:cleared', 'local')
  now = 2_000
  store.record(local)
  now = 3_000
  store.clear()
  const snapshotEpoch = store.syncSnapshotEpoch()
  const snapshot = store.getSyncSnapshot()
  assert.deepEqual(snapshot.entries, [])
  assert.deepEqual(snapshot.deletions.map(entry => entry.track.id), ['netease:stored'])
  await store.applySyncPayload({
    entries: [
      { track: local, playedAt: 9_000 },
      { track: track('netease:stored'), playedAt: 500 },
    ],
    deletions: [{ songId: 'stored', album: 'netease', deletedAt: 3_000 }],
  }, () => true, snapshotEpoch)
  assert.deepEqual(store.entries, [])
  assert.deepEqual(store.deletions.map(entry => entry.track.id), ['netease:stored', 'local:cleared'])
  const saved = persisted.at(-1)[1].history
  assert.deepEqual(saved.entries, [])
  assert.deepEqual(saved.deletions.map(entry => entry.track.id), ['netease:stored', 'local:cleared'])
  now = 4_000
  store.record(local)
  await store.applySyncPayload({ entries: [], deletions: [] })
  assert.deepEqual(store.entries.map(entry => entry.track.id), ['local:cleared'])
  assert.deepEqual(store.deletions, [], 'a local replay removes its older tombstone')
})

await databaseRegression('sync merge sorts and deduplicates preserved local records and reconciles tombstone times', async store => {
  const newer = track('local:newer', 'local')
  const removed = track('local:removed', 'local')
  store.entries = [
    { track: newer, playedAt: 1_000, resumePositionMs: 1 },
    { track: removed, playedAt: 2_000 },
    { track: newer, playedAt: 4_000, resumePositionMs: 42 },
  ]
  store.deletions = [
    { track: removed, deletedAt: 1_000 },
    { track: newer, deletedAt: 3_000 },
    { track: removed, deletedAt: 2_000 },
  ]
  await store.applySyncPayload({ entries: [{ track: track('netease:cloud'), playedAt: 3_500 }], deletions: [] })
  assert.deepEqual(store.entries.map(entry => [entry.track.id, entry.playedAt]), [
    ['local:newer', 4_000], ['netease:cloud', 3_500],
  ])
  assert.equal(store.rememberedPosition(newer), 42)
  assert.deepEqual(store.deletions.map(entry => [entry.track.id, entry.deletedAt]), [['local:removed', 2_000]])
})

await databaseRegression('a newer remote replay removes an older tombstone and equal timestamps favor deletion', async store => {
  now = 2_000
  store.remove('netease:stored')
  const deleted = { songId: 'stored', album: 'netease', deletedAt: 2_000 }
  await store.applySyncPayload({
    entries: [{ track: track('netease:stored'), playedAt: 3_000, resumePositionMs: 42 }],
    deletions: [deleted],
  })
  assert.deepEqual(store.entries.map(entry => entry.track.id), ['netease:stored'])
  assert.deepEqual(store.deletions, [])
  assert.equal(store.rememberedPosition(track('netease:stored')), 42)
  await store.applySyncPayload({
    entries: [{ track: track('netease:stored'), playedAt: 2_000 }],
    deletions: [deleted],
  })
  assert.deepEqual(store.entries, [])
  assert.deepEqual(store.deletions.map(entry => entry.track.id), ['netease:stored'])
})

for (const mutation of ['record', 'remove', 'clear']) {
  await regression(`local-file ${mutation} during sync matching defers the payload and preserves the edit`, async store => {
    const local = track('local:pending', 'local')
    now = 2_000
    store.updateResumePosition(local, 42_000)
    const waiting = delayDigest()
    const pending = store.applySyncPayload(payload())
    await new Promise(resolve => setImmediate(resolve))
    assert.ok(waiting.used)
    now = 3_000
    if (mutation === 'record') store.record(track('local:new', 'local'))
    else if (mutation === 'remove') store.remove(local.id)
    else store.clear()
    const entryIds = store.entries.map(entry => entry.track.id)
    const deletionIds = store.deletions.map(entry => entry.track.id)
    const savedWrites = writes
    const savedEvents = [...events]
    waiting.release()
    assert.equal(await pending, 'deferred')
    assertPreserved(store, entryIds, deletionIds, savedWrites, savedEvents)
  })
}

console.log(`History sync regressions: ${cases - failures.length}/${cases} passed`)
assert.equal(failures.length, 0, failures.join(', '))
