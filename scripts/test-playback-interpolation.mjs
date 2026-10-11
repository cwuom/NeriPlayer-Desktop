import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { ref } from 'vue'

const source = readFileSync(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const names = new Set(['commitBackendPosition', 'clampPlaybackPosition', '_startInterpolationLoop'])
const functions = []
function visit(node) {
  if (ts.isFunctionDeclaration(node) && names.has(node.name?.text)) functions.push(node.getText(parsed))
  ts.forEachChild(node, visit)
}
visit(parsed)
assert.equal(functions.length, names.size)
const declarations = parsed.statements.filter(node =>
  ts.isVariableStatement(node) && node.declarationList.declarations.some(declaration =>
    /^_interp|^POSITION_|^CLOCK_JUMP_/.test(declaration.name.getText(parsed)),
  ),
).map(node => node.getText(parsed))
const compiled = ts.transpileModule([...declarations, ...functions].join('\n'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

function runtime({ speed = 1, duration = 180_000 } = {}) {
  let now = 100
  const frames = []
  const context = {
    performance: { now: () => now },
    requestAnimationFrame: callback => { frames.push(callback); return frames.length },
    durationMs: ref(duration), currentTrack: ref({ durationMs: duration }),
    positionMs: ref(10_000), interpolatedPositionMs: ref(10_000),
  }
  const methods = new Function(...Object.keys(context), `
    let pendingSeek = null;
    ${compiled}
    _interpAnchorMs = 10000;
    _interpAnchorTime = performance.now();
    _interpRenderedMs = 10000;
    _interpIsPlaying = true;
    _interpDurationMs = durationMs.value;
    _interpSpeed = ${speed};
    _startInterpolationLoop();
    return {
      commitBackendPosition,
      pause: () => { _interpIsPlaying = false; },
      seek: targetMs => { pendingSeek = { targetMs }; },
    };
  `)(...Object.values(context))
  return {
    ...methods,
    get position() { return context.interpolatedPositionMs.value },
    get queuedFrames() { return frames.length },
    backend(time, position) { now = time; methods.commitBackendPosition(position, duration) },
    frame(time) {
      now = time
      const callback = frames.shift()
      assert.ok(callback, 'interpolation has a scheduled frame')
      callback(time)
    },
  }
}

let passed = 0, failed = 0
function regression(name, run) {
  try { run(); passed++; console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

regression('returning after backend progress continued does not count background time twice', () => {
  const r = runtime()
  r.frame(100)
  for (let elapsed = 200; elapsed <= 10_000; elapsed += 200) r.backend(100 + elapsed, 10_000 + elapsed)
  r.frame(10_100)
  assert.equal(r.position, 20_000)
  r.backend(10_300, 20_200)
  r.frame(10_300)
  assert.ok(r.position >= 20_200 && r.position < 20_500, 'progress remains aligned after recovery')
})

regression('returning without any background events uses the anchor once', () => {
  const r = runtime()
  r.frame(100)
  r.frame(10_100)
  assert.equal(r.position, 20_000)
})

regression('short background gaps recover without a permanent lead below the clock jump guard', () => {
  const r = runtime()
  r.frame(100)
  r.backend(2100, 12_000)
  r.frame(2100)
  assert.equal(r.position, 12_000)
})

regression('foreground recovery applies playback speed once', () => {
  const r = runtime({ speed: 2 })
  r.frame(100)
  r.backend(10_100, 30_000)
  r.frame(10_100)
  assert.equal(r.position, 30_000)
})

regression('background buffering recovers to the latest backend position', () => {
  const r = runtime()
  r.frame(100)
  r.backend(10_100, 10_400)
  r.frame(10_100)
  assert.equal(r.position, 10_400)
})

regression('normal frames keep advancing smoothly after recovery', () => {
  const r = runtime()
  r.frame(100)
  r.frame(10_100)
  const restored = r.position
  r.frame(10_116)
  assert.ok(r.position > restored && r.position <= restored + 32)
  const previous = r.position
  r.frame(10_132)
  assert.ok(r.position > previous && r.position <= previous + 32)
})

regression('a pending seek stays frozen through a background frame gap', () => {
  const r = runtime()
  r.frame(100)
  r.seek(45_000)
  r.frame(10_100)
  assert.equal(r.position, 45_000)
})

regression('a pause during background playback restores the paused position and stops the loop', () => {
  const r = runtime()
  r.frame(100)
  r.pause()
  r.backend(10_100, 15_000)
  r.frame(10_100)
  assert.equal(r.position, 15_000)
  assert.equal(r.queuedFrames, 0)
})

console.log(`Playback interpolation regressions: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
