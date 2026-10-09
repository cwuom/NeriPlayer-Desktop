import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/lyrics/lyricsCache.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText

const store = new Map()
const calls = []
const persistentCache = {
  async getCachedValue(bucket, key) { calls.push(['get', bucket, key]); return store.get(key) ?? null },
  async setCachedValue(bucket, key, value) { calls.push(['set', bucket, key]); store.set(key, value) },
  async removeCachedValue(bucket, key) { calls.push(['remove', bucket, key]); store.delete(key) },
}
const lyricSources = new Map()
const lyricSource = {
  rememberLyricSource: (track, value) => { if (value) lyricSources.set(track.id, value); else lyricSources.delete(track.id) },
  lyricSourceOf: track => lyricSources.get(track.id) ?? null,
}
const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === '@/utils/persistentCache') return persistentCache
  if (name === './lyricsRequest') return { lyricsIdentity: track => track.id }
  if (name === './lyricSource') return lyricSource
  throw new Error(`Unexpected lyrics cache dependency: ${name}`)
}, exports)

const lines = [{ startMs: 0, durationMs: 1000, words: [], text: 'hello' }]

const online = { id: 'netease:1', source: 'netease', title: 't', artist: 'a', album: '', durationMs: 1 }
await exports.saveCachedLyrics(online, lines)
assert.deepEqual((await exports.getCachedLyrics(online)).map(line => line.text), ['hello'])

// 歌词来源随缓存一起存：重启后读缓存，偏移量仍按 AMLL TTML 的默认算
lyricSource.rememberLyricSource(online, 'amll_ttml')
await exports.saveCachedLyrics(online, lines)
lyricSources.clear()
assert.equal((await exports.getCachedLyrics(online)).length, 1)
assert.equal(lyricSource.lyricSourceOf(online), 'amll_ttml')

// 改版前写下的缓存是纯数组：照常读出，来源记为未知
const legacy = { id: 'qq:legacy', source: 'qq', title: 't', artist: 'a', album: '', durationMs: 1 }
lyricSource.rememberLyricSource(legacy, 'lrclib')
store.set(`v4:${legacy.id}`, lines)
assert.deepEqual((await exports.getCachedLyrics(legacy)).map(line => line.text), ['hello'])
assert.equal(lyricSource.lyricSourceOf(legacy), null)

// v3 缓存缺音译、没去制作信息行，升级后不再读
const stale = { id: 'netease:stale', source: 'netease', title: 't', artist: 'a', album: '', durationMs: 1 }
store.set(`v3:${stale.id}`, { source: 'netease', lines })
assert.equal(await exports.getCachedLyrics(stale), null)

// 本地歌曲每次都重新读取歌词文件，不读也不写持久缓存
calls.length = 0
const local = { id: 'local:D:\\Music\\a.flac', source: 'local', title: 't', artist: 'a', album: '', durationMs: 1 }
await exports.saveCachedLyrics(local, lines)
assert.equal(await exports.getCachedLyrics(local), null)
assert.deepEqual(calls, [])

// 网易云私人雷达歌单的详情缓存按账号区分，其余歌单键不变
const detailSource = await readFile(new URL('../src/modules/library/playlistDetailCache.ts', import.meta.url), 'utf8')
const detailCompiled = ts.transpileModule(detailSource, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const detail = {}
new Function('require', 'exports', detailCompiled)(name => {
  if (name === '@/utils/persistentCache') return persistentCache
  throw new Error(`Unexpected detail cache dependency: ${name}`)
}, detail)
const radarA = detail.playlistDetailCacheKey('netease-playlist', 5320167908, 'alice')
const radarB = detail.playlistDetailCacheKey('netease-playlist', 5320167908, 'bob')
assert.notEqual(radarA, radarB)
assert.match(detail.playlistDetailCacheKey('netease-playlist', 5320167908, null), /:public$/)
assert.equal(detail.playlistDetailCacheKey('netease-playlist', 42, 'alice'), 'netease-playlist:42')
assert.equal(detail.playlistDetailCacheKey('netease-album', 5320167908, 'alice'), 'netease-album:5320167908')

console.log('lyrics cache tests passed')
