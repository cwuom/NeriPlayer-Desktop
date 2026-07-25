import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import ts from 'typescript'

const sourceUrl = new URL('../src/modules/playback/playbackRequest.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const transpiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
  fileName: pathToFileURL(sourceUrl.pathname).href,
})
const moduleUrl = `data:text/javascript;base64,${Buffer.from(transpiled.outputText).toString('base64')}`
const {
  hasVisiblePlaybackSession,
  initialPlaybackPrefetchWindow,
  isPlaybackSeekCompletionCurrent,
  playbackSessionTrackKey,
  resolvePlaybackLoadStart,
  shouldDeferPlaybackSeek,
  shouldResolvePlaybackSourceInParallel,
} = await import(moduleUrl)

const logSanitizerSourceUrl = new URL('../src/utils/logSanitizer.ts', import.meta.url)
const logSanitizerSource = await readFile(logSanitizerSourceUrl, 'utf8')
const logSanitizerTranspiled = ts.transpileModule(logSanitizerSource, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
  fileName: pathToFileURL(logSanitizerSourceUrl.pathname).href,
})
const logSanitizerModuleUrl = `data:text/javascript;base64,${Buffer.from(logSanitizerTranspiled.outputText).toString('base64')}`
const { summarizeLogError } = await import(logSanitizerModuleUrl)

const signedUrlError = summarizeLogError(
  new Error('request failed: https://media.example/audio.flac?token=secret'),
)
assert.match(signedUrlError, /request failed: \[url\]/)
assert.doesNotMatch(signedUrlError, /token=secret/)

assert.equal(shouldDeferPlaybackSeek(8, 7), true)
assert.equal(shouldDeferPlaybackSeek(8, 8), false)

assert.deepEqual(resolvePlaybackLoadStart(8, 1200, null), {
  positionMs: 1200,
  seekSeq: null,
})
assert.deepEqual(resolvePlaybackLoadStart(8, 1200, {
  requestGeneration: 8,
  positionMs: 42_500,
  seekSeq: 3,
}), {
  positionMs: 42_500,
  seekSeq: 3,
})
assert.deepEqual(resolvePlaybackLoadStart(8, 1200, {
  requestGeneration: 7,
  positionMs: 42_500,
  seekSeq: 3,
}), {
  positionMs: 1200,
  seekSeq: null,
})

assert.equal(isPlaybackSeekCompletionCurrent(8, 8, 3, 3), true)
assert.equal(isPlaybackSeekCompletionCurrent(8, 9, 3, 3), false)
assert.equal(isPlaybackSeekCompletionCurrent(8, 8, 3, 4), false)

assert.equal(hasVisiblePlaybackSession(false, 'netease:1'), false)
assert.equal(hasVisiblePlaybackSession(true, null), false)
assert.equal(hasVisiblePlaybackSession(true, 'netease:1'), true)
assert.equal(playbackSessionTrackKey(false, 'playlist:1', 'netease:1'), 'empty')
assert.equal(playbackSessionTrackKey(true, 'playlist:1', 'netease:1'), 'playlist:1')
assert.equal(playbackSessionTrackKey(true, '', 'netease:1'), 'netease:1')
assert.equal(shouldResolvePlaybackSourceInParallel(false, false), true)
assert.equal(shouldResolvePlaybackSourceInParallel(true, false), true)
assert.equal(shouldResolvePlaybackSourceInParallel(false, true), false)

const prefetchTracks = [{ id: 'first' }, { id: 'second' }, { id: 'third' }, { id: 'fourth' }]
assert.deepEqual(initialPlaybackPrefetchWindow(prefetchTracks), prefetchTracks.slice(0, 3))
assert.deepEqual(initialPlaybackPrefetchWindow(prefetchTracks, 1), prefetchTracks.slice(0, 1))
assert.deepEqual(initialPlaybackPrefetchWindow(prefetchTracks, 0), [])

const playerStoreSource = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
assert.match(
  playerStoreSource,
  /void invoke<void>\('begin_playback_request'/,
  'playback preclaim must not block cold-start cache and URL resolution',
)
assert.match(
  playerStoreSource,
  /commitTrack\(\)\s+isLoadingAudio\.value = true\s+hasPlaybackSession\.value = true/,
  'a user-initiated load must keep MiniPlayer visible while audio is preparing',
)
assert.match(
  playerStoreSource,
  /const restored = restorePersistedPlaybackQueue\([\s\S]*hasPlaybackSession\.value = restored\.hasPlaybackSession/,
  'a restored track must keep MiniPlayer visible without auto-starting audio',
)
assert.match(
  playerStoreSource,
  /function flushPlayerState\(\)[\s\S]*localStorage\.setItem\(PLAYER_STATE_KEY/,
  'player state must support an immediate close-time flush',
)
assert.match(
  playerStoreSource,
  /const compactState = persistedPlayerState\(true\)[\s\S]*localStorage\.setItem\(PLAYER_STATE_KEY, JSON\.stringify\(compactState\)\)/,
  'an oversized queue must fall back to a compact current-track state',
)

const appSource = await readFile(new URL('../src/App.vue', import.meta.url), 'utf8')
assert.match(
  appSource,
  /window\.addEventListener\('beforeunload', handleBeforeUnload\)/,
  'the app must flush player state before the WebView closes',
)
assert.match(
  appSource,
  /function handleCloseRequested\(event: CloseRequestedEvent\)[\s\S]*?handleBeforeUnload\(\)[\s\S]*?event\.preventDefault\(\)[\s\S]*?closeApplication\(\)[\s\S]*?onCloseRequested\(handleCloseRequested\)/,
  'native window close must flush player state before entering coordinated shutdown',
)
assert.match(
  appSource,
  /window\.addEventListener\('pagehide', handleBeforeUnload\)/,
  'the app must flush player state when the WebView page is hidden',
)

console.log('playback request tests passed')
