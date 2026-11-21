import assert from 'node:assert/strict'
import { readdir, readFile } from 'node:fs/promises'
import { ref } from 'vue'
import ts from 'typescript'

// player store 拆成 player.ts 与 player/ 下的切片，拼接全部源码后按名称查找实现
const playerDir = new URL('../src/stores/player/', import.meta.url)
const playerFiles = [new URL('../src/stores/player.ts', import.meta.url),
  ...(await readdir(playerDir)).filter(name => name.endsWith('.ts')).sort().map(name => new URL(name, playerDir))]
const source = (await Promise.all(playerFiles.map(file => readFile(file, 'utf8')))).join('\n')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
const names = ['markCommandSource', 'isRemoteSyncGuardActive', 'markOptimisticSeek']
const functions = new Map()
function visit(node) {
  if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text)) {
    functions.set(node.name.text, node.getText(parsed))
  }
  ts.forEachChild(node, visit)
}
visit(parsed)
for (const name of names) assert.ok(functions.has(name), `missing command source operation ${name}`)
const compiled = ts.transpileModule(names.map(name => functions.get(name)).join('\n'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

function fixture() {
  return new Function('lastCommandSource', 'lastCommandSequence', 'lastSeekCommand', 'Date', `
    let _remoteSyncGuardUntil = 0
    let playbackRequestToken = 1, lastSeekedMs = null
    const setRenderedPosition = value => value
    const clearPauseGuard = () => {}, armPendingSeek = () => {}
    ${compiled}
    return { markCommandSource, isRemoteSyncGuardActive, lastCommandSequence, markOptimisticSeek, lastSeekCommand }
  `)(ref('local'), ref(0), ref({ seq: 0 }), { now: () => 10000 })
}

const state = fixture()
assert.equal(state.isRemoteSyncGuardActive(), false)
state.markCommandSource('remote_sync')
assert.equal(state.isRemoteSyncGuardActive(), true, 'applying room playback must suppress synchronization echoes')
state.markCommandSource('local_safety')
assert.equal(state.isRemoteSyncGuardActive(), true, 'internal recovery must not turn a room update into a user command')
state.markCommandSource('local')
assert.equal(state.isRemoteSyncGuardActive(), false,
  'a user command immediately after a room update must be eligible for room reporting')
state.markCommandSource('remote_sync')
assert.equal(state.isRemoteSyncGuardActive(), true, 'the next remote update must start echo suppression again')
const before = state.lastCommandSequence.value
state.markCommandSource('local')
state.markCommandSource('local')
assert.equal(state.lastCommandSequence.value, before + 2,
  'consecutive user commands must be distinguishable from an older asynchronous completion')
state.markOptimisticSeek(12000, 'local', { fromPlaybackStart: true })
assert.equal(state.lastSeekCommand.value.fromPlaybackStart, true,
  'starting a new session at a remembered position must not be mistaken for seeking the old session')
state.markOptimisticSeek(15000, 'local')
assert.equal(state.lastSeekCommand.value.fromPlaybackStart, false,
  'an explicit seek must keep its separate room reporting behavior')
console.log('playback command source guard tests passed')
