import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/LyricsView.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, {
  id: 'lyrics-display-controls-test', genDefaultAs: 'component', inlineTemplate: true,
})
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

class ListenerTarget {
  listeners = new Map()
  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set())
    this.listeners.get(type).add(listener)
  }
  removeEventListener(type, listener) { this.listeners.get(type)?.delete(listener) }
  dispatch(type, event) {
    for (const listener of [...(this.listeners.get(type) ?? [])]) listener(event)
  }
}

function element(type) {
  return Object.assign(new ListenerTarget(), {
    type, children: [], props: {}, clientWidth: 700, clientHeight: 600,
    appendChild(node) { node.parent = this; node.parentElement = this; this.children.push(node) },
  })
}

const renderer = vue.createRenderer({
  createElement: element, createText: text => ({ text }), createComment: text => ({ text }),
  setText(node, text) { node.text = text },
  setElementText(node, text) { node.text = text; node.children = [] },
  patchProp(node, key, _, value) { node.props[key] = value },
  parentNode: node => node.parent ?? null,
  nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  insert(node, parent, anchor = null) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
  },
  remove(node) { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1) },
})

const richLyrics = [{
  startMs: 0, durationMs: 30000, text: '歌词', translation: 'Translated lyric', roman: 'ge ci',
  words: [
    { startMs: 0, durationMs: 15000, text: '歌', romanWord: 'ge' },
    { startMs: 15000, durationMs: 15000, text: '词', romanWord: 'ci' },
  ],
}]

