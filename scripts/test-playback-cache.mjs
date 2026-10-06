import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const urls = new Map()
async function load(path) {
  if (urls.has(path)) return urls.get(path)
  let source = ts.transpileModule(await readFile(new URL(path, import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  for (const match of [...source.matchAll(/from ['"]([^'"]+)['"]/g)]) {
    const specifier = match[1]
    let dependency
    if (specifier === '@tauri-apps/api/core') {
      dependency = `data:text/javascript;base64,${Buffer.from('export const invoke = (c, a) => globalThis.__cacheInvoke(c, a)').toString('base64')}`
    } else {
      const target = specifier.startsWith('@/')
        ? `../src/${specifier.slice(2)}.ts`
        : new URL(`${specifier}.ts`, new URL(path, import.meta.url)).href
      dependency = await load(target)
    }
    source = source.replace(`from '${specifier}'`, `from '${dependency}'`)
      .replace(`from "${specifier}"`, `from '${dependency}'`)
  }
  const url = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
  urls.set(path, url)
  return url
}

const source = await import(await load('../src/modules/playback/playbackSource.ts'))
const { PlaybackPrefetchManager } = await import(await load('../src/modules/playback/playbackPrefetch.ts'))
const settings = { neteaseQuality: 'exhigh', qqMusicQuality: 'high', biliQuality: 'high', youtubeQuality: 'high' }
const youtube = { id: 'youtube:cached-song', title: 'song', artist: 'artist', album: '', durationMs: 180_000, source: 'youtube' }
const stream = url => [{ url, bitrate: 128_000, mime_type: 'audio/mp4' }]
function deferred() {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}
const realNow = Date.now
let now = 1_800_000_000_000
Date.now = () => now
let failed = 0
let passed = 0
async function run(name, test) {
  now = 1_800_000_000_000
  try { await test(); passed++; console.log(`ok ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}
try {
  await run('YouTube cache stops 90 seconds before the signed URL expires', async () => {
    const resolver = new source.PlaybackUrlResolver()
    let calls = 0
    globalThis.__cacheInvoke = async () => { calls++; return stream(`https://rr.googlevideo.com/audio?expire=${Math.floor(now / 1000) + 100}`) }
    await resolver.resolve(youtube, settings)
    now += 11_000
    await resolver.resolve(youtube, settings)
    assert.equal(calls, 2)
  })
  await run('YouTube retains valid resolution for the Android eight-minute TTL', async () => {
    const resolver = new source.PlaybackUrlResolver()
    let calls = 0
    globalThis.__cacheInvoke = async () => { calls++; return stream('https://rr.googlevideo.com/audio') }
    await resolver.resolve(youtube, settings)
    now += 120_000
    await resolver.resolve(youtube, settings)
    assert.equal(calls, 1)
    now += 360_001
    await resolver.resolve(youtube, settings)
    assert.equal(calls, 2)
  })
  await run('an older request cannot overwrite a force-refreshed resolution', async () => {
    const resolver = new source.PlaybackUrlResolver()
    const old = deferred()
    let calls = 0
    globalThis.__cacheInvoke = async () => ++calls === 1 ? old.promise : stream('https://rr.googlevideo.com/fresh')
    const pending = resolver.resolve(youtube, settings)
    await resolver.resolve(youtube, settings, { forceRefresh: true })
    old.resolve(stream('https://rr.googlevideo.com/old'))
    await pending
    assert.equal((await resolver.resolve(youtube, settings)).url, 'https://rr.googlevideo.com/fresh')
  })
  await run('clear prevents late in-flight results from restoring an old session cache', async () => {
    const resolver = new source.PlaybackUrlResolver()
    const old = deferred()
    let calls = 0
    globalThis.__cacheInvoke = async () => ++calls === 1 ? old.promise : stream('https://rr.googlevideo.com/new-session')
    const pending = resolver.resolve(youtube, settings)
    resolver.clear()
    old.resolve(stream('https://rr.googlevideo.com/old-session'))
    await pending
    assert.equal((await resolver.resolve(youtube, settings)).url, 'https://rr.googlevideo.com/new-session')
    assert.equal(calls, 2)
  })
  await run('a newer playback generation does not share the cancelled generation request', async () => {
    const resolver = new source.PlaybackUrlResolver()
    const old = deferred()
    let calls = 0
    globalThis.__cacheInvoke = async () => ++calls === 1 ? old.promise : stream('https://rr.googlevideo.com/new-generation')
    const first = resolver.resolve(youtube, settings, { requestGeneration: 1 })
    const second = resolver.resolve(youtube, settings, { requestGeneration: 2 })
    await Promise.resolve()
    assert.equal(calls, 2)
    old.resolve(stream('https://rr.googlevideo.com/old-generation'))
    await first
    assert.equal((await second).url, 'https://rr.googlevideo.com/new-generation')
  })
  await run('a foreground generation does not inherit a generationless in-flight prefetch', async () => {
    const resolver = new source.PlaybackUrlResolver()
    const old = deferred()
    let calls = 0
    globalThis.__cacheInvoke = async () => ++calls === 1 ? old.promise : stream('https://rr.googlevideo.com/foreground')
    const first = resolver.resolve(youtube, settings)
    const second = resolver.resolve(youtube, settings, { requestGeneration: 2 })
    try {
      await new Promise(setImmediate)
      assert.equal(calls, 2)
    } finally {
      old.resolve(stream('https://rr.googlevideo.com/generationless'))
      await Promise.all([first, second])
    }
    assert.equal((await second).url, 'https://rr.googlevideo.com/foreground')
    assert.equal((await resolver.resolve(youtube, settings, { requestGeneration: 3 })).url, 'https://rr.googlevideo.com/foreground')
    assert.equal(calls, 2, 'completed cache remains reusable by later generations')
  })
  await run('matching playback generations share their pending resolution', async () => {
    const resolver = new source.PlaybackUrlResolver()
    const waiting = deferred()
    let calls = 0
    globalThis.__cacheInvoke = async () => { calls++; return waiting.promise }
    const first = resolver.resolve(youtube, settings, { requestGeneration: 7 })
    const second = resolver.resolve(youtube, settings, { requestGeneration: 7 })
    waiting.resolve(stream('https://rr.googlevideo.com/shared-generation'))
    assert.deepEqual(await first, await second)
    assert.equal(calls, 1)
  })
  await run('prefetch windows propagate their playback generation to each YouTube IPC', async () => {
    const manager = new PlaybackPrefetchManager()
    const generations = []
    globalThis.__cacheInvoke = async (_, args) => {
      generations.push(args.requestGeneration)
      return stream(`https://rr.googlevideo.com/${args.videoId}`)
    }
    manager.prefetchWindow([youtube, { ...youtube, id: 'youtube:next-song' }], settings, new source.PlaybackUrlResolver(), 11)
    await new Promise(setImmediate)
    assert.deepEqual(generations, [11, 11])
    assert.ok(manager.take(youtube, settings))
  })
  await run('a newer prefetch generation replaces the old job without accepting late results', async () => {
    const manager = new PlaybackPrefetchManager()
    const resolver = new source.PlaybackUrlResolver()
    const old = deferred()
    const fresh = deferred()
    const generations = []
    globalThis.__cacheInvoke = async (_, args) => {
      generations.push(args.requestGeneration)
      return generations.length === 1 ? old.promise : fresh.promise
    }
    manager.prefetch(youtube, settings, resolver, 1)
    manager.prefetch(youtube, settings, resolver, 2)
    try {
      await new Promise(setImmediate)
      assert.deepEqual(generations, [1, 2])
      old.resolve(stream('https://rr.googlevideo.com/stale-prefetch'))
      await new Promise(setImmediate)
      assert.equal(manager.take(youtube, settings), null)
      manager.prefetch(youtube, settings, resolver, 2)
      assert.equal(generations.length, 2, 'stale cleanup must retain the newer job token')
      fresh.resolve(stream('https://rr.googlevideo.com/current-prefetch'))
      await new Promise(setImmediate)
      assert.equal(manager.take(youtube, settings).url, 'https://rr.googlevideo.com/current-prefetch')
    } finally {
      old.resolve(stream('https://rr.googlevideo.com/stale-prefetch'))
      fresh.resolve(stream('https://rr.googlevideo.com/current-prefetch'))
      await new Promise(setImmediate)
    }
  })
  await run('completed prefetch cache remains usable after the playback generation changes', async () => {
    const manager = new PlaybackPrefetchManager()
    const resolver = new source.PlaybackUrlResolver()
    let calls = 0
    globalThis.__cacheInvoke = async () => { calls++; return stream('https://rr.googlevideo.com/completed-prefetch') }
    manager.prefetch(youtube, settings, resolver, 1)
    await new Promise(setImmediate)
    manager.prefetch(youtube, settings, resolver, 2)
    await new Promise(setImmediate)
    assert.equal(calls, 1)
    assert.equal(manager.take(youtube, settings).url, 'https://rr.googlevideo.com/completed-prefetch')
  })
  await run('force refresh propagates to the YouTube backend cache', async () => {
    globalThis.__cacheInvoke = async (command, args) => {
      assert.equal(command, 'get_youtube_audio_url')
      assert.equal(args.forceRefresh, true)
      return stream('https://rr.googlevideo.com/refreshed')
    }
    assert.equal((await new source.PlaybackUrlResolver().resolve(youtube, settings, { forceRefresh: true })).type, 'success')
  })
  await run('normal requests wait for a force refresh instead of reusing the rejected URL', async () => {
    const resolver = new source.PlaybackUrlResolver()
    globalThis.__cacheInvoke = async () => stream('https://rr.googlevideo.com/rejected')
    await resolver.resolve(youtube, settings)
    const fresh = deferred()
    globalThis.__cacheInvoke = async () => fresh.promise
    const refresh = resolver.resolve(youtube, settings, { forceRefresh: true })
    const normal = resolver.resolve(youtube, settings)
    fresh.resolve(stream('https://rr.googlevideo.com/replacement'))
    await refresh
    assert.equal((await normal).url, 'https://rr.googlevideo.com/replacement')
  })
  await run('prefetch respects signed expiry instead of extending a stale URL', async () => {
    const manager = new PlaybackPrefetchManager()
    globalThis.__cacheInvoke = async () => stream(`https://rr.googlevideo.com/audio?expire=${Math.floor(now / 1000) + 100}`)
    const resolver = new source.PlaybackUrlResolver()
    manager.prefetch(youtube, settings, resolver)
    await new Promise(setImmediate)
    assert.ok(manager.take(youtube, settings))
    manager.prefetch(youtube, settings, resolver)
    await new Promise(setImmediate)
    now += 11_000
    assert.equal(manager.take(youtube, settings), null)
  })
  await run('NetEase uses actual returned quality and verifies song identity', async () => {
    const track = { ...youtube, id: 'netease:123', source: 'netease' }
    globalThis.__cacheInvoke = async () => ({ url: 'https://m.music.126.net/audio', format: 'flac', bitrate: 999_000, level: 'lossless', song_id: 123 })
    const result = await new source.PlaybackUrlResolver().resolve(track, settings)
    assert.equal(result.qualityKey, 'lossless')
    assert.match(result.cacheKey, /-lossless$/)
    globalThis.__cacheInvoke = async () => ({ url: 'https://m.music.126.net/wrong', format: 'mp3', bitrate: 128_000, song_id: 999 })
    assert.equal((await new source.PlaybackUrlResolver().resolve(track, settings)).type, 'failure')
  })
  await run('Bilibili uses the actual stream quality and matching cache key', async () => {
    globalThis.__cacheInvoke = async () => ({ url: 'https://a.bilivideo.com/audio', bandwidth: 192_000, codecs: 'mp4a.40.2', quality_key: 'high', mime_type: 'audio/mp4' })
    const result = await new source.PlaybackUrlResolver().resolve({ ...youtube, id: 'bilibili:BV1test', source: 'bilibili', album: 'Bilibili|42' }, { ...settings, biliQuality: 'lossless' })
    assert.equal(result.qualityKey, 'high')
    assert.match(result.cacheKey, /-high$/)
  })
  await run('Bilibili quality fallback displays and caches the candidate that actually played', async () => {
    globalThis.__cacheInvoke = async () => ({
      url: 'https://a.bilivideo.com/dolby', bandwidth: 256_000, codecs: 'ec-3', quality_key: 'dolby', mime_type: 'audio/mp4',
      candidates: [{ url: 'https://b.bilivideo.com/aac', bandwidth: 128_000, codecs: 'mp4a.40.2', quality_key: 'medium', mime_type: 'audio/mp4' }],
    })
    const resolved = await new source.PlaybackUrlResolver().resolve({ ...youtube, id: 'bilibili:BV1test', source: 'bilibili', album: 'Bilibili|42' }, { ...settings, biliQuality: 'dolby' })
    const selected = source.selectPlaybackCandidate(resolved, 1)
    assert.equal(selected.url, 'https://b.bilivideo.com/aac')
    assert.equal(selected.audioInfo.qualityKey, 'medium')
    assert.equal(selected.audioInfo.codecLabel, 'AAC')
    assert.match(source.playbackCacheWriteOptions(resolved, 1).cacheKey, /-medium$/)
    assert.equal(resolved.audioInfo.qualityKey, 'dolby')
  })
  await run('HLS recovery bypasses direct links and carries an explicit stream type', async () => {
    globalThis.__cacheInvoke = async (_, args) => {
      assert.equal(args.avoidDirect, true)
      return [{ url: 'https://manifest.googlevideo.com/api/manifest/hls_playlist/test', bitrate: 128_000, mime_type: 'audio/mp4', stream_type: 'hls' }]
    }
    const result = await new source.PlaybackUrlResolver().resolve({ ...youtube, audioUrl: 'https://rr.googlevideo.com/rejected' }, settings, { avoidDirect: true })
    assert.equal(result.streamType, 'hls')
    assert.match(result.url, /\/manifest\//)
  })
  await run('room-shared YouTube HLS URLs keep their type and the protocol candidate limit', async () => {
    const hlsUrl = 'https://manifest.googlevideo.com/api/manifest/hls_playlist/id/test'
    const directUrl = 'https://rr.googlevideo.com/direct'
    const resolved = await new source.PlaybackUrlResolver().resolve({ ...youtube, audioUrl: directUrl, syncPayload: { streamUrls: [directUrl, hlsUrl] } }, settings)
    assert.equal(resolved.streamType, 'direct')
    assert.deepEqual(resolved.candidateUrls, [])
    const hls = await new source.PlaybackUrlResolver().resolve({ ...youtube, audioUrl: hlsUrl }, settings)
    assert.equal(hls.streamType, 'hls')
  })
  await run('YouTube high quality does not select 48 kbps below an available 128 kbps stream', async () => {
    globalThis.__cacheInvoke = async () => [
      { url: 'https://rr.googlevideo.com/128', bitrate: 128_000, mime_type: 'audio/mp4' },
      { url: 'https://rr.googlevideo.com/48', bitrate: 48_000, mime_type: 'audio/mp4' },
    ]
    assert.equal((await new source.PlaybackUrlResolver().resolve(youtube, { ...settings, youtubeQuality: 'high' })).url, 'https://rr.googlevideo.com/128')
  })
  await run('HLS and direct streams have separate persistent cache entries', async () => {
    globalThis.__cacheInvoke = async () => stream('https://rr.googlevideo.com/direct')
    const direct = await new source.PlaybackUrlResolver().resolve(youtube, settings)
    globalThis.__cacheInvoke = async () => [{ url: 'https://manifest.googlevideo.com/hls', bitrate: 128_000, mime_type: 'audio/mp4', stream_type: 'hls' }]
    const hls = await new source.PlaybackUrlResolver().resolve(youtube, settings, { avoidDirect: true })
    assert.notEqual(direct.cacheKey, hls.cacheKey)
    assert.equal(hls.cacheKey, `${direct.cacheKey}-hls`)
    assert.deepEqual(source.playbackCacheReadCandidates(youtube, settings).map(item => item.cacheKey), [direct.cacheKey, hls.cacheKey])
  })
  await run('YouTube Android quality aliases and unknown values use the same bitrate threshold', async () => {
    globalThis.__cacheInvoke = async () => [
      { url: 'https://rr.googlevideo.com/direct128', bitrate: 128_000, mime_type: 'audio/mp4', stream_type: 'direct' },
      { url: 'https://manifest.googlevideo.com/hls192', bitrate: 192_000, mime_type: 'audio/mp4', stream_type: 'hls' },
    ]
    for (const quality of ['very-high', 'jymaster', 'unknown']) {
      assert.equal((await new source.PlaybackUrlResolver().resolve(youtube, { ...settings, youtubeQuality: quality })).url, 'https://manifest.googlevideo.com/hls192')
    }
    for (const quality of ['low', 'standard', 'medium', 'high', 'higher']) {
      assert.equal((await new source.PlaybackUrlResolver().resolve(youtube, { ...settings, youtubeQuality: quality })).url, 'https://rr.googlevideo.com/direct128')
    }
  })
  await run('HLS signed expiry in the manifest path also limits the resolution cache', async () => {
    const resolver = new source.PlaybackUrlResolver()
    let calls = 0
    globalThis.__cacheInvoke = async () => {
      calls++
      return [{ url: `https://manifest.googlevideo.com/api/manifest/hls_playlist/expire/${Math.floor(now / 1000) + 100}/id/fixture`, bitrate: 128_000, mime_type: 'application/vnd.apple.mpegurl', stream_type: 'hls' }]
    }
    await resolver.resolve(youtube, settings)
    now += 11_000
    await resolver.resolve(youtube, settings)
    assert.equal(calls, 2)
  })
  await run('downloads select the playable Bilibili candidate with its actual quality', async () => {
    globalThis.__cacheInvoke = async () => ({
      url: 'https://a.bilivideo.com/dolby', bandwidth: 256_000, codecs: 'ec-3', quality_key: 'dolby', mime_type: 'audio/mp4',
      candidates: [{ url: 'https://b.bilivideo.com/aac', bandwidth: 128_000, codecs: 'mp4a.40.2', quality_key: 'medium', mime_type: 'audio/mp4' }],
    })
    const selected = await source.resolveDownloadSource({ ...youtube, id: 'bilibili:BV1test', source: 'bilibili', album: 'Bilibili|42' }, { ...settings, biliQuality: 'dolby' })
    assert.equal(selected.url, 'https://b.bilivideo.com/aac')
    assert.equal(selected.audioInfo.qualityKey, 'medium')
    assert.equal(selected.codec, 'AAC')
  })
  await run('downloads reject unsupported codecs even when their container is MP4', async () => {
    globalThis.__cacheInvoke = async () => [{ url: 'https://rr.googlevideo.com/opus', bitrate: 256_000, mime_type: 'audio/mp4; codecs="opus"' }]
    await assert.rejects(source.resolveDownloadSource(youtube, settings), /supported audio codec/i)
  })
  await run('downloads use playable AAC quality selection and reject NetEase previews', async () => {
    globalThis.__cacheInvoke = async () => [
      { url: 'https://rr.googlevideo.com/opus', bitrate: 256_000, mime_type: 'audio/webm; codecs="opus"' },
      { url: 'https://rr.googlevideo.com/aac', bitrate: 128_000, mime_type: 'audio/mp4; codecs="mp4a.40.2"' },
    ]
    assert.equal((await source.resolveDownloadSource(youtube, settings)).url, 'https://rr.googlevideo.com/aac')
    globalThis.__cacheInvoke = async () => ({ url: 'https://m.music.126.net/preview', is_preview: true, level: 'exhigh' })
    await assert.rejects(source.resolveDownloadSource({ ...youtube, id: 'netease:123', source: 'netease' }, settings), /preview/i)
  })
  await run('track invalidation refreshes both direct and HLS resolutions', async () => {
    let calls = 0
    globalThis.__cacheInvoke = async (_, args) => [{
      url: `https://rr.googlevideo.com/${++calls}`,
      bitrate: 128_000, mime_type: 'audio/mp4', stream_type: args.avoidDirect ? 'hls' : 'direct',
    }]
    const resolver = new source.PlaybackUrlResolver()
    await resolver.resolve(youtube, settings)
    await resolver.resolve(youtube, settings, { avoidDirect: true })
    resolver.invalidate(youtube, settings)
    await resolver.resolve(youtube, settings)
    await resolver.resolve(youtube, settings, { avoidDirect: true })
    assert.equal(calls, 4)
  })
} finally { Date.now = realNow; delete globalThis.__cacheInvoke }
console.log(`playback cache tests: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
