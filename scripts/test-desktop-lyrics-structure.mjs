import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function readOptional(relativePath) {
  try {
    return await readFile(new URL(relativePath, import.meta.url), 'utf8')
  } catch {
    return ''
  }
}

const [bridgeSource, activeLineSource, windowPositionSource, lyricLoaderSource, windowSource, mainSource, nowPlayingSource, settingsSource, capabilitySource, lyricsCapabilitySource, tauriConfigSource] = await Promise.all([
  readOptional('../src/modules/desktopLyrics/bridge.ts'),
  readOptional('../src/modules/desktopLyrics/activeLine.ts'),
  readOptional('../src/modules/desktopLyrics/windowPosition.ts'),
  readOptional('../src/modules/lyrics/loadTrackLyrics.ts'),
  readOptional('../src/components/DesktopLyricsWindow.vue'),
  readOptional('../src/main.ts'),
  readOptional('../src/components/NowPlaying.vue'),
  readOptional('../src/views/SettingsView.vue'),
  readOptional('../src-tauri/capabilities/default.json'),
  readOptional('../src-tauri/capabilities/desktop-lyrics.json'),
  readOptional('../src-tauri/tauri.conf.json'),
])

assert.ok(bridgeSource, 'desktop lyrics bridge module must exist')
assert.match(bridgeSource, /new WebviewWindow\(DESKTOP_LYRICS_WINDOW_LABEL/)
for (const option of [
  'transparent: true',
  'decorations: false',
  'alwaysOnTop: true',
  'skipTaskbar: true',
]) {
  assert.ok(bridgeSource.includes(option), `desktop lyrics window must use ${option}`)
}
assert.match(bridgeSource, /emitTo\(DESKTOP_LYRICS_WINDOW_LABEL, DESKTOP_LYRICS_STATE_EVENT/)
assert.match(bridgeSource, /listen\(DESKTOP_LYRICS_READY_EVENT/)
assert.match(lyricLoaderSource, /loadLyricsSingleFlight/)
assert.match(lyricLoaderSource, /getCachedLyrics/)
assert.match(bridgeSource, /effectiveOffsetMs/)

assert.ok(activeLineSource, 'desktop lyric active-line resolver must exist')
const compiledActiveLine = ts.transpileModule(activeLineSource, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const activeLineModuleUrl = `data:text/javascript;base64,${Buffer.from(compiledActiveLine).toString('base64')}`
const { resolveDesktopLyricLine, resolveNextDesktopLyricLine } = await import(activeLineModuleUrl)
const lyricLines = [
  { startMs: 1_000, text: 'first', translation: 'one' },
  { startMs: 3_000, text: 'second', translation: 'two' },
]
assert.equal(resolveDesktopLyricLine(lyricLines, 400, 500), null)
assert.equal(resolveDesktopLyricLine(lyricLines, 500, 500)?.text, 'first')
assert.equal(resolveDesktopLyricLine(lyricLines, 2_600, 500)?.text, 'second')
assert.equal(resolveNextDesktopLyricLine(lyricLines, lyricLines[0])?.text, 'second')
assert.equal(resolveNextDesktopLyricLine(lyricLines, lyricLines[1]), null)
assert.equal(resolveNextDesktopLyricLine(lyricLines, { startMs: 1_000, text: 'first' }), null)

assert.ok(windowPositionSource, 'desktop lyric window-position resolver must exist')
const compiledWindowPosition = ts.transpileModule(windowPositionSource, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const windowPositionModuleUrl = `data:text/javascript;base64,${Buffer.from(compiledWindowPosition).toString('base64')}`
const { clampWindowPositionToWorkAreas } = await import(windowPositionModuleUrl)
const primaryWorkArea = {
  workArea: {
    position: { x: 0, y: 0 },
    size: { width: 1_920, height: 1_040 },
  },
}
const leftWorkArea = {
  workArea: {
    position: { x: -1_280, y: 0 },
    size: { width: 1_280, height: 1_024 },
  },
}
assert.deepEqual(
  clampWindowPositionToWorkAreas({ x: 200, y: 100 }, { width: 720, height: 156 }, [primaryWorkArea]),
  { x: 200, y: 100 },
  'an already visible window position must be preserved',
)
assert.deepEqual(
  clampWindowPositionToWorkAreas({ x: 2_200, y: 1_200 }, { width: 720, height: 156 }, [primaryWorkArea]),
  { x: 1_200, y: 884 },
  'coordinates from a disconnected monitor must be clamped into an available work area',
)
assert.deepEqual(
  clampWindowPositionToWorkAreas({ x: -1_100, y: 700 }, { width: 720, height: 156 }, [leftWorkArea, primaryWorkArea]),
  { x: -1_100, y: 700 },
  'negative coordinates on an available monitor must remain valid',
)
assert.deepEqual(
  clampWindowPositionToWorkAreas({ x: 1_500, y: 100 }, { width: 1_440, height: 312 }, [primaryWorkArea]),
  { x: 480, y: 100 },
  'a DPI-increased physical window rectangle must be clamped using its current outer size',
)
assert.equal(clampWindowPositionToWorkAreas({ x: 20, y: 20 }, { width: 720, height: 156 }, []), null)
assert.equal(clampWindowPositionToWorkAreas({ x: Number.NaN, y: 20 }, { width: 720, height: 156 }, [primaryWorkArea]), null)
assert.equal(clampWindowPositionToWorkAreas({ x: 20, y: 20 }, { width: 0, height: 156 }, [primaryWorkArea]), null)
assert.equal(
  clampWindowPositionToWorkAreas({ x: 20, y: 20 }, { width: 2_000, height: 1_200 }, [primaryWorkArea]),
  null,
  'a work area that cannot contain the full window rectangle is not a safe restore target',
)

assert.ok(windowSource, 'desktop lyrics window component must exist')
assert.match(windowSource, /listen<DesktopLyricsSnapshot>\(DESKTOP_LYRICS_STATE_EVENT/)
assert.match(windowSource, /startDragging\(\)/)
assert.match(windowSource, /locked\.value = !locked\.value/)
assert.match(windowSource, /getCurrentWindow\(\)\.hide\(\)/)
assert.match(windowSource, /onMoved\(/)
assert.match(windowSource, /setPosition\(/)
assert.match(windowSource, /localStorage\.setItem/)
assert.match(windowSource, /availableMonitors\(\)/)
assert.match(windowSource, /appWindow\.outerSize\(\)/)
assert.match(windowSource, /clampWindowPositionToWorkAreas\(storedPosition, windowSize, monitors\)/)
assert.match(windowSource, /await appWindow\.center\(\)/)
assert.ok(
  windowSource.indexOf('availableMonitors()') < windowSource.indexOf('appWindow.setPosition'),
  'available monitor work areas must be checked before restoring a saved position',
)
assert.match(windowSource, /const nextLine = computed\(/)
assert.match(windowSource, /resolveNextDesktopLyricLine\(state\.lyrics, currentLine\.value\)/)
assert.match(windowSource, /const nextText = computed\(/)
assert.match(windowSource, /<p v-if="nextText" class="desktop-lyrics-next">/)
assert.ok(
  windowSource.indexOf('class="desktop-lyrics-translation"')
    < windowSource.indexOf('class="desktop-lyrics-next"'),
  'the optional current-line translation must not replace the following lyric line',
)
assert.match(
  windowSource,
  /\.desktop-lyrics-line,\s*\.desktop-lyrics-translation,\s*\.desktop-lyrics-next\s*\{[^}]*overflow:\s*hidden;[^}]*text-overflow:\s*ellipsis;[^}]*white-space:\s*nowrap;/s,
)
assert.match(windowSource, /\.desktop-lyrics-shell\s*\{[^}]*overflow:\s*hidden;/s)
assert.match(windowSource, /\.desktop-lyrics-line-group\s*\{[^}]*max-height:\s*100%;[^}]*overflow:\s*hidden;/s)
assert.match(
  windowSource,
  /@media \(max-width: 520px\)\s*\{[\s\S]*?grid-template-columns:\s*minmax\(0, 1fr\) 76px;/,
  'the 420px layout must leave the lyric column shrinkable',
)
assert.doesNotMatch(windowSource, /usePlayerStore|play_url|play_file|togglePlayPause/)

function pxValue(selector, property) {
  const selectorPattern = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const propertyPattern = property.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = windowSource.match(new RegExp(`${selectorPattern}\\s*\\{[^}]*${propertyPattern}:\\s*(\\d+(?:\\.\\d+)?)px;`, 's'))
  assert.ok(match, `${selector} must declare ${property} in physical CSS pixels for compact-window sizing`)
  return Number(match[1])
}

const shellVerticalPadding = 2 * pxValue('.desktop-lyrics-shell', 'padding-block')
const shellBorder = 2 * pxValue('.desktop-lyrics-shell', 'border-width')
const denseLyricStackHeight = pxValue('.desktop-lyrics-line', 'line-height')
  + pxValue('.desktop-lyrics-translation', 'line-height')
  + pxValue('.desktop-lyrics-next', 'line-height')
  + 2 * pxValue('.desktop-lyrics-line-group', 'gap')
for (const windowHeight of [112, 156]) {
  assert.ok(
    denseLyricStackHeight <= windowHeight - shellVerticalPadding - shellBorder,
    `current lyric, optional translation, and next lyric must all fit at ${windowHeight}px tall`,
  )
}

for (const option of [
  'width: 720',
  'height: 156',
  'minWidth: 420',
  'minHeight: 112',
  'resizable: true',
]) {
  assert.ok(bridgeSource.includes(option), `desktop lyrics window must preserve ${option}`)
}

assert.match(mainSource, /DESKTOP_LYRICS_WINDOW_LABEL/)
assert.match(mainSource, /DesktopLyricsWindow/)
assert.match(mainSource, /startDesktopLyricsBridge/)
assert.match(nowPlayingSource, /openDesktopLyricsWindow/)
assert.match(nowPlayingSource, /data-np-action="desktop-lyrics"/)
assert.match(settingsSource, /openDesktopLyricsWindow/)
assert.match(settingsSource, /data-settings-action="desktop-lyrics"/)

const capabilities = JSON.parse(capabilitySource)
const lyricsCapabilities = JSON.parse(lyricsCapabilitySource)
assert.deepEqual(capabilities.windows, ['main'])
assert.deepEqual(lyricsCapabilities.windows, ['desktop-lyrics'])
for (const externalLoginWindow of ['netease-login', 'bilibili-login', 'youtube-login']) {
  assert.equal(capabilities.windows.includes(externalLoginWindow), false)
  assert.equal(lyricsCapabilities.windows.includes(externalLoginWindow), false)
}
assert.ok(
  capabilities.permissions.includes('core:webview:allow-create-webview-window'),
  'main window needs permission to create the desktop lyrics webview window',
)
assert.equal(
  lyricsCapabilities.permissions.includes('core:webview:allow-create-webview-window'),
  false,
  'the desktop lyrics window must not create additional webviews',
)
assert.equal(lyricsCapabilities.permissions.includes('store:default'), false)
assert.equal(lyricsCapabilities.permissions.includes('dialog:default'), false)
assert.equal(lyricsCapabilities.permissions.includes('clipboard-manager:allow-read-text'), false)
assert.ok(
  lyricsCapabilities.permissions.includes('core:window:allow-show'),
  'the initially hidden desktop lyrics window must be able to reveal itself after mounting',
)
assert.ok(lyricsCapabilities.permissions.includes('core:window:allow-set-position'))
assert.ok(lyricsCapabilities.permissions.includes('core:window:allow-center'))

const tauriConfig = JSON.parse(tauriConfigSource)
assert.equal(tauriConfig.app.macOSPrivateApi, true, 'transparent macOS windows require macOSPrivateApi')

const dataModuleUrl = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const tauriMockUrl = dataModuleUrl(`
  export const invoke = (...args) => globalThis.__desktopLyricsInvoke(...args)
`)
const cacheMockUrl = dataModuleUrl(`
  export const getCachedLyrics = () => globalThis.__desktopLyricsCachedLyrics ?? null
  export const saveCachedLyrics = () => {}
`)
const requestMockUrl = dataModuleUrl(`
  export const loadLyricsSingleFlight = (_track, loader) => loader()
`)
const formatMockUrl = dataModuleUrl(`
  export const mapBackendLyrics = value => value
  export const mergeParsedLyricsWithTranslations = (original, translations) =>
    original.map((line, index) => ({ ...line, translation: translations[index]?.text }))
  export const resolveStoredLyricStateFromPayload = payload => {
    if (payload && Object.prototype.hasOwnProperty.call(payload, 'matchedLyric')) {
      return String(payload.matchedLyric ?? '').trim()
        ? { kind: 'present', text: String(payload.matchedLyric) }
        : { kind: 'cleared' }
    }
    return payload?.originalLyric
      ? { kind: 'present', text: payload.originalLyric }
      : { kind: 'absent' }
  }
  export const resolveStoredTranslatedLyricStateFromPayload = payload =>
    payload?.matchedTranslatedLyric
      ? { kind: 'present', text: payload.matchedTranslatedLyric }
      : { kind: 'absent' }
`)
const compiledLyricLoader = ts.transpileModule(
  lyricLoaderSource
    .replace("from '@tauri-apps/api/core'", `from '${tauriMockUrl}'`)
    .replace("from './lyricsCache'", `from '${cacheMockUrl}'`)
    .replace("from './lyricsRequest'", `from '${requestMockUrl}'`)
    .replace("from './lyricsFormat'", `from '${formatMockUrl}'`),
  {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  },
).outputText
const lyricLoaderModuleUrl = dataModuleUrl(compiledLyricLoader)
const { loadTrackLyrics } = await import(lyricLoaderModuleUrl)
const originalLine = { startMs: 1_000, durationMs: 2_000, text: 'original' }
const cachedLine = { startMs: 1_000, durationMs: 2_000, text: 'stale cached lyric' }
const lyricInvokeCalls = []
globalThis.__desktopLyricsInvoke = async (command, args) => {
  lyricInvokeCalls.push({ command, args })
  if (args.content === 'broken translation') throw new Error('invalid translation LRC')
  return [originalLine]
}
globalThis.__desktopLyricsCachedLyrics = [cachedLine]
const synchronizedEdit = await loadTrackLyrics({
  id: 'netease:edited',
  title: 'edited test',
  artist: 'artist',
  durationMs: 3_000,
  audioUrl: '',
  syncPayload: { matchedLyric: 'valid synchronized edit' },
})
assert.deepEqual(synchronizedEdit, [originalLine], 'synchronized lyric edits must override stale cached lyrics')
assert.deepEqual(lyricInvokeCalls.map(call => call.command), ['parse_lrc_content'])

lyricInvokeCalls.length = 0
const clearedLyrics = await loadTrackLyrics({
  id: 'netease:cleared',
  title: 'cleared test',
  artist: 'artist',
  durationMs: 3_000,
  audioUrl: '',
  syncPayload: { matchedLyric: '' },
})
assert.deepEqual(clearedLyrics, [], 'an explicit synchronized clear must override stale cached lyrics')
assert.deepEqual(lyricInvokeCalls, [], 'an explicit synchronized clear must not parse or fetch lyrics')

const absentStoredLyrics = await loadTrackLyrics({
  id: 'netease:cached',
  title: 'cache test',
  artist: 'artist',
  durationMs: 3_000,
  audioUrl: '',
  syncPayload: {},
})
assert.deepEqual(absentStoredLyrics, [cachedLine], 'cache remains available when synchronized lyric state is absent')
assert.deepEqual(lyricInvokeCalls, [], 'a visible cache hit must not fetch lyrics')

globalThis.__desktopLyricsCachedLyrics = null
const lyricsWithBrokenTranslation = await loadTrackLyrics({
  id: 'netease:1',
  title: 'test',
  artist: 'artist',
  durationMs: 3_000,
  audioUrl: '',
  syncPayload: {
    originalLyric: 'valid original',
    matchedTranslatedLyric: 'broken translation',
  },
})
assert.deepEqual(
  lyricsWithBrokenTranslation,
  [originalLine],
  'a malformed stored translation must not discard valid original lyrics',
)

console.log('desktop lyrics structure tests passed')
