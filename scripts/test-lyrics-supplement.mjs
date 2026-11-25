import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function loadModule(relativePath, dependencies = {}) {
  const source = await readFile(new URL(relativePath, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `unexpected dependency: ${name}`)
    return dependencies[name]
  }, exports)
  return exports
}

const format = await loadModule('../src/modules/lyrics/lyricsFormat.ts')
const offset = await loadModule('../src/modules/lyrics/lyricOffset.ts')
const match = await loadModule('../src/modules/lyrics/lyricMatch.ts', {
  '@tauri-apps/api/core': {},
  './lyricsFormat': format,
  './lyricSearchTitle': await loadModule('../src/modules/lyrics/lyricSearchTitle.ts'),
})

async function subject(matchLyrics = async () => []) {
  const module = await loadModule('../src/modules/lyrics/lyricsSupplement.ts', {
    './lyricsFormat': format,
    './lyricOffset': offset,
    './lyricMatch': { ...match, matchLyrics },
  })
  return module
}

const track = { id: 'bilibili:1', title: '夜空', artist: '歌手', album: '专辑', durationMs: 200000 }
const needsBoth = { translation: true, romanization: true }
const original = [
  { startMs: 10000, durationMs: 2000, text: '夜空', words: [
    { startMs: 10000, durationMs: 800, text: '夜' },
    { startMs: 10800, durationMs: 1000, text: '空' },
  ] },
  { startMs: 14000, durationMs: 1000, text: '光', words: [
    { startMs: 14000, durationMs: 1000, text: '光' },
  ] },
]
const secondary = [
  { startMs: 10600, durationMs: 1600, text: '平台原文', words: [], translation: '夜晚天空', roman: 'yozora' },
  { startMs: 14100, durationMs: 1200, text: '平台原文二', words: [], translation: '光芒', roman: 'hikari' },
]
function candidate(source, lines = secondary, overrides = {}) {
  return {
    id: 'matched-1', source, title: '夜空', artist: '歌手', album: '专辑', durationMs: 200000,
    format: 'lrc', score: 288, durationDeltaMs: 0, confidence: 'high', wordTimed: false,
    hasTranslation: lines.some(line => !!line.translation?.trim()),
    hasRomanization: lines.some(line => !!line.roman?.trim()), lines, ...overrides,
  }
}
function deferred() {
  let resolve, reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  return { promise, resolve, reject }
}

let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('补齐翻译与音译时保留酷狗原文和逐字时间轴', async () => {
  const requests = []
  const module = await subject(async request => { requests.push(request); return [candidate('netease')] })
  const before = structuredClone(original)
  const result = await module.supplementKugouLyrics(track, original, needsBoth)
  assert.deepEqual(result.map(line => [line.translation, line.roman]), [['夜晚天空', 'yozora'], ['光芒', 'hikari']])
  const withoutSupplement = line => {
    const rest = { ...line }
    delete rest.translation
    delete rest.roman
    return rest
  }
  assert.deepEqual(result.map(withoutSupplement), before)
  assert.deepEqual(original, before, '不能修改原数组或逐字信息')
  assert.deepEqual(requests, [{
    keyword: '夜空 歌手', title: '夜空', artist: '歌手', album: '专辑', durationMs: 200000,
    preferWordTimed: false, sources: ['netease'],
  }])
})

await test('网易云只有音译时从 QQ 补翻译且不采用 QQ 音译', async () => {
  const requests = []
  const module = await subject(async request => {
    requests.push(request.sources[0])
    return request.sources[0] === 'netease'
      ? [candidate('netease', secondary.map(line => ({ ...line, translation: undefined })))]
      : [candidate('qq', secondary.map(line => ({ ...line, roman: 'wrong-qq-roman' })))]
  })
  const result = await module.supplementKugouLyrics(track, original, needsBoth)
  assert.deepEqual(requests, ['netease', 'qq'])
  assert.deepEqual(result.map(line => [line.translation, line.roman]), [['夜晚天空', 'yozora'], ['光芒', 'hikari']])
})

await test('QQ 提供音译时也只接收 QQ 翻译', async () => {
  const module = await subject(async request => request.sources[0] === 'qq' ? [candidate('qq')] : [])
  const result = await module.supplementKugouLyrics(track, original, needsBoth)
  assert.deepEqual(result.map(line => line.translation), ['夜晚天空', '光芒'])
  assert.deepEqual(result.map(line => line.roman), [undefined, undefined])
})