async function mount({ lyrics = richLyrics, settingOverrides = {} } = {}) {
  let now = 1000, frameId = 0
  const frames = new Map()
  // The component now uses the shared frame loop (@/utils/frameLoop); wire it to the same
  // fake frame table here
  const scheduleFrame = callback => { const id = ++frameId; frames.set(id, callback); return id }
  const cancelFrame = id => frames.delete(id)
  const document = Object.assign(new ListenerTarget(), { hidden: false, fonts: { status: 'loaded' } })
  const window = new ListenerTarget()
  const players = []
  class FakeDomLyricPlayer extends ListenerTarget {
    size = [0, 0]
    currentLyricGroups = []
    lyricGroupSize = new WeakMap()
    times = []
    element = element('amll-player')
    constructor() { super(); players.push(this) }
    setLyricLines(lines, time) { this.lines = lines; this.initialTime = time }
    getLyricLines() { return this.lines }
    getElement() { return this.element }
    setCurrentTime(time, seek = false) { this.times.push([time, seek]) }
    setOptimizeOptions() {}
    setEnableBlur() {}
    setBlurAmount() {}
    setWordFadeWidth() {}
    update() {}
    calcLayout() {}
    resetScroll() {}
    resume() {}
    pause() {}
    dispose() {}
  }
  const settings = vue.reactive({
    advancedLyrics: true, showTranslation: false, showRomanization: false,
    lyricBlur: true, lyricBlurAmount: 1, lyricFontScale: 1, lyricEmphasisThresholdMs: 1000,
    ...settingOverrides,
  })
  const clipboardWrites = []
  const menu = {}
  const dependencies = {
    vue,
    '@amll-core/lyric-player/dom/index.ts': { DomLyricPlayer: FakeDomLyricPlayer },
    '@/stores/settings': { useSettingsStore: () => settings },
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@tauri-apps/plugin-clipboard-manager': { writeText: async text => { clipboardWrites.push(text) } },
    '@/components/ui/ContextMenu.vue': { default: {
      props: ['open', 'x', 'y', 'items'], emits: ['click', 'update:open'], inheritAttrs: false,
      setup(props, { emit }) {
        menu.props = props
        menu.click = item => emit('click', item)
        menu.close = () => emit('update:open', false)
        return () => null
      },
    } },
    '@/utils/frameLoop': { scheduleFrame, cancelFrame },
    '@/utils/contextMenu': { createContextMenuItem: (label, options) => ({ label, ...options }) },
  }
  const component = new Function(
    'require', 'exports', 'document', 'window', 'performance', 'requestAnimationFrame',
    'cancelAnimationFrame', 'ResizeObserver', `${compiled}\nreturn component`,
  )(name => {
    assert.ok(name in dependencies, `unexpected dependency: ${name}`)
    return dependencies[name]
  }, {}, document, window, { now: () => now }, scheduleFrame, cancelFrame, undefined)
  const clock = vue.ref(4200)
  const seekEvents = []
  const props = vue.reactive({ lyrics, currentTimeMs: () => clock.value, isPlaying: true, lyricOffsetMs: 700 })
  const root = { children: [] }
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...props, onSeek: time => seekEvents.push(time) }) })
  app.mount(root)
  const settle = async () => {
    await vue.nextTick()
    for (let index = 0; index < 6; index++) {
      now += 16
      for (const [id, callback] of [...frames]) {
        if (frames.delete(id)) callback(now)
      }
    }
    await vue.nextTick()
  }
  await settle()
  assert.equal(players.length, 1, 'the real template must mount the AMLL host')
  const player = players[0]
  player.times = []
  function find(predicate, node = root) {
    if (predicate(node)) return node
    for (const child of node.children ?? []) {
      const match = find(predicate, child)
      if (match) return match
    }
  }
  const findButton = setting => find(node => node.type === 'button' && node.props['aria-label'] === `settings.${setting}`) ?? null
  const findControls = () => find(node => node.props?.role === 'group' && node.props['aria-label'] === 'settings.lyrics') ?? null
  const host = find(node => typeof node.props?.class === 'string' && node.props.class.split(' ').includes('lyrics-scroll'))
  const button = setting => {
    const node = findButton(setting)
    assert.ok(node, `missing lyrics display button: settings.${setting}`)
    return node
  }
  return {
    settings, player, props, clock, seekEvents, settle, button, findButton, findControls,
    host, menu, clipboardWrites,
    async controlsContextMenu() {
      const controls = findControls()
      assert.ok(controls, 'available controls must exist before hiding')
      assert.equal(typeof controls.props.onContextmenu, 'function', 'controls must handle their own right click')
      const event = contextMenuEvent()
      controls.props.onContextmenu(event)
      await settle()
      assert.ok(event.prevented, 'the controls context menu must prevent the native menu')
      assert.ok(event.stopped, 'the controls context menu must not reach the lyric menu handler')
      return event
    },
    async hideControls() {
      await this.controlsContextMenu()
      await this.selectMenu('hide-display-controls')
    },
    async lineContextMenu(overrides = {}) {
      const event = contextMenuEvent({ type: 'line-contextmenu', lineIndex: 0, ...overrides })
      player.dispatch('line-contextmenu', event)
      await settle()
      return event
    },
    async hostContextMenu() {
      const event = contextMenuEvent()
      host.props.onContextmenu(event)
      await settle()
      return event
    },
    async selectMenu(id) {
      const item = menu.props.items.find(item => item.id === id)
      assert.ok(item, `missing lyric menu action: ${id}`)
      menu.click(item)
      await settle()
    },
    async dismissMenu() { menu.close(); await settle() },
    async click(setting) {
      const node = button(setting)
      assert.ok(!node.props.disabled, 'display preferences must remain available')
      let stopped = false
      const event = { type: 'click', stopPropagation() { stopped = true } }
      assert.equal(typeof node.props.onClick, 'function', 'the rendered button must have a click handler')
      node.props.onClick(event)
      assert.ok(stopped, 'display clicks must stop before reaching the lyric host')
      await settle()
    },
    assertPressed(translation, romanization) {
      assert.equal(String(button('show_translation').props['aria-pressed']), String(translation))
      assert.equal(String(button('show_romanization').props['aria-pressed']), String(romanization))
    },
    assertAvailable(translation, romanization) {
      assert.equal(Boolean(findButton('show_translation')), translation, 'translation button availability must follow the source lyrics')
      assert.equal(Boolean(findButton('show_romanization')), romanization, 'romanization button availability must follow the source lyrics')
      assert.equal(Boolean(findControls()), translation || romanization, 'the display group must exist only when it has an available button')
    },
    assertPlaybackUnchanged() {
      assert.deepEqual(seekEvents, [], 'display preferences must not emit a playback seek')
      assert.equal(clock.value, 4200, 'display preferences must preserve the playback clock')
      assert.equal(player.initialTime, 4900)
      assert.ok(player.times.every(([time]) => time === 4900), 'AMLL reflow must stay at the current time plus offset')
    },
    stop() { app.unmount(); assert.equal(frames.size, 0) },
  }
}

