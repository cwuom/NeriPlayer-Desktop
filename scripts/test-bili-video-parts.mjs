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
const invoke = (command, args) => new Promise((resolve, reject) => pending.push({ command, args, resolve, reject }))
const { useBiliVideoPartsStore } = evaluate(await readFile(new URL('src/stores/biliVideoParts.ts', root), 'utf8'), {
  pinia, vue, '@tauri-apps/api/core': { invoke },
  '@/stores/player': { usePlayerStore: () => player },
  '@/stores/listenTogether': { useListenTogetherStore: () => room },
  '@/stores/toast': { useToastStore: () => ({ error: message => notices.push(message) }) },
  '@/i18n': { default: { global: { t: key => key } } },
  '@/modules/playback/biliVideoParts': policy,
  '@/modules/playback/biliVideoSkip': skipPolicy,
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

for (const path of ['src/views/BiliPlaylistView.vue', 'src/views/BiliArtistView.vue']) {
  assert.match(await readFile(new URL(path, root), 'utf8'), /biliParts\.request\(/)
}
assert.match(await readFile(new URL('src/App.vue', root), 'utf8'), /<BiliVideoPartsDialog\s*\/>/)
console.log('Bilibili multipart: metadata, 100-part queue, explicit CID, cancellation, stale response, retry and entry points passed')
