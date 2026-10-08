import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/stores/listenTogether/reconnect.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const transpiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  fileName: sourceUrl.pathname,
  reportDiagnostics: true,
})
assert.equal(transpiled.diagnostics?.length ?? 0, 0)
const { MAX_RECONNECT_ATTEMPTS, reconnectDelayMs, isTerminalReconnectError } =
  await import(`data:text/javascript;base64,${Buffer.from(transpiled.outputText).toString('base64')}`)

assert.equal(MAX_RECONNECT_ATTEMPTS, 15)
assert.deepEqual([0, 0.5, 1].map(random => reconnectDelayMs(1, () => random)), [1200, 1500, 1800])
assert.deepEqual([2, 3, 4, 5].map(attempt => reconnectDelayMs(attempt, () => 0.5)), [3000, 5000, 8000, 12000])
assert.equal(reconnectDelayMs(9, () => 1), 14400, 'later attempts stay on the longest base delay')

for (const terminal of [
  'WebSocket connect failed: HTTP error: 410 Gone',
  'HTTP 404: room not found',
  'http error: 401 Unauthorized',
  'room closed',
  'Room not initialized',
  'session not found in DO',
  'unauthorized',
]) {
  assert.equal(isTerminalReconnectError(terminal), true, terminal)
}
for (const transient of [
  'WebSocket connect failed: HTTP error: 502 Bad Gateway',
  'connection reset by peer',
  'timed out',
  'HTTP 4100',
  '',
  undefined,
]) {
  assert.equal(isTerminalReconnectError(transient), false, String(transient))
}

console.log('listen together reconnect policy tests passed')
