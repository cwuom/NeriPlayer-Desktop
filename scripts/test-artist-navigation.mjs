import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/artistNavigation.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { neteaseSongArtists } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

assert.deepEqual(neteaseSongArtists({
  songs: [{ id: 123, ar: [{ id: 1, name: ' 歌手甲 ' }, { id: '2', name: '歌手乙' }, { id: 1, name: '重复' }] }],
}, 123), [{ id: 1, name: '歌手甲' }, { id: 2, name: '歌手乙' }])
assert.deepEqual(neteaseSongArtists({ songs: [{ id: 124, ar: [{ id: 1, name: '其他歌手' }] }] }, 123), [])
assert.deepEqual(neteaseSongArtists({
  songs: [{ id: '123', artists: [{ id: 3, name: '旧字段' }, { id: 0, name: '无效' }, { id: 4.5, name: '无效' }, { id: 5, name: '' }, null] }],
}, 123), [{ id: 3, name: '旧字段' }])
for (const value of [null, [], {}, { songs: 'wrong' }, { songs: [null, 42] }]) {
  assert.deepEqual(neteaseSongArtists(value, 123), [])
}
console.log('Artist navigation fixtures passed')
