import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import ts from 'typescript'

const path = new URL('../src/modules/settings/uiScale.ts', import.meta.url)
assert.ok(existsSync(path), 'main-window UI scaling must have a runtime installer')
const source = await readFile(path, 'utf8')
const settingsSource = await readFile(new URL('../src/stores/settings.ts', import.meta.url), 'utf8')
const normalizerSource = settingsSource.match(/export function normalizeUiScale[\s\S]*?\n}/)?.[0]
assert.ok(normalizerSource)
const normalizer = ts.transpileModule(`${normalizerSource}`, {
  compilerOptions: { module: ts.ModuleKind.CommonJS },
}).outputText
const normalization = {}
new Function('exports', 'UI_SCALE_DEFAULT', 'UI_SCALE_MIN', 'UI_SCALE_MAX', 'UI_SCALE_STEP', normalizer)(normalization, 100, 25, 1000, 5)
Object.assign(normalization, { UI_SCALE_DEFAULT: 100, UI_SCALE_MIN: 25, UI_SCALE_MAX: 1000, UI_SCALE_STEP: 5 })

function load({ native = true, supported = true, mac = false, setZoom = async () => {}, check = async () => supported } = {}) {
  const exports = {}
  const properties = new Map()
  const geometry = { width: 1280, height: 800, dpi: 1, workWidth: 3840, workHeight: 2160, x: 3000, y: 1500 }
  const minimums = []
  const listeners = new Map()
  class Size { constructor(width, height) { this.width = width; this.height = height } }
  class Position { constructor(x, y) { this.x = x; this.y = y } }
  const hostWindow = {
    innerSize: async () => new Size(geometry.width * geometry.dpi, geometry.height * geometry.dpi),
    outerSize: async () => new Size(geometry.width * geometry.dpi, geometry.height * geometry.dpi),
    outerPosition: async () => new Position(geometry.x, geometry.y),
    scaleFactor: async () => geometry.dpi,
    isMaximized: async () => false,
    isFullscreen: async () => false,
    setMinSize: async size => minimums.push(size),
    setSize: async size => { geometry.width = size.width; geometry.height = size.height },
    setPosition: async position => { geometry.x = position.x; geometry.y = position.y },
  }
  for (const event of ['Resized', 'Moved', 'ScaleChanged']) {
    hostWindow[`on${event}`] = async handler => {
      listeners.set(event, handler)
      return () => listeners.delete(event)
    }
  }
  globalThis.document = { documentElement: { style: { setProperty: (key, value) => properties.set(key, value) } } }
  const dependencies = {
    vue,
    '@tauri-apps/api/core': { isTauri: () => native, invoke: check },
    '@tauri-apps/api/webview': { getCurrentWebview: () => ({ setZoom }) },
    '@tauri-apps/api/window': {
      LogicalSize: Size, PhysicalPosition: Position, getCurrentWindow: () => hostWindow,
      currentMonitor: async () => ({
        scaleFactor: geometry.dpi,
        workArea: { size: new Size(geometry.workWidth, geometry.workHeight), position: new Position(0, 0) },
      }),
    },
    '@/stores/settings': normalization,
    '@/modules/shortcuts/platform': { isMacPlatform: mac },
    '@/utils/logger': { createLogger: () => ({ warn() {} }) },
  }
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `unexpected dependency: ${name}`)
    return dependencies[name]
  }, exports)
  return { ...exports, properties, geometry, minimums, listeners, hostWindow }
}

const tick = async () => { await vue.nextTick(); await new Promise(resolve => setImmediate(resolve)) }

{
  const runtime = load({ mac: true })
  const stop = await runtime.installUiScale(vue.reactive({ uiScale: 25 }))
  assert.deepEqual({ ...runtime.minimums.at(-1) }, { width: 240, height: 219 }, 'macOS reserves the native traffic-light height at small scales')
  assert.equal(runtime.uiScaleLimitForSize(960, 485, 52), 60, 'small-screen limits include the unscaled macOS title bar')
  stop()
}

{
  const scales = []
  const runtime = load({ setZoom: async value => scales.push(value) })
  const settings = vue.reactive({ uiScale: 150 })
  const stop = await runtime.installUiScale(settings)
  assert.deepEqual(scales, [1.5], 'startup restores saved scale before the window is shown')
  assert.equal(runtime.properties.get('--app-ui-scale'), '1.5')
  assert.deepEqual({ ...runtime.minimums.at(-1) }, { width: 1440, height: 1080 })
  assert.equal(runtime.geometry.width, 1440, 'increasing scale grows an undersized window')
  assert.equal(runtime.geometry.height, 1080)
  assert.equal(runtime.geometry.x, 2400, 'grown windows remain within the monitor work area')
  assert.equal(runtime.geometry.y, 1080)
  settings.uiScale = 75
  await tick()
  settings.uiScale = 100
  await tick()
  assert.deepEqual(scales, [1.5, 0.75, 1])
  stop()
  settings.uiScale = 200
  await tick()
  assert.equal(scales.length, 3, 'unmounted applications stop applying scale')
  assert.equal(runtime.listeners.size, 0, 'native window listeners are released')
}

