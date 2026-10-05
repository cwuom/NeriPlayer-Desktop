import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinia from 'pinia'
import * as vue from 'vue'

const source = await readFile(new URL('../src/stores/sync.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true },
}).outputText
const calls = []
const challenges = {
  github: { backend: 'github', target: 'owner/repo', fingerprint: 'a'.repeat(64), sourceProtocol: 3, targetProtocol: 4 },
  webdav: { backend: 'webdav', target: 'https://example.test/backup/', fingerprint: 'b'.repeat(64), sourceProtocol: 0, targetProtocol: 4 },
}
const approved = new Set()
const invoke = async (command, args) => {
  calls.push({ command, args })
  if (command === 'approve_sync_protocol_upgrade') { approved.add(args.challenge.backend); return }
  if (command === 'sync_github' || command === 'sync_webdav') {
    const backend = command === 'sync_github' ? 'github' : 'webdav'
    if (!approved.has(backend)) throw `SYNC_PROTOCOL_UPGRADE_REQUIRED:${JSON.stringify(challenges[backend])}`
    return { success: true, message: 'Sync complete' }
  }
  if (command === 'get_github_sync_config') return { configured: true, owner: 'owner', repo: 'repo', autoSync: true }
  if (command === 'get_webdav_sync_config') return { configured: true, serverUrl: 'https://example.test', autoSync: true }
  if (command === 'get_sync_preferences') return { historyUpdateMode: 'immediate' }
  if (command === 'import_config') return { success: true, settings: {} }
}
const toast = { success() {}, error(message) { throw new Error(`Unexpected toast: ${message}`) } }
const mocks = {
  pinia, vue,
  '@tauri-apps/api/core': { invoke },
  './toast': { useToastStore: () => toast },
  './history': { useHistoryStore: () => ({ getSyncSnapshot: () => ({ entries: [], deletions: [] }), applySyncPayload: async () => {} }) },
  './settings': { useSettingsStore: () => ({ applySnapshot() {} }) },
  './auth': { useAuthStore: () => ({ async checkStatus() {} }) },
  '@/i18n': { __esModule: true, default: { global: { t: key => key } }, setLocale() {} },
  '@/utils/logger': { createLogger: () => ({ error() {} }) },
}
const exports = {}
new Function('require', 'exports', compiled)(name => {
  assert.ok(mocks[name], `Unexpected dependency: ${name}`)
  return mocks[name]
}, exports)
const { parseSyncProtocolUpgrade, useSyncStore } = exports
assert.deepEqual(parseSyncProtocolUpgrade(`SYNC_PROTOCOL_UPGRADE_REQUIRED:${JSON.stringify(challenges.github)}`), challenges.github)
assert.equal(parseSyncProtocolUpgrade('ordinary failure'), null)
assert.equal(parseSyncProtocolUpgrade('SYNC_PROTOCOL_UPGRADE_REQUIRED:{invalid'), null)
for (const patch of [{ backend: 'unknown' }, { fingerprint: 'invalid' }, { sourceProtocol: 4 }, { targetProtocol: 5 }, { target: '' }]) {
  assert.equal(parseSyncProtocolUpgrade(`SYNC_PROTOCOL_UPGRADE_REQUIRED:${JSON.stringify({ ...challenges.github, ...patch })}`), null)
}
pinia.setActivePinia(pinia.createPinia())
const store = useSyncStore()
await store.loadConfigs()
await store.syncAuto()
assert.equal(store.pendingProtocolUpgrade.backend, 'github')
const previousCalls = calls.length
await store.syncAuto()
assert.equal(calls.length, previousCalls, 'automatic sync must not repeatedly fetch an unapproved migration')
await store.approveProtocolUpgrade()
assert.equal(store.pendingProtocolUpgrade.backend, 'webdav', 'both providers must retain their independent challenges')
assert.equal(calls.filter(item => item.command === 'approve_sync_protocol_upgrade').length, 1)
await store.approveProtocolUpgrade()
assert.equal(store.pendingProtocolUpgrade, null)
assert.equal(calls.filter(item => item.command === 'approve_sync_protocol_upgrade').length, 2)
assert.equal(store.isSyncing, false)
approved.clear()
await store.syncAuto()
assert.equal(store.pendingProtocolUpgrade.backend, 'github')
globalThis.localStorage = { removeItem() {}, setItem() {} }
await store.importConfig()
assert.equal(store.pendingProtocolUpgrade, null, 'imported configuration must discard challenges for the previous accounts and targets')
const afterImport = calls.length
await store.syncAuto()
assert.ok(calls.length > afterImport, 'automatic sync must inspect imported targets again')
approved.add('github')
await store.syncGitHub()
assert.equal(store.pendingProtocolUpgrade.backend, 'webdav', 'a successful manual sync must discard a challenge already resolved by another client')
await vue.nextTick()
console.log('test-sync-protocol-upgrade: passed actual store approval, provider queue, automatic retry guard, and invalid challenges')
