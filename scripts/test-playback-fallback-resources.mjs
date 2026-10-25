import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, extname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const project = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const modules = new Map()
let invokeHandler

function load(file) {
  file = resolve(project, file)
  if (!extname(file)) file += '.ts'
  if (modules.has(file)) return modules.get(file)
  const exports = {}
  modules.set(file, exports)
  const compiled = ts.transpileModule(readFileSync(file, 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, esModuleInterop: true },
  }).outputText
  new Function('require', 'exports', compiled)(name => {
    if (name === '@tauri-apps/api/core') return { invoke: (command, args) => invokeHandler(command, args) }
    if (name.startsWith('@/')) return load(`src/${name.slice(2)}`)
    if (name.startsWith('.')) return load(resolve(dirname(file), name))
    return require(name)
  }, exports)
  return exports
}

const { PlaybackUrlResolver, playbackPrefetchCacheId } = load('src/modules/playback/playbackSource.ts')
const { PlaybackPrefetchManager } = load('src/modules/playback/playbackPrefetch.ts')
const settings = {
  neteaseQuality: 'exhigh', qqMusicQuality: 'high', biliQuality: 'high', youtubeQuality: 'high',
  neteaseAutoSourceSwitch: true,
}
const track = id => ({
  id: `netease:${id}`, title: `song-${id}`, artist: 'artist', album: 'album',
  durationMs: 180_000, source: 'netease', audioUrl: '',
})
const unavailable = { url: null, bitrate: 0, format: 'mp3', unavailable_reason: 'no_permission' }
const tick = () => new Promise(setImmediate)
function deferred() {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}
function fallbackCandidates(title) {
  return [1, 2].map(index => ({
    id: `bilibili:BV${title}-${index}`, title, artist: 'artist', album: `Bilibili|${index}`,
    url: '', duration_ms: 180_000,
  }))
}
function biliResponse(args) {
  return {
    url: `https://a.bilivideo.com/${args.bvid}.m4a`, bandwidth: 128_000, codecs: 'mp4a.40.2',
    quality_key: 'high', candidates: [], bvid: args.bvid, cid: args.cid,
  }
}
function harness() {
  const manager = new PlaybackPrefetchManager()
  const resolver = new PlaybackUrlResolver(() => Promise.resolve())
  const searchGates = new Map(), searches = [], biliCalls = []
  let blocked = true
  invokeHandler = async (command, args) => {
    if (command === 'get_netease_song_url') return unavailable
    if (command === 'find_netease_bili_sources') {
      searches.push(args.title)
      if (blocked) {
        const gate = deferred()
        searchGates.set(args.title, gate)
        await gate.promise
      }
      return fallbackCandidates(args.title)
    }
    assert.equal(command, 'get_bili_audio_url')
    biliCalls.push(args.bvid)
    return biliResponse(args)
  }
  return {
    manager, resolver, searches, biliCalls,
    release(title) { searchGates.get(title)?.resolve() },
    async close() {
      blocked = false
      for (const gate of searchGates.values()) gate.resolve()
      await tick()
      manager.clear()
      resolver.clear()
    },
  }
}

