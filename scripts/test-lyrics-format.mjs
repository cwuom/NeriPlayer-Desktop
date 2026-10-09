import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function loadTsModule(relativePath) {
  const sourceUrl = new URL(relativePath, import.meta.url)
  const source = await readFile(sourceUrl, 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
  return import(moduleUrl)
}

const {
  hasWordTimedEntries,
  lyricLineToLrc,
  lyricLineToYrc,
  toEditableLyricsText,
  withUpdatedLyricsPayload,
  resolveStoredLyricText,
  resolveStoredLyricStateFromPayload,
  resolveStoredRomanizedLyricStateFromPayload,
  mergeWordTimedLyricsWithBaseline,
  mergeParsedLyricsWithRomanization,
  mergeParsedLyricsWithTranslations,
  materializeStoredLyrics,
  resolveKnownNeteaseLyricSongId,
  shouldBackfillNeteaseRomanization,
  toEditableRomanizationText,
} = await loadTsModule('../src/modules/lyrics/lyricsFormat.ts')

const wordLine = {
  startMs: 10000,
  durationMs: 2000,
  text: '你好',
  words: [
    { startMs: 10000, durationMs: 500, text: '你' },
    { startMs: 10500, durationMs: 500, text: '好' },
  ],
}

const plainLine = {
  startMs: 12000,
  durationMs: 3000,
  text: '世界',
  words: [],
}

assert.equal(hasWordTimedEntries([wordLine]), true)
assert.equal(hasWordTimedEntries([plainLine]), false)

const yrc = lyricLineToYrc(wordLine)
assert.equal(yrc, '[10000,2000](10000,500,0)你(10500,500,0)好')

const lrc = lyricLineToLrc(plainLine, plainLine.text)
assert.match(lrc, /^\[00:12\.00\]世界$/)

const editable = toEditableLyricsText([wordLine, plainLine])
assert.match(editable, /\[10000,2000\]/)
// 混排时 plain 行也走 YRC 行头, 避免 parse_auto 丢行
assert.match(editable, /\[12000,3000\]世界/)
assert.doesNotMatch(editable, /\[00:12\.00\]/)

// 无逐字时整段走 LRC
const plainOnly = toEditableLyricsText([plainLine])
assert.match(plainOnly, /^\[00:12\.00\]世界$/)

const payload = withUpdatedLyricsPayload(
  { matchedLyric: 'old', name: 'song' },
  yrc,
  null,
  'NETEASE',
)
assert.equal(payload.matchedLyric, yrc)
assert.equal(payload.originalLyric, 'old')
assert.equal(payload.matchedLyricSource, 'NETEASE')
assert.equal(resolveStoredLyricText(payload), yrc)

const nowPlaying = await readFile(
  new URL('../src/components/NowPlaying.vue', import.meta.url),
  'utf8',
)
assert.match(nowPlaying, /parse_lrc_content/)
assert.match(nowPlaying, /toEditableLyricsText/)
assert.match(nowPlaying, /resolveStoredLyricStateFromPayload|resolveStoredLyricText/)
assert.match(nowPlaying, /withUpdatedLyricsPayload/)
assert.match(nowPlaying, /commitLyricsToTrack|persistTrackSyncPayload/)

// CURRENT version 标记
assert.equal(payload.syncMetadataVersion, 1)

// 用户编辑标记为已编辑，修订号至少是当前时间且严格大于原值，同步时才能盖过别的设备
assert.equal(payload.lyricSyncEdited, true)
const reset = { lyricSyncRevision: 9e12, lyricSyncEdited: false }
const reedited = withUpdatedLyricsPayload(reset, 'x', null, null, 1_000)
assert.equal(reedited.lyricSyncEdited, true)
assert.equal(reedited.lyricSyncRevision, 9e12 + 1)
assert.equal(withUpdatedLyricsPayload({ lyric_sync_revision: 5 }, 'x', null, null, 2_000).lyricSyncRevision, 2_000)

// 有意清空写空串 (CLEARED), 保留 original
const cleared = withUpdatedLyricsPayload(payload, null, null, null)
assert.equal(cleared.matchedLyric, '')
assert.equal(cleared.originalLyric, 'old')
assert.equal(resolveStoredLyricStateFromPayload(cleared).kind, 'cleared')
assert.equal(resolveStoredLyricText(cleared), '')

// absent: 无 matched/original
assert.equal(resolveStoredLyricStateFromPayload({ name: 'x' }).kind, 'absent')
assert.equal(resolveStoredLyricText({ name: 'x' }), null)

// present via original fallback when matched missing
assert.equal(
  resolveStoredLyricStateFromPayload({ originalLyric: '[00:01.00]a' }).kind,
  'present',
)

const baselineAnnotations = [
  { startMs: 1000, durationMs: 1000, text: 'Hello', words: [], translation: '你好', roman: 'halo' },
  { startMs: 3000, durationMs: 1000, text: 'World', words: [], translation: '世界', roman: 'world' },
]
const upgradedWords = [
  { startMs: 1100, durationMs: 1000, text: 'Hello', words: [{ startMs: 1100, durationMs: 600, text: 'Hello' }] },
  { startMs: 1700, durationMs: 500, text: 'other', words: [] },
  { startMs: 3100, durationMs: 1000, text: 'World', words: [], translation: '外源翻译', roman: 'external' },
]
const mergedUpgrade = mergeWordTimedLyricsWithBaseline(baselineAnnotations, upgradedWords)
assert.equal(mergedUpgrade[0].translation, '你好')
assert.equal(mergedUpgrade[0].roman, 'halo')
assert.equal(mergedUpgrade[1].translation, undefined)
assert.equal(mergedUpgrade[1].roman, undefined)
assert.equal(mergedUpgrade[2].translation, '外源翻译')
assert.equal(mergedUpgrade[2].roman, 'external')
assert.deepEqual(mergedUpgrade[0].words, upgradedWords[0].words)
assert.equal(upgradedWords[0].translation, undefined)
assert.equal(baselineAnnotations[0].translation, '你好')

// 音译与翻译一样单独存成 matchedRomanizedLyric（对齐 Android），不传时保持不变
const romanLines = [
  { startMs: 1000, durationMs: 1000, text: '夜', words: [], roman: 'yoru' },
  { startMs: 3000, durationMs: 1000, text: '空', words: [] },
]
assert.equal(toEditableRomanizationText(romanLines), '[00:01.00]yoru', '没有音译的行不导出')
const withRoman = withUpdatedLyricsPayload({ matchedRomanizedLyric: 'old roman' }, 'x', null, 'KUGOU', 1_000, '[00:01.00]yoru')
assert.equal(withRoman.matchedRomanizedLyric, '[00:01.00]yoru')
assert.equal(withRoman.originalRomanizedLyric, 'old roman', '首次覆盖保留原音译')
const untouched = withUpdatedLyricsPayload(withRoman, 'y', null, null, 2_000)
assert.equal(untouched.matchedRomanizedLyric, '[00:01.00]yoru', '只改原文/翻译的旧调用方不动音译')
const romanCleared = withUpdatedLyricsPayload(withRoman, 'z', null, null, 3_000, null)
assert.equal(romanCleared.matchedRomanizedLyric, '', '换成没有音译的歌词时清空')
assert.equal(resolveStoredRomanizedLyricStateFromPayload(romanCleared).kind, 'cleared')
assert.equal(resolveStoredRomanizedLyricStateFromPayload({ originalRomanizedLyric: '[00:01.00]a' }).kind, 'present')

const mergedRoman = mergeParsedLyricsWithRomanization(
  [{ startMs: 1000, durationMs: 1000, text: '夜', words: [], translation: '夜晚' }, { startMs: 3000, durationMs: 1000, text: '空', words: [], roman: 'sora-old' }],
  [{ startMs: 1010, durationMs: 0, text: 'yoru', words: [] }],
)
assert.equal(mergedRoman[0].roman, 'yoru', '按时间轴容差并到对应行')
assert.equal(mergedRoman[0].translation, '夜晚', '不动翻译')
assert.equal(mergedRoman[1].roman, 'sora-old', '没对上的行保留原音译')

// 翻译与 YRC 行首相差 0.6~1s（超出 450ms）时按区间重叠逐行对上，对齐 Android matchTranslationsToLineIndices
const yrcLines = [[24300, 3280], [27580, 2220], [33420, 2750], [36200, 3950], [45580, 4170], [50070, 2910]]
  .map(([startMs, durationMs], i) => ({ startMs, durationMs, text: `l${i}`, words: [] }))
const lrcTranslations = [24450, 28220, 32760, 36950, 44560, 49990]
  .map((startMs, i, all) => ({ startMs, durationMs: (all[i + 1] ?? startMs + 5000) - startMs, text: `t${i}`, words: [] }))
assert.deepEqual(
  mergeParsedLyricsWithTranslations(yrcLines, lrcTranslations).map(l => l.translation),
  ['t0', 't1', 't2', 't3', 't4', 't5'],
)
const creditMerged = mergeParsedLyricsWithTranslations(
  [{ startMs: 1000, durationMs: 0, text: '作词：a', words: [] }, { startMs: 1000, durationMs: 2000, text: 'Hello', words: [] }, { startMs: 3000, durationMs: 2000, text: 'Hi', words: [] }],
  [{ startMs: 1000, durationMs: 2000, text: '你好', words: [] }, { startMs: 3000, durationMs: 2000, text: '//', words: [] }],
)
assert.deepEqual(creditMerged.map(l => l.translation), [undefined, '你好', undefined], '同刻向组尾对齐，占位符不显示')

// 同步歌词缺音译时的网易云补全（Android loadNeteaseRomanizedFallback / resolveKnownNeteaseLyricSongId）
assert.equal(resolveKnownNeteaseLyricSongId({ id: 'netease:123' }), 123)
assert.equal(resolveKnownNeteaseLyricSongId({ id: 'bilibili:1', syncPayload: { matchedLyricSource: 'CLOUD_MUSIC', matchedSongId: '456' } }), 456)
assert.equal(resolveKnownNeteaseLyricSongId({ id: 'netease:123', syncPayload: { matchedLyricSource: 'QQ_MUSIC', matchedSongId: '456' } }), 123, 'QQ 的匹配 ID 不能拿去网易云取词')
assert.equal(resolveKnownNeteaseLyricSongId({ id: 'youtube:abc' }), null)
const plainSynced = [{ startMs: 0, durationMs: 1000, text: 'a', words: [] }]
assert.equal(shouldBackfillNeteaseRomanization({ matchedLyric: 'x' }, plainSynced), true)
assert.equal(shouldBackfillNeteaseRomanization({ matchedLyric: 'x', lyricSyncEdited: true }, plainSynced), true, '只编辑过原文时仍补音译')
assert.equal(shouldBackfillNeteaseRomanization({ lyricSyncEdited: true, matchedRomanizedLyric: '' }, plainSynced), false, '用户有意清空音译')
assert.equal(
  shouldBackfillNeteaseRomanization({ matchedLyric: 'x', lyricSyncEdited: true, matchedRomanizedLyric: null, originalRomanizedLyric: null }, plainSynced),
  true,
  'Android 同步来的 null 音译是没有，不是清空',
)
assert.equal(resolveStoredRomanizedLyricStateFromPayload({ matchedRomanizedLyric: null }).kind, 'absent')
assert.equal(resolveStoredLyricStateFromPayload({ matchedLyric: null, originalLyric: '[00:01.00]a' }).kind, 'present', 'null 时回落到 original')
assert.equal(shouldBackfillNeteaseRomanization({}, [{ ...plainSynced[0], roman: 'a' }]), false, '已有音译')
assert.equal(shouldBackfillNeteaseRomanization({}, []), false)

// 存储歌词落地：原文 + 翻译 + 音译；副轨解析失败只丢副轨
const parsedTracks = {
  '[00:01.00]夜': [{ startMs: 1000, durationMs: 1000, text: '夜', words: [] }],
  '[00:01.00]夜晚': [{ startMs: 1000, durationMs: 0, text: '夜晚', words: [] }],
  '[00:01.00]yoru': [{ startMs: 1000, durationMs: 0, text: 'yoru', words: [] }],
}
const parse = async text => {
  if (!(text in parsedTracks)) throw new Error(`bad ${text}`)
  return parsedTracks[text]
}
const stored = await materializeStoredLyrics({
  matchedLyric: '[00:01.00]夜', matchedTranslatedLyric: '[00:01.00]夜晚', matchedRomanizedLyric: '[00:01.00]yoru',
}, parse)
assert.deepEqual([stored[0].text, stored[0].translation, stored[0].roman], ['夜', '夜晚', 'yoru'])
const parsedParts = []
await materializeStoredLyrics(
  { matchedLyric: '[00:01.00]夜', matchedTranslatedLyric: '[00:01.00]夜晚' },
  async (text, part) => { parsedParts.push(part); return parse(text) },
)
assert.deepEqual(parsedParts, ['original', 'secondary'], '只有原文按匹配歌词去牛皮癣')
const secondaryErrors = []
const brokenRoman = await materializeStoredLyrics(
  { matchedLyric: '[00:01.00]夜', matchedRomanizedLyric: 'broken' }, parse, error => secondaryErrors.push(error))
assert.equal(brokenRoman[0].roman, undefined)
assert.equal(secondaryErrors.length, 1, '音译解析失败要报出来，不能静默')
assert.equal(await materializeStoredLyrics({ name: 'x' }, parse), null, '没有本地歌词时可在线拉取')
assert.deepEqual(await materializeStoredLyrics({ matchedLyric: '' }, parse), [], '有意清空')
await assert.rejects(() => materializeStoredLyrics({ matchedLyric: 'broken' }, parse), '原文解析失败照常抛出')

console.log('lyrics format tests passed')
