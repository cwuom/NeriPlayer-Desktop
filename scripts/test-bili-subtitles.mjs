import assert from 'node:assert/strict'
import { loadModule, subtitle as s, deferred, flush } from './bili-subtitle-test-loader.mjs'
import { ref } from 'vue'

let cases = 0
async function test(name, run) { await run(); cases++; console.log(`PASS ${name}`) }
const preferences = { enabled: true, language: '', aiFallback: true }
const track = { id: 'bilibili:170001:22', title: 'Video', artist: 'Uploader', album: 'Bilibili|22|BV1xx411c7mD', durationMs: 60000 }
const row = (from, to, content) => ({ from, to, content })
const payload = rows => ({ body: rows })
const rawTrack = (lan, extra = {}) => ({ id_str: lan, lan, lan_doc: lan, subtitle_url: `//i0.hdslb.com/${lan}.json`, ...extra })
const tracks = (...languages) => s.parseBiliSubtitleTracks({ subtitles: languages.map(lan => rawTrack(lan)) })

await test('official URLs normalize and malformed or external destinations are rejected', () => {
  for (const url of ['//i0.hdslb.com/a?auth_key=x', 'http://i0.hdslb.com/a?auth_key=x', 'i0.hdslb.com/a?auth_key=x']) {
    assert.equal(s.normalizeBiliSubtitleUrl(url), 'https://i0.hdslb.com/a?auth_key=x')
  }
  for (const url of [null, '', 'https://hdslb.com.evil.test/x', 'https://evilhdslb.com/x', 'http://127.0.0.1', 'file:///etc/passwd', 'https://u:p@i0.hdslb.com/a', 'https://i0.hdslb.com:8080/a']) {
    assert.equal(s.normalizeBiliSubtitleUrl(url), null)
  }
  const result = s.parseBiliSubtitleTracks({ subtitles: [rawTrack('zh-CN', { id_str: '9007199254740993' }), rawTrack('ai-en'), rawTrack('ja', { type: 1 }), { lan: 'en' }, null] })
  assert.equal(result[0].id, '9007199254740993')
  assert.deepEqual(result.map(track => track.isAi), [false, true, true])
})

await test('targets preserve BV, AV, legacy numeric IDs, explicit parts and downloaded metadata', () => {
  assert.deepEqual(s.biliSubtitleTarget(track), { bvid: 'BV1xx411c7mD', avid: null, cid: 22 })
  assert.deepEqual(s.biliSubtitleTarget({ ...track, album: '' }), { bvid: '', avid: 170001, cid: 22 })
  assert.deepEqual(s.biliSubtitleTarget({ ...track, id: 'bilibili:1700010002', album: '' }), { bvid: '', avid: 1700010002, cid: null })
  assert.deepEqual(s.biliSubtitleTarget({ ...track, id: 'local:file', source: 'local', album: '', syncPayload: { channelId: 'bilibili', audioId: '170001', subAudioId: '33' } }), { bvid: '', avid: 170001, cid: 33 })
  assert.equal(s.biliSubtitleTarget({ ...track, id: 'netease:1', album: '' }), null)
})

await test('language aliases distinguish Chinese scripts and explicit AI selections', () => {
  for (const [a, b, expected] of [['zh-CN', 'zh-Hans', true], ['zh-SG', 'zh-Hans', true], ['zh-TW', 'zh-Hant', true], ['zh-HK', 'zh-CN', false], ['en-US', 'en', true], ['AI-ZH', 'ai-zh', true], ['ai-zh', 'zh', false], ['ai-zh', 'ai-zh-CN', false]]) {
    assert.equal(s.matchesBiliSubtitleLanguage(a, b), expected)
  }
  const candidates = tracks('en', 'ai-en', 'ai-zh', 'zh-CN', 'ja')
  assert.deepEqual(s.biliSubtitleCandidates(candidates, preferences).map(t => t.language), ['zh-CN', 'ai-zh', 'en', 'ja', 'ai-en'])
  assert.deepEqual(s.biliSubtitleCandidates(candidates, { ...preferences, aiFallback: false }).map(t => t.language), ['zh-CN', 'en', 'ja'])
  assert.equal(s.biliSubtitleCandidates(candidates, { ...preferences, aiFallback: false, language: 'ai-en' })[0].language, 'ai-en')
  assert.equal(s.translatedBiliSubtitleTrack(candidates[3], tracks('ai-zh', 'zh-TW', 'en'), true).language, 'en')
})

