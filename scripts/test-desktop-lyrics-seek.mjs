import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { ref, watch } from 'vue'
import ts from 'typescript'

async function loadModule(path, dependencies = {}) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const key = `desktop-lyrics-seek-${Math.random()}`
  globalThis[key] = dependencies
  compiled = compiled.replace(/import\s*\{([\s\S]*?)\}\s*from\s*['"]([^'"]+)['"];?/g, (_, bindings, specifier) => {
    assert.ok(specifier in dependencies, `unexpected dependency ${specifier}`)
    return `const { ${bindings.replace(/\bas\b/g, ':')} } = globalThis[${JSON.stringify(key)}][${JSON.stringify(specifier)}];`
  })
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
  } finally {
    delete globalThis[key]
  }
}

const styleModule = await loadModule('../src/modules/desktopLyrics/style.ts')
const timelineModule = await loadModule('../src/modules/desktopLyrics/timeline.ts')
const frameModule = await loadModule('../src/modules/desktopLyrics/frame.ts', {
  './style': styleModule, './timeline': timelineModule,
})
const playerSource = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsedPlayer = ts.createSourceFile('player.ts', playerSource, ts.ScriptTarget.ES2022, true)
const playerFunctions = new Map()
function visit(node) {
  if (ts.isFunctionDeclaration(node) && ['livePositionMs', 'isPlaybackClockAdvancing'].includes(node.name?.text)) {
    playerFunctions.set(node.name.text, node.getText(parsedPlayer))
  }
  ts.forEachChild(node, visit)
}
visit(parsedPlayer)
assert.ok(playerFunctions.has('livePositionMs'), 'the real player clock must be exercised')
assert.ok(playerFunctions.has('isPlaybackClockAdvancing'), 'the real player clock state must be exercised')
const compiledPlayer = ts.transpileModule([...playerFunctions.values()].join('\n'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText

async function flush() {
  for (let index = 0; index < 8; index++) await Promise.resolve()
}

async function withRuntime(run, { playing = true, speed = 1 } = {}) {
  let now = 50_000
  const frames = []
  const intervals = new Map()
  const originalNow = Date.now
  const originalInterval = globalThis.setInterval
  const originalClearInterval = globalThis.clearInterval
  Date.now = () => now
  globalThis.setInterval = callback => {
    const id = intervals.size + 1
    intervals.set(id, callback)
    return id
  }
  globalThis.clearInterval = id => intervals.delete(id)
  const clock = new Function('performance', 'initialPlaying', 'initialSpeed', `
    let pendingSeek = null;
    let _interpIsPlaying = initialPlaying;
    let _interpSpeed = initialSpeed;
    let _interpRenderedMs = 10000;
    let _interpAnchorMs = 10000;
    let _interpAnchorTime = performance.now();
    let _interpDurationMs = 300000;
    ${compiledPlayer}
    function anchor(position) {
      _interpRenderedMs = position;
      _interpAnchorMs = position;
      _interpAnchorTime = performance.now();
    }
    return {
      livePositionMs,
      isPlaybackClockAdvancing,
      playing: () => _interpIsPlaying,
      speed: () => _interpSpeed,
      seek(targetMs) { pendingSeek = { targetMs }; anchor(targetMs); },
      settle(positionMs) { pendingSeek = null; anchor(positionMs); },
    };
  `)({ now: () => now }, playing, speed)
  const player = {
    currentTrack: { id: 'local:seek-regression', title: 'seek', artist: 'test', durationMs: 300_000 },
    livePositionMs: clock.livePositionMs,
    isPlaybackClockAdvancing: clock.isPlaybackClockAdvancing,
    effectivePlaybackSpeed: clock.speed,
    get isPlaying() { return clock.playing() },
  }
  const currentLyrics = {
    lines: [0, 10_000, 60_000, 90_000, 120_000].map(startMs => ({
      startMs, durationMs: 30_000, text: `line ${startMs}`, words: [],
    })),
    acquire: () => () => {},
  }
  let dispose
  try {
    const bridge = await loadModule('../src/modules/desktopLyrics/bridge.ts', {
      vue: { ref, watch },
      '@tauri-apps/api/core': { invoke: async (command, args) => {
        if (command === 'publish_desktop_lyrics') frames.push(args.frame)
      } },
      '@tauri-apps/api/event': { listen: async () => () => {} },
      '@/stores/player': { usePlayerStore: () => player },
      '@/stores/settings': { useSettingsStore: () => ({ desktopLyrics: undefined }) },
      '@/stores/lyricOffset': { useLyricOffsetStore: () => ({ effectiveOffsetMs: () => 0 }) },
      '@/stores/currentLyrics': { useCurrentLyricsStore: () => currentLyrics },
      './frame': frameModule,
      './style': styleModule,
      './timeline': timelineModule,
      '@/utils/logger': { createLogger: () => ({ warn() {} }) },
      '@/utils/logSanitizer': { summarizeLogError: String },
    })
    dispose = bridge.installDesktopLyricsBridge()
    await bridge.openDesktopLyricsWindow()
    await flush()
    const runtime = {
      clock, frames,
      latest: () => frames.at(-1),
      desktopAt: (delta = 0) => timelineModule.anchoredPositionMs(frames.at(-1), now + delta),
      async tick(delta = 150) {
        now += delta
        for (const callback of intervals.values()) callback()
        await flush()
      },
    }
    await run(runtime)
  } finally {
    dispose?.()
    Date.now = originalNow
    globalThis.setInterval = originalInterval
    globalThis.clearInterval = originalClearInterval
  }
}

let passed = 0
let failed = 0
async function regression(name, run, options) {
  try {
    await withRuntime(run, options)
    passed++
    console.log(`PASS ${name}`)
  } catch (error) {
    failed++
    console.error(`FAIL ${name}: ${error.stack}`)
  }
}

await regression('pending seek never sweeps ahead between desktop lyric anchors', async runtime => {
  runtime.clock.seek(60_000)
  await runtime.tick()
  assert.equal(runtime.latest().isPlaying, true, 'the toolbar must retain the playing state')
  for (let index = 0; index < 8; index++) {
    assert.equal(runtime.desktopAt(149), 60_000, 'the pending seek target must stay frozen between frames')
    await runtime.tick()
    assert.equal(runtime.desktopAt(), 60_000)
  }
})

await regression('long pending seeks do not publish repeated drift corrections', async runtime => {
  runtime.clock.seek(60_000)
  await runtime.tick()
  const initialCount = runtime.frames.length
  for (let index = 0; index < 40; index++) await runtime.tick()
  assert.ok(runtime.frames.length - initialCount <= 2, 'only periodic anchors are needed while the target is frozen')
  assert.equal(runtime.desktopAt(10_000), 60_000, 'even an old frozen anchor cannot advance')
})

await regression('seek confirmation restarts the desktop clock at the same target and playback speed', async runtime => {
  runtime.clock.seek(60_000)
  await runtime.tick()
  await runtime.tick()
  const initialCount = runtime.frames.length
  runtime.clock.settle(60_000)
  await runtime.tick(0)
  assert.equal(runtime.frames.length, initialCount + 1, 'clock resumption must publish even without a position change')
  assert.equal(runtime.desktopAt(), 60_000)
  assert.equal(runtime.desktopAt(400), 60_500, 'desktop interpolation must resume at 1.25x')
}, { speed: 1.25 })

await regression('forward backward and consecutive seeks display the newest target without intermediate sweeps', async runtime => {
  for (const target of [120_000, 10_000, 90_000]) {
    runtime.clock.seek(target)
    await runtime.tick()
    assert.equal(runtime.desktopAt(), target)
    assert.equal(runtime.desktopAt(149), target, 'each new target remains frozen before confirmation')
  }
  runtime.clock.seek(60_000)
  runtime.clock.seek(120_000)
  await runtime.tick()
  assert.equal(runtime.desktopAt(149), 120_000, 'the latest seek wins before the next publication')
  runtime.clock.settle(120_000)
  await runtime.tick(0)
  assert.equal(runtime.desktopAt(500), 120_500)
})

await regression('paused seeks remain fixed before and after backend confirmation', async runtime => {
  runtime.clock.seek(60_000)
  await runtime.tick()
  assert.equal(runtime.latest().isPlaying, false)
  assert.equal(runtime.desktopAt(2_000), 60_000)
  runtime.clock.settle(60_000)
  await runtime.tick()
  assert.equal(runtime.desktopAt(2_000), 60_000)
}, { playing: false })

await regression('failed seek rollback restores the backend position and running clock', async runtime => {
  runtime.clock.seek(120_000)
  await runtime.tick()
  assert.equal(runtime.desktopAt(149), 120_000, 'the failed request must not animate before its outcome')
  runtime.clock.settle(10_500)
  await runtime.tick(0)
  assert.equal(runtime.desktopAt(), 10_500, 'rollback must replace the optimistic target')
  assert.equal(runtime.desktopAt(500), 11_000)
  assert.equal(runtime.latest().isPlaying, true)
})

console.log(`desktop lyrics seek: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
