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

const styleModule = await loadModule('../src/modules/desktopLyrics/style.ts')
const timelineModule = await loadModule('../src/modules/desktopLyrics/timeline.ts')
const frameModule = await loadModule('../src/modules/desktopLyrics/frame.ts', { './style': styleModule, './timeline': timelineModule })
const { buildDesktopLyricsFrame } = frameModule
const { createDesktopLyricsLoader } = await loadModule('../src/modules/desktopLyrics/loader.ts')
const { mergeWordTimedLyricsWithBaseline } = await loadModule('../src/modules/lyrics/lyricsFormat.ts')
const track = { id: 'netease:123', title: '互通', artist: '歌手', durationMs: 5000 }
const lines = [
  { startMs: 1000, durationMs: 1000, text: '第一行', translation: 'first', words: [] },
  { startMs: 2000, durationMs: 1000, text: '', words: [{ startMs: 2000, durationMs: 500, text: '逐字' }] },
  { startMs: 3000, durationMs: 1000, text: '第三行', roman: 'dai san', words: [] },
]
const style = styleModule.normalizeDesktopLyricsStyle(null)
const frame = (positionMs, lyricOffsetMs = 0, extra = {}) => buildDesktopLyricsFrame({
  track, lines, positionMs, lyricOffsetMs, isPlaying: true, rate: 1.25, now: 50_000, accent: null, style, ...extra,
})
// 帧带当前行附近的几行（上一行 + 当前 + 后三行），歌词窗口自己按锚点插值
assert.deepEqual(frame(0).lines.map(line => line.text), ['第一行', '逐字', '第三行'], '前奏时带上开头几行')
assert.equal(frame(0).firstIndex, 0)
assert.equal(frame(2000).firstIndex, 0, '当前第二行时从上一行开始带')
assert.equal(frame(2000).lines[1].words[0].text, '逐字', '逐字时间轴随帧带过去')
assert.equal(frame(2000).lines[1].text, '逐字', '只有逐字的行用字拼出文本')
assert.equal(frame(1000).lines[0].translation, 'first')
assert.equal(frame(3000).lines.at(-1).roman, 'dai san')
assert.equal(frameModule.desktopLyricsLineIndex(lines, 1500, 500), 1, '主窗口按加了偏移的位置判断换行')
assert.equal(frameModule.desktopLyricsLineIndex(lines, 3500, -1500), 1, '负偏移从当前位置往前算')
assert.equal(frameModule.desktopLyricsLineIndex(lines, Number.NaN, 0), -1)
assert.deepEqual(
  [frame(2000).positionMs, frame(2000).anchorAt, frame(2000).rate, frame(2000).offsetMs, frame(2000).isPlaying],
  [2000, 50_000, 1.25, 0, true],
  '锚点：位置、墙钟、倍速、偏移',
)
assert.equal(frame(2000, 300).offsetMs, 300)
assert.equal(frame(0, 0, { rate: 0 }).rate, 1, '无效倍速按 1')
assert.equal(frame(0, 0, { accent: '#ABCDEF' }).accent, '#abcdef')
assert.equal(frame(0, 0, { accent: 'red; x: y' }).accent, '', '主题色只收 #rrggbb')
assert.equal(frame(0, 0, { style: { layout: 'double', fontSize: 999 } }).style.fontSize, 96, '外观在发帧前规整')
const many = Array.from({ length: 20 }, (_, index) => ({ startMs: index * 1000, durationMs: 1000, text: `L${index}`, words: [] }))
const windowed = buildDesktopLyricsFrame({ track, lines: many, positionMs: 10_500, lyricOffsetMs: 0, isPlaying: true, rate: 1, now: 0, accent: null, style })
assert.equal(windowed.firstIndex, 9)
assert.deepEqual(windowed.lines.map(line => line.text), ['L9', 'L10', 'L11', 'L12', 'L13'], '最多 5 行')
const empty = buildDesktopLyricsFrame({ track: null, lines, positionMs: 5000, lyricOffsetMs: 0, isPlaying: true, rate: 1, now: 1, accent: null, style })
assert.deepEqual([empty.trackId, empty.lines.length, empty.isPlaying], ['', 0, false])
const longWords = Array.from({ length: 400 }, (_, index) => ({ startMs: index, durationMs: 1, text: '字'.repeat(100) }))
const bounded = buildDesktopLyricsFrame({
  track: { ...track, title: 'x'.repeat(5000) },
  lines: [{ startMs: 0, durationMs: 1, text: '😀'.repeat(5000), translation: '\\'.repeat(5000), words: longWords }],
  positionMs: 0, lyricOffsetMs: 0, isPlaying: true, rate: 1, now: 0, accent: null, style,
})
assert.ok(Buffer.byteLength(bounded.title) <= 1024)
assert.ok(Buffer.byteLength(bounded.lines[0].text) <= 1024)
assert.ok(!/[\uD800-\uDBFF]$/.test(bounded.lines[0].text), 'truncation must not split a surrogate pair')
assert.equal(bounded.lines[0].words.length, 128, '每行最多 128 个字')
assert.ok(bounded.lines[0].words.every(word => Buffer.byteLength(word.text) <= 64))
const worst = buildDesktopLyricsFrame({
  track, positionMs: 1, lyricOffsetMs: 0, isPlaying: true, rate: 1, now: 0, accent: null, style,
  lines: [0, 1, 2, 3, 4].map(startMs => ({ startMs, durationMs: 1, text: '\u0001'.repeat(4096), translation: '\\'.repeat(4096), roman: '"'.repeat(4096), words: longWords })),
})
assert.ok(Buffer.byteLength(JSON.stringify(worst)) < 128 * 1024, 'escaped text must still fit the backend frame budget')

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

