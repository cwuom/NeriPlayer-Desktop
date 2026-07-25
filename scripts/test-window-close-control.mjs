import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const [titleBarSource, appSource, capabilitiesSource] = await Promise.all([
  readFile(new URL('../src/components/TitleBar.vue', import.meta.url), 'utf8'),
  readFile(new URL('../src/App.vue', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/capabilities/default.json', import.meta.url), 'utf8'),
])
const capabilities = JSON.parse(capabilitiesSource)

assert.match(
  titleBarSource,
  /requestClose:\s*\[\]/,
  'the custom title bar must delegate application shutdown to the app shell',
)
assert.match(titleBarSource, /emit\(['"]requestClose['"]\)/)
assert.match(
  titleBarSource,
  /class="tb-ctrl tb-close"[^>]*data-tauri-drag-region="false"/,
  'the close button must stay outside the custom title-bar drag region',
)
assert.doesNotMatch(
  titleBarSource,
  /function close\(\)[\s\S]*?appWindow\.close\(\)/,
  'the title bar must not close only the main webview while auxiliary windows remain alive',
)

assert.match(appSource, /getAllWebviewWindows/)
assert.match(appSource, /@request-close="closeApplication"/)
assert.match(
  appSource,
  /async function closeApplication\(\)[\s\S]*?handleBeforeUnload\(\)[\s\S]*?getAllWebviewWindows\(\)/,
  'player state must be flushed before native windows start closing',
)
assert.ok(
  capabilities.permissions.includes('core:window:allow-destroy'),
  'Tauri close-request handling and auxiliary-window cleanup require destroy permission',
)
assert.match(
  appSource,
  /function handleCloseRequested\(event: CloseRequestedEvent\)[\s\S]*?event\.preventDefault\(\)[\s\S]*?if \(isClosingApplication\) return[\s\S]*?closeApplication\(\)/,
  'every system close request must stay intercepted while coordinated shutdown is running',
)
assert.match(
  appSource,
  /onCloseRequested\(handleCloseRequested\)/,
  'the native close-request listener must use coordinated shutdown',
)
assert.match(
  appSource,
  /filter\(candidate => candidate\.label !== appWindow\.label\)[\s\S]*?candidate\.destroy\(\)[\s\S]*?appWindow\.destroy\(\)/,
  'auxiliary windows must be destroyed before the main window without re-entering close interceptors',
)
assert.doesNotMatch(
  appSource,
  /appWindow\.close\(\)/,
  'coordinated shutdown must not depend on allowing a recursive main-window close request',
)
assert.match(
  appSource,
  /if \(!isTauri\(\)\)[\s\S]*?window\.close\(\)[\s\S]*?return/,
  'browser preview must use the browser close path before invoking native window APIs',
)
assert.doesNotMatch(
  appSource,
  /candidate\.destroy\(\)\.catch/,
  'auxiliary-window cleanup failures must keep the main window open for another close attempt',
)
assert.match(
  appSource,
  /catch \(error\)[\s\S]*?isClosingApplication = false/,
  'native shutdown failures must reset the close guard',
)

console.log('window close control tests passed')
