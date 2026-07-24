import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import ts from 'typescript'

const sourceUrl = new URL('../src/modules/playback/playerState.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const playerStoreSource = await readFile(
  new URL('../src/stores/player.ts', import.meta.url),
  'utf8',
)
const transpiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
  fileName: pathToFileURL(sourceUrl.pathname).href,
})
const moduleUrl = `data:text/javascript;base64,${Buffer.from(transpiled.outputText).toString('base64')}`
const {
  buildPersistedPlaybackQueue,
  restorePersistedPlaybackQueue,
} = await import(moduleUrl)

const first = { id: 'netease:1', playlistKey: 'playlist:first', audioUrl: '' }
const current = { id: 'netease:1', playlistKey: 'playlist:current', audioUrl: '' }
const last = { id: 'netease:2', playlistKey: 'playlist:last', audioUrl: '' }

const full = buildPersistedPlaybackQueue([first, current, last], 1, current, false)
assert.deepEqual(full, {
  queue: [first, current, last],
  queueIndex: 1,
  hasPlaybackSession: true,
  currentTrackId: 'netease:1',
  currentTrackPlaylistKey: 'playlist:current',
})

const compact = buildPersistedPlaybackQueue([first, current, last], 1, current, true)
assert.deepEqual(compact, {
  queue: [current],
  queueIndex: 0,
  hasPlaybackSession: true,
  currentTrackId: 'netease:1',
  currentTrackPlaylistKey: 'playlist:current',
})

const normalize = raw => {
  if (!raw || typeof raw !== 'object') throw new Error('invalid track')
  return raw
}
const canRestore = track => !!track.id && (!!track.audioUrl || !track.id.startsWith('local:'))

const restored = restorePersistedPlaybackQueue(
  [null, { id: 'local:missing', audioUrl: '' }, first, current, last],
  3,
  true,
  'netease:1',
  'playlist:current',
  normalize,
  canRestore,
)
assert.deepEqual(restored.queue, [first, current, last])
assert.equal(restored.queueIndex, 1)
assert.equal(restored.currentTrack, current)
assert.equal(restored.hasPlaybackSession, true)

const legacyRestored = restorePersistedPlaybackQueue(
  [first, last],
  99.8,
  undefined,
  undefined,
  undefined,
  normalize,
  canRestore,
)
assert.equal(legacyRestored.queueIndex, 1)
assert.equal(legacyRestored.currentTrack, last)
assert.equal(legacyRestored.hasPlaybackSession, true)

const empty = restorePersistedPlaybackQueue(
  [{ id: 'local:missing', audioUrl: '' }],
  0,
  true,
  'local:missing',
  undefined,
  normalize,
  canRestore,
)
assert.deepEqual(empty.queue, [])
assert.equal(empty.queueIndex, -1)
assert.equal(empty.currentTrack, null)
assert.equal(empty.hasPlaybackSession, false)

const queuedWithoutSession = buildPersistedPlaybackQueue([first, last], -1, null, false)
assert.equal(queuedWithoutSession.queueIndex, -1)
assert.equal(queuedWithoutSession.hasPlaybackSession, false)
const restoredWithoutSession = restorePersistedPlaybackQueue(
  queuedWithoutSession.queue,
  queuedWithoutSession.queueIndex,
  queuedWithoutSession.hasPlaybackSession,
  queuedWithoutSession.currentTrackId,
  queuedWithoutSession.currentTrackPlaylistKey,
  normalize,
  canRestore,
)
assert.deepEqual(restoredWithoutSession.queue, [first, last])
assert.equal(restoredWithoutSession.queueIndex, -1)
assert.equal(restoredWithoutSession.currentTrack, null)
assert.equal(restoredWithoutSession.hasPlaybackSession, false)

