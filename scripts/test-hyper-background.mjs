import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const componentSource = await readFile(new URL('../src/components/HyperBackground.vue', import.meta.url), 'utf8')
const script = compileScript(parse(componentSource).descriptor, { id: 'hyper-background-test', genDefaultAs: 'component' })
const compile = source => ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const componentCode = compile(script.content)
const frameLoopCode = compile(await readFile(new URL('../src/utils/frameLoop.ts', import.meta.url), 'utf8'))
const renderer = vue.createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText() {}, setElementText() {}, patchProp() {},
  parentNode: node => node.parent, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { node.parent.children = node.parent.children.filter(child => child !== node) },
})

class ListenerTarget {
  listeners = new Map()
  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set())
    this.listeners.get(type).add(listener)
  }
  removeEventListener(type, listener) { this.listeners.get(type)?.delete(listener) }
  dispatch(type) {
    for (const listener of [...(this.listeners.get(type) || [])]) listener({ type, preventDefault() {} })
  }
  listenerCount() { return [...this.listeners.values()].reduce((count, set) => count + set.size, 0) }
}

function mount(overrides = {}, { hidden = false, dpr = 1, softwareRenderer = false } = {}) {
  let now = 1000, nextFrame = 0, resizeCallback
  const frames = new Map(), draws = [], canvasSizes = []
  const requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame }
  const cancelAnimationFrame = id => frames.delete(id)
  const frameLoop = {}
  new Function('exports', 'requestAnimationFrame', 'cancelAnimationFrame', frameLoopCode)(
    frameLoop, requestAnimationFrame, cancelAnimationFrame,
  )
  const props = vue.reactive({ playing: true, lowPerformance: false, suspended: false, ...overrides })
  if (props.lowPerformance) frameLoop.setFrameInterval(1000 / 30)
  const document = Object.assign(new ListenerTarget(), { hidden })
  const canvas = Object.assign(new ListenerTarget(), { clientWidth: 1280, clientHeight: 720 })
  let width = 300, height = 150
  Object.defineProperties(canvas, {
    width: { get: () => width, set(value) { width = value; canvasSizes.push([width, height]) } },
    height: { get: () => height, set(value) { height = value; canvasSizes.push([width, height]) } },
  })
  const noop = () => {}
  const gl = {
    VERTEX_SHADER: 0x8b31, FRAGMENT_SHADER: 0x8b30, COMPILE_STATUS: 0x8b81,
    LINK_STATUS: 0x8b82, ARRAY_BUFFER: 0x8892, STATIC_DRAW: 0x88e4,
    FLOAT: 0x1406, TRIANGLE_STRIP: 5, RENDERER: 0x1f01, VENDOR: 0x1f00,
    createShader: () => ({}), createProgram: () => ({}), createBuffer: () => ({}),
    getShaderParameter: () => true, getProgramParameter: () => true,
    getParameter: () => softwareRenderer ? 'SwiftShader software renderer' : 'hardware renderer', getExtension: () => null,
    getAttribLocation: () => 0, getUniformLocation: () => ({}),
    shaderSource: noop, compileShader: noop, deleteShader: noop, attachShader: noop,
    linkProgram: noop, detachShader: noop, useProgram: noop, bindBuffer: noop,
    bufferData: noop, enableVertexAttribArray: noop, vertexAttribPointer: noop,
    viewport: noop, uniform2f: noop, uniform1f: noop, uniform4fv: noop,
    deleteBuffer: noop, deleteProgram: noop,
    drawArrays(mode, start, count) {
      assert.deepEqual([mode, start, count], [5, 0, 4], 'each draw covers the canvas')
      draws.push({ now, width, height })
    },
  }
  canvas.getContext = type => { assert.equal(type, 'webgl'); return gl }
  const dependencies = {
    vue,
    '@/utils/frameLoop': frameLoop,
    '@/utils/logger': { createLogger: () => ({ info: noop, warn: noop, error: noop }) },
    '@/shaders/hyperBackground': { hyperBackgroundVertexShader: '', hyperBackgroundFragmentShader: '' },
  }
  class ResizeObserver {
    constructor(callback) { resizeCallback = callback }
    observe() {}
    disconnect() { resizeCallback = undefined }
  }
  const component = new Function('require', 'exports', 'document', 'window', 'performance', 'ResizeObserver', `${componentCode}\nreturn component`)(
    name => { assert.ok(name in dependencies, `unexpected dependency: ${name}`); return dependencies[name] },
    {}, document, { devicePixelRatio: dpr }, { now: () => now }, ResizeObserver,
  )
  const setup = component.setup
  component.setup = (input, context) => {
    const state = setup(input, context)
    state.canvas.value = canvas
    return () => null
  }
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...props }) })
  app.mount({ children: [] })
  return {
    props, draws, document, canvas, canvasSizes,
    get pendingFrames() { return frames.size },
    frame(elapsed = 1000 / 60) {
      now += elapsed
      for (const [id, callback] of [...frames]) if (frames.delete(id)) callback(now)
    },
    resize(w, h) { canvas.clientWidth = w; canvas.clientHeight = h; resizeCallback?.() },
    dispose() { app.unmount() },
  }
}