await test('cue parsing handles seconds, missing ends, duplicates and malformed entries', () => {
  assert.deepEqual(s.parseBiliSubtitleCues(payload([row(2, 1, ' B '), row('0.125', '1.5', 'A'), row(.125, 1.5, 'A'), row(null, 2, 'bad'), row(-1, 1, 'bad'), row(Infinity, 4, 'bad'), row(4, 5, ' '), row(5, null, 'C'), null])), [
    { startMs: 125, endMs: 1500, text: 'A' }, { startMs: 2000, endMs: 2001, text: 'B' }, { startMs: 5000, endMs: 5001, text: 'C' },
  ])
})

const align = (a, b) => s.alignBiliSubtitleLyrics(s.parseBiliSubtitleCues(payload(a)), s.parseBiliSubtitleCues(payload(b)))
await test('a long translation stays on every matching original row without moving to the next sentence', () => {
  const result = align([row(50, 52, 'a'), row(52, 54, 'b'), row(54, 57, 'c'), row(57, 60, 'd')], [row(50, 54, 'AB'), row(54, 57, 'C'), row(57, 60, 'D')])
  assert.deepEqual(result.map(line => line.translation), ['AB', 'AB', 'C', 'D'])
  assert.ok(result.every(line => line.words.length === 0))
})
await test('short translations combine; visible cue intervals stop at the next start', () => {
  assert.equal(align([row(0, 6, 'long')], [row(0, 2, 'A'), row(2, 4, 'B'), row(4, 6, 'C')])[0].translation, 'A\nB\nC')
  const result = align([row(0, 10, 'a'), row(4, 8, 'b')], [row(0, 10, 'A'), row(4, 8, 'B')])
  assert.deepEqual(result.map(line => [line.durationMs, line.translation]), [[4000, 'A'], [4000, 'B']])
})
await test('small boundary overlaps, gaps and placeholder translations do not attach to adjacent sentences', () => {
  const result = align([row(0, 2, 'a'), row(2, 4, 'b'), row(8, 9, 'c')], [row(1.9, 4, 'B'), row(4, 7, 'gap'), row(8, 9, '//')])
  assert.deepEqual(result.map(line => line.translation), [undefined, 'B', undefined])
  assert.equal(align([row(0, 1, '作曲: spoken words')], [row(0, 1, '作词: credit')])[0].text, '作曲: spoken words')
})

function runtime(handler) {
  const calls = []
  const listeners = new Map()
  globalThis.window = { addEventListener: (name, callback) => listeners.set(name, callback) }
  const loader = loadModule('../src/modules/lyrics/biliSubtitleLoader.ts', {
    vue: { ref }, './biliSubtitle': s,
    '@tauri-apps/api/core': { invoke: async (command, args) => { calls.push([command, args]); return handler(command, args, calls) } },
  })
  delete globalThis.window
  return { ...loader, calls, listeners }
}
const metadata = (subtitles = [rawTrack('zh-CN'), rawTrack('en')]) => ({ bvid: 'BV1xx411c7mD', cid: 22, loginRequired: false, subtitle: { subtitles } })
const defaultHandler = (command, args) => command === 'get_bili_subtitle_tracks' ? metadata() : payload([row(0, 2, args.url.includes('/en.') ? 'translation' : 'original')])