assert.match(
  playerStoreSource,
  /const currentResolvedStreamUrl = ref<string \| null>\(null\)/,
  'the active resolved URL must be ephemeral player state',
)
const playFunctionSource = playerStoreSource.slice(
  playerStoreSource.indexOf('async function play('),
  playerStoreSource.indexOf('\n  async function togglePlayPause'),
)
const playbackFailureCatchSource = playFunctionSource.slice(
  playFunctionSource.lastIndexOf('} catch (e) {'),
)
assert.match(
  playFunctionSource,
  /const token = \+\+playbackRequestToken\s+currentResolvedStreamUrl\.value = null/,
  'every new playback request must invalidate the previously resolved URL synchronously',
)
assert.match(
  playFunctionSource,
  /if \(token !== playbackRequestToken\) return\s+currentResolvedStreamUrl\.value = played\.streamUrl/,
  'only the current playback request may publish its winning remote URL',
)
assert.match(
  playbackFailureCatchSource,
  /currentResolvedStreamUrl\.value = null/,
  'a failed current playback request must clear its resolved URL',
)
assert.match(
  playbackFailureCatchSource,
  /\+\+playbackRequestToken\s+currentResolvedStreamUrl\.value = null/,
  'a failed current playback request must fence pending URL resolution before clearing its resolved URL',
)
const trackEndedSource = playerStoreSource.slice(
  playerStoreSource.indexOf('async function handleTrackEnded'),
  playerStoreSource.indexOf('\n  async function next'),
)
assert.match(
  trackEndedSource,
  /currentResolvedStreamUrl\.value = null\s+await pause\(\)\s+positionMs\.value = 0/,
  'stopping at the end of the queue must clear the resolved URL before awaiting IPC',
)
assert.equal(
  trackEndedSource.match(/\+\+playbackRequestToken\s+currentResolvedStreamUrl\.value = null\s+await pause\(\)/g)?.length,
  3,
  'sleep-timer and natural queue-end stop paths must fence pending URL resolution before clearing the resolved URL',
)
const clearQueueSource = playerStoreSource.slice(
  playerStoreSource.indexOf('function clearQueue'),
  playerStoreSource.indexOf('\n  //', playerStoreSource.indexOf('function clearQueue')),
)
assert.match(
  clearQueueSource,
  /currentResolvedStreamUrl\.value = null/,
  'clearing the queue must clear the resolved URL',
)
assert.match(
  clearQueueSource,
  /function clearQueue\(\) \{\r?\n\s*\+\+playbackRequestToken\r?\n\s*queue\.value = \[\]/,
  'clearing the queue must invalidate pending playback requests',
)
const removeFromQueueSource = playerStoreSource.slice(
  playerStoreSource.indexOf('function removeFromQueue'),
  playerStoreSource.indexOf('\n  //', playerStoreSource.indexOf('function removeFromQueue')),
)
const currentRemovalSource = removeFromQueueSource.slice(
  removeFromQueueSource.indexOf('else if (wasCurrentTrack)'),
)
assert.match(
  currentRemovalSource,
  /\r?\n\s*\+\+playbackRequestToken\r?\n\s*currentTrack\.value/,
  'removing the current track must invalidate pending playback requests',
)
assert.doesNotMatch(
  removeFromQueueSource.slice(
    removeFromQueueSource.indexOf('if (index < queueIndex.value)'),
    removeFromQueueSource.indexOf('else if (wasCurrentTrack)'),
  ),
  /\+\+playbackRequestToken/,
  'removing a non-current track must not invalidate current playback',
)
assert.doesNotMatch(
  playerStoreSource.slice(
    playerStoreSource.indexOf('function savePlayerState'),
    playerStoreSource.indexOf('function loadPlayerState'),
  ),
  /currentResolvedStreamUrl/,
  'resolved URLs must not be persisted',
)
assert.match(
  playerStoreSource.slice(playerStoreSource.lastIndexOf('\n  return {')),
  /currentResolvedStreamUrl/,
  'listen together must be able to read the active resolved URL',
)

console.log('player state tests passed')
