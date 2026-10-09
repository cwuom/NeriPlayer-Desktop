import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
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
const invocations = []
let reply = []
const match = await loadModule('../src/modules/lyrics/lyricMatch.ts', {
  '@tauri-apps/api/core': { invoke: async (command, args) => { invocations.push({ command, args }); return reply } },
  './lyricsFormat': format,
})

assert.deepEqual(match.defaultLyricMatchSources('netease'), ['kugou', 'netease', 'amll_ttml'], 'Android 默认：AMLL + 网易云 + 酷狗')
assert.deepEqual(match.defaultLyricMatchSources('youtube'), ['kugou', 'netease', 'qq', 'lrclib'], 'YouTube 曲目不查 AMLL')
assert.equal(match.defaultLyricMatchKeyword(' Lemon ', ' 米津玄師 '), 'Lemon 米津玄師')
assert.equal(match.defaultLyricMatchKeyword('Lemon', ''), 'Lemon')

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

console.log('lyric match tests passed')
