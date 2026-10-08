import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/download/downloadQueue.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { DownloadQueue } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
let limit = 2
const queue = new DownloadQueue(() => limit)
const started = []
for (const id of ['a', 'b', 'c', 'd', 'e']) queue.enqueue(id, () => started.push(id))
assert.deepEqual(started, ['a', 'b'])
assert.equal(queue.enqueue('a', () => assert.fail('duplicate must not start')), false)
assert.equal(queue.cancelPending('c'), true)
limit = 1
queue.refresh()
queue.finish('a')
assert.deepEqual(started, ['a', 'b'])
queue.finish('b')
assert.deepEqual(started, ['a', 'b', 'd'])
limit = 3
queue.refresh()
assert.deepEqual(started, ['a', 'b', 'd', 'e'])
assert.equal(queue.enqueue('a', () => started.push('retry-a')), true)
assert.deepEqual(started, ['a', 'b', 'd', 'e', 'retry-a'])
queue.finish('already-finished')
assert.equal(queue.cancelPending('d'), false)

const maxStarted = []
const maxQueue = new DownloadQueue(() => 100)
for (let id = 0; id < 20; id++) maxQueue.enqueue(String(id), () => maxStarted.push(id))
assert.equal(maxStarted.length, 8)
console.log('download queue tests passed')
