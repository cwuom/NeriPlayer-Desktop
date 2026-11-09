import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

const root = new URL('../', import.meta.url)
const explore = await readFile(new URL('src/views/ExploreView.vue', root), 'utf8')
assert.match(explore, /biliParts\.request\(/, 'clicking a parent Bilibili video must offer its parts before playback')

function evaluate(source, dependencies) {
  const exports = {}
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `unexpected dependency ${name}`)
    return dependencies[name]
  }, exports)
  return exports
}

const policy = evaluate(await readFile(new URL('src/modules/playback/biliVideoParts.ts', root), 'utf8'), {})
const skipPolicy = evaluate(await readFile(new URL('src/modules/playback/biliVideoSkip.ts', root), 'utf8'), {})
const pending = []
const played = []
const player = { currentTrack: null, playAll: (...args) => played.push(args) }
const room = { localControlRestriction: null }
const notices = []
const warnings = []
const savedNameOrders = new Map()
const storageWrites = []
let storageFailure = null
const storage = {
  getItem(key) {
    if (storageFailure === 'getItem') throw new Error('storage read denied')
    return savedNameOrders.get(key) ?? null
  },
  setItem(key, value) {
    if (storageFailure === 'setItem') throw new Error('storage quota exceeded')
    storageWrites.push(['set', key, value])
    savedNameOrders.set(key, value)
  },
  removeItem(key) {
    if (storageFailure === 'removeItem') throw new Error('storage removal denied')
    storageWrites.push(['remove', key])
    savedNameOrders.delete(key)
  },
}
globalThis.localStorage = storage
const invoke = (command, args) => new Promise((resolve, reject) => pending.push({ command, args, resolve, reject }))
const { useBiliVideoPartsStore } = evaluate(await readFile(new URL('src/stores/biliVideoParts.ts', root), 'utf8'), {
  pinia, vue, '@tauri-apps/api/core': { invoke },
  '@/stores/player': { usePlayerStore: () => player },
  '@/stores/listenTogether': { useListenTogetherStore: () => room },
  '@/stores/toast': { useToastStore: () => ({ error: message => notices.push(message) }) },
  '@/i18n': { default: { global: { t: key => key } } },
  '@/modules/playback/biliVideoParts': policy,
  '@/modules/playback/biliVideoSkip': skipPolicy,
  '@/utils/logger': { createLogger: () => ({ warn: (...args) => warnings.push(args) }) },
})

const info = {
  bvid: 'BV14Dn16tELC', aid: 117482891253595,
  title: '世界纯音乐100首', owner: '北流嘉措', cover: 'https://example.test/cover.jpg',
  cid: 42946137046, duration: 21653,
  pages: Array.from({ length: 100 }, (_, index) => ({
    cid: 42946137046 + index, page: index + 1,
    part: index === 0 ? "01.Annie's Wonderland (安妮的仙境) - Bandari" : `${index + 1}.曲目${index + 1}`,
    duration: 207 + index,
  })),
}
const parent = { id: 'bilibili:BV14Dn16tELC', title: info.title, artist: info.owner, album: '', durationMs: 21653000, coverUrl: '', audioUrl: '' }
const tracks = policy.buildBiliPartTracks(info, parent)
assert.equal(tracks.length, 100)
assert.equal(new Set(tracks.map(track => track.id)).size, 100)
assert.equal(tracks[0].title, "Annie's Wonderland (安妮的仙境)")
assert.equal(tracks[0].artist, 'Bandari')
assert.equal(tracks[1].artist, info.owner)
assert.equal(tracks[0].durationMs, 207000)
assert.equal(tracks[0].album, `Bilibili|${info.pages[0].cid}|${info.bvid}`)
assert.equal(tracks[0].syncPayload.audioId, String(info.aid))
assert.equal(tracks[0].syncPayload.subAudioId, String(info.pages[0].cid))
assert.equal(policy.buildBiliPartTracks({ ...info, pages: [{ ...info.pages[0], part: '01.A - B - C' }] }, parent)[0].artist, info.owner)