function assertAuxiliaryLyrics(player, translation, romanization, romanWords) {
  assert.equal(player.lines[0].translatedLyric, translation)
  assert.equal(player.lines[0].romanLyric, romanization)
  assert.deepEqual(player.lines[0].words.map(word => word.romanWord ?? ''), romanWords)
}

function contextMenuEvent(overrides = {}) {
  return {
    type: 'contextmenu', clientX: 240, clientY: 320, prevented: false, stopped: false,
    preventDefault() { this.prevented = true },
    stopPropagation() { this.stopped = true },
    ...overrides,
  }
}

const hideControlsItem = { id: 'hide-display-controls', label: 'lyrics.hide_display_controls' }
const showControlsItem = { id: 'show-display-controls', label: 'lyrics.show_display_controls' }

function assertMenu(menu, x, y, items) {
  assert.equal(menu.props.open, true)
  assert.equal(menu.props.x, x)
  assert.equal(menu.props.y, y)
  assert.deepEqual(menu.props.items.map(({ id, label }) => ({ id, label })), items)
}

function assertLyricMenu(menu, x, y, displayItem = null) {
  assertMenu(menu, x, y, [
    { id: 'copy-line', label: 'lyrics.copy_line' },
    { id: 'copy-all', label: 'lyrics.copy_all' },
    { id: 'seek', label: 'lyrics.seek_here' },
    ...(displayItem ? [displayItem] : []),
  ])
}

let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('display buttons independently toggle translation and line and word romanization', async () => {
  const r = await mount()
  try {
    assertAuxiliaryLyrics(r.player, '', '', ['', ''])
    r.assertPressed(false, false)
    await r.click('show_translation')
    assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [true, false])
    assertAuxiliaryLyrics(r.player, 'Translated lyric', '', ['', ''])
    r.assertPressed(true, false)
    await r.click('show_romanization')
    assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [true, true])
    assertAuxiliaryLyrics(r.player, 'Translated lyric', 'ge ci', ['ge', 'ci'])
    r.assertPressed(true, true)
    await r.click('show_translation')
    assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [false, true])
    assertAuxiliaryLyrics(r.player, '', 'ge ci', ['ge', 'ci'])
    r.assertPressed(false, true)
    await r.click('show_romanization')
    assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [false, false])
    assertAuxiliaryLyrics(r.player, '', '', ['', ''])
    r.assertPressed(false, false)
    r.assertPlaybackUnchanged()
  } finally { r.stop() }
})

await test('external display setting changes update pressed states and AMLL content', async () => {
  const r = await mount({ settingOverrides: { showTranslation: true } })
  try {
    r.assertPressed(true, false)
    assertAuxiliaryLyrics(r.player, 'Translated lyric', '', ['', ''])
    Object.assign(r.settings, { showTranslation: false, showRomanization: true })
    await r.settle()
    r.assertPressed(false, true)
    assertAuxiliaryLyrics(r.player, '', 'ge ci', ['ge', 'ci'])
    r.settings.showTranslation = true
    await r.settle()
    r.assertPressed(true, true)
    assertAuxiliaryLyrics(r.player, 'Translated lyric', 'ge ci', ['ge', 'ci'])
    r.assertPlaybackUnchanged()
  } finally { r.stop() }
})

const plainLine = { startMs: 0, durationMs: 30000, text: 'Plain lyric', words: [] }
const wordRomanLine = {
  startMs: 0, durationMs: 30000, text: '歌词',
  words: [
    { startMs: 0, durationMs: 15000, text: '歌' },
    { startMs: 15000, durationMs: 15000, text: '词', romanWord: 'ci' },
  ],
}

