import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/stores/listenTogether/playbackSync.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const transpiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  fileName: sourceUrl.pathname,
  reportDiagnostics: true,
})
assert.equal(transpiled.diagnostics?.length ?? 0, 0)
const {
  SOFT_SYNC_RECHECK_INTERVAL_MS,
  resolveExpectedPosition,
  resolvePositionSync,
  resolveSoftSyncRecheckAction,
  acceptRoomState,
  updateServerClockOffset,
  trackOccurrenceAt,
  findTrackOccurrenceIndex,
} = await import(`data:text/javascript;base64,${Buffer.from(transpiled.outputText).toString('base64')}`)

const track = { stableKey: 'netease:1', durationMs: 10_000 }
const state = {
  roomId: 'ABC234', version: 5, track, queue: [track], currentIndex: 0,
  playback: { state: 'playing', basePositionMs: 2000, baseTimestampMs: 5000, playbackRate: 1 },
}
assert.equal(resolveExpectedPosition(state, 8000), 5000)
assert.equal(resolveExpectedPosition(state, 4000), 1000)
assert.equal(resolveExpectedPosition(state, 30_000), 10_000)
assert.equal(resolveExpectedPosition(state, 8000, 3500), 3500)
assert.equal(resolveExpectedPosition({ ...state, track: undefined }, 8000), 5000)
assert.equal(resolveExpectedPosition({ ...state, playback: { ...state.playback, baseTimestampMs: 0 } }, 8000), 2000)
assert.equal(resolveExpectedPosition({ ...state, playback: { ...state.playback, state: 'paused' } }, 8000), 2000)
assert.equal(resolveExpectedPosition({ ...state, playback: { ...state.playback, playbackRate: 1.5 } }, 8000), 6500)
assert.equal(resolveExpectedPosition({ ...state, playback: { ...state.playback, repeatMode: 1 } }, 16_000), 3000)
assert.equal(resolveExpectedPosition({ ...state, playback: { ...state.playback, repeatMode: 1 } }, 8000, 11_000), 1000)
assert.equal(resolveExpectedPosition({ ...state, playback: { ...state.playback, state: 'paused', repeatMode: 1 } }, 8000, 11_000), 1000)
assert.equal(resolveExpectedPosition(state, 1000), 0)
assert.equal(resolveExpectedPosition(state, 8000, Number.NaN), 0)

const sync = {
  expectedPositionMs: 5000, localPositionMs: 5000,
  desiredPlaying: true, isController: false, causeType: 'PLAY',
}
function atDrift(drift, extra = {}) {
  return resolvePositionSync({ ...sync, expectedPositionMs: 5000 + drift, ...extra })
}
assert.deepEqual(atDrift(599), { rate: null })
assert.deepEqual(atDrift(600), { rate: 1.03 })
assert.deepEqual(atDrift(1499), { rate: 1.03 })
assert.deepEqual(atDrift(1500), { rate: 1.05 })
assert.deepEqual(atDrift(-600), { rate: 0.97 })
assert.deepEqual(atDrift(-1500), { rate: 0.95 })
assert.deepEqual(atDrift(2499), { rate: 1.05 })
assert.deepEqual(atDrift(2500), { rate: null })
assert.deepEqual(atDrift(2501), { seekTo: 7501, rate: null })
assert.deepEqual(atDrift(3000, { causeType: 'HEARTBEAT' }), { rate: null })
assert.deepEqual(atDrift(5000, { causeType: 'HEARTBEAT' }), { rate: null })
assert.deepEqual(atDrift(5001, { causeType: 'HEARTBEAT' }), { seekTo: 10_001, rate: null })
assert.deepEqual(atDrift(800, { desiredPlaying: false }), { rate: null })
assert.deepEqual(atDrift(801, { desiredPlaying: false }), { seekTo: 5801, rate: null })
assert.deepEqual(atDrift(1000, { isController: true }), { rate: null })
assert.deepEqual(atDrift(0, { forcePositionSync: true }), { seekTo: 5000, rate: null })
assert.deepEqual(atDrift(0, { causeType: 'LISTENER_SAFETY_RESUME' }), { seekTo: 5000, rate: null })
assert.deepEqual(atDrift(1, { playbackContextChanged: true }), { seekTo: 5001, rate: null })
assert.deepEqual(resolvePositionSync({ ...sync, expectedPositionMs: 0, localPositionMs: 500, targetIndexChanged: true }), { rate: null })
assert.deepEqual(resolvePositionSync({ ...sync, expectedPositionMs: 0, localPositionMs: 501, targetIndexChanged: true }), { seekTo: 0, rate: null })
for (const causeType of ['HEARTBEAT', 'WATCHDOG', 'WATCHDOG_STALL', 'LINK_READY', 'LINK_UNAVAILABLE', 'MEMBER_JOINED', 'MEMBER_LEFT']) {
  assert.deepEqual(resolvePositionSync({ ...sync, expectedPositionMs: 0, causeType }), { rate: null })
  assert.deepEqual(atDrift(6000, { causeType, trackSwitchGracePeriodActive: true }), { rate: null })
}
assert.deepEqual(resolvePositionSync({ ...sync, expectedPositionMs: 0, causeType: 'SEEK' }), { seekTo: 0, rate: null })
assert.deepEqual(atDrift(6000, { causeType: 'SEEK', trackSwitchGracePeriodActive: true }), { seekTo: 11_000, rate: null })
assert.deepEqual(resolvePositionSync({ ...sync, expectedPositionMs: 0, desiredPlaying: false, causeType: 'HEARTBEAT' }), { seekTo: 0, rate: null })