const emptyPartInfo = {
  ...info,
  title: 'Song - Artist',
  pages: ['', '   \t  '].map((part, index) => ({ ...info.pages[index], part })),
}
assert.deepEqual(
  policy.buildBiliPartTracks(emptyPartInfo, parent).map(({ title, artist }) => [title, artist]),
  [['Song', 'Artist'], ['Song', 'Artist']],
  'empty part names must retain the default parsing of the parent video title',
)
assert.deepEqual(
  policy.buildBiliPartTracks(emptyPartInfo, parent, 'artist-first').map(({ title, artist }) => [title, artist]),
  [['Artist', 'Song'], ['Artist', 'Song']],
  'the selected name order must also apply when falling back to the parent title',
)
assert.deepEqual(
  policy.buildBiliPartTracks({ ...emptyPartInfo, title: 'Song' }, parent)
    .map(({ title, artist }) => [title, artist]),
  [['Song', info.owner], ['Song', info.owner]],
  'a parent title without a separator must keep the uploader as artist',
)

const artistFirstInfo = {
  ...info,
  bvid: 'BV1Mvx76YEkz',
  aid: 117518475724747,
  title: '许嵩歌曲合集',
  cid: 43127933379,
  duration: 712,
  pages: [
    { cid: 43127933379, page: 1, part: '许嵩 - 通关', duration: 250 },
    { cid: 43127934572, page: 2, part: '许嵩 - 双人旁', duration: 233 },
    { cid: 43127998217, page: 3, part: '许嵩 - 三尺', duration: 229 },
  ],
}
const artistFirstParent = {
  ...parent,
  id: `bilibili:${artistFirstInfo.bvid}`,
  title: artistFirstInfo.title,
  durationMs: artistFirstInfo.duration * 1000,
}
const corrected = policy.buildBiliPartTracks(artistFirstInfo, artistFirstParent, 'artist-first')
assert.deepEqual(corrected.map(({ title, artist, durationMs }) => [title, artist, durationMs]), [
  ['通关', '许嵩', 250000],
  ['双人旁', '许嵩', 233000],
  ['三尺', '许嵩', 229000],
])
const duetInfo = {
  ...artistFirstInfo,
  pages: [{ ...artistFirstInfo.pages[0], part: '许嵩、何曼婷 - 素颜' }],
}
assert.deepEqual(
  policy.buildBiliPartTracks(duetInfo, artistFirstParent, 'artist-first')
    .map(({ title, artist }) => [title, artist]),
  [['素颜', '许嵩、何曼婷']],
  'the selected order must preserve the full artist list of a duet',
)
assert.deepEqual(corrected.map(({ id, syncPayload }) => [id, syncPayload]), [
  ['bilibili:117518475724747:43127933379', {
    channelId: 'bilibili', audioId: '117518475724747', subAudioId: '43127933379', bvid: 'BV1Mvx76YEkz',
  }],
  ['bilibili:117518475724747:43127934572', {
    channelId: 'bilibili', audioId: '117518475724747', subAudioId: '43127934572', bvid: 'BV1Mvx76YEkz',
  }],
  ['bilibili:117518475724747:43127998217', {
    channelId: 'bilibili', audioId: '117518475724747', subAudioId: '43127998217', bvid: 'BV1Mvx76YEkz',
  }],
])
const ambiguousParts = {
  ...artistFirstInfo,
  pages: [
    { ...artistFirstInfo.pages[0], part: '通关' },
    { ...artistFirstInfo.pages[1], part: '许嵩 - 双人旁 - 现场版' },
  ],
}
assert.deepEqual(
  policy.buildBiliPartTracks(ambiguousParts, artistFirstParent, 'artist-first')
    .map(({ title, artist }) => [title, artist]),
  [['通关', artistFirstInfo.owner], ['许嵩 - 双人旁 - 现场版', artistFirstInfo.owner]],
)

pinia.setActivePinia(pinia.createPinia())
const store = useBiliVideoPartsStore()
let fallback = 0
await store.request({ ...parent, id: 'netease:1' }, () => fallback++)
assert.equal(fallback, 1)
assert.equal(pending.length, 0)
await store.request(tracks[1], () => fallback++)
assert.equal(fallback, 2)
assert.equal(pending.length, 0, 'already selected CID must play directly')

const loading = store.request(parent, () => fallback++)
assert.equal(store.open, true)
assert.equal(store.loading, true)
assert.equal(played.length, 0, 'opening the picker must not replace the current queue')
assert.deepEqual(pending[0].args, { bvid: info.bvid, avid: null })
pending[0].resolve(info)
await loading
assert.equal(store.info.pages.length, 100)
store.choose(info.pages[49].cid)
assert.equal(played.length, 1)
assert.equal(played[0][0].length, 100)
assert.equal(played[0][1], tracks[49].id)
assert.equal(store.open, false)