await test('低置信度和错误来源候选不能补入副轨', async () => {
  const module = await subject(async request => [
    candidate(request.sources[0], secondary, { confidence: 'low' }),
    candidate(request.sources[0], secondary, { confidence: 'medium' }),
    candidate('kugou'),
  ])
  assert.deepEqual(await module.supplementKugouLyrics(track, original, needsBoth), original)
})

await test('高置信度但时长未知或与曲目差距过大的候选不能补入副轨', async () => {
  for (const durationMs of [0, -1, NaN, Infinity, 212001, 187999]) {
    const module = await subject(async request => [candidate(request.sources[0], secondary, { durationMs, durationDeltaMs: 0 })])
    assert.strictEqual(await module.supplementKugouLyrics(track, original, needsBoth), original, `不可靠候选时长 ${durationMs}`)
  }
})

await test('候选时长容差使用 6% 并限制在 7 至 15 秒且边界可接受', async () => {
  for (const [durationMs, tolerance] of [[10000, 7000], [200000, 12000], [400000, 15000]]) {
    const reliable = await subject(async () => [candidate('netease', secondary, { durationMs: durationMs + tolerance })])
    assert.equal((await reliable.supplementKugouLyrics({ ...track, durationMs }, original, needsBoth))[0].translation, '夜晚天空')
    const unreliable = await subject(async request => [candidate(request.sources[0], secondary, { durationMs: durationMs + tolerance + 1 })])
    assert.strictEqual(await unreliable.supplementKugouLyrics({ ...track, durationMs }, original, needsBoth), original)
  }
})

await test('有副轨但无法对齐的候选不会阻止后续候选', async () => {
  const module = await subject(async () => [
    candidate('netease', secondary.map(line => ({ ...line, startMs: line.startMs + 50000 }))),
    candidate('netease'),
  ])
  const result = await module.supplementKugouLyrics(track, original, needsBoth)
  assert.deepEqual(result.map(line => [line.translation, line.roman]), [['夜晚天空', 'yozora'], ['光芒', 'hikari']])
})

await test('网易云翻译无法对齐时仍尝试 QQ 翻译', async () => {
  const module = await subject(async request => request.sources[0] === 'netease'
    ? [candidate('netease', secondary.map(line => ({ ...line, startMs: line.startMs + 50000 })))]
    : [candidate('qq')])
  const result = await module.supplementKugouLyrics(track, original, { translation: true, romanization: false })
  assert.deepEqual(result.map(line => line.translation), ['夜晚天空', '光芒'])
  assert.deepEqual(result.map(line => line.roman), [undefined, undefined])
})

await test('重合并只填请求副轨并保护最新主词、逐字和已有副轨', async () => {
  const module = await subject()
  const latest = [
    { ...original[0], startMs: 10100, translation: '用户翻译', roman: 'user-roman' },
    original[1],
  ]
  const result = module.mergeKugouSecondaryLyrics(latest, secondary, needsBoth)
  assert.deepEqual(result[0], latest[0])
  assert.equal(result[1].translation, '光芒')
  assert.equal(result[1].roman, 'hikari')
  assert.deepEqual(result[1].words, original[1].words)
  const romanOnly = module.mergeKugouSecondaryLyrics(original, secondary, { translation: false, romanization: true })
  assert.deepEqual(romanOnly.map(line => line.translation), [undefined, undefined])
  assert.deepEqual(romanOnly.map(line => line.roman), ['yozora', 'hikari'])
  const translatedOnly = module.mergeKugouSecondaryLyrics(original, secondary, { translation: true, romanization: false })
  assert.deepEqual(translatedOnly.map(line => line.roman), [undefined, undefined])
})

await test('空白副轨允许补全但未对齐时保留原始空白字段', async () => {
  const module = await subject()
  const blank = original.map(line => ({ ...line, translation: ' \n ', roman: '\t' }))
  const result = module.mergeKugouSecondaryLyrics(blank, secondary, needsBoth)
  assert.deepEqual(result.map(line => [line.translation, line.roman]), [['夜晚天空', 'yozora'], ['光芒', 'hikari']])
  const noAlignment = secondary.map(line => ({ ...line, startMs: line.startMs + 50000 }))
  assert.strictEqual(module.mergeKugouSecondaryLyrics(blank, noAlignment, needsBoth), blank)
})