const playerCalls = []
const player = {
  currentTrack: track, lyrics: [lines[0]], livePositionMs: () => 1000, isPlaying: true,
  effectivePlaybackSpeed: () => 1,
  pause: async () => { playerCalls.push('pause') },
  resume: async () => { playerCalls.push('resume') },
  next: async () => { playerCalls.push('next') },
  previous: async () => { playerCalls.push('previous') },
}
const settings = { preferWordTimedLyrics: false, defaultLyricSource: 'automatic', desktopLyrics: undefined }
const watchers = []
const intervals = new Map()
const invocations = []
const listeners = {}
const listener = event => listeners['desktop-lyrics:closed'](event)
let releasedListeners = 0
let openedSettings = 0
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
        if (options?.immediate) callback()
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
    '@tauri-apps/api/event': { listen: async (name, handler) => {
      listeners[name] = handler
      return () => { releasedListeners++ }
    } },
    '@/stores/player': { usePlayerStore: () => player },
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/stores/lyricOffset': { useLyricOffsetStore: () => ({ effectiveOffsetMs: () => 0 }) },
    '@/modules/lyrics/lyricOffset': { readSyncedLyricSource: () => null },
    '@/modules/lyrics/lyricsFetch': {
      fetchAutomaticLyrics: () => new Promise(resolve => { fetchRelease = fetched => resolve({ source: 'netease', lines: fetched }) }),
      fetchWordTimedLyrics: async () => ({ source: null, lines: [] }),
      preferredLyricMatchSource: () => null,
      fetchPreferredSourceLyrics: async () => null,
    },
    '@/modules/lyrics/lyricSource': { rememberLyricSource() {} },
    '@/modules/lyrics/lyricsCache': { getCachedLyrics: () => null, saveCachedLyrics: () => { cachedAfterClose++ } },
    '@/modules/lyrics/lyricsRequest': { loadLyricsSingleFlight: (_, fetch) => fetch(), hasWordTimedLyrics: () => false },
    '@/modules/lyrics/lyricsFormat': {
      resolveStoredLyricStateFromPayload: () => ({ kind: 'absent' }),
      materializeStoredLyrics: async () => null,
      mapBackendLyrics: value => value,
      mergeWordTimedLyricsWithBaseline: (_, value) => value,
    },
    './frame': frameModule,
    './loader': { createDesktopLyricsLoader },
    './style': styleModule,
    './timeline': timelineModule,
    '@/utils/logger': { createLogger: () => ({ warn() {} }) },
    '@/utils/logSanitizer': { summarizeLogError: String },
  })
  const dispose = installDesktopLyricsBridge({ openSettings: () => { openedSettings++ } })
  assert.equal(intervals.size, 0, 'inactive bridge must not start a timer')
  assert.equal(watchers.length, 0, 'inactive bridge must not fetch lyrics')
  await openDesktopLyricsWindow()
  await Promise.resolve()
  assert.equal(intervals.size, 1)
  assert.equal(watchers.length, 2, '歌词和锁定状态各一个')
  const opened = invocations.find(([command]) => command === 'open_desktop_lyrics')[1]
  assert.equal(opened.bounds, null, '第一次打开没有保存的位置，由后端放到屏幕底部居中')
  assert.deepEqual(invocations.find(([command]) => command === 'set_desktop_lyrics_lock')[1].locked, false, '打开后同步锁定状态')
  const frameCount = invocations.filter(([command]) => command === 'publish_desktop_lyrics').length
  for (const tick of intervals.values()) tick()
  await Promise.resolve()
  assert.equal(invocations.filter(([command]) => command === 'publish_desktop_lyrics').length, frameCount, '没变化时不发帧：歌词窗口自己插值')
  player.livePositionMs = () => 60_000
  for (const tick of intervals.values()) tick()
  await Promise.resolve()
  assert.equal(invocations.filter(([command]) => command === 'publish_desktop_lyrics').length, frameCount + 1, 'seek 后位置对不上锚点，马上发新帧')
  player.livePositionMs = () => 1000

  // 工具栏操作在主窗口执行
  listeners['desktop-lyrics:action']({ payload: { action: 'font-larger' } })
  assert.equal(settings.desktopLyrics.fontSize, 38)
  listeners['desktop-lyrics:action']({ payload: { action: 'cycle-layout' } })
  assert.equal(settings.desktopLyrics.layout, 'double')
  listeners['desktop-lyrics:action']({ payload: { action: 'toggle-play' } })
  listeners['desktop-lyrics:action']({ payload: { action: 'next' } })
  assert.deepEqual(playerCalls, ['pause', 'next'])
  listeners['desktop-lyrics:action']({ payload: { action: 'open-settings' } })
  assert.equal(openedSettings, 1)
  listeners['desktop-lyrics:action']({ payload: { action: 'lock' } })
  assert.equal(settings.desktopLyrics.locked, true)
  watchers[1].callback()
  assert.equal(invocations.filter(([command]) => command === 'set_desktop_lyrics_lock').at(-1)[1].locked, true, '锁定交给后端设置点击穿透')
  listeners['desktop-lyrics:bounds']({ payload: { x: 100.4, y: 900, width: 50, height: 200 } })
  assert.deepEqual(settings.desktopLyrics.bounds, { x: 100, y: 900, width: 360, height: 200 }, '记住位置，尺寸按下限规整')
  listeners['desktop-lyrics:bounds']({ payload: { x: 'nope' } })
  assert.deepEqual(settings.desktopLyrics.bounds, { x: 100, y: 900, width: 360, height: 200 }, '坏数据不覆盖已记住的位置')
  settings.desktopLyrics = undefined

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
  assert.equal(watchers[2].stopped, true)
  assert.equal(watchers[3].stopped, true)
  dispose()
  assert.equal(releasedListeners, 3, '关闭、工具栏操作、位置三个监听都要释放')
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