let failed = 0, passed = 0
async function test(name, run) {
  try { await run(); passed++; console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

for (const refreshRate of [60, 120, 144]) {
  await test(`the background keeps its 60fps budget on a ${refreshRate}Hz display`, () => {
    const r = mount()
    try {
      assert.equal(r.draws.length, 1, 'a visible canvas draws synchronously before the first scheduled frame')
      for (let index = 0; index < refreshRate; index++) r.frame(1000 / refreshRate)
      const count = r.draws.length - 1
      assert.ok(count >= 55 && count <= 61, `expected 55-61 frames in one second, got ${count}`)
    } finally { r.dispose() }
  })
  await test(`low performance mode stays near 30fps on a ${refreshRate}Hz display`, () => {
    const r = mount({ lowPerformance: true })
    try {
      for (let index = 0; index < refreshRate; index++) r.frame(1000 / refreshRate)
      const count = r.draws.length - 1
      assert.ok(count >= 26 && count <= 31, `expected 26-31 frames without stacked throttling, got ${count}`)
    } finally { r.dispose() }
  })
}

await test('the software renderer keeps its 20fps budget on a high refresh display', () => {
  const r = mount({}, { softwareRenderer: true })
  try {
    for (let index = 0; index < 144; index++) r.frame(1000 / 144)
    const count = r.draws.length - 1
    assert.ok(count >= 19 && count <= 21, `expected 19-21 frames in one second, got ${count}`)
  } finally { r.dispose() }
})

await test('paused playback draws once, redraws on resize, then stays idle', async () => {
  const r = mount({ playing: false })
  try {
    assert.equal(r.draws.length, 1)
    assert.equal(r.pendingFrames, 0)
    r.resize(900, 600)
    r.frame()
    assert.deepEqual(r.draws.at(-1), { now: 1000 + 1000 / 60, width: 900, height: 600 })
    assert.equal(r.draws.length, 2)
    assert.equal(r.pendingFrames, 0, 'a paused resized canvas does not keep a frame loop alive')
    r.props.playing = true
    await vue.nextTick()
    r.frame()
    assert.equal(r.draws.length, 3)
    assert.equal(r.pendingFrames, 1, 'playback restarts the stopped loop')
  } finally { r.dispose() }
})

await test('a suspended first mount waits for resume before drawing', async () => {
  const r = mount({ suspended: true })
  try {
    assert.equal(r.draws.length, 0)
    assert.equal(r.pendingFrames, 0)
    r.props.suspended = false
    await vue.nextTick()
    r.frame()
    assert.equal(r.draws.length, 1)
    assert.equal(r.pendingFrames, 1)
    r.props.suspended = true
    await vue.nextTick()
    r.frame()
    assert.equal(r.draws.length, 1, 'suspension retains the previous picture without another draw')
    assert.equal(r.pendingFrames, 0)
  } finally { r.dispose() }
})

await test('a hidden first mount waits for visibility before drawing', () => {
  const r = mount({}, { hidden: true })
  try {
    assert.equal(r.draws.length, 0)
    assert.equal(r.pendingFrames, 0)
    r.document.hidden = false
    r.document.dispatch('visibilitychange')
    r.frame()
    assert.equal(r.draws.length, 1)
    assert.equal(r.pendingFrames, 1)
  } finally { r.dispose() }
})

await test('hidden windows cancel frames and restore one loop after a long gap', () => {
  const r = mount()
  try {
    r.document.hidden = true
    r.document.dispatch('visibilitychange')
    assert.equal(r.pendingFrames, 0)
    r.resize(900, 600)
    r.frame(10000)
    assert.equal(r.draws.length, 1)
    assert.equal(r.pendingFrames, 0)
    r.document.hidden = false
    r.document.dispatch('visibilitychange')
    r.frame()
    assert.equal(r.draws.length, 2)
    assert.equal(r.pendingFrames, 1)
  } finally { r.dispose() }
  assert.equal(r.pendingFrames, 0)
  assert.equal(r.document.listenerCount(), 0)
  assert.equal(r.canvas.listenerCount(), 0)
})

await test('fractional scaling does not clear and reallocate the canvas on every frame', () => {
  const r = mount({}, { dpr: 1.25 })
  try {
    assert.deepEqual(r.draws[0], { now: 1000, width: 1600, height: 900 })
    const initialResizes = r.canvasSizes.length
    for (let index = 0; index < 60; index++) r.frame()
    assert.equal(r.canvasSizes.length, initialResizes)
  } finally { r.dispose() }
})

await test('context loss stops rendering and restoring a paused canvas paints it again', () => {
  const r = mount({ playing: false })
  try {
    r.canvas.dispatch('webglcontextlost')
    assert.equal(r.pendingFrames, 0)
    r.canvas.dispatch('webglcontextrestored')
    assert.equal(r.draws.length, 2)
    assert.equal(r.pendingFrames, 0)
  } finally { r.dispose() }
})

if (failed) throw new Error(`Hyper background: ${failed} failed, ${passed} passed`)
console.log(`Hyper background: ${passed} passed`)