await test('仅酷狗整轨缺失的副轨需要补全', async () => {
  const module = await subject()
  assert.deepEqual(module.kugouSecondaryLyricNeeds(original, null, 'KUGOU'), needsBoth)
  assert.deepEqual(module.kugouSecondaryLyricNeeds([], null, 'kugou'), { translation: false, romanization: false })
  for (const source of [null, undefined, 'netease', 'qq', 'local']) {
    assert.deepEqual(module.kugouSecondaryLyricNeeds(original, null, source), { translation: false, romanization: false })
  }
  const partiallyTranslated = [{ ...original[0], translation: '已有翻译' }, original[1]]
  assert.deepEqual(module.kugouSecondaryLyricNeeds(partiallyTranslated, null, 'kugou'), { translation: false, romanization: true })
  const partiallyRomanized = [original[0], { ...original[1], roman: 'existing' }]
  assert.deepEqual(module.kugouSecondaryLyricNeeds(partiallyRomanized, null, 'kugou'), { translation: true, romanization: false })
  assert.deepEqual(module.kugouSecondaryLyricNeeds(original.map(line => ({ ...line, translation: ' ', roman: '\n' })), null, 'kugou'), needsBoth)
})

await test('用户编辑确认或清空副轨后分别禁止自动补全', async () => {
  const module = await subject()
  const cases = [
    [{ lyricSyncEdited: true, matchedTranslatedLyric: '' }, { translation: false, romanization: true }],
    [{ lyric_sync_edited: true, matched_romanized_lyric: '' }, { translation: true, romanization: false }],
    [{ lyricSyncEdited: true, matchedTranslatedLyric: '确认翻译', originalRomanizedLyric: '确认音译' }, { translation: false, romanization: false }],
    [{ lyricSyncEdited: true, matchedTranslatedLyric: null, matchedRomanizedLyric: null }, needsBoth],
    [{ lyricSyncEdited: false, matchedTranslatedLyric: '', matchedRomanizedLyric: '' }, needsBoth],
    [{ lyricSyncEdited: false, lyric_sync_edited: true, matched_translated_lyric: '', original_romanized_lyric: '' }, { translation: false, romanization: false }],
  ]
  for (const [payload, expected] of cases) assert.deepEqual(module.kugouSecondaryLyricNeeds(original, payload, 'kugou'), expected)
})

await test('时长未知、无主词、无需求或缺少歌名歌手时不查询', async () => {
  let calls = 0
  const module = await subject(async () => { calls++; return [candidate('netease')] })
  for (const durationMs of [undefined, null, 0, -1, NaN, Infinity]) {
    assert.deepEqual(await module.supplementKugouLyrics({ ...track, durationMs }, original, needsBoth), original)
  }
  assert.deepEqual(await module.supplementKugouLyrics(track, [], needsBoth), [])
  assert.deepEqual(await module.supplementKugouLyrics(track, original, { translation: false, romanization: false }), original)
  assert.deepEqual(await module.supplementKugouLyrics({ ...track, title: ' ' }, original, needsBoth), original)
  assert.deepEqual(await module.supplementKugouLyrics({ ...track, artist: '\t' }, original, needsBoth), original)
  assert.equal(calls, 0)
})

await test('全部查询失败仍完整保留主词', async () => {
  const sources = []
  const module = await subject(async request => { sources.push(request.sources[0]); throw new Error('offline') })
  assert.strictEqual(await module.supplementKugouLyrics(track, original, needsBoth), original)
  assert.deepEqual(sources, ['netease', 'qq'])
})

await test('网易云失败仍从 QQ 补翻译', async () => {
  const module = await subject(async request => {
    if (request.sources[0] === 'netease') throw new Error('offline')
    return [candidate('qq')]
  })
  const result = await module.supplementKugouLyrics(track, original, needsBoth)
  assert.deepEqual(result.map(line => line.translation), ['夜晚天空', '光芒'])
  assert.deepEqual(result.map(line => line.roman), [undefined, undefined])
})

await test('仅缺音译时不向 QQ 查询', async () => {
  const sources = []
  const module = await subject(async request => { sources.push(request.sources[0]); return [] })
  assert.deepEqual(await module.supplementKugouLyrics(track, original, { translation: false, romanization: true }), original)
  assert.deepEqual(sources, ['netease'])
})

await test('同曲目并发与完成后复用匹配请求而重新合并各自主词', async () => {
  const pending = deferred()
  let calls = 0
  const module = await subject(() => { calls++; return pending.promise })
  const changed = [{ ...original[0], text: '不同当前主词' }, original[1]]
  const first = module.supplementKugouLyrics(track, original, needsBoth)
  const second = module.supplementKugouLyrics(track, changed, needsBoth)
  assert.equal(calls, 1)
  pending.resolve([candidate('netease')])
  const results = await Promise.all([first, second])
  assert.equal(results[0][0].text, '夜空')
  assert.equal(results[1][0].text, '不同当前主词')
  assert.equal(results[1][0].translation, '夜晚天空')
  const completed = await module.supplementKugouLyrics(track, changed, needsBoth)
  assert.equal(calls, 1)
  assert.equal(completed[0].text, '不同当前主词')
})