await test('simultaneous consumers share requests and complete bilingual captions are cached in memory', async () => {
  const r = runtime(defaultHandler)
  const [a, b] = await Promise.all([r.loadBiliSubtitleLyrics(track, preferences), r.loadBiliSubtitleLyrics(track, preferences)])
  assert.deepEqual(a, b)
  assert.equal(a.source, 'bilibili_cc')
  assert.equal(a.lines[0].translation, 'translation')
  assert.equal(r.calls.length, 3)
  await r.loadBiliSubtitleLyrics(track, preferences)
  assert.equal(r.calls.length, 3)
  const english = await r.loadBiliSubtitleLyrics(track, { ...preferences, language: 'en' })
  assert.equal(english.track.language, 'en')
  assert.equal(english.lines[0].translation, 'original')
  await r.loadBiliSubtitleLyrics({ ...track, id: 'bilibili:170001:33', album: 'Bilibili|33|BV1xx411c7mD' }, preferences)
  assert.equal(r.calls.filter(([command]) => command === 'get_bili_subtitle_tracks').length, 2, 'parts have separate cache entries')
})
await test('disabled subtitles issue no requests and login-required results are retryable', async () => {
  const r = runtime(() => ({ ...metadata([]), loginRequired: true }))
  assert.deepEqual((await r.loadBiliSubtitleLyrics(track, { ...preferences, enabled: false })).lines, [])
  assert.equal(r.calls.length, 0)
  assert.equal((await r.loadBiliSubtitleTracks(track)).loginRequired, true)
  await r.loadBiliSubtitleLyrics(track, preferences)
  assert.equal(r.calls.length, 2)
})
await test('failed preferred and translated tracks return usable content without caching partial results', async () => {
  let broken = true
  const r = runtime((command, args) => {
    if (command === 'get_bili_subtitle_tracks') return metadata()
    if (broken && args.url.includes('/en.')) throw new Error('expired')
    return defaultHandler(command, args)
  })
  const fallback = await r.loadBiliSubtitleLyrics(track, { ...preferences, language: 'en' })
  assert.equal(fallback.track.language, 'zh-CN')
  assert.equal(fallback.translatedTrack, undefined)
  broken = false
  const retried = await r.loadBiliSubtitleLyrics(track, { ...preferences, language: 'en' })
  assert.equal(retried.track.language, 'en')
  assert.equal(retried.translatedTrack.language, 'zh-CN')
  assert.equal(r.calls.filter(([command]) => command === 'get_bili_subtitle_tracks').length, 2)
})
await test('failed translation fetches remain retryable even when the original track matched', async () => {
  let fail = true
  const r = runtime((command, args) => {
    if (command !== 'get_bili_subtitle_tracks' && args.url.includes('/en.') && fail) throw new Error('offline')
    return defaultHandler(command, args)
  })
  assert.equal((await r.loadBiliSubtitleLyrics(track, preferences)).lines[0].translation, undefined)
  fail = false
  assert.equal((await r.loadBiliSubtitleLyrics(track, preferences)).lines[0].translation, 'translation')
})
await test('cache invalidation rejects late parsed captions', async () => {
  const late = deferred()
  let block = true
  const r = runtime((command, args) => command === 'get_bili_subtitle_payload' && block ? late.promise : defaultHandler(command, args))
  const old = r.loadBiliSubtitleLyrics(track, preferences)
  await flush()
  r.invalidateBiliSubtitleCache()
  block = false
  const fresh = await r.loadBiliSubtitleLyrics(track, preferences)
  late.resolve(payload([row(0, 2, 'obsolete')]))
  assert.deepEqual((await old).lines, [])
  assert.equal((await r.loadBiliSubtitleLyrics(track, preferences)).lines[0].text, fresh.lines[0].text)
})
await test('old metadata cannot replace a refreshed signed track list after invalidation', async () => {
  const late = deferred()
  let count = 0
  const r = runtime(() => ++count === 1 ? late.promise : metadata([rawTrack('ja')]))
  const old = r.loadBiliSubtitleTracks(track)
  r.invalidateBiliSubtitleCache()
  await r.loadBiliSubtitleTracks(track)
  late.resolve(metadata([rawTrack('en')]))
  await old
  assert.equal((await r.loadBiliSubtitleTracks(track)).tracks[0].language, 'ja')
  assert.equal(count, 2)
})
await test('authentication changes clear both caption and signed track caches', async () => {
  const r = runtime(defaultHandler)
  await r.loadBiliSubtitleLyrics(track, preferences)
  r.listeners.get('neri:auth-changed')({ detail: { platform: 'bilibili' } })
  await r.loadBiliSubtitleLyrics(track, preferences)
  assert.equal(r.calls.length, 6)
})

console.log(`Bilibili subtitle regressions: ${cases} passed`)
