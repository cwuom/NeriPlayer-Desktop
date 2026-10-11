import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileStyleAsync, compileTemplate, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/views/TrayPopupView.vue', import.meta.url), 'utf8')
const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const parsed = ts.createSourceFile('TrayPopupView.ts', script, ts.ScriptTarget.ES2022, true)
const statements = parsed.statements.filter(statement => !ts.isImportDeclaration(statement)).map(statement => statement.getText(parsed)).join('\n')
const compiled = ts.transpileModule(statements, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const bindings = parsed.statements.flatMap(statement => {
  if (ts.isVariableStatement(statement)) return statement.declarationList.declarations.flatMap(declaration =>
    ts.isIdentifier(declaration.name) ? [declaration.name.text] : [])
  return ts.isFunctionDeclaration(statement) && statement.name ? [statement.name.text] : []
})
const template = compileTemplate({ source: parse(source).descriptor.template.content, filename: 'TrayPopupView.vue', id: 'tray-popup-test' })
assert.deepEqual(template.errors, [])
const rendered = {}
new Function('require', 'exports', ts.transpileModule(template.code, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(name => { assert.equal(name, 'vue'); return vue }, rendered)

function deferred() {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}

function snapshot(title, dark = true, coverUrl = '') {
  return {
    locale: 'en',
    track: { title, artist: 'Artist', coverUrl },
    theme: { dark, vars: { '--md-surface-container': dark ? 'rgb(33, 31, 38)' : 'rgb(241, 237, 243)' } },
    isPlaying: true,
    desktopLyricsOpen: false,
  }
}

function runtime() {
  let mounted, unmounted
  const events = new Map(), classes = new Set(), properties = new Map(), calls = []
  const initial = deferred(), waitingForSnapshot = deferred()
  const root = {
    classList: {
      add: name => classes.add(name),
      remove: name => classes.delete(name),
      toggle: (name, enabled) => enabled ? classes.add(name) : classes.delete(name),
    },
    style: { setProperty: (name, value) => properties.set(name, value) },
  }
  const dependencies = {
    computed: vue.computed,
    nextTick: vue.nextTick,
    ref: vue.ref,
    watch: vue.watch,
    onMounted: callback => { mounted = callback },
    onUnmounted: callback => { unmounted = callback },
    useI18n: () => ({ t: key => key }),
    setLocale: () => {},
    isTauri: () => true,
    listen: async (event, callback) => { events.set(event, callback); return () => events.delete(event) },
    invoke: async (command, args) => {
      calls.push({ command, args })
      if (command === 'get_tray_popup_state') {
        waitingForSnapshot.resolve()
        return initial.promise
      }
    },
    document: { documentElement: root },
    window: { innerWidth: 300, addEventListener() {}, removeEventListener() {}, matchMedia: () => ({ matches: false }) },
  }
  const scope = vue.effectScope()
  const actual = scope.run(() => new Function(...Object.keys(dependencies), `${compiled}\nreturn { ${bindings.join(', ')} }`)(...Object.values(dependencies)))
  const context = vue.proxyRefs({ ...actual, t: dependencies.useI18n().t })
  const renderCache = []
  function find(node, predicate) {
    if (predicate(node)) return node
    for (const child of Array.isArray(node.children) ? node.children : []) {
      const result = find(child, predicate)
      if (result) return result
    }
  }
  function coverImage() {
    const tree = rendered.render(context, renderCache)
    const cover = find(tree, node => node.props?.class === 'tp-cover')
    return cover.children.find(node => node.type === 'img')
  }
  function errorImage(image = coverImage()) {
    assert.ok(image, 'a cover image must be rendered before simulating its failure')
    const target = { src: image.props.src, getAttribute: name => image.props[name] == null ? null : String(image.props[name]) }
    image.props.onError({ currentTarget: target, target })
  }
  return {
    ...actual, coverImage, errorImage, mounted: () => mounted(),
    unmounted: () => { unmounted(); scope.stop() },
    initial, waitingForSnapshot, events, classes, properties, calls,
  }
}

async function mountedView(initialState) {
  const view = runtime()
  const mount = view.mounted()
  await view.waitingForSnapshot.promise
  view.initial.resolve(initialState)
  await mount
  await vue.nextTick()
  return view
}

let failures = 0
async function test(name, run) {
  try { await run(); console.log(`passed: ${name}`) }
  catch (error) { failures++; console.error(`failed: ${name}: ${error.message}`) }
}

await test('a live tray update stays current when the initial snapshot resolves later', async () => {
  const view = runtime()
  const mount = view.mounted()
  await view.waitingForSnapshot.promise
  view.events.get('tray-popup:state')({ payload: snapshot('New song', false) })
  view.initial.resolve(snapshot('Old song'))
  await mount
  assert.equal(view.state.value.track.title, 'New song')
  assert.equal(view.classes.has('light-theme'), true)
  assert.equal(view.properties.get('--md-surface-container'), 'rgb(241, 237, 243)')
  view.unmounted()
})

await test('unmounting before the initial snapshot resolves prevents late page updates', async () => {
  const view = runtime()
  const mount = view.mounted()
  await view.waitingForSnapshot.promise
  view.unmounted()
  view.initial.resolve(snapshot('Late song', false))
  await mount
  assert.equal(view.state.value.track, null)
  assert.equal(view.classes.has('light-theme'), false)
  assert.equal(view.events.size, 0)
  assert.equal(view.calls.some(call => call.command === 'tray_popup_ready'), false)
})

await test('reopening retries a cover after a temporary image failure with the same URL', async () => {
  const view = await mountedView(snapshot('Song', true, 'https://p4.music.126.net/song.jpg'))
  try {
    view.errorImage()
    assert.equal(view.coverSrc.value, '')
    view.events.get('tray-popup:shown')()
    await vue.nextTick()
    assert.equal(view.coverSrc.value, 'https://p4.music.126.net/song.jpg')
    assert.equal(view.coverImage().props.src, 'https://p4.music.126.net/song.jpg')
  } finally { view.unmounted() }
})

await test('late errors from a previous song do not hide the current song cover', async () => {
  const view = await mountedView(snapshot('Old song', true, 'https://p4.music.126.net/old.jpg'))
  try {
    const oldImage = view.coverImage()
    view.events.get('tray-popup:state')({ payload: snapshot('New song', true, 'https://p4.music.126.net/new.jpg') })
    await vue.nextTick()
    view.errorImage(oldImage)
    assert.equal(view.coverSrc.value, 'https://p4.music.126.net/new.jpg')
  } finally { view.unmounted() }
})

await test('a late error from an earlier attempt cannot suppress the retry of the same cover URL', async () => {
  const url = 'https://p4.music.126.net/retry.jpg'
  const view = await mountedView(snapshot('Song', true, url))
  try {
    const firstImage = view.coverImage()
    view.errorImage(firstImage)
    view.events.get('tray-popup:shown')()
    await vue.nextTick()
    const retry = view.coverImage()
    assert.notEqual(retry.key, firstImage.key, 'reopening must create a fresh image load attempt')
    view.errorImage(firstImage)
    assert.equal(view.coverSrc.value, url)
  } finally { view.unmounted() }
})

await test('reopening preserves an already displayed cover without starting another image load', async () => {
  const view = await mountedView(snapshot('Song', true, 'https://p4.music.126.net/loaded.jpg'))
  try {
    const key = view.coverImage().key
    view.events.get('tray-popup:shown')()
    await vue.nextTick()
    assert.equal(view.coverImage().key, key)
  } finally { view.unmounted() }
})

await test('a different song sharing an album cover gets a fresh image attempt', async () => {
  const url = 'https://p4.music.126.net/album.jpg'
  const view = await mountedView(snapshot('First song', true, url))
  try {
    view.errorImage()
    view.events.get('tray-popup:state')({ payload: snapshot('Second song', true, url) })
    await vue.nextTick()
    assert.equal(view.coverSrc.value, url)
  } finally { view.unmounted() }
})

await test('playback state updates do not repeatedly retry a failed cover', async () => {
  const current = snapshot('Song', true, 'https://p4.music.126.net/song.jpg')
  const view = await mountedView(current)
  try {
    view.errorImage()
    view.events.get('tray-popup:state')({ payload: { ...current, isPlaying: false } })
    await vue.nextTick()
    assert.equal(view.coverSrc.value, '')
  } finally { view.unmounted() }
})

await test('theme overrides keep tray opacity on the backdrop image', async () => {
  const { descriptor } = parse(source)
  const style = descriptor.styles[0]
  const result = await compileStyleAsync({
    source: style.content,
    filename: 'TrayPopupView.vue',
    id: 'data-v-tray-test',
    scoped: style.scoped,
    preprocessLang: style.lang,
  })
  assert.deepEqual(result.errors, [])
  assert.match(result.code, /html\.light-theme\s+\.tp-hero-backdrop\s+img\s*\{\s*opacity:\s*0\.28/)
  assert.match(result.code, /html\.dark-theme\s+\.tp-item--quit:hover\s*\{/)
  assert.doesNotMatch(result.code, /html\.(?:light|dark)-theme\s*\{/)
})

async function loadCommonJs(path) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  const exports = {}
  new Function('exports', ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText)(exports)
  return exports
}

const coverCacheModule = await loadCommonJs('../src/utils/bilibiliCoverCache.ts')
const trackCoverModule = await loadCommonJs('../src/utils/trackCover.ts')
const lyricTimeline = await loadCommonJs('../src/modules/desktopLyrics/timeline.ts')
const bridgeSource = await readFile(new URL('../src/modules/tray/bridge.ts', import.meta.url), 'utf8')
const bridgeCompiled = ts.transpileModule(bridgeSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const dataUrl = text => `data:image/png;base64,${Buffer.from(text).toString('base64')}`

function bridgeRuntime(url, mac = false) {
  const requests = new Map(), timers = new Set(), intervals = new Set(), events = new Map(), snapshots = []
  const player = vue.reactive({ hasPlaybackSession: true, currentTrack: { id: '1', title: 'First song', artist: 'Artist', coverUrl: url }, positionMs: 0, livePositionMs() { return this.positionMs } })
  const settings = vue.reactive({ showMenuBarLyrics: false })
  const offsets = vue.reactive({ value: 0, effectiveOffsetMs() { return this.value } })
  let consumers = 0
  const lyrics = vue.reactive({ lines: [], acquire() { consumers++; return () => { consumers-- } } })
  const cache = new coverCacheModule.BilibiliCoverCache(
    source => {
      const request = deferred()
      requests.set(source, request)
      return request.promise
    }, async () => {},
  )
  const root = { classList: { contains: () => true } }
  let observers = 0
  const dependencies = {
    vue,
    '@tauri-apps/api/core': {
      isTauri: () => true,
      invoke: async (command, args) => {
        if (command === 'publish_tray_snapshot') snapshots.push(args.snapshot)
      },
    },
    '@tauri-apps/api/event': { listen: async (event, callback) => { events.set(event, callback); return () => events.delete(event) } },
    '@/i18n': { default: { global: { locale: vue.ref('en'), t: key => key } } },
    '@/stores/player': { usePlayerStore: () => player },
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/stores/currentLyrics': { useCurrentLyricsStore: () => lyrics },
    '@/stores/lyricOffset': { useLyricOffsetStore: () => offsets },
    '@/modules/shortcuts/platform': { isMacPlatform: mac },
    '@/modules/desktopLyrics/timeline': lyricTimeline,
    '@/modules/desktopLyrics/bridge': { desktopLyricsOpen: vue.ref(false) },
    '@/utils/trackCover': trackCoverModule,
    '@/utils/bilibiliCover': {
      ...coverCacheModule,
      peekCoverImage: source => cache.peek(source), resolveCoverImage: source => cache.resolve(source),
    },
    '@/utils/logger': { createLogger: () => ({ warn() {} }) },
    '@/utils/logSanitizer': { summarizeLogError: error => error.message },
  }
  const exports = {}
  new Function('require', 'exports', 'document', 'getComputedStyle', 'MutationObserver', 'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', bridgeCompiled)(
    name => { assert.ok(name in dependencies, `missing bridge dependency ${name}`); return dependencies[name] },
    exports, { documentElement: root }, () => ({ getPropertyValue: () => '' }),
    class { observe() { observers++ } disconnect() { observers-- } },
    callback => { timers.add(callback); return callback }, timer => timers.delete(timer),
    callback => { intervals.add(callback); return callback }, timer => intervals.delete(timer),
  )
  let stop = () => {}
  return {
    player, snapshots, settings, lyrics, offsets, events,
    get consumers() { return consumers },
    tick() { for (const callback of intervals) callback() },
    prepare: source => cache.resolve(source),
    resolve: (source, value) => {
      const request = requests.get(source)
      assert.ok(request, `cover request must exist for ${source}`)
      request.resolve(value)
    },
    start() { stop = exports.installTrayBridge({ openNowPlaying() {}, flushBeforeQuit: async () => {} }) },
    async flush() {
      for (let i = 0; i < 8; i++) await vue.nextTick()
      for (const timer of [...timers]) { timers.delete(timer); timer() }
      await vue.nextTick()
    },
    stop() {
      stop()
      assert.equal(timers.size, 0)
      assert.equal(observers, 0)
      assert.equal(events.size, 0)
      assert.equal(intervals.size, 0)
      assert.equal(consumers, 0)
    },
  }
}

await test('macOS menu bar lyrics load independently and follow the live playback clock and lyric offset', async () => {
  const bridge = bridgeRuntime('', true)
  bridge.start()
  try {
    assert.equal(bridge.consumers, 0)
    bridge.settings.showMenuBarLyrics = true
    bridge.lyrics.lines = [{ startMs: 1000, text: '第一句' }, { startMs: 2000, text: '第二句' }]
    await bridge.flush()
    assert.equal(bridge.consumers, 1)
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, 'First song')
    bridge.player.positionMs = 1200
    bridge.tick()
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, '第一句')
    bridge.offsets.value = 1000
    bridge.tick()
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, '第二句')
    bridge.settings.showMenuBarLyrics = false
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, '')
    assert.equal(bridge.consumers, 0)
  } finally { bridge.stop() }
})

await test('menu bar lyrics are bounded, single line, and cleared when playback ends', async () => {
  const bridge = bridgeRuntime('', true)
  bridge.settings.showMenuBarLyrics = true
  bridge.lyrics.lines = [{ startMs: 0, text: ' a\n\tb\u0000 ' }]
  bridge.start()
  try {
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, 'a b')
    bridge.lyrics.lines = [{ startMs: 0, text: '🎵'.repeat(30) }]
    bridge.tick()
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, `${'🎵'.repeat(24)}…`)
    const count = bridge.snapshots.length
    bridge.tick()
    await bridge.flush()
    assert.equal(bridge.snapshots.length, count, 'unchanged lyric lines must not be republished')
    bridge.player.hasPlaybackSession = false
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, '')
    assert.equal(bridge.consumers, 0)
  } finally { bridge.stop() }
})