await test('非空匹配结果复用且不同歌曲身份或元数据不会误用缓存', async () => {
  let calls = 0
  const module = await subject(async request => { calls++; return [candidate(request.sources[0], original)] })
  const romanOnly = { translation: false, romanization: true }
  await module.supplementKugouLyrics(track, original, romanOnly)
  await module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(calls, 1)
  for (const changed of [
    { id: 'bilibili:2' }, { title: '夜空二' }, { artist: '另一歌手' }, { album: '另一专辑' }, { durationMs: 200001 },
  ]) await module.supplementKugouLyrics({ ...track, ...changed }, original, romanOnly)
  assert.equal(calls, 6)
  await module.supplementKugouLyrics(track, original, { translation: true, romanization: false })
  assert.equal(calls, 7, '网易云查询可复用，但 QQ 请求不能误用网易云缓存')
})

await test('空结果仅在查询中去重，完成后允许重新查询并补上音译', async () => {
  const pending = deferred()
  let calls = 0
  const module = await subject(() => {
    calls++
    return calls === 1 ? pending.promise : Promise.resolve([candidate('netease')])
  })
  const romanOnly = { translation: false, romanization: true }
  const first = module.supplementKugouLyrics(track, original, romanOnly)
  const second = module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(calls, 1, '尚未完成的空结果请求也必须去重')
  pending.resolve([])
  const results = await Promise.all([first, second])
  assert.strictEqual(results[0], original)
  assert.strictEqual(results[1], original)
  const recovered = await module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(recovered[0].roman, 'yozora', '空结果可能来自离线失败，后续查询必须允许恢复')
  assert.equal(calls, 2)
})

await test('真正拒绝的查询可在下次重试', async () => {
  let calls = 0
  const module = await subject(async () => {
    calls++
    if (calls === 1) throw new Error('temporary')
    return [candidate('netease')]
  })
  const romanOnly = { translation: false, romanization: true }
  assert.deepEqual(await module.supplementKugouLyrics(track, original, romanOnly), original)
  assert.equal((await module.supplementKugouLyrics(track, original, romanOnly))[0].roman, 'yozora')
  assert.equal(calls, 2)
})

await test('缓存有界且旧查询失败不会删除同键的新请求', async () => {
  const stale = deferred()
  const replacement = deferred()
  let originalCalls = 0
  const module = await subject(request => {
    if (request.title !== track.title) return Promise.resolve([candidate('netease', original)])
    originalCalls++
    return originalCalls === 1 ? stale.promise : replacement.promise
  })
  const romanOnly = { translation: false, romanization: true }
  const oldest = module.supplementKugouLyrics(track, original, romanOnly)
  for (let index = 0; index < 200; index++) {
    await module.supplementKugouLyrics({ ...track, title: `另一首${index}` }, original, romanOnly)
  }
  const newer = module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(originalCalls, 2, '旧结果被逐出后重新查询，缓存不能无限增长')
  stale.reject(new Error('old request failed'))
  assert.deepEqual(await oldest, original)
  const concurrent = module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(originalCalls, 2, '旧失败只能删除自身缓存记录')
  replacement.resolve([candidate('netease')])
  const [first, second] = await Promise.all([newer, concurrent])
  assert.equal(first[0].roman, 'yozora')
  assert.equal(second[0].roman, 'yozora')
})

await test('已逐出的旧请求返回空结果时不会删除同键的新请求', async () => {
  const stale = deferred()
  const replacement = deferred()
  let originalCalls = 0
  const module = await subject(request => {
    if (request.title !== track.title) return Promise.resolve([candidate('netease', original)])
    originalCalls++
    return originalCalls === 1 ? stale.promise : replacement.promise
  })
  const romanOnly = { translation: false, romanization: true }
  const oldest = module.supplementKugouLyrics(track, original, romanOnly)
  for (let index = 0; index < 200; index++) {
    await module.supplementKugouLyrics({ ...track, title: `另一首${index}` }, original, romanOnly)
  }
  const newer = module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(originalCalls, 2)
  stale.resolve([])
  assert.strictEqual(await oldest, original)
  const concurrent = module.supplementKugouLyrics(track, original, romanOnly)
  assert.equal(originalCalls, 2, '旧空结果只能删除自身缓存记录')
  replacement.resolve([candidate('netease')])
  const results = await Promise.all([newer, concurrent])
  assert.deepEqual(results.map(lines => lines[0].roman), ['yozora', 'yozora'])
})

console.log(`Lyrics supplementation regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