const cancelled = store.request(parent, () => fallback++)
store.close()
pending[1].resolve(info)
await cancelled
assert.equal(store.open, false)
assert.equal(store.info, null)
assert.equal(played.length, 1)

const outdated = store.request(parent, () => fallback++)
await store.request({ ...parent, id: 'netease:2' }, () => fallback++)
pending[2].resolve(info)
await outdated
assert.equal(store.open, false, 'late video response must not reopen a dismissed picker')

const failed = store.request(parent, () => fallback++)
pending[3].reject(new Error('unavailable'))
await failed
assert.equal(store.failed, true)
assert.equal(store.open, true)
assert.equal(fallback, 3, 'failure must not silently play the first part')
const retry = store.retry()
pending[4].resolve({ ...info, pages: [info.pages[0]] })
await retry
assert.equal(fallback, 4, 'single-part videos keep the original playback path')
assert.equal(store.open, false)

const superseded = store.request(parent, () => fallback++)
player.currentTrack = { id: 'netease:new-selection' }
pending[5].resolve({ ...info, pages: [info.pages[0]] })
await superseded
assert.equal(fallback, 4, 'a late single-part result must not interrupt a newer queue selection')
assert.equal(store.open, false)

room.localControlRestriction = 'controller_offline'
await store.request(parent, () => fallback++)
assert.equal(pending.length, 6, 'restricted room members must not start selection requests')
assert.equal(notices.at(-1), 'listen_together.control_blocked_controller_offline')
room.localControlRestriction = null
const beforeRestricted = store.request(parent, () => fallback++)
pending[6].resolve(info)
await beforeRestricted
room.localControlRestriction = 'member_control_disabled'
store.choose(info.pages[5].cid)
assert.equal(played.length, 1, 'room restriction must be rechecked before replacing the queue')
assert.equal(store.open, false)
assert.equal(notices.at(-1), 'listen_together.control_blocked_member_control')

room.localControlRestriction = null
const artistFirstLoading = store.request(artistFirstParent, () => fallback++)
pending.at(-1).resolve(artistFirstInfo)
await artistFirstLoading
assert.deepEqual(store.tracks, policy.buildBiliPartTracks(artistFirstInfo, artistFirstParent))
store.nameOrder = 'artist-first'
assert.deepEqual(store.tracks, corrected, 'the picker preview must reflect the selected name order')
const previewTracks = store.tracks
store.choose(artistFirstInfo.pages[1].cid)
assert.equal(played.length, 2)
assert.deepEqual(played.at(-1), [corrected, corrected[1].id], 'the chosen order must apply to every part')
assert.equal(played.at(-1)[0], previewTracks, 'playback must use the tracks shown in the preview')
const artistFirstKey = 'neri:bili-part-name-order:v1:BV1Mvx76YEkz'
assert.equal(savedNameOrders.get(artistFirstKey), 'artist-first')

const nextVideoLoading = store.request(parent, () => fallback++)
pending.at(-1).resolve(info)
await nextVideoLoading
assert.equal(store.nameOrder, 'title-first', 'the previous video order must not leak into another video')
store.choose(info.pages[0].cid)
assert.deepEqual(played.at(-1), [tracks, tracks[0].id])

async function openParts(targetStore, detail = artistFirstInfo, input = artistFirstParent) {
  const request = targetStore.request(input, () => fallback++)
  pending.at(-1).resolve(detail)
  await request
  assert.equal(targetStore.failed, false)
  assert.equal(targetStore.open, true)
}

pinia.setActivePinia(pinia.createPinia())
const restartedStore = useBiliVideoPartsStore()
await openParts(restartedStore, artistFirstInfo, { ...artistFirstParent, id: 'bilibili:117518475724747' })
assert.deepEqual(pending.at(-1).args, { bvid: null, avid: artistFirstInfo.aid })
assert.equal(restartedStore.nameOrder, 'artist-first', 'a new store must restore the canonical video preference for AV input')
assert.deepEqual(restartedStore.tracks, corrected)
restartedStore.choose(artistFirstInfo.pages[0].cid)
assert.equal(savedNameOrders.get(artistFirstKey), 'artist-first')
assert.equal(savedNameOrders.has('neri:bili-part-name-order:v1:117518475724747'), false, 'AV input must save under the canonical BV key')
await openParts(restartedStore)
const writesBeforeCancel = storageWrites.length
restartedStore.nameOrder = 'title-first'
restartedStore.close()
assert.equal(storageWrites.length, writesBeforeCancel, 'cancelling an order edit must not save it')
assert.equal(savedNameOrders.get(artistFirstKey), 'artist-first')

