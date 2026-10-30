import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as vue from 'vue'
import * as pinia from 'pinia'

const calls = []
const notices = []
const applied = []
const accountChanges = []
let response = { success: false, requiresPassword: true, filePath: 'fixture.json' }
const storage = new Map()
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
  removeItem: key => storage.delete(key),
}
const mocks = {
  vue, pinia,
  '@tauri-apps/api/core': { invoke: async (command, args) => {
    calls.push({ command, args })
    if (command === 'import_config' || command === 'export_config') {
      if (response instanceof Error) throw response.message
      return response
    }
    if (command === 'get_sync_preferences') return { historyUpdateMode: 'immediate' }
    return { configured: false }
  } },
  './toast': { useToastStore: () => ({ success: value => notices.push(value), error: value => notices.push(value) }) },
  './history': { useHistoryStore: () => ({}) },
  './lyricOffset': { useLyricOffsetStore: () => ({}) },
  './biliVideoSkip': { useBiliVideoSkipStore: () => ({}) },
  './settings': { useSettingsStore: () => ({ snapshot: () => ({ locale: 'en' }), applySnapshot: value => applied.push(value) }) },
  './auth': { useAuthStore: () => ({ checkStatus: async () => accountChanges.push('check'), notifyAuthChanged: platform => accountChanges.push(platform) }) },
  '@/i18n': { __esModule: true, default: { global: { t: key => key } }, setLocale() {} },
  '@/utils/logger': { createLogger: () => ({ error() {} }) },
}
const source = await readFile(new URL('../src/stores/sync.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true } }).outputText
const exports = {}
new Function('require', 'exports', compiled)(name => {
  assert.ok(name in mocks, `Unexpected dependency: ${name}`)
  return mocks[name]
}, exports)
pinia.setActivePinia(pinia.createPinia())
const store = exports.useSyncStore()

await store.exportConfig({ password: ' fixture password ', includeLoginState: true })
assert.deepEqual(calls.at(-1).args, {
  settings: { locale: 'en' }, listenTogetherUserUuid: '', password: ' fixture password ', includeLoginState: true,
}, 'export must pass the unchanged password and explicit credential choice')
await store.exportConfig()
assert.equal(calls.at(-1).args.includeLoginState, false, 'credentials must be excluded by default')
assert.equal(calls.at(-1).args.password, undefined)
assert.equal(notices.length, 0, 'cancelled/preflight operations must not claim success')

let result = await store.importConfig()
assert.equal(result.requiresPassword, true)
assert.deepEqual(applied, [])
assert.deepEqual(accountChanges, [])
response = { success: false, requiresConfirmation: true, platform: 'android', filePath: 'fixture.json' }
result = await store.importConfig({ filePath: 'fixture.json', password: ' fixture password ' })
assert.equal(result.requiresConfirmation, true)
assert.deepEqual(calls.at(-1).args, { filePath: 'fixture.json', password: ' fixture password ' })
assert.deepEqual(applied, [], 'Android preview must not apply settings')
response = new Error('CONFIG_DECRYPTION_FAILED')
result = await store.importConfig({ filePath: 'fixture.json', password: 'wrong' })
assert.equal(result.success, false)
assert.equal(result.error, 'settings.config_decryption_failed')
assert.equal(notices.at(-1), 'settings.config_decryption_failed')
assert.deepEqual(applied, [])
response = { success: true, settings: { locale: 'zh-CN' }, listenTogetherUserUuid: 'fixture-uuid', platform: 'android' }
await store.importConfig({ filePath: 'fixture.json', password: ' fixture password ', confirmedAndroid: true })
assert.deepEqual(calls.findLast(call => call.command === 'import_config').args, {
  filePath: 'fixture.json', password: ' fixture password ', confirmedAndroid: true,
})
assert.deepEqual(applied, [{ locale: 'zh-CN' }])
assert.deepEqual(accountChanges, ['check', 'netease', 'bilibili', 'youtube'], 'import must refresh IDs and invalidate old account caches')
assert.equal(storage.get('neri:lt-uuid'), 'fixture-uuid')
assert.equal(notices.at(-1), 'settings.import_config_success')
console.log('PASS configuration IPC options, password/Android preflight, failure safety, and account refresh')