for (const [name, lyrics, translation, romanization] of [
  ['translation only', [{ ...plainLine, translation: ' Translation ' }], true, false],
  ['line romanization only', [{ ...plainLine, roman: ' Romanization ' }], false, true],
  ['word romanization only', [wordRomanLine], false, true],
  ['no auxiliary lyrics', [plainLine], false, false],
  ['whitespace auxiliary text', [{
    ...wordRomanLine, translation: ' \n\t ', roman: '\t ',
    words: [{ startMs: 0, durationMs: 30000, text: '歌词', romanWord: ' \n\t ' }],
  }], false, false],
  ['ruby annotations only', [{
    ...plainLine,
    words: [{
      startMs: 0, durationMs: 30000, text: '詞',
      ruby: [{ startMs: 0, durationMs: 30000, text: 'ことば' }],
    }],
  }], false, false],
  ['both auxiliary tracks with their display settings off', richLyrics, true, true],
]) {
  await test(`display control availability follows ${name}`, async () => {
    const r = await mount({ lyrics })
    try {
      r.assertAvailable(translation, romanization)
      assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [false, false])
      assert.equal(r.player.lines[0].translatedLyric, '')
      assert.equal(r.player.lines[0].romanLyric, '')
      assert.ok(r.player.lines[0].words.every(word => !word.romanWord))
      r.assertPlaybackUnchanged()
    } finally { r.stop() }
  })
}

await test('display controls respond when auxiliary source lyrics arrive and disappear', async () => {
  const r = await mount({ lyrics: [plainLine] })
  try {
    r.assertAvailable(false, false)
    r.props.lyrics = richLyrics
    await r.settle()
    r.assertAvailable(true, true)
    r.assertPressed(false, false)
    assertAuxiliaryLyrics(r.player, '', '', ['', ''])
    await r.click('show_translation')
    await r.click('show_romanization')
    assertAuxiliaryLyrics(r.player, 'Translated lyric', 'ge ci', ['ge', 'ci'])
    r.props.lyrics = [{ ...plainLine, translation: 'Only translation' }]
    await r.settle()
    r.assertAvailable(true, false)
    assertAuxiliaryLyrics(r.player, 'Only translation', '', [''])
    r.props.lyrics = [plainLine]
    await r.settle()
    r.assertAvailable(false, false)
    assertAuxiliaryLyrics(r.player, '', '', [''])
    assert.equal(r.player.lines[0].words[0].word, 'Plain lyric')
    assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [true, true])
    r.props.lyrics = [wordRomanLine]
    await r.settle()
    r.assertAvailable(false, true)
    assert.equal(String(r.button('show_romanization').props['aria-pressed']), 'true')
    assertAuxiliaryLyrics(r.player, '', '', ['', 'ci'])
    r.assertPlaybackUnchanged()
  } finally { r.stop() }
})

await test('right clicking display controls opens a hide menu and changes visibility only when its action is selected', async () => {
  const r = await mount({ settingOverrides: { showTranslation: true } })
  try {
    const settingsBefore = { ...r.settings }
    await r.controlsContextMenu()
    assertMenu(r.menu, 240, 320, [hideControlsItem])
    r.assertAvailable(true, true)
    r.assertPressed(true, false)
    assert.deepEqual({ ...r.settings }, settingsBefore)
    assertAuxiliaryLyrics(r.player, 'Translated lyric', '', ['', ''])
    r.assertPlaybackUnchanged()
    await r.dismissMenu()
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(true, true)
    await r.controlsContextMenu()
    await r.selectMenu('hide-display-controls')
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(false, false)
    r.host.dispatch('pointerdown', { type: 'pointerdown' })
    r.host.dispatch('keydown', { type: 'keydown', key: 'Tab' })
    r.props.lyrics = [...richLyrics]
    await r.settle()
    r.assertAvailable(false, false)
    assert.deepEqual({ ...r.settings }, settingsBefore)
    r.assertPlaybackUnchanged()
  } finally { r.stop() }
})

