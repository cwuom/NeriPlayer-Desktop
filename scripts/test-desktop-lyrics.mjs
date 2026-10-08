import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function loadModule(path, dependencies) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  compiled += `\n//# sourceURL=${path.split('/').at(-1)}.test.js\n`
  const key = `desktop-lyrics-test-${Math.random()}`
  if (dependencies) {
    globalThis[key] = dependencies
    compiled = compiled.replace(/import\s*\{([\s\S]*?)\}\s*from\s*['"]([^'"]+)['"];?/g, (_, bindings, specifier) => {
      assert.ok(specifier in dependencies, `unexpected dependency ${specifier}`)
      return `const { ${bindings.replace(/\bas\b/g, ':')} } = globalThis[${JSON.stringify(key)}][${JSON.stringify(specifier)}];`
    })
  }
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
  } finally {
    delete globalThis[key]
  }
}

const { buildDesktopLyricsFrame } = await loadModule('../src/modules/desktopLyrics/frame.ts')
const { createDesktopLyricsLoader } = await loadModule('../src/modules/desktopLyrics/loader.ts')
const { mergeWordTimedLyricsWithBaseline } = await loadModule('../src/modules/lyrics/lyricsFormat.ts')
const track = { id: 'netease:123', title: '互通', artist: '歌手', durationMs: 5000 }
const lines = [
  { startMs: 1000, durationMs: 1000, text: '第一行', translation: 'first', words: [] },
  { startMs: 2000, durationMs: 1000, text: '', words: [{ startMs: 2000, durationMs: 500, text: '逐字' }] },
  { startMs: 3000, durationMs: 1000, text: '第三行', words: [] },
]
const frame = (positionMs, lyricOffsetMs = 0) => buildDesktopLyricsFrame({ track, lines, positionMs, lyricOffsetMs, isPlaying: false })
assert.equal(frame(0).current, '')
assert.equal(frame(0).next, '第一行')
assert.equal(frame(1000).current, '第一行')
assert.equal(frame(1000).translation, 'first')
assert.equal(frame(2000).current, '逐字')
assert.equal(frame(2000).previous, '第一行')
assert.equal(frame(2000).next, '第三行')
assert.equal(frame(1500, 500).current, '逐字')
assert.equal(frame(3500, -1500).current, '逐字', 'seeking and negative offset must select from the current position')
assert.equal(frame(Number.NaN).current, '')
assert.equal(frame(2000).isPlaying, false)
assert.deepEqual(buildDesktopLyricsFrame({ track: null, lines, positionMs: 5000, lyricOffsetMs: 0, isPlaying: true }), {
  trackId: '', title: '', artist: '', previous: '', current: '', next: '', translation: '', isPlaying: false,
})
const bounded = buildDesktopLyricsFrame({ track: { ...track, title: 'x'.repeat(5000) }, lines: [{ ...lines[0], text: '😀'.repeat(5000) }], positionMs: 2000, isPlaying: true, lyricOffsetMs: 0 })
assert.ok(bounded.title.length <= 1024)
assert.ok(Buffer.byteLength(bounded.current) <= 4096)
assert.ok(!/[\uD800-\uDBFF]$/.test(bounded.current), 'truncation must not split a surrogate pair')
const escaped = buildDesktopLyricsFrame({ track, lines: [0, 1, 2].map(startMs => ({ ...lines[0], startMs, text: '\u0001'.repeat(4096), translation: '\\'.repeat(4096) })), positionMs: 1, isPlaying: true, lyricOffsetMs: 0 })
assert.ok(Buffer.byteLength(JSON.stringify(escaped)) < 24 * 1024, 'escaped text must still fit the backend frame budget')

