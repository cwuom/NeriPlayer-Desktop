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
  mergeWordTimedLyricsWithBaseline,
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

console.log('lyrics format tests passed')
