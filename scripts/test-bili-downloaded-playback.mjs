import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'

const source = readFileSync(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const declarations = new Map()
let lookup
function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name) declarations.set(node.name.text, node.getText(parsed))
  if (ts.isVariableDeclaration(node) && node.name.getText(parsed) === 'downloaded' && node.initializer) lookup = node.initializer.getText(parsed)
  ts.forEachChild(node, visit)
}
visit(parsed)
assert.ok(lookup)
const policy = {}
new Function('exports', ts.transpileModule(readFileSync(new URL('../src/modules/playback/biliVideoSkip.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(policy)
const compiled = ts.transpileModule(declarations.get('downloadedTrackForPlayback') || '', {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const candidate = (id, cid) => ({ id, source: 'bilibili', album: cid ? `Bilibili|${cid}|BV1parts` : '', filePath: 'C:/Downloads/part.m4a' })
for (const fixture of [
  { track: { id: 'bilibili:12345:222', album: 'Bilibili|222|BV1parts' }, downloads: [candidate('bilibili:12345', 222)], want: 'bilibili:12345' },
  { track: { id: 'bilibili:12345:222', album: 'Bilibili|222|BV1parts' }, downloads: [candidate('bilibili:BV1parts', 222)], want: 'bilibili:BV1parts' },
  { track: { id: 'bilibili:12345:222', album: 'Bilibili|222|BV1parts' }, downloads: [candidate('bilibili:12345', 111)], want: undefined },
  { track: { id: 'bilibili:12345:222', album: 'Bilibili|222|BV1parts' }, downloads: [candidate('bilibili:12345')], want: undefined },
  { track: { id: 'bilibili:12345:222', album: 'Bilibili|222|BV1parts' }, downloads: [candidate('bilibili:12345:222', 222), candidate('bilibili:12345', 222)], want: 'bilibili:12345:222' },
  { track: { id: 'bilibili:12345:222', album: 'Bilibili|111|BV1parts', syncPayload: { subAudioId: '222' } }, downloads: [candidate('bilibili:12345', 111)], want: undefined },
  { track: { id: 'netease:12345', album: '' }, downloads: [candidate('netease:12345')], want: 'netease:12345' },
]) {
  const context = { ...policy, track: { source: 'bilibili', ...fixture.track },
    useDownloadStore: () => ({ getDownloadedTrack: id => fixture.downloads.find(item => item.id === id) }),
  }
  const actual = new Function(...Object.keys(context), `${compiled}\nreturn ${lookup}`)(...Object.values(context))
  assert.equal(actual?.id, fixture.want)
}
console.log('Bilibili downloaded playback regressions passed: 7')