let changes = []
let fetchCalls = 0
let upgradeCalls = 0
let stored = null
let cache = null
let resolveFetch
let resolveUpgrade
const loader = createDesktopLyricsLoader({
  materialize: async () => stored,
  cached: () => cache,
  fetch: () => { fetchCalls++; return new Promise(resolve => { resolveFetch = resolve }) },
  cache: () => {},
  mergeUpgrade: (_, upgrade) => upgrade,
  canUpgrade: () => true,
  upgrade: () => { upgradeCalls++; return new Promise(resolve => { resolveUpgrade = resolve }) },
  onChange: value => changes.push(value),
})
stored = []
await loader.load(track)
assert.equal(fetchCalls, 0, 'intentional sync clear must never refill online')
assert.equal(upgradeCalls, 0)
stored = [lines[0]]
await loader.load(track)
assert.equal(changes.at(-1)[0].text, '第一行')
assert.equal(upgradeCalls, 0, 'synced lyrics must not be replaced by external word matching')
stored = null
cache = [lines[0]]
await loader.load(track)
assert.equal(changes.at(-1)[0].text, '第一行', 'cached baseline must display before an upgrade resolves')
assert.equal(upgradeCalls, 1)
loader.dispose()
const beforeDispose = changes.length
resolveUpgrade({ source: 'amll_ttml', lines: [lines[1]] })
await Promise.resolve()
await Promise.resolve()
assert.equal(changes.length, beforeDispose, 'closing the window must ignore late upgrades')

changes = []
cache = null
let currentResolve
const guarded = createDesktopLyricsLoader({
  materialize: async () => null,
  cached: () => null,
  fetch: t => new Promise(resolve => {
    if (t.id === track.id) resolveFetch = resolve
    else currentResolve = resolve
  }),
  cache: () => {}, canUpgrade: () => false, upgrade: async () => ({ source: null, lines: [] }),
  mergeUpgrade: (_, upgrade) => upgrade,
  onChange: value => changes.push(value),
})
const old = guarded.load(track)
// 让旧曲目的请求先真正发出，再切到新曲目
await flushMicrotasks()
const fresh = guarded.load({ ...track, id: 'youtube:new' })
await flushMicrotasks()
resolveFetch([lines[0]])
await old
assert.equal(changes.at(-1).length, 0, 'old request cannot display on a new song')
currentResolve([lines[2]])
await fresh
assert.equal(changes.at(-1)[0].text, '第三行')
guarded.dispose()

const rejected = createDesktopLyricsLoader({
  materialize: async () => null, cached: () => [lines[0]],
  fetch: async () => { throw new Error('offline') }, cache: () => {},
  canUpgrade: () => true, upgrade: async () => { throw new Error('offline upgrade') },
  mergeUpgrade: (_, upgrade) => upgrade,
  onChange: value => changes.push(value),
})
await rejected.load(track)
await Promise.resolve()
assert.equal(changes.at(-1)[0].text, '第一行')
rejected.dispose()

// 升级返回时用户已改了歌词（canUpgrade 变 false）：结果作废，来源也不能被换成 TTML
const adoptedSources = []
let allowUpgrade = true
let resolveRejectedUpgrade
const discarded = createDesktopLyricsLoader({
  materialize: async () => null, cached: () => [lines[0]],
  fetch: async () => [], cache: () => {},
  canUpgrade: () => allowUpgrade,
  upgrade: () => new Promise(resolve => { resolveRejectedUpgrade = resolve }),
  mergeUpgrade: (_, upgrade) => upgrade,
  adoptSource: (_, source) => adoptedSources.push(['adopt', source]),
  onChange: () => {},
})
await discarded.load(track)
allowUpgrade = false
resolveRejectedUpgrade({ source: 'amll_ttml', lines: [lines[1]] })
await flushMicrotasks()
assert.deepEqual(adoptedSources, [], 'a discarded upgrade must not change the lyric source')
discarded.dispose()

let mergedDisplay
let mergedCache
const preserveText = createDesktopLyricsLoader({
  materialize: async () => null,
  cached: () => [{ ...lines[0], roman: 'dai ichi' }],
  fetch: async () => [],
  cache: (_, value) => { mergedCache = value },
  canUpgrade: () => true,
  upgrade: async () => ({ source: 'amll_ttml', lines: [{ ...lines[0], startMs: 1200, translation: undefined, words: [{ startMs: 1200, durationMs: 500, text: '第一行' }] }] }),
  mergeUpgrade: mergeWordTimedLyricsWithBaseline,
  adoptSource: (_, source) => adoptedSources.push(['adopt', source]),
  onChange: value => { mergedDisplay = value; adoptedSources.push(['display']) },
})
await preserveText.load(track)
await Promise.resolve()
assert.deepEqual(adoptedSources.slice(-2), [['adopt', 'amll_ttml'], ['display']],
  'an applied upgrade records its source before display and cache pick defaults from it')