await openParts(restartedStore)
const writesBeforeInvalidChoice = storageWrites.length
restartedStore.choose(-1)
assert.equal(storageWrites.length, writesBeforeInvalidChoice, 'an invalid CID must not save a preference')
assert.equal(restartedStore.open, true)
restartedStore.nameOrder = 'title-first'
restartedStore.choose(artistFirstInfo.pages[0].cid)
assert.equal(savedNameOrders.has(artistFirstKey), false, 'switching back to the default must remove the saved preference')
await openParts(restartedStore)
assert.equal(restartedStore.nameOrder, 'title-first')
restartedStore.close()

const writesBeforeFailure = storageWrites.length
const failedPreferenceRequest = restartedStore.request(artistFirstParent, () => fallback++)
pending.at(-1).reject(new Error('video unavailable'))
await failedPreferenceRequest
assert.equal(restartedStore.failed, true)
assert.equal(storageWrites.length, writesBeforeFailure, 'failed video requests must not save a preference')
restartedStore.close()

await openParts(restartedStore)
restartedStore.nameOrder = 'artist-first'
room.localControlRestriction = 'member_control_disabled'
const playedBeforeBlockedChoice = played.length
const writesBeforeBlockedChoice = storageWrites.length
restartedStore.choose(artistFirstInfo.pages[0].cid)
assert.equal(played.length, playedBeforeBlockedChoice)
assert.equal(storageWrites.length, writesBeforeBlockedChoice, 'a blocked choice must not save a preference')
room.localControlRestriction = null

const writesBeforeBlockedRequest = storageWrites.length
const pendingBeforeBlockedRequest = pending.length
room.localControlRestriction = 'controller_offline'
await restartedStore.request(artistFirstParent, () => fallback++)
assert.equal(pending.length, pendingBeforeBlockedRequest)
assert.equal(storageWrites.length, writesBeforeBlockedRequest, 'a blocked request must not save a preference')
room.localControlRestriction = null

savedNameOrders.set(artistFirstKey, 'unknown-order')
await openParts(restartedStore)
assert.equal(restartedStore.nameOrder, 'title-first', 'invalid stored values must use the default order')
restartedStore.close()

storageFailure = 'getItem'
const warningsBeforeReadFailure = warnings.length
await openParts(restartedStore)
assert.equal(restartedStore.nameOrder, 'title-first', 'storage read errors must not prevent opening the picker')
assert.ok(warnings.length > warningsBeforeReadFailure)
restartedStore.close()
storageFailure = null

delete globalThis.localStorage
await openParts(restartedStore)
assert.equal(restartedStore.nameOrder, 'title-first', 'unavailable storage must use the default order')
restartedStore.close()
globalThis.localStorage = storage

await openParts(restartedStore)
restartedStore.nameOrder = 'artist-first'
storageFailure = 'setItem'
const warningsBeforeWriteFailure = warnings.length
const playedBeforeWriteFailure = played.length
restartedStore.choose(artistFirstInfo.pages[0].cid)
assert.equal(played.length, playedBeforeWriteFailure + 1, 'quota errors must not block playback')
assert.deepEqual(played.at(-1), [corrected, corrected[0].id])
assert.ok(warnings.length > warningsBeforeWriteFailure)
storageFailure = null

savedNameOrders.set(artistFirstKey, 'artist-first')
await openParts(restartedStore)
restartedStore.nameOrder = 'title-first'
storageFailure = 'removeItem'
const warningsBeforeRemoveFailure = warnings.length
const playedBeforeRemoveFailure = played.length
restartedStore.choose(artistFirstInfo.pages[0].cid)
assert.equal(played.length, playedBeforeRemoveFailure + 1, 'storage removal errors must not block playback')
assert.deepEqual(played.at(-1)[0], policy.buildBiliPartTracks(artistFirstInfo, artistFirstParent))
assert.ok(warnings.length > warningsBeforeRemoveFailure)
storageFailure = null

for (const path of ['src/views/BiliPlaylistView.vue', 'src/views/BiliArtistView.vue']) {
  assert.match(await readFile(new URL(path, root), 'utf8'), /biliParts\.request\(/)
}
assert.match(await readFile(new URL('src/App.vue', root), 'utf8'), /<BiliVideoPartsDialog\s*\/>/)
console.log('Bilibili multipart: metadata order, 100-part queue, explicit CID, cancellation, stale response, retry and entry points passed')
