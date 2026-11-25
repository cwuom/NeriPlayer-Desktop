import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

// 歌词匹配的前端部分：默认平台、结果校验映射、来源标签、时长显示（打分在后端 matcher.rs）
async function loadModule(relativePath, dependencies) {
  const source = await readFile(new URL(relativePath, import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `缺少依赖 ${name}`)
    const clause = node.importClause
    const replacement = clause?.namedBindings
      ? `const { ${clause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text).join(', ')} } = deps[${JSON.stringify(name)}]`
      : ''
    compiled = compiled.slice(0, node.getStart(parsed)) + replacement + compiled.slice(node.end)
  }
  globalThis.__lyricMatchTest = dependencies
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis.__lyricMatchTest;\n${compiled}`).toString('base64')}`)
  } finally { delete globalThis.__lyricMatchTest }
}

const format = await loadModule('../src/modules/lyrics/lyricsFormat.ts', {})
const searchTitle = await loadModule('../src/modules/lyrics/lyricSearchTitle.ts', {})
const invocations = []
let reply = []
let invokeMatch = async () => reply
const match = await loadModule('../src/modules/lyrics/lyricMatch.ts', {
  '@tauri-apps/api/core': {
    invoke: async (command, args) => { invocations.push({ command, args }); return invokeMatch(args) },
    Channel: class { constructor(onmessage) { this.onmessage = onmessage || (() => {}) } },
  },
  './lyricsFormat': format,
  './lyricSearchTitle': searchTitle,
})

assert.deepEqual(match.defaultLyricMatchSources('netease'), ['kugou', 'netease', 'amll_ttml'], 'Android 默认：AMLL + 网易云 + 酷狗')
assert.deepEqual(match.defaultLyricMatchSources('youtube'), ['kugou', 'netease', 'qq', 'lrclib'], 'YouTube 曲目不查 AMLL')
assert.equal(match.defaultLyricMatchKeyword(' Lemon ', ' 米津玄師 '), 'Lemon 米津玄師')
assert.equal(match.defaultLyricMatchKeyword('Lemon', ''), 'Lemon')
assert.equal(match.defaultLyricMatchKeyword('【洛天依】《歌行四方》PV', '某UP主'), '歌行四方 某UP主', '书名号内的歌名优先作为关键字')
assert.equal(match.defaultLyricMatchKeyword('光年之外 (《太空旅客》主题曲)', '邓紫棋'), '光年之外 (《太空旅客》主题曲) 邓紫棋', '括号里的出处不当歌名')

assert.equal(match.lyricMatchSourceTag('netease'), 'CLOUD_MUSIC', '与偏移来源和 Android MusicPlatform 一致')
assert.equal(match.lyricMatchSourceTag('qq'), 'QQ_MUSIC')
assert.equal(match.lyricMatchSourceTag('amll_ttml'), 'AMLL_TTML')
assert.equal(match.lyricMatchSourceTag('kugou'), 'KUGOU')

assert.equal(match.formatMatchDuration(255_400), '4:15')
assert.equal(match.formatMatchDuration(0), '')
assert.equal(match.formatMatchDelta(1_500), '1.5s')
assert.equal(match.formatMatchDelta(45_000), '45s')

const backendResult = {
  id: '1', source: 'kugou', title: 'Lemon', artist: '米津玄師', album: 'STRAY SHEEP', durationMs: 255000,
  format: 'yrc', score: 288, durationDeltaMs: 0, confidence: 'high', wordTimed: true,
  hasTranslation: true, hasRomanization: false,
  lines: [
    { start_ms: 1000, duration_ms: 2000, text: '夢ならば', translation: '如果这是梦', words: [{ start_ms: 1000, duration_ms: 500, text: '夢' }] },
    { start_ms: 4000, duration_ms: 0, text: '  ', words: [] },
  ],
}
reply = [
  backendResult,
  { ...backendResult, source: 'youtube' },
  { ...backendResult, id: '2', lines: [] },
  { ...backendResult, id: '3', source: 'lrclib', format: 'weird', confidence: 'certain', durationDeltaMs: null },
  null,
]
const results = await match.matchLyrics({
  keyword: 'Lemon', title: 'Lemon', artist: '米津玄師', durationMs: 255000, preferWordTimed: true, sources: ['kugou'],
})
assert.equal(invocations[0].command, 'match_lyrics')
assert.deepEqual(invocations[0].args.request.sources, ['kugou'], '请求整体放在 request 参数里')
assert.deepEqual(results.map(result => result.id), ['1', '3'], '未知平台、没有歌词行的结果丢掉')
assert.equal(results[0].lines.length, 1, '空白行不算歌词')
assert.equal(results[0].lines[0].startMs, 1000, '后端 snake_case 映射成前端字段')
assert.equal(results[0].lines[0].words[0].text, '夢')
assert.equal(results[0].lines[0].translation, '如果这是梦')
assert.equal(results[1].format, 'lrc', '未知格式按 LRC')
assert.equal(results[1].confidence, 'low', '未知置信度按低')
assert.equal(results[1].durationDeltaMs, null)

