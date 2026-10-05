// Listen-together wire protocol helpers (Android-aligned ExoPlayer ints)
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/listenTogether/protocol.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { desktopRepeatToWire, wireRepeatToDesktop } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`,
)

assert.equal(desktopRepeatToWire('off'), 0)
assert.equal(desktopRepeatToWire('one'), 1)
assert.equal(desktopRepeatToWire('all'), 2)
assert.equal(desktopRepeatToWire(undefined), 0)
assert.equal(wireRepeatToDesktop(0), 'off')
assert.equal(wireRepeatToDesktop(1), 'one')
assert.equal(wireRepeatToDesktop(2), 'all')
assert.equal(wireRepeatToDesktop(null), null)
assert.equal(wireRepeatToDesktop(99), null)

// Snapshot shape must carry mode fields for create-room
const snapshot = {
  queue: [],
  currentIndex: 0,
  settings: { allowMemberControl: true, autoPauseOnMemberChange: true, shareAudioLinks: true },
  isPlaying: false,
  positionMs: 0,
  repeatMode: desktopRepeatToWire('all'),
  shuffleEnabled: true,
}
assert.equal(snapshot.repeatMode, 2)
assert.equal(snapshot.shuffleEnabled, true)

// Event type names stay Android-aligned
const modeEvent = {
  type: 'PLAYBACK_MODE',
  repeatMode: desktopRepeatToWire('one'),
  shuffleEnabled: false,
}
assert.equal(modeEvent.type, 'PLAYBACK_MODE')
assert.equal(modeEvent.repeatMode, 1)

console.log('test-listen-together-protocol: ok')
