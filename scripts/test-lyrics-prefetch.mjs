import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function loadModule(path, dependencies) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const key = `lyrics-prefetch-test-${Math.random()}`
  globalThis[key] = dependencies
  compiled = compiled.replace(/import\s*\{([\s\S]*?)\}\s*from\s*['"]([^'"]+)['"];?/g, (_, bindings, specifier) => {
    assert.ok(specifier in dependencies, `unexpected dependency ${specifier}`)
    return `const { ${bindings.replace(/\bas\b/g, ':')} } = globalThis[${JSON.stringify(key)}][${JSON.stringify(specifier)}];`
  })
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
  } finally {
    delete globalThis[key]
  }
}

const timed = [{ startMs: 0, durationMs: 1000, text: '你好', words: [{ startMs: 0, durationMs: 500, text: '你' }] }]
const plain = [{ startMs: 0, durationMs: 1000, text: '你好', words: [] }]
const hasWordTimedLyrics = lines => lines.some(line => line.words.some(word => word.text.trim() && word.durationMs > 0))

// 共享的自动取词：B 站开逐字优先先试逐字，没有才走后端瀑布；平台 ID 从曲目 ID 里拆
const invocations = []
let wordTimedResult = { source: 'amll_ttml', lines: timed }
const fetchModule = await loadModule('../src/modules/lyrics/lyricsFetch.ts', {
  '@tauri-apps/api/core': { invoke: async (command, args) => {
    invocations.push([command, args])
    if (command === 'fetch_word_timed_lyrics') return wordTimedResult
    if (command === 'fetch_lyrics') return { source: 'netease', lines: plain }
    return null
  } },
  './lyricsFormat': { mapBackendLyrics: lines => lines, toEditableRomanizationText: () => '' },
  './lyricMatch': { defaultLyricMatchKeyword: () => '', matchLyrics: async () => [] },
  './lyricsRequest': { hasWordTimedLyrics },
})
const { fetchAutomaticLyrics, fetchLyricsArgsFor } = fetchModule

assert.deepEqual(fetchLyricsArgsFor({ id: 'netease:42', title: 'T', artist: 'A', durationMs: 183_900 }), {
  title: 'T', artist: 'A', durationSecs: 183, audioPath: null, neteaseId: 42, qqSongMid: null, youtubeVideoId: null,
})
assert.equal(fetchLyricsArgsFor({ id: 'qq:003abc', title: 'T', artist: 'A' }).qqSongMid, '003abc')
assert.equal(fetchLyricsArgsFor({ id: 'youtube:dQw4w9', title: 'T', artist: 'A' }).youtubeVideoId, 'dQw4w9')
assert.equal(fetchLyricsArgsFor({ id: 'netease:abc', title: 'T', artist: 'A' }).neteaseId, null, 'non-numeric NetEase ids must not become NaN')
assert.equal(fetchLyricsArgsFor({ id: 'bilibili:BV1', title: 'T', artist: 'A', durationMs: 0 }).durationSecs, 0)

const bili = { id: 'bilibili:BV1', title: 'T', artist: 'A', durationMs: 60_000 }
assert.equal((await fetchAutomaticLyrics(bili, 'bilibili', true)).source, 'amll_ttml')
assert.deepEqual(invocations.map(([command]) => command), ['fetch_word_timed_lyrics'], 'word timed hit skips the waterfall')
invocations.length = 0
wordTimedResult = { source: 'kugou', lines: plain }
assert.equal((await fetchAutomaticLyrics(bili, 'bilibili', true)).source, 'netease', 'line-timed result from the word-timed path falls back')
invocations.length = 0
await fetchAutomaticLyrics({ id: 'netease:7', title: 'T', artist: 'A' }, 'netease', true)
assert.deepEqual(invocations.map(([command]) => command), ['fetch_lyrics'], 'only Bilibili tries word-timed lyrics first')