// 填回编辑器 / 写回载荷用的文本：逐字结果导出 YRC，翻译导出 LRC
const editable = format.toEditableLyricsText(results[0].lines)
assert.match(editable, /^\[1000,2000\]\(1000,500,0\)夢/)
assert.equal(format.toEditableTranslationText(results[0].lines), '[00:01.00]如果这是梦')

const descriptor = parse(await readFile(new URL('../src/components/LyricsMatchPanel.vue', import.meta.url), 'utf8')).descriptor
const script = compileScript(descriptor, { id: 'lyric-match-panel-test', genDefaultAs: 'component', inlineTemplate: true })
const compiledPanel = ts.transpileModule(script.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const renderer = vue.createRenderer({
  createElement: type => ({ type, tagName: type.toUpperCase(), children: [], props: {}, addEventListener() {}, removeEventListener() {}, getRootNode: () => null }),
  createText: text => ({ type: 'text', text }), createComment: text => ({ type: 'comment', text }),
  setText: (node, text) => { node.text = text },
  setElementText: (node, text) => { node.text = text; node.children = [] },
  patchProp: (node, key, _previous, value) => { node.props[key] = value },
  parentNode: node => node.parent,
  nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] || null,
  insert(node, parent, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(node)
    else parent.children.splice(index, 0, node)
  },
  remove(node) { node.parent.children.splice(node.parent.children.indexOf(node), 1); node.parent = null },
})
const flush = async () => { await Promise.resolve(); await Promise.resolve(); await vue.nextTick() }
globalThis.Document = class {}
globalThis.ShadowRoot = class {}
function deferred() {
  let resolve, reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  return { promise, resolve, reject }
}
function mountPanel() {
  const picks = []
  const dependencies = {
    vue,
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@/modules/lyrics/lyricMatch': match,
    '@/utils/logger': { createLogger: () => ({ warn() {} }) },
  }
  const component = new Function('require', 'exports', 'HTMLElement', `${compiledPanel}\nreturn component`)(name => {
    assert.ok(name in dependencies, `unexpected panel dependency: ${name}`)
    return dependencies[name]
  }, {}, class {})
  const root = { children: [] }
  const app = renderer.createApp(component, {
    title: 'Lemon', artist: '米津玄師', album: 'STRAY SHEEP', durationMs: 255000,
    playbackSource: 'netease', preferWordTimed: true, applyLabel: 'apply', onPick: result => picks.push(result),
  })
  app.mount(root)
  const find = className => {
    const found = []
    const visit = node => {
      if (String(node.props?.class || '').split(/\s+/).includes(className)) found.push(node)
      for (const child of node.children || []) visit(child)
    }
    visit(root)
    return found
  }
  return { find, picks, stop: () => app.unmount() }
}
function pendingSearches() {
  const pending = []
  invokeMatch = args => {
    const command = { ...deferred(), args }
    pending.push(command)
    return command.promise
  }
  return pending
}
function progress(command, value) { command.args.onResults?.onmessage(value) }
let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('opening the panel and selecting sources never searches until an explicit action', async () => {
  invocations.length = 0
  invokeMatch = async () => []
  const panel = mountPanel()
  try {
    await flush()
    panel.find('lyric-match-chip')[2].props.onClick()
    await flush()
    assert.equal(invocations.length, 0, 'opening or changing a source must not issue network commands')
    panel.find('lyric-match-search')[0].props.onClick()
    await flush()
    assert.equal(invocations.length, 1)
    assert.deepEqual(invocations[0].args.request.sources, ['kugou', 'netease', 'qq', 'amll_ttml'])
  } finally { panel.stop() }
})

