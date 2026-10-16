import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/utils/frameLoop.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

/** Reload the module per case so its scheduling state cannot leak between cases */
function loadFrameLoop() {
  const exports = {}
  const frames = new Map()
  let frameId = 0
  let now = 0
  new Function('require', 'exports', 'requestAnimationFrame', 'cancelAnimationFrame', compiled)(
    name => { throw new Error(`unexpected dependency: ${name}`) },
    exports,
    callback => { const id = ++frameId; frames.set(id, callback); return id },
    id => frames.delete(id),
  )
  return {
    ...exports,
    /** Registered browser rAFs: direct evidence that the three loops share one dispatch */
    get scheduledFrames() { return frames.size },
    advance(elapsed = 16) {
      now += elapsed
      const batch = [...frames]
      for (const [id, callback] of batch) {
        if (!frames.delete(id)) continue
        callback(now)
      }
    },
  }
}

let passed = 0
function test(name, run) {
  run()
  passed++
  console.log(`PASS ${name}`)
}

test('background, lyrics and waveform share one rAF: three registrations, one dispatch', () => {
  const loop = loadFrameLoop()
  const ran = []
  loop.scheduleFrame(() => ran.push('background'))
  loop.scheduleFrame(() => ran.push('lyrics'))
  loop.scheduleFrame(() => ran.push('waveform'))
  assert.equal(loop.scheduledFrames, 1, 'three registrations merge into one rAF')
  loop.advance()
  assert.deepEqual(ran, ['background', 'lyrics', 'waveform'], 'all of them run in one frame, in registration order')
  assert.equal(loop.scheduledFrames, 0, 'callbacks that did not re-queue are gone after one frame')
})

test('a callback that re-queues itself does not start a second loop', () => {
  const loop = loadFrameLoop()
  let ticks = 0
  const task = () => {
    ticks++
    loop.scheduleFrame(task)
  }
  loop.scheduleFrame(task)
  loop.advance()
  loop.advance()
  loop.advance()
  assert.equal(ticks, 3)
  assert.equal(loop.scheduledFrames, 1, 'the re-queued callback always occupies a single dispatch')
})

test('cancelled callbacks do not run, and cancelling the last one stops the loop', () => {
  const loop = loadFrameLoop()
  const ran = []
  const handle = loop.scheduleFrame(() => ran.push('cancelled'))
  const kept = loop.scheduleFrame(() => ran.push('kept'))
  loop.cancelFrame(handle)
  loop.advance()
  assert.deepEqual(ran, ['kept'])
  loop.cancelFrame(kept)
  assert.equal(loop.scheduledFrames, 0, 'emptying the callback table cancels the rAF too')
})

test('the frame cap only runs after the interval, and removing it restores per-frame work', () => {
  const loop = loadFrameLoop()
  let ticks = 0
  const task = () => { ticks++; loop.scheduleFrame(task) }
  loop.setFrameInterval(1000 / 30)
  loop.scheduleFrame(task)
  loop.advance(16)
  assert.equal(ticks, 1, 'the first frame always runs, otherwise the picture freezes')
  loop.advance(16)
  assert.equal(ticks, 1, '16ms is short of 33ms, so this frame is skipped')
  loop.advance(16)
  assert.equal(ticks, 2, 'accumulated 32ms crosses the interval')
  loop.setFrameInterval(0)
  loop.advance(16)
  assert.equal(ticks, 3, 'without a cap every frame runs')
})

test('callbacks receive the shared dispatch timestamp instead of reading their own clock', () => {
  const loop = loadFrameLoop()
  const stamps = []
  loop.scheduleFrame(now => stamps.push(now))
  loop.advance(16)
  assert.deepEqual(stamps, [16])
})

console.log(`Shared frame loop: ${passed} passed`)
