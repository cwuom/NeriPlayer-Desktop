import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const [titleBarSource, mainSource, appSource] = await Promise.all([
  readFile(new URL('../src/components/TitleBar.vue', import.meta.url), 'utf8'),
  readFile(new URL('../src/main.ts', import.meta.url), 'utf8'),
  readFile(new URL('../src/App.vue', import.meta.url), 'utf8'),
])

assert.match(
  titleBarSource,
  /try\s*\{[\s\S]*?getCurrentWindow\(\)[\s\S]*?\}\s*catch/,
  'TitleBar must guard the synchronous browser-only getCurrentWindow failure',
)
assert.match(
  titleBarSource,
  /if \(!appWindow\) return/,
  'native title-bar actions must be no-ops in browser preview',
)
assert.match(
  mainSource,
  /try\s*\{[\s\S]*?getCurrentWindow\(\)\.show\(\)[\s\S]*?\}\s*catch/,
  'main window reveal must guard the synchronous browser-only failure',
)
assert.match(
  appSource,
  /try\s*\{[\s\S]*?listen\(['"]playlists-changed['"][\s\S]*?\}\s*catch/,
  'backend event listeners must not reject the browser preview mount',
)

console.log('browser preview safety tests passed')