assert.equal(SOFT_SYNC_RECHECK_INTERVAL_MS, 500)
const recheck = {
  currentRate: 1.05, sessionConnected: true, isController: false,
  desiredPlaying: true, localPlaying: true, currentTrackMatchesRoom: true,
  expectedPositionMs: 6500, localPositionMs: 5000,
}
assert.equal(resolveSoftSyncRecheckAction(recheck), 'keep_rate')
assert.equal(resolveSoftSyncRecheckAction({ ...recheck, currentRate: 1 }), 'none')
assert.equal(resolveSoftSyncRecheckAction({ ...recheck, expectedPositionMs: 5599 }), 'reset_rate')
assert.equal(resolveSoftSyncRecheckAction({ ...recheck, expectedPositionMs: 5600 }), 'keep_rate')
assert.equal(resolveSoftSyncRecheckAction({ ...recheck, expectedPositionMs: 7500 }), 'apply_state')
for (const extra of [{ sessionConnected: false }, { isController: true }, { desiredPlaying: false }, { localPlaying: false }, { currentTrackMatchesRoom: false }]) {
  assert.equal(resolveSoftSyncRecheckAction({ ...recheck, ...extra }), 'reset_rate')
}

assert.equal(acceptRoomState(null, state, state.roomId), state)
assert.equal(acceptRoomState(state, { ...state, version: 4 }, state.roomId), null)
assert.equal(acceptRoomState(state, { ...state, roomId: 'DEF234', version: 8 }, state.roomId), null)
assert.equal(acceptRoomState(state, { ...state, version: 6 }, state.roomId, 7), null)
assert.equal(acceptRoomState(state, { ...state, playback: { ...state.playback, basePositionMs: 9999 } }, state.roomId), state)
assert.equal(acceptRoomState(state, { ...state, version: Number.NaN }, state.roomId), null)
const newer = { ...state, version: 6 }
assert.equal(acceptRoomState(state, newer, state.roomId), newer)

const clock = {
  previousOffsetMs: 0, serverNowMs: 10_200, sentAtWallMs: 10_000,
  sentAtElapsedMs: 1000, nowWallMs: 10_200, nowElapsedMs: 1200,
}
assert.equal(updateServerClockOffset(clock), 100)
assert.equal(updateServerClockOffset({ ...clock, previousOffsetMs: 200 }), 170)
assert.equal(updateServerClockOffset({ ...clock, sentAtWallMs: 0 }), 0)
assert.equal(updateServerClockOffset({ ...clock, previousOffsetMs: 200, serverNowMs: 0 }), 200)
assert.equal(updateServerClockOffset({ ...clock, previousOffsetMs: 200, serverNowMs: Number.NaN }), 200)
assert.equal(updateServerClockOffset({ ...clock, previousOffsetMs: 200, nowElapsedMs: 999 }), 200)
assert.equal(updateServerClockOffset({ ...clock, previousOffsetMs: 200, nowElapsedMs: 31_001 }), 200)
assert.equal(updateServerClockOffset({ ...clock, nowElapsedMs: 31_000 }), -14_800)

const duplicates = [{ stableKey: 'a' }, { stableKey: 'b' }, { stableKey: 'a' }, { stableKey: 'a' }]
assert.deepEqual(trackOccurrenceAt(duplicates, 2), { stableKey: 'a', occurrence: 1 })
assert.equal(trackOccurrenceAt(duplicates, -1), null)
assert.equal(trackOccurrenceAt(duplicates, 4), null)
assert.equal(findTrackOccurrenceIndex(duplicates, { stableKey: 'a', occurrence: 2 }), 3)
assert.equal(findTrackOccurrenceIndex(duplicates, { stableKey: 'a', occurrence: 3 }), -1)
assert.equal(findTrackOccurrenceIndex(duplicates, { stableKey: 'a', occurrence: -1 }), -1)
assert.equal(findTrackOccurrenceIndex(duplicates, null), -1)

console.log('listen together sync tests passed')
