import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/albumDisplay.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { displayAlbum } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

for (const [encoded, expected] of [
  ['Netease暖味ばんび～な', '暖味ばんび～な'],
  ['Neteasethree', 'three'],
  ['Neteaserye', 'rye'],
  ['Netease', ''],
  ['  netease   ', ''],
  ['  NeTeAsE 专辑名  ', '专辑名'],
  ['Bilibili|123456', ''],
  ['bilibili|987654|part', ''],
  ['Bilibili', ''],
  ['NeteaseBilibili|123456', 'Bilibili|123456'],
  ['three', 'three'],
  ['暖味ばんび～な', '暖味ばんび～な'],
  ['  Honest Album  ', 'Honest Album'],
  ['The Netease Collection', 'The Netease Collection'],
  ['Bilibili Live Album', 'Bilibili Live Album'],
  ['', ''],
  [null, ''],
  [undefined, ''],
  [123, ''],
]) {
  assert.equal(displayAlbum(encoded), expected, String(encoded))
}

const track = Object.freeze({
  album: 'Netease暖味ばんび～な',
  syncPayload: Object.freeze({ album: 'Netease暖味ばんび～な' }),
  playlistKey: '42|Netease暖味ばんび～な|',
})
assert.equal(displayAlbum(track.album), '暖味ばんび～な')
assert.equal(track.album, 'Netease暖味ばんび～な')
assert.equal(track.syncPayload.album, 'Netease暖味ばんび～な')
assert.equal(track.playlistKey, '42|Netease暖味ばんび～な|')
console.log('album display normalization tests passed')
