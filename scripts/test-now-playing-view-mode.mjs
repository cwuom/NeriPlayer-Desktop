import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/modules/nowPlaying/viewMode.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { canEnterLyricsMode, resolveNowPlayingViewMode } = await import(moduleUrl)

assert.equal(canEnterLyricsMode(false, false), false)
assert.equal(canEnterLyricsMode(true, false), true)
assert.equal(canEnterLyricsMode(false, true), true)

assert.equal(
  resolveNowPlayingViewMode('cover', 'lyrics', false, false),
  'cover',
)
assert.equal(
  resolveNowPlayingViewMode('cover', 'lyrics', true, false),
  'lyrics',
)
assert.equal(
  resolveNowPlayingViewMode('cover', 'lyrics', false, true),
  'lyrics',
)
assert.equal(
  resolveNowPlayingViewMode('lyrics', 'cover', true, false),
  'cover',
)
assert.equal(
  resolveNowPlayingViewMode('lyrics', 'lyrics', false, false),
  'cover',
)

console.log('now playing view mode tests passed')