await test('completed source candidates remain visible and applicable while other sources are pending', async () => {
  const pending = pendingSearches()
  const panel = mountPanel()
  try {
    pending.length = 0
    panel.find('lyric-match-input')[0].props.onKeydown({ key: 'Enter' })
    const command = pending[0]
    progress(command, [backendResult])
    await flush()
    assert.equal(panel.find('lyric-match-item').length, 1, 'a slow source must not hide completed source results')
    panel.find('lyric-match-head')[0].props.onClick()
    await flush()
    panel.find('lyric-match-apply')[0].props.onClick()
    assert.equal(panel.picks[0].id, '1')
    assert.equal(panel.picks[0].lines[0].startMs, 1000)
    command.resolve([backendResult]); await flush()
  } finally { panel.stop() }
})

await test('later ranked candidates preserve the expanded candidate and its apply action', async () => {
  const pending = pendingSearches()
  const panel = mountPanel()
  try {
    pending.length = 0
    panel.find('lyric-match-search')[0].props.onClick()
    const command = pending[0]
    progress(command, [backendResult]); await flush()
    assert.equal(panel.find('lyric-match-head').length, 1, 'partial candidates must be previewable')
    panel.find('lyric-match-head')[0].props.onClick(); await flush()
    const higher = { ...backendResult, id: 'higher', source: 'netease', score: 300 }
    progress(command, [higher, backendResult]); await flush()
    assert.equal(panel.find('lyric-match-preview').length, 1)
    panel.find('lyric-match-apply')[0].props.onClick()
    assert.equal(panel.picks[0].id, '1', 'ranking changes must not switch the preview to a different candidate')
    command.resolve([higher, backendResult]); await flush()
  } finally { panel.stop() }
})

await test('changing sources cancels the active view without starting a replacement search', async () => {
  const pending = pendingSearches()
  const panel = mountPanel()
  try {
    pending.length = 0
    panel.find('lyric-match-search')[0].props.onClick()
    const command = pending[0]
    panel.find('lyric-match-chip')[0].props.onClick(); await flush()
    assert.equal(pending.length, 1)
    progress(command, [backendResult]); command.resolve([backendResult]); await flush()
    assert.equal(panel.find('lyric-match-item').length, 0, 'results for the previous source selection must be ignored')
    assert.equal(panel.find('lyric-match-search')[0].props.disabled, false)
  } finally { panel.stop() }
})

await test('a newer explicit search ignores old progress and completion, including after unmount', async () => {
  const pending = pendingSearches()
  const panel = mountPanel()
  try {
    pending.length = 0
    const input = panel.find('lyric-match-input')[0]
    input.props.onKeydown({ key: 'Enter' })
    input.props['onUpdate:modelValue']('Lemon official')
    await flush()
    panel.find('lyric-match-input')[0].props.onKeydown({ key: 'Enter' })
    assert.equal(pending.length, 2)
    const current = { ...backendResult, id: 'current' }
    progress(pending[1], [current]); await flush()
    progress(pending[0], [backendResult]); pending[0].resolve([backendResult]); await flush()
    assert.equal(panel.find('lyric-match-head').length, 1, 'the latest search must expose its completed source')
    panel.find('lyric-match-head')[0].props.onClick(); await flush()
    panel.find('lyric-match-apply')[0].props.onClick()
    assert.equal(panel.picks[0].id, 'current')
    panel.stop()
    progress(pending[1], [backendResult]); pending[1].resolve([current]); await flush()
    assert.equal(panel.find('lyric-match-item').length, 0)
  } finally { if (panel.find('lyric-match').length) panel.stop() }
})

await test('final command results cannot be overwritten by a delayed partial snapshot', async () => {
  const pending = pendingSearches()
  const snapshots = []
  const finished = match.matchLyrics({ keyword: 'Lemon', title: 'Lemon', artist: '米津玄師', durationMs: 255000, preferWordTimed: true, sources: ['kugou'] }, value => snapshots.push(value))
  const command = pending[0]
  progress(command, [backendResult])
  await flush()
  assert.equal(snapshots.length, 1)
  const final = { ...backendResult, id: 'final' }
  command.resolve([final])
  assert.equal((await finished)[0].id, 'final')
  progress(command, [backendResult])
  assert.equal(snapshots.length, 1, 'channel delivery may finish after invoke; final results remain authoritative')
})

console.log(`Lyric match regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