assert.equal(mergedDisplay[0].startMs, 1200)
assert.equal(mergedDisplay[0].translation, 'first', 'external word timing must preserve matching baseline translation')
assert.equal(mergedDisplay[0].roman, 'dai ichi')
assert.deepEqual(mergedCache, mergedDisplay, 'the cache must retain the same translation and roman as the display')
preserveText.dispose()

const player = { currentTrack: track, lyrics: [lines[0]], livePositionMs: () => 1000, isPlaying: true }
const settings = { advancedLyrics: false }
const watchers = []
const intervals = new Map()
const invocations = []
let listener
let releasedListeners = 0
let rejectOpen = false
let deferredOpens = false
const openRequests = []
let nativeSession = ''
let childExists = false
async function flushMicrotasks() { for (let i = 0; i < 6; i++) await Promise.resolve() }
let fetchRelease
let cachedAfterClose = 0
const originalInterval = globalThis.setInterval
const originalClearInterval = globalThis.clearInterval
globalThis.setInterval = callback => { const id = intervals.size + 1; intervals.set(id, callback); return id }
globalThis.clearInterval = id => intervals.delete(id)
try {
  const { installDesktopLyricsBridge, openDesktopLyricsWindow } = await loadModule('../src/modules/desktopLyrics/bridge.ts', {
    vue: {
      ref: value => ({ value }),
      watch: (_, callback, options) => {
        const watcher = { callback, stopped: false }
        watchers.push(watcher)
        if (options.immediate) callback()
        return () => { watcher.stopped = true }
      },
    },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      invocations.push([command, args])
      if (command === 'open_desktop_lyrics' && rejectOpen) throw new Error('window creation failed')
      if (command === 'open_desktop_lyrics') {
        nativeSession = args.sessionId
        childExists = true
        if (deferredOpens) return new Promise(resolve => openRequests.push({ sessionId: args.sessionId, resolve }))
      }
      if (command === 'close_desktop_lyrics' && args.sessionId === nativeSession) childExists = false
      if (command === 'fetch_lyrics') return new Promise(resolve => { fetchRelease = resolve })
    } },
    '@tauri-apps/api/event': { listen: async (_, handler) => {
      listener = handler
      return () => { releasedListeners++ }
    } },
    '@/stores/player': { usePlayerStore: () => player },
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/stores/lyricOffset': { useLyricOffsetStore: () => ({ effectiveOffsetMs: () => 0 }) },
    '@/modules/lyrics/lyricOffset': { readSyncedLyricSource: () => null },
    '@/modules/lyrics/lyricsFetch': {
      fetchLyrics: () => new Promise(resolve => { fetchRelease = fetched => resolve({ source: 'netease', lines: fetched }) }),
      fetchWordTimedLyrics: async () => ({ source: null, lines: [] }),
    },
    '@/modules/lyrics/lyricSource': { rememberLyricSource() {} },
    '@/modules/lyrics/lyricsCache': { getCachedLyrics: () => null, saveCachedLyrics: () => { cachedAfterClose++ } },
    '@/modules/lyrics/lyricsRequest': { loadLyricsSingleFlight: (_, fetch) => fetch(), hasWordTimedLyrics: () => false },
    '@/modules/lyrics/lyricsFormat': {
      resolveStoredLyricStateFromPayload: () => ({ kind: 'absent' }),
      resolveStoredTranslatedLyricStateFromPayload: () => ({ kind: 'absent' }),
      mapBackendLyrics: value => value,
      mergeParsedLyricsWithTranslations: value => value,
      mergeWordTimedLyricsWithBaseline: (_, value) => value,
    },
    './frame': { buildDesktopLyricsFrame },
    './loader': { createDesktopLyricsLoader },
    '@/utils/logger': { createLogger: () => ({ warn() {} }) },
    '@/utils/logSanitizer': { summarizeLogError: String },
  })
  const dispose = installDesktopLyricsBridge()
  assert.equal(intervals.size, 0, 'inactive bridge must not start a timer')
  assert.equal(watchers.length, 0, 'inactive bridge must not fetch lyrics')
  await openDesktopLyricsWindow()
  await Promise.resolve()
  assert.equal(intervals.size, 1)
  assert.equal(watchers.length, 1)
  const frameCount = invocations.filter(([command]) => command === 'publish_desktop_lyrics').length
  for (const tick of intervals.values()) tick()
  await Promise.resolve()
  assert.equal(invocations.filter(([command]) => command === 'publish_desktop_lyrics').length, frameCount, 'identical frames must not flood IPC')

  player.currentTrack = { ...track, id: 'youtube:late' }
  player.lyrics = []
  watchers[0].callback()
  // 缓存读取是异步的，等在线请求真正发出后再关窗
  await flushMicrotasks()
  const firstSession = invocations.find(([command]) => command === 'open_desktop_lyrics')[1].sessionId
  listener({ payload: { sessionId: firstSession } })
  assert.equal(intervals.size, 0)
  assert.equal(watchers[0].stopped, true)
  fetchRelease([lines[2]])
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(cachedAfterClose, 0, 'late responses after close cannot mutate the lyrics cache')

  rejectOpen = true
  await assert.rejects(openDesktopLyricsWindow(), /window creation failed/)
  assert.equal(intervals.size, 0, 'failed window creation must clean up the timer')
  assert.equal(watchers[1].stopped, true)
  dispose()
  assert.equal(releasedListeners, 1)
  assert.equal(intervals.size, 0)

  rejectOpen = false
  deferredOpens = true
  player.lyrics = [lines[0]]
  const disposeSingle = installDesktopLyricsBridge()
  const firstOpen = openDesktopLyricsWindow()
  const secondOpen = openDesktopLyricsWindow()
  await flushMicrotasks()
  assert.equal(openRequests.length, 1, 'concurrent clicks must share one native open request')
  openRequests[0].resolve()
  await Promise.all([firstOpen, secondOpen])
  assert.equal(intervals.size, 1)
  listener({ payload: { sessionId: crypto.randomUUID() } })
  assert.equal(intervals.size, 1, 'an old window destroy event cannot deactivate the current session')
  disposeSingle()
  assert.equal(intervals.size, 0)

  const disposeOld = installDesktopLyricsBridge()
  const oldOpen = openDesktopLyricsWindow()
  await flushMicrotasks()
  assert.equal(openRequests.length, 2)
  disposeOld()
  assert.equal(intervals.size, 0)
  const disposeNew = installDesktopLyricsBridge()
  const newOpen = openDesktopLyricsWindow()
  await flushMicrotasks()
  assert.equal(openRequests.length, 3)
  openRequests[2].resolve()
  await newOpen
  assert.equal(intervals.size, 1)
  openRequests[1].resolve()
  await oldOpen
  assert.equal(intervals.size, 1, 'late old open cleanup cannot stop the new bridge')
  assert.equal(childExists, true, 'session-bound cleanup must preserve a child adopted by the new bridge')
  const cleanup = invocations.filter(([command]) => command === 'close_desktop_lyrics').at(-1)[1]
  assert.equal(cleanup.sessionId, openRequests[1].sessionId)
  assert.notEqual(cleanup.sessionId, nativeSession)
  listener({ payload: { sessionId: openRequests[1].sessionId } })
  assert.equal(intervals.size, 1)
  disposeNew()
  assert.equal(intervals.size, 0)
  assert.equal(childExists, false)
} finally {
  globalThis.setInterval = originalInterval
  globalThis.clearInterval = originalClearInterval
}

console.log('desktop lyrics frame, loader and bridge lifecycle tests passed')
