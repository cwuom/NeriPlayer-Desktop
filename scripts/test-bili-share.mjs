import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'

const vue = readFileSync(new URL('../src/components/NowPlaying.vue', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('NowPlaying.ts', vue.match(/<script setup[^>]*>([\s\S]*?)<\/script>/)[1], ts.ScriptTarget.ES2022, true)
const share = parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'shareSong')
assert.ok(share)
const compiled = ts.transpileModule(share.getText(parsed), { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const policy = {}
new Function('exports', ts.transpileModule(readFileSync(new URL('../src/modules/playback/biliVideoSkip.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(policy)
for (const fixture of [
  { id: 'bilibili:123456:222', album: 'Bilibili|222|BV1actual', syncPayload: { audioId: '123456', subAudioId: '222' }, want: 'https://www.bilibili.com/video/BV1actual?cid=222' },
  { id: 'bilibili:BV1legacy', album: '', want: 'https://www.bilibili.com/video/BV1legacy' },
  { id: 'bilibili:123456', album: '', want: 'https://www.bilibili.com/video/av123456' },
  { id: 'bilibili:BV1legacy:2', album: '', want: 'https://www.bilibili.com/video/BV1legacy' },
]) {
  let copied
  const context = { ...policy, player: { currentTrack: { title: 'Song', artist: 'UP', ...fixture } },
    writeClipboardText: async text => { copied = text }, toast: { success() {}, error() {} }, t: key => key,
    showMoreSheet: { value: true },
  }
  const run = new Function(...Object.keys(context), `${compiled}\nreturn shareSong`)(...Object.values(context))
  await run()
  assert.equal(copied, `Song - UP\n${fixture.want}`)
}
console.log('Bilibili share regressions passed: 4')