// 预取判定顺序与正在播放页一致
function prefetchRuntime(state = {}) {
  const calls = []
  const sources = new Map(Object.entries(state.sources ?? {}))
  const deps = {
    './lyricsCache': {
      getCachedLyrics: async () => { calls.push('cache-read'); return state.cached ?? null },
      saveCachedLyrics: async (_, lines, source) => { calls.push(['cache-write', source, lines.length]) },
    },
    './lyricsFetch': {
      fetchAutomaticLyrics: async () => { calls.push('automatic'); return state.automatic ?? { source: 'netease', lines: plain } },
      fetchPreferredSourceLyrics: async (_, source) => { calls.push(['preferred', source]); return state.preferred ?? null },
      preferredLyricMatchSource: (_, preference) => (preference === 'automatic' ? null : preference),
    },
    './lyricsFormat': { resolveStoredLyricStateFromPayload: payload => ({ kind: payload?.kind ?? 'absent' }) },
    './lyricOffset': { normalizeLyricSource: value => value ?? null },
    './lyricsRequest': { loadLyricsSingleFlight: (_, loader) => loader() },
    './lyricSource': {
      lyricSourceOf: track => sources.get(track.id) ?? null,
      rememberLyricSource: (track, source) => { calls.push(['remember', source]); sources.set(track.id, source) },
    },
  }
  return { calls, deps }
}
const options = (defaultLyricSource = 'automatic') => ({ playbackSource: 'netease', defaultLyricSource, preferWordTimed: true })
const track = { id: 'netease:1', source: 'netease', title: 'T', artist: 'A', durationMs: 60_000 }

{
  const r = prefetchRuntime()
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics({ ...track, id: 'local:1', source: 'local' }, options()), 'skipped')
  assert.equal(await prefetchTrackLyrics({ ...track, syncPayload: { kind: 'present' } }, options()), 'skipped', 'synced lyrics need no network')
  assert.equal(await prefetchTrackLyrics({ ...track, syncPayload: { kind: 'cleared' } }, options()), 'skipped', 'cleared lyrics must stay cleared')
  assert.deepEqual(r.calls, [], 'skipped tracks touch neither cache nor network')
}
{
  const r = prefetchRuntime({ cached: plain })
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics(track, options()), 'cached')
  assert.deepEqual(r.calls, ['cache-read'])
}
{
  const r = prefetchRuntime()
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics(track, options()), 'fetched')
  assert.deepEqual(r.calls, ['cache-read', 'automatic', ['remember', 'netease'], ['cache-write', 'netease', 1]])
}
{
  const r = prefetchRuntime({ automatic: { source: null, lines: [] } })
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics(track, options()), 'empty')
  assert.ok(!r.calls.some(call => Array.isArray(call) && call[0] === 'cache-write'), 'no lyrics must not write a negative cache')
}
{
  // 缓存来源不是默认歌词源：先按默认源匹配，命中后覆盖缓存
  const r = prefetchRuntime({ cached: plain, sources: { 'netease:1': 'netease' }, preferred: { source: 'kugou', lines: timed } })
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics(track, options('kugou')), 'fetched')
  assert.deepEqual(r.calls, ['cache-read', ['preferred', 'kugou'], ['remember', 'kugou'], ['cache-write', 'kugou', 1]])
}
{
  // 默认歌词源没匹配上时保留已有缓存，不再走自动取词
  const r = prefetchRuntime({ cached: plain, sources: { 'netease:1': 'netease' } })
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics(track, options('kugou')), 'cached')
  assert.deepEqual(r.calls, ['cache-read', ['preferred', 'kugou']])
}
{
  // 缓存已是默认歌词源：直接命中
  const r = prefetchRuntime({ cached: plain, sources: { 'netease:1': 'kugou' } })
  const { prefetchTrackLyrics } = await loadModule('../src/modules/lyrics/lyricsPrefetch.ts', r.deps)
  assert.equal(await prefetchTrackLyrics(track, options('kugou')), 'cached')
  assert.deepEqual(r.calls, ['cache-read'])
}

console.log('lyrics prefetch tests passed')