await test('lyric line menus show a visibility action while retaining copy and seek actions', async () => {
  const r = await mount({ lyrics: [richLyrics[0], {
    startMs: 6000, durationMs: 10000, text: 'Another lyric', words: [],
  }] })
  try {
    await r.hideControls()
    const opened = await r.lineContextMenu({ lineIndex: 1, clientX: 350, clientY: 375 })
    assert.ok(opened.prevented)
    assert.ok(opened.stopped)
    assertLyricMenu(r.menu, 350, 375, showControlsItem)
    r.assertAvailable(false, false)
    assert.deepEqual(r.clipboardWrites, [])
    r.assertPlaybackUnchanged()
    await r.selectMenu('copy-line')
    assert.deepEqual(r.clipboardWrites, ['Another lyric'])
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(false, false)
    await r.lineContextMenu({ lineIndex: 1 })
    await r.selectMenu('copy-all')
    assert.deepEqual(r.clipboardWrites, ['Another lyric', '歌词\nAnother lyric'])
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(false, false)
    await r.lineContextMenu({ lineIndex: 1 })
    await r.selectMenu('seek')
    assert.deepEqual(r.seekEvents, [6000])
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(false, false)
    assert.equal(r.clock.value, 4200)
    await r.lineContextMenu({ lineIndex: 1 })
    assertLyricMenu(r.menu, 240, 320, showControlsItem)
    await r.selectMenu('show-display-controls')
    r.assertAvailable(true, true)
    r.assertPressed(false, false)
    assert.equal(r.menu.props.open, false)
    assert.deepEqual(r.seekEvents, [6000])
    await r.lineContextMenu({ lineIndex: 1 })
    assertLyricMenu(r.menu, 240, 320, hideControlsItem)
    r.assertAvailable(true, true)
    await r.selectMenu('hide-display-controls')
    r.assertAvailable(false, false)
    assert.equal(r.menu.props.open, false)
    assert.deepEqual([r.settings.showTranslation, r.settings.showRomanization], [false, false])
  } finally { r.stop() }
})

await test('host context menus restore hidden controls only when the show action is selected', async () => {
  const r = await mount()
  try {
    await r.hideControls()
    const opened = await r.hostContextMenu()
    assert.ok(opened.prevented)
    assertMenu(r.menu, 240, 320, [showControlsItem])
    r.assertAvailable(false, false)
    r.assertPlaybackUnchanged()
    await r.dismissMenu()
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(false, false)
    await r.hostContextMenu()
    await r.selectMenu('show-display-controls')
    r.assertAvailable(true, true)
    assert.equal(r.menu.props.open, false)
    r.player.currentLyricGroups = [{ element: {
      isConnected: true, getBoundingClientRect: () => ({ top: 300, height: 40 }),
    } }]
    await r.hostContextMenu()
    assertLyricMenu(r.menu, 240, 320, hideControlsItem)
    r.assertAvailable(true, true)
    await r.selectMenu('hide-display-controls')
    r.assertAvailable(false, false)
    await r.hostContextMenu()
    assertLyricMenu(r.menu, 240, 320, showControlsItem)
    await r.selectMenu('copy-line')
    assert.deepEqual(r.clipboardWrites, ['歌词'])
    assert.equal(r.menu.props.open, false)
    r.assertAvailable(false, false)
    await r.hostContextMenu()
    await r.selectMenu('show-display-controls')
    r.assertAvailable(true, true)
    assert.equal(r.menu.props.open, false)
    r.assertPlaybackUnchanged()
  } finally { r.stop() }
})

await test('lyrics without auxiliary tracks keep copy and seek menu actions without a controls visibility action', async () => {
  const r = await mount({ lyrics: [plainLine] })
  try {
    await r.lineContextMenu()
    assertLyricMenu(r.menu, 240, 320)
    r.assertAvailable(false, false)
    await r.dismissMenu()
    r.player.currentLyricGroups = [{ element: {
      isConnected: true, getBoundingClientRect: () => ({ top: 300, height: 40 }),
    } }]
    await r.hostContextMenu()
    assertLyricMenu(r.menu, 240, 320)
    r.assertAvailable(false, false)
    r.assertPlaybackUnchanged()
  } finally { r.stop() }
})

console.log(`Lyrics display controls regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