await test('the macOS tray menu toggles the persistent lyric preference', async () => {
  const bridge = bridgeRuntime('', true)
  bridge.start()
  try {
    await bridge.flush()
    bridge.events.get('tray:toggle-menu-bar-lyrics')()
    await bridge.flush()
    assert.equal(bridge.settings.showMenuBarLyrics, true)
    assert.equal(bridge.snapshots.at(-1).showMenuBarLyrics, true)
  } finally { bridge.stop() }
})

await test('other platforms do not acquire lyrics for a restored macOS preference', async () => {
  const bridge = bridgeRuntime('')
  bridge.settings.showMenuBarLyrics = true
  bridge.start()
  try {
    await bridge.flush()
    assert.equal(bridge.consumers, 0)
    assert.equal(bridge.snapshots.at(-1).menuBarLyric, '')
  } finally { bridge.stop() }
})

await test('a cover resolved after the first tray snapshot is republished as validated image data', async () => {
  const url = 'https://p4.music.126.net/song.jpg'
  const bridge = bridgeRuntime(url)
  const pending = bridge.prepare(url)
  bridge.start()
  try {
    assert.equal(bridge.snapshots.at(-1).track.coverUrl, url)
    bridge.resolve(url, dataUrl('resolved-cover'))
    await pending
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).track.coverUrl, dataUrl('resolved-cover'))
  } finally { bridge.stop() }
})