{
  const runtime = load()
  assert.equal(typeof runtime.uiScaleLimitForSize, 'function', 'scale limits must depend on available display space')
  for (const [width, height, expected] of [[960, 720, 100], [1920, 1080, 150], [2560, 1400, 190], [3840, 2100, 290], [1366 / 1.5, 728 / 1.5, 65]]) {
    assert.equal(runtime.uiScaleLimitForSize(width, height), expected)
  }
  const scales = []
  const sized = load({ setZoom: async value => scales.push(value) })
  const settings = vue.reactive({ uiScale: 250 })
  const stop = await sized.installUiScale(settings)
  assert.equal(scales.at(-1), 2.5)
  sized.geometry.dpi = 1.5
  sized.geometry.workWidth = 2560
  sized.geometry.workHeight = 1400
  sized.listeners.get('ScaleChanged')()
  await new Promise(resolve => setTimeout(resolve, 150))
  await tick()
  assert.equal(sized.uiScaleMax.value, 125, 'the work area is converted using system DPI')
  assert.equal(settings.uiScale, 125, 'moving to a smaller display clamps the saved value')
  assert.equal(scales.at(-1), 1.25)
  assert.deepEqual({ ...sized.minimums.at(-1) }, { width: 1200, height: 900 })
  stop()
}

{
  const runtime = load({ setZoom: async value => { if (value === 1.5) throw new Error('startup failure') } })
  const settings = vue.reactive({ uiScale: 150 })
  const stop = await runtime.installUiScale(settings)
  assert.equal(settings.uiScale, 100)
  assert.deepEqual({ ...runtime.minimums.at(-1) }, { width: 960, height: 720 }, 'startup failures restore minimum window constraints')
  stop()
}

{
  const scales = []
  const runtime = load({ setZoom: async value => scales.push(value) })
  const settings = vue.reactive({ uiScale: 100 })
  const stop = await runtime.installUiScale(settings)
  let release
  runtime.hostWindow.setMinSize = async () => new Promise(resolve => { release = resolve })
  settings.uiScale = 125
  await tick()
  stop()
  release()
  await tick()
  assert.deepEqual(scales, [1], 'disposal during an IPC call prevents later native mutations')
  assert.equal(runtime.geometry.height, 800, 'disposed applications do not resize the window')
}

for (const native of [false, true]) {
  let calls = 0
  const runtime = load({ native, supported: false, setZoom: async () => calls++ })
  const stop = await runtime.installUiScale(vue.reactive({ uiScale: 150 }))
  assert.equal(calls, 0, 'preview and unsupported macOS never call the native zoom API')
  assert.equal(runtime.uiScaleSupported.value, false)
  stop()
}

{
  let release
  const scales = []
  const runtime = load({ setZoom: async value => {
    scales.push(value)
    if (value === 1.25) await new Promise(resolve => { release = resolve })
    if (value === 1.75) throw new Error('native failure')
  } })
  const settings = vue.reactive({ uiScale: 100 })
  const stop = await runtime.installUiScale(settings)
  settings.uiScale = 125
  await tick()
  settings.uiScale = 150
  await tick()
  assert.deepEqual(scales, [1, 1.25], 'native updates are serialized')
  release()
  await tick()
  assert.deepEqual(scales, [1, 1.25, 1.5], 'the latest requested value wins')
  settings.uiScale = 175
  await tick()
  assert.equal(settings.uiScale, 150, 'failed changes restore the last applied value')
  assert.equal(runtime.uiScaleError.value, true, 'failed updates remain visible until a successful change')
  assert.equal(runtime.properties.get('--app-ui-scale'), '1.5')
  settings.uiScale = 200
  await tick()
  assert.equal(scales.at(-1), 2, 'a rejected update does not poison later requests')
  stop()
}

{
  const runtime = load({ check: async () => { throw new Error('IPC failure') } })
  const stop = await runtime.installUiScale(vue.reactive({ uiScale: 150 }))
  assert.equal(runtime.uiScaleSupported.value, false)
  assert.equal(runtime.uiScaleError.value, true)
  stop()
}

console.log('PASS UI scale startup, live changes, reset, platform guard, serialization, recovery, and cleanup')
