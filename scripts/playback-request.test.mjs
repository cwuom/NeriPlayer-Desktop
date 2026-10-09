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
// 已有完整缓存时不再发网络解析（对齐 Android 离线缓存直接播放）
assert.equal(shouldResolvePlaybackSourceInParallel(true, false, true), false)
assert.equal(shouldResolvePlaybackSourceInParallel(true, false, false), true)

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
  /void invoke<void>\('begin_playback_request', \{[\s\S]*?silencePrevious: !keepsPreviousAudible,/,
  'switching tracks must silence the previous track before the new source resolves',
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
  /async function flushPlayerState\(\): Promise<void> \{[\s\S]*if \(preloadedUserData\(\)\) \{\s*await persistPlayerStateToDatabase\(\)/,
  'the close-time flush must be awaitable and write the user database first',
)
assert.match(
  playerStoreSource,
  /state: queueUnchanged \? \{ \.\.\.state, queue: null \} : state/,
  'progress saves must not resend an unchanged queue',
)
assert.match(
  playerStoreSource,
  /const compactState = persistedPlayerState\(true\)[\s\S]*localStorage\.setItem\(PLAYER_STATE_KEY, JSON\.stringify\(compactState\)\)/,
  'an oversized queue must fall back to a compact current-track state',
)
assert.match(
  playerStoreSource,
  /const refreshed = await resolvePlaybackUrl\(\s*track,\s*true,\s*\)/,
  'a failed playback retry must refresh the URL without silently changing quality',
)
assert.doesNotMatch(
  playerStoreSource,
  /const refreshed = await resolvePlaybackUrl\(\s*track,\s*true,\s*getPlaybackSourceKind\(track\) === 'youtube'/,
  'a failed YouTube retry must preserve the user-selected quality',
)

const playerStoreAst = ts.createSourceFile('player.ts', playerStoreSource, ts.ScriptTarget.Latest, true)
let recoveryBranch
function findRecoveryBranch(node) {
  if (ts.isIfStatement(node) && node.expression.getText(playerStoreAst) === 'shouldRestorePreviousPlaybackState') {
    recoveryBranch = node
  }
  ts.forEachChild(node, findRecoveryBranch)
}
findRecoveryBranch(playerStoreAst)
assert.ok(recoveryBranch, 'the store must retain its crossfade failure recovery branch')
const recoveryStatements = recoveryBranch.parent.statements
const endLoadingStatement = recoveryStatements[recoveryStatements.indexOf(recoveryBranch) + 1]
assert.ok(endLoadingStatement)

// 执行真实恢复分支，仅把后端查询替换为可控 Promise，复现请求在 await 期间被替换
const recoveryCode = ts.transpileModule(`
  export function createRecoveryHarness(invoke) {
    let playbackRequestToken = 8
    const token = 8
    const isPlaying = { value: false }
    const isLoadingAudio = { value: true }
    let _interpIsPlaying = false
    let interpolationRestarts = 0
    const shouldRestorePreviousPlaybackState = true
    function _startInterpolationLoop() { interpolationRestarts++ }
    async function recover() {
      ${recoveryBranch.getText(playerStoreAst)}
      ${endLoadingStatement.getText(playerStoreAst)}
    }
    return {
      recover,
      supersede() { playbackRequestToken++ },
      snapshot() {
        return { isPlaying: isPlaying.value, isLoadingAudio: isLoadingAudio.value,
          interpolating: _interpIsPlaying, interpolationRestarts }
      },
    }
  }
`, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText
const { createRecoveryHarness } = await import(`data:text/javascript;base64,${Buffer.from(recoveryCode).toString('base64')}`)

async function recoverWhileRequestChanges(outcome, superseded) {
  let completeQuery
  let failQuery
  const query = new Promise((resolve, reject) => {
    completeQuery = resolve
    failQuery = reject
  })
  const harness = createRecoveryHarness(() => query)
  const recovering = harness.recover()
  if (superseded) harness.supersede()
  if (outcome === 'failure') failQuery(new Error('backend state unavailable'))
  else completeQuery({ is_playing: outcome === 'playing' })
  await recovering
  return harness.snapshot()
}

const staleRecoveryFailures = []
for (const outcome of ['playing', 'failure']) {
  try {
    assert.deepEqual(await recoverWhileRequestChanges(outcome, true), {
      isPlaying: false, isLoadingAudio: true, interpolating: false, interpolationRestarts: 0,
    }, `a superseded crossfade recovery must not overwrite the latest request after ${outcome}`)
  } catch (error) {
    staleRecoveryFailures.push(error.message)
  }
}
assert.deepEqual(staleRecoveryFailures, [])
for (const outcome of ['playing', 'stopped', 'failure']) {
  const expectedPlaying = outcome !== 'stopped'
  assert.deepEqual(await recoverWhileRequestChanges(outcome, false), {
    isPlaying: expectedPlaying, isLoadingAudio: false, interpolating: expectedPlaying,
    interpolationRestarts: expectedPlaying ? 1 : 0,
  }, `the current crossfade recovery must preserve ${outcome} backend behavior`)
}

const appSource = await readFile(new URL('../src/App.vue', import.meta.url), 'utf8')
assert.match(
  appSource,
  /window\.addEventListener\('beforeunload', handleBeforeUnload\)/,
  'the app must flush player state before the WebView closes',
)
assert.match(
  appSource,
  /getCurrentWindow\(\)\.onCloseRequested\(handleCloseRequested\)/,
  'the app must flush player state for a native Tauri window close',
)
assert.match(
  appSource,
  /window\.addEventListener\('pagehide', handleBeforeUnload\)/,
  'the app must flush player state when the WebView page is hidden',
)

console.log('playback request tests passed')