await test('a previous track cover resolving late cannot replace the current track cover', async () => {
  const first = 'https://p4.music.126.net/first.jpg'
  const second = 'https://p4.music.126.net/second.jpg'
  const bridge = bridgeRuntime(first)
  const oldCover = bridge.prepare(first)
  bridge.start()
  try {
    const currentCover = bridge.prepare(second)
    bridge.player.currentTrack = { id: '2', title: 'Second song', artist: 'Artist', coverUrl: second }
    await bridge.flush()
    bridge.resolve(second, dataUrl('second-cover'))
    await currentCover
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).track.coverUrl, dataUrl('second-cover'))
    bridge.resolve(first, dataUrl('first-cover'))
    await oldCover
    await bridge.flush()
    assert.equal(bridge.snapshots.at(-1).track.title, 'Second song')
    assert.equal(bridge.snapshots.at(-1).track.coverUrl, dataUrl('second-cover'))
  } finally { bridge.stop() }
})

await test('disposing the tray bridge prevents a pending cover from publishing late', async () => {
  const url = 'https://p4.music.126.net/disposed.jpg'
  const bridge = bridgeRuntime(url)
  const pending = bridge.prepare(url)
  bridge.start()
  await bridge.flush()
  const count = bridge.snapshots.length
  bridge.stop()
  bridge.resolve(url, dataUrl('disposed-cover'))
  await pending
  await bridge.flush()
  assert.equal(bridge.snapshots.length, count)
})

await test('oversized cached cover data falls back to its URL instead of being cleared by the tray backend', async () => {
  const url = 'https://p4.music.126.net/oversized.jpg'
  const bridge = bridgeRuntime(url)
  const pending = bridge.prepare(url)
  bridge.resolve(url, dataUrl('a'.repeat(800_000)))
  await pending
  bridge.start()
  try {
    await bridge.flush()
    assert.ok(bridge.snapshots.at(-1).track.coverUrl === url, 'oversized data must retain its original URL')
  } finally { bridge.stop() }
})

await test('oversized cover data resolving after publication keeps the original URL', async () => {
  const url = 'https://p4.music.126.net/oversized-pending.jpg'
  const bridge = bridgeRuntime(url)
  const pending = bridge.prepare(url)
  bridge.start()
  try {
    bridge.resolve(url, dataUrl('b'.repeat(800_000)))
    await pending
    await bridge.flush()
    assert.ok(bridge.snapshots.at(-1).track.coverUrl === url, 'oversized resolved data must retain its original URL')
  } finally { bridge.stop() }
})

if (failures) process.exit(1)