let passed = 0, failed = 0
async function test(name, run) {
  try { await run(); passed++; console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

await test('sweeping restricted NetEase rows starts one fallback search and retains only the latest intent', async () => {
  const r = harness()
  try {
    for (let id = 1; id <= 100; id++) r.manager.prefetchIntent(track(id), settings, r.resolver)
    await tick()
    assert.deepEqual(r.searches, ['song-1'])
    r.release('song-1')
    await tick()
    assert.deepEqual(r.searches, ['song-1', 'song-100'])
    r.release('song-100')
    await tick()
    assert.equal(r.manager.take(track(100), settings)?.source, 'bilibili')
  } finally { await r.close() }
})

await test('rapid playback changes replace obsolete queued fallback prefetches', async () => {
  const r = harness()
  try {
    r.manager.prefetch(track(1), settings, r.resolver)
    await tick()
    for (let id = 2; id <= 50; id++) {
      r.manager.replacePlaybackDemand(playbackPrefetchCacheId(track(id), settings))
      r.manager.prefetchWindow([track(id + 1)], settings, r.resolver)
    }
    assert.deepEqual(r.searches, ['song-1'])
    r.release('song-1')
    await tick()
    assert.deepEqual(r.searches, ['song-1', 'song-51'])
  } finally { await r.close() }
})

await test('clearing prefetches retains the running permit and stops obsolete fallback IPCs', async () => {
  const r = harness()
  try {
    r.manager.prefetch(track(1), settings, r.resolver)
    await tick()
    r.manager.clear()
    r.manager.prefetch(track(2), settings, r.resolver)
    await tick()
    assert.deepEqual(r.searches, ['song-1'], 'clear cannot create a second physical background request')
    r.release('song-1')
    await tick()
    assert.deepEqual(r.biliCalls, [], 'obsolete search results cannot initiate candidate requests')
    assert.deepEqual(r.searches, ['song-1', 'song-2'])
  } finally { await r.close() }
})

await test('clearing a track does not cancel a foreground request that joined its prefetch', async () => {
  const r = harness()
  try {
    r.manager.prefetch(track(1), settings, r.resolver)
    await tick()
    r.manager.replacePlaybackDemand(playbackPrefetchCacheId(track(1), settings))
    const foreground = r.resolver.resolve(track(1), settings, { requestGeneration: 10 })
    r.manager.clearForTrack(track(1), settings)
    r.release('song-1')
    const result = await foreground
    assert.equal(result.type, 'success')
    assert.equal(result.source, 'bilibili')
    assert.deepEqual(r.searches, ['song-1'], 'foreground takeover reuses the ongoing search')
    assert.deepEqual(r.biliCalls, ['BVsong-1-1', 'BVsong-1-2'])
    assert.equal(r.manager.take(track(1), settings), null)
  } finally { await r.close() }
})

await test('clearing a background track stops its later candidate requests', async () => {
  const r = harness()
  try {
    r.manager.prefetch(track(1), settings, r.resolver)
    await tick()
    r.manager.clearForTrack(track(1), settings)
    r.release('song-1')
    await tick()
    assert.deepEqual(r.biliCalls, [])
    assert.equal(r.manager.take(track(1), settings), null)
  } finally { await r.close() }
})

await test('a superseded foreground takeover cannot keep its generationless fallback alive', async () => {
  const r = harness()
  try {
    r.manager.prefetch(track(1), settings, r.resolver)
    await tick()
    r.manager.replacePlaybackDemand(playbackPrefetchCacheId(track(1), settings))
    const previous = r.resolver.resolve(track(1), settings, { requestGeneration: 10 })
    r.manager.replacePlaybackDemand(playbackPrefetchCacheId(track(2), settings))
    const current = r.resolver.resolve(track(2), settings, { requestGeneration: 11 })
    await tick()
    r.release('song-1')
    const stale = await previous
    assert.equal(stale.type, 'failure')
    assert.match(stale.message, /superseded/i)
    assert.deepEqual(r.biliCalls, [])
    r.release('song-2')
    assert.equal((await current).type, 'success')
  } finally { await r.close() }
})

await test('switching to cached or local playback cancels the previous foreground takeover', async () => {
  for (const demand of [playbackPrefetchCacheId(track(2), settings), null]) {
    const r = harness()
    try {
      r.manager.prefetch(track(1), settings, r.resolver)
      await tick()
      r.manager.replacePlaybackDemand(playbackPrefetchCacheId(track(1), settings))
      const previous = r.resolver.resolve(track(1), settings, { requestGeneration: 10 })
      r.manager.replacePlaybackDemand(demand)
      r.release('song-1')
      const stale = await previous
      assert.equal(stale.type, 'failure')
      assert.match(stale.message, /superseded/i)
      assert.deepEqual(r.biliCalls, [])
    } finally { await r.close() }
  }
})

await test('an unrelated foreground playback request bypasses the occupied fallback prefetch permit', async () => {
  const r = harness()
  try {
    r.manager.prefetch(track(1), settings, r.resolver)
    await tick()
    const foreground = r.resolver.resolve(track(2), settings, { requestGeneration: 20 })
    await tick()
    assert.deepEqual(r.searches, ['song-1', 'song-2'])
    r.release('song-2')
    assert.equal((await foreground).type, 'success')
  } finally { await r.close() }
})

await test('clearing the URL resolver stops further NetEase quality probes', async () => {
  const resolver = new PlaybackUrlResolver(() => Promise.resolve())
  const gate = deferred(), commands = []
  invokeHandler = async command => { commands.push(command); await gate.promise; return unavailable }
  const pending = resolver.resolve(track(1), settings, { requestGeneration: 1 })
  await tick()
  resolver.clear()
  gate.resolve()
  const result = await pending
  assert.deepEqual(commands, ['get_netease_song_url'])
  assert.equal(result.type, 'failure')
  assert.match(result.message, /superseded/i)
})

await test('invalidating a resolution stops later Bilibili candidates after the current IPC settles', async () => {
  const resolver = new PlaybackUrlResolver(() => Promise.resolve())
  const gate = deferred(), biliCalls = []
  invokeHandler = async (command, args) => {
    if (command === 'get_netease_song_url') return unavailable
    if (command === 'find_netease_bili_sources') return fallbackCandidates(args.title)
    assert.equal(command, 'get_bili_audio_url')
    biliCalls.push(args.bvid)
    await gate.promise
    return biliResponse(args)
  }
  const pending = resolver.resolve(track(1), settings, { requestGeneration: 1 })
  await tick()
  resolver.invalidate(track(1), settings)
  gate.resolve()
  const result = await pending
  assert.deepEqual(biliCalls, ['BVsong-1-1'])
  assert.equal(result.type, 'failure')
  assert.match(result.message, /superseded/i)
})

console.log(`Playback fallback resources: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
