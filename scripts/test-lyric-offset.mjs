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
  DEFAULT_LYRIC_OFFSET_MS,
  LYRIC_OFFSET_SOURCES,
  normalizeLyricDefaultOffsetMs,
  normalizeLyricSource,
  resolveLyricOffsetSource,
  resolveLyricDefaultOffsetMs,
  rebaseLyricUserOffsetMs,
  shouldRebaseLyricOffset,
  clampLyricOffsetMs,
  lyricUserOffsetStorageKey,
  readSyncedUserOffsetMs,
  readSyncedLyricSource,
  withUpdatedUserOffsetPayload,
  formatLyricOffsetMs,
} = await loadTsModule('../src/modules/lyrics/lyricOffset.ts')

// 五个来源与默认值对齐 Android LyricOffsetDefaults
assert.deepEqual([...LYRIC_OFFSET_SOURCES], ['netease', 'qq', 'kugou', 'lrclib', 'amll_ttml'])
assert.deepEqual(DEFAULT_LYRIC_OFFSET_MS, { netease: 1000, qq: 500, kugou: 0, lrclib: 0, amll_ttml: 0 })

// 默认偏移按 50ms 对齐、夹到 ±5000ms；半步向正无穷取整（Kotlin roundToLong）
assert.equal(normalizeLyricDefaultOffsetMs(1024), 1000)
assert.equal(normalizeLyricDefaultOffsetMs(-75), -50)
assert.equal(normalizeLyricDefaultOffsetMs(75), 100)
assert.equal(normalizeLyricDefaultOffsetMs(9000), 5000)
assert.equal(normalizeLyricDefaultOffsetMs(-9000), -5000)
assert.equal(normalizeLyricDefaultOffsetMs(Number.NaN, 500), 500)
assert.ok(!Object.is(normalizeLyricDefaultOffsetMs(-10), -0))

// 歌词来源归一：后端 snake_case、Android 枚举名、桌面端旧写法
for (const [raw, expected] of [
  ['netease', 'netease'], ['CLOUD_MUSIC', 'netease'],
  ['qq', 'qq'], ['QQ_MUSIC', 'qq'], ['QqMusic', 'qq'],
  ['kugou', 'kugou'], ['Kugou', 'kugou'],
  ['lrclib', 'lrclib'], ['LRCLIB', 'lrclib'], ['LrcLib', 'lrclib'],
  ['amll_ttml', 'amll_ttml'], ['AmllTtml', 'amll_ttml'],
  ['youtube', 'none'], ['local', 'none'], ['LOCAL_EDIT', 'none'],
  ['', null], [null, null], [undefined, null], ['something', null], [42, null],
]) {
  assert.equal(normalizeLyricSource(raw), expected, String(raw))
}

// 默认按当前歌词的实际来源选；网易云曲目显示 AMLL TTML 逐字歌词时用 TTML 的默认
assert.equal(resolveLyricOffsetSource('amll_ttml', 'netease'), 'amll_ttml')
assert.equal(resolveLyricOffsetSource('lrclib', 'youtube'), 'lrclib')
assert.equal(resolveLyricOffsetSource('kugou', 'bilibili'), 'kugou')
// 手动编辑、YouTube、本地歌词与来源未知时对齐 Android：QQ 曲目用 QQ 默认，其它回落网易云默认
assert.equal(resolveLyricOffsetSource('LOCAL_EDIT', 'netease'), 'netease')
assert.equal(resolveLyricOffsetSource('LOCAL_EDIT', 'qq'), 'qq')
assert.equal(resolveLyricOffsetSource('youtube', 'youtube'), 'netease')
assert.equal(resolveLyricOffsetSource('local', 'local'), 'netease')
assert.equal(resolveLyricOffsetSource(null, 'netease'), 'netease')
assert.equal(resolveLyricOffsetSource(null, 'qq'), 'qq')
assert.equal(resolveLyricOffsetSource(null, 'youtube'), 'netease')
assert.equal(resolveLyricOffsetSource(null, 'bilibili'), 'netease')
assert.equal(resolveLyricOffsetSource(null, null), 'netease')

const defaults = { netease: 1000, qq: 500, kugou: -200, lrclib: 150, amll_ttml: 300 }
assert.equal(resolveLyricDefaultOffsetMs('netease', defaults), 1000)
assert.equal(resolveLyricDefaultOffsetMs('kugou', defaults), -200)
assert.equal(resolveLyricDefaultOffsetMs('amll_ttml', defaults), 300)
assert.equal(resolveLyricDefaultOffsetMs(null, defaults), 0)

// rebase: 改默认时, 已调过的歌曲保持绝对时序不变
const rebased = rebaseLyricUserOffsetMs(250, 1000, 700)
assert.equal(rebased, 550)
assert.equal(1000 + 250, 700 + rebased)

// shouldRebase: 仅"来源一致且 delta != 0"才 rebase
assert.equal(shouldRebaseLyricOffset('lrclib', 'lrclib', 100), true)
assert.equal(shouldRebaseLyricOffset('lrclib', 'lrclib', 0), false)
assert.equal(shouldRebaseLyricOffset('netease', 'lrclib', 100), false)
assert.equal(shouldRebaseLyricOffset(null, 'netease', 100), false)

// clamp: 归一为整数并夹到安全边界, 非有限值归零
assert.equal(clampLyricOffsetMs(30500), 30000)
assert.equal(clampLyricOffsetMs(-40000), -30000)
assert.equal(clampLyricOffsetMs(1.6), 2)
assert.equal(clampLyricOffsetMs(Number.NaN), 0)
assert.equal(clampLyricOffsetMs(Number.POSITIVE_INFINITY), 0)

// 存储键: 优先 id, 兜底 playlistKey, 均无则空串
assert.equal(lyricUserOffsetStorageKey({ id: 'netease:1' }), 'netease:1')
assert.equal(lyricUserOffsetStorageKey({ id: '', playlistKey: 'k' }), 'k')
assert.equal(lyricUserOffsetStorageKey(null), '')

// sync-in: 读取 Android 写下的逐曲偏移与歌词来源, 缺失/非法均回退
assert.equal(readSyncedUserOffsetMs({ syncPayload: { userLyricOffsetMs: 300 } }), 300)
assert.equal(readSyncedUserOffsetMs({ syncPayload: { user_lyric_offset_ms: 180 } }), 180)
assert.equal(readSyncedUserOffsetMs({ syncPayload: {} }), 0)
assert.equal(readSyncedUserOffsetMs({}), 0)
assert.equal(readSyncedUserOffsetMs({ syncPayload: { userLyricOffsetMs: 'x' } }), 0)
assert.equal(readSyncedLyricSource({ matchedLyricSource: 'QQ_MUSIC' }), 'QQ_MUSIC')
assert.equal(readSyncedLyricSource({ matched_lyric_source: 'CLOUD_MUSIC' }), 'CLOUD_MUSIC')
assert.equal(readSyncedLyricSource({ matchedLyricSource: '  ' }), null)
assert.equal(readSyncedLyricSource(undefined), null)

const offsetPayload = withUpdatedUserOffsetPayload({ matchedLyric: 'x' }, 250)
assert.equal(offsetPayload.userLyricOffsetMs, 250)
assert.equal(offsetPayload.syncMetadataVersion, 1)
assert.equal(offsetPayload.matchedLyric, 'x')

assert.equal(formatLyricOffsetMs(1000), '+1000ms')
assert.equal(formatLyricOffsetMs(-250), '-250ms')
assert.equal(formatLyricOffsetMs(0), '0ms')

console.log('lyric offset tests passed')
