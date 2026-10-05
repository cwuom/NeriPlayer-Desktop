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
const queuedResponses = new Map()
function delayInvoke(command) {
  let resolve
  let reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  const queue = queuedResponses.get(command) ?? []
  queue.push(promise)
  queuedResponses.set(command, queue)
  return { resolve, reject }
}
const invoke = async (command, args) => {
  calls.push({ command, args })
  const queue = queuedResponses.get(command)
  if (queue?.length) return queue.shift()
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
const historyApplications = []
const historyCommits = []
const mocks = {
  pinia, vue,
  '@tauri-apps/api/core': { invoke },
  './toast': { useToastStore: () => toast },
  './history': { useHistoryStore: () => ({
    getSyncSnapshot: () => ({ entries: [], deletions: [] }),
    async applySyncPayload(payload, isCurrent = () => true) {
      historyApplications.push(payload)
      const pending = queuedResponses.get('history_apply')?.shift()
      if (pending) await pending
      if (isCurrent()) historyCommits.push(payload)
    },
  }) },
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
const failures = []
let cases = 0
async function regression(name, run) {
  cases++
  pinia.setActivePinia(pinia.createPinia())
  const current = useSyncStore()
  try {
    await run(current)
    await vue.nextTick()
  } catch (error) {
    failures.push({ name, error })
    console.error(`FAIL ${name}: ${error.message}`)
  } finally {
    queuedResponses.clear()
  }
}

for (const [method, command, args, provider, response] of [
  ['validateGitHubToken', 'validate_github_token', ['fixture'], 'github', { username: 'old' }],
  ['createGitHubRepo', 'create_github_repo', ['old'], 'github', { owner: 'old', repo: 'old' }],
  ['useExistingGitHubRepo', 'use_existing_github_repo', ['old', 'old'], 'github', { owner: 'old', repo: 'old' }],
  ['configureGitHub', 'configure_github_sync', ['fixture', 'old'], 'github', { owner: 'old', repo: 'old' }],
  ['configureWebDav', 'configure_webdav_sync', ['https://old.test', 'fixture', 'fixture'], 'webdav', {}],
]) {
  await regression(`${method} cannot revive a disconnected provider`, async current => {
    const delayed = delayInvoke(command)
    const pending = current[method](...args)
    await (provider === 'github' ? current.disconnectGitHub() : current.disconnectWebDav())
    delayed.resolve(response)
    assert.equal(await pending, method === 'validateGitHubToken' ? null : false)
    assert.equal(current[provider].configured, false)
    assert.equal(current[provider].autoSync, false)
    assert.equal(current.dialogError, null)
  })
}

for (const provider of ['github', 'webdav']) {
  const command = provider === 'github' ? 'configure_github_sync' : 'configure_webdav_sync'
  const configure = (current, name) => provider === 'github'
    ? current.configureGitHub('fixture', name)
    : current.configureWebDav(`https://${name}.test`, 'fixture', 'fixture')
  const response = name => provider === 'github' ? { owner: name, repo: name } : {}
  const identity = current => provider === 'github' ? current.github.repo : current.webdav.serverUrl

  await regression(`${provider} latest configuration wins out of order`, async current => {
    const old = delayInvoke(command)
    const pendingOld = configure(current, 'old')
    const fresh = delayInvoke(command)
    const pendingFresh = configure(current, 'fresh')
    fresh.resolve(response('fresh'))
    assert.equal(await pendingFresh, true)
    old.resolve(response('old'))
    assert.equal(await pendingOld, false)
    assert.equal(identity(current), provider === 'github' ? 'fresh' : 'https://fresh.test')
  })

  await regression(`${provider} stale rejection preserves the latest error`, async current => {
    const old = delayInvoke(command)
    const pendingOld = configure(current, 'old')
    const fresh = delayInvoke(command)
    const pendingFresh = configure(current, 'fresh')
    fresh.reject(new Error('fresh failure'))
    assert.equal(await pendingFresh, false)
    const freshError = current.dialogError
    old.reject(new Error('old failure'))
    assert.equal(await pendingOld, false)
    assert.equal(current.dialogError, freshError)
  })

  await regression(`${provider} late disconnect does not erase a newer configuration`, async current => {
    const disconnect = delayInvoke(provider === 'github' ? 'disconnect_github_sync' : 'disconnect_webdav_sync')
    const pendingDisconnect = provider === 'github' ? current.disconnectGitHub() : current.disconnectWebDav()
    const fresh = delayInvoke(command)
    const pendingFresh = configure(current, 'fresh')
    fresh.resolve(response('fresh'))
    assert.equal(await pendingFresh, true)
    disconnect.resolve()
    await pendingDisconnect
    assert.equal(current[provider].configured, true)
    assert.equal(identity(current), provider === 'github' ? 'fresh' : 'https://fresh.test')
  })
}

await regression('provider configurations remain independent', async current => {
  const gh = delayInvoke('configure_github_sync')
  const ghPending = current.configureGitHub('fixture', 'github-new')
  const wd = delayInvoke('configure_webdav_sync')
  const wdPending = current.configureWebDav('https://webdav-new.test', 'fixture', 'fixture')
  wd.resolve({})
  gh.resolve({ owner: 'new', repo: 'github-new' })
  assert.equal(await wdPending, true)
  assert.equal(await ghPending, true)
  assert.equal(current.github.repo, 'github-new')
  assert.equal(current.webdav.serverUrl, 'https://webdav-new.test')
})

await regression('an older provider error cannot overwrite the active dialog error', async current => {
  const gh = delayInvoke('configure_github_sync')
  const ghPending = current.configureGitHub('fixture', 'old')
  const wd = delayInvoke('configure_webdav_sync')
  const wdPending = current.configureWebDav('https://fresh.test', 'fixture', 'fixture')
  wd.reject(new Error('active webdav failure'))
  await wdPending
  const error = current.dialogError
  gh.reject(new Error('older github failure'))
  await ghPending
  assert.equal(current.dialogError, error)
})

await regression('a delayed config read cannot restore a disconnected provider', async current => {
  const read = delayInvoke('get_github_sync_config')
  const loading = current.loadConfigs()
  await current.disconnectGitHub()
  read.resolve({ configured: true, owner: 'old', repo: 'old', autoSync: true })
  await loading
  assert.equal(current.github.configured, false)
  assert.equal(current.github.autoSync, false)
  assert.equal(current.webdav.configured, true)
})

await regression('the latest config read wins without changing the configuration generation', async current => {
  const old = delayInvoke('get_github_sync_config')
  const pendingOld = current.loadConfigs()
  const fresh = delayInvoke('get_github_sync_config')
  const pendingFresh = current.loadConfigs()
  fresh.resolve({ configured: true, owner: 'fresh', repo: 'fresh', autoSync: false })
  await pendingFresh
  old.resolve({ configured: true, owner: 'old', repo: 'old', autoSync: true })
  await pendingOld
  assert.equal(current.github.repo, 'fresh')
  assert.equal(current.github.autoSync, false)
})

await regression('preferences remain editable while config reads are pending', async current => {
  const read = delayInvoke('get_github_sync_config')
  const preferences = delayInvoke('get_sync_preferences')
  const loading = current.loadConfigs()
  const before = calls.length
  current.syncFrequency = 'every_15_minutes'
  await vue.nextTick()
  const preferenceSaved = calls.slice(before).some(call => call.command === 'update_sync_preferences'
    && call.args.historyUpdateMode === 'every_15_minutes')
  read.resolve({ configured: false, historyUpdateMode: 'immediate' })
  preferences.resolve({ historyUpdateMode: 'immediate' })
  await loading
  assert.ok(preferenceSaved)
  assert.equal(current.syncFrequency, 'every_15_minutes')
})

for (const provider of ['github', 'webdav']) {
  await regression(`${provider} stale sync response cannot refresh config after disconnect`, async current => {
    const command = provider === 'github' ? 'sync_github' : 'sync_webdav'
    const delayed = delayInvoke(command)
    const syncing = provider === 'github' ? current.syncGitHub() : current.syncWebDav()
    await (provider === 'github' ? current.disconnectGitHub() : current.disconnectWebDav())
    const before = calls.length
    delayed.resolve({ success: true, message: 'old completion' })
    await syncing
    assert.equal(current[provider].configured, false)
    assert.equal(current.lastResult, null)
    assert.equal(current.isSyncing, false)
    assert.ok(!calls.slice(before).some(call => call.command.startsWith('get_')))
  })

  await regression(`${provider} stale sync error cannot restore an upgrade challenge`, async current => {
    const delayed = delayInvoke(provider === 'github' ? 'sync_github' : 'sync_webdav')
    const syncing = provider === 'github' ? current.syncGitHub() : current.syncWebDav()
    await (provider === 'github' ? current.disconnectGitHub() : current.disconnectWebDav())
    delayed.reject(`SYNC_PROTOCOL_UPGRADE_REQUIRED:${JSON.stringify(challenges[provider])}`)
    await syncing
    assert.equal(current.pendingProtocolUpgrade, null)
    assert.equal(current.isSyncing, false)
  })
}

await regression('stale upgrade approval does not start sync for a disconnected provider', async current => {
  approved.delete('github')
  await current.syncGitHub()
  const delayed = delayInvoke('approve_sync_protocol_upgrade')
  const pending = current.approveProtocolUpgrade()
  await current.disconnectGitHub()
  const before = calls.length
  delayed.resolve()
  await pending
  assert.equal(current.pendingProtocolUpgrade, null)
  assert.equal(current.isSyncing, false)
  assert.ok(!calls.slice(before).some(call => call.command === 'sync_github'))
})

await regression('import invalidates old configuration responses for both providers', async current => {
  const gh = delayInvoke('configure_github_sync')
  const ghPending = current.configureGitHub('fixture', 'old')
  const wd = delayInvoke('configure_webdav_sync')
  const wdPending = current.configureWebDav('https://old.test', 'fixture', 'fixture')
  await current.importConfig()
  const imported = { github: { ...current.github }, webdav: { ...current.webdav } }
  gh.resolve({ owner: 'old', repo: 'old' })
  wd.resolve({})
  assert.equal(await ghPending, false)
  assert.equal(await wdPending, false)
  assert.deepEqual(current.github, imported.github)
  assert.deepEqual(current.webdav, imported.webdav)
})

await regression('a delayed import reload preserves a later provider configuration', async current => {
  const delayed = delayInvoke('import_config')
  const importing = current.importConfig()
  const gh = delayInvoke('configure_github_sync')
  const configuring = current.configureGitHub('fixture', 'fresh')
  gh.resolve({ owner: 'fresh', repo: 'fresh' })
  assert.equal(await configuring, true)
  delayed.resolve({ success: true, settings: {} })
  await importing
  assert.equal(current.github.repo, 'fresh')
  assert.equal(current.webdav.configured, true)
})

for (const [method, command, args] of [
  ['createGitHubRepo', 'create_github_repo', ['fresh']],
  ['useExistingGitHubRepo', 'use_existing_github_repo', ['fresh', 'fresh']],
  ['configureGitHub', 'configure_github_sync', ['fixture', 'fresh']],
]) {
  await regression(`${method} retains preferences edited during its request`, async current => {
    current.github.configured = true
    current.github.autoSync = true
    const delayed = delayInvoke(command)
    const configuring = current[method](...args)
    current.github.autoSync = false
    current.github.dataSaver = false
    current.github.silentFailures = true
    current.syncFrequency = 'every_30_minutes'
    await vue.nextTick()
    delayed.resolve({ owner: 'fresh', repo: 'fresh' })
    assert.equal(await configuring, true)
    assert.equal(current.github.autoSync, false)
    assert.equal(current.github.dataSaver, false)
    assert.equal(current.github.silentFailures, true)
    assert.equal(current.github.historyUpdateMode, 'every_30_minutes')
  })
}

await regression('WebDAV connection retains its auto-sync edit during the request', async current => {
  current.webdav.configured = true
  current.webdav.autoSync = true
  const delayed = delayInvoke('configure_webdav_sync')
  const configuring = current.configureWebDav('https://fresh.test', 'fixture', 'fixture')
  current.webdav.autoSync = false
  await vue.nextTick()
  delayed.resolve({})
  assert.equal(await configuring, true)
  assert.equal(current.webdav.autoSync, false)
})

await regression('config reads preserve provider preferences edited during the read', async current => {
  current.github.configured = true
  current.webdav.configured = true
  const read = delayInvoke('get_github_sync_config')
  const webdavRead = delayInvoke('get_webdav_sync_config')
  const loading = current.loadConfigs()
  current.github.autoSync = true
  current.github.dataSaver = false
  current.github.silentFailures = true
  current.webdav.autoSync = true
  await vue.nextTick()
  read.resolve({ configured: true, owner: 'fresh', repo: 'fresh', autoSync: false, dataSaver: true, silentFailures: false })
  webdavRead.resolve({ configured: true, autoSync: false })
  await loading
  assert.equal(current.github.autoSync, true)
  assert.equal(current.github.dataSaver, false)
  assert.equal(current.github.silentFailures, true)
  assert.equal(current.webdav.autoSync, true)
})

await regression('loaded backend preferences do not trigger saving them back', async current => {
  const gh = delayInvoke('get_github_sync_config')
  const wd = delayInvoke('get_webdav_sync_config')
  const preferences = delayInvoke('get_sync_preferences')
  const before = calls.length
  const loading = current.loadConfigs()
  gh.resolve({ configured: true, autoSync: true, dataSaver: false, silentFailures: true })
  wd.resolve({ configured: true, autoSync: true })
  preferences.resolve({ historyUpdateMode: 'every_10_minutes' })
  await loading
  await vue.nextTick()
  assert.equal(current.syncFrequency, 'every_10_minutes')
  assert.ok(!calls.slice(before).some(call => call.command.startsWith('update_')))
})

for (const provider of ['github', 'webdav']) {
  await regression(`${provider} disconnect during history await prevents history and UI commit`, async current => {
    const transfer = delayInvoke(provider === 'github' ? 'sync_github' : 'sync_webdav')
    const history = delayInvoke('history_apply')
    const beforeApplications = historyApplications.length
    const beforeCommits = historyCommits.length
    const pending = provider === 'github' ? current.syncGitHub() : current.syncWebDav()
    transfer.resolve({ success: true, message: 'old completion', history: { entries: [], deletions: [] } })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(historyApplications.length, beforeApplications + 1)
    await (provider === 'github' ? current.disconnectGitHub() : current.disconnectWebDav())
    const before = calls.length
    history.resolve()
    await pending
    assert.equal(historyCommits.length, beforeCommits)
    assert.equal(current[provider].configured, false)
    assert.equal(current.lastResult, null)
    assert.equal(current.isSyncing, false)
    assert.ok(!calls.slice(before).some(call => call.command.startsWith('get_')))
  })
}

await regression('validating a new GitHub identity refreshes its reset repository state', async current => {
  current.github = {
    configured: true, owner: 'old', repo: 'old', autoSync: true, lastSyncTime: 123,
    dataSaver: true, silentFailures: false, historyUpdateMode: 'immediate',
  }
  const validating = delayInvoke('validate_github_token')
  const configRead = delayInvoke('get_github_sync_config')
  const pending = current.validateGitHubToken('fixture')
  validating.resolve({ username: 'fresh' })
  configRead.resolve({ configured: false, owner: 'fresh', repo: '', autoSync: false, lastSyncTime: 0 })
  assert.equal(await pending, 'fresh')
  assert.equal(current.github.configured, false)
  assert.equal(current.github.owner, 'fresh')
  assert.equal(current.github.repo, '')
  assert.equal(current.github.autoSync, false)
  assert.equal(current.github.lastSyncTime, 0)
})

await regression('token validation delayed in config read cannot advance a newer setup', async current => {
  const validating = delayInvoke('validate_github_token')
  const configRead = delayInvoke('get_github_sync_config')
  const pending = current.validateGitHubToken('fixture')
  validating.resolve({ username: 'old' })
  await new Promise(resolve => setImmediate(resolve))
  const configuring = delayInvoke('configure_github_sync')
  const newPending = current.configureGitHub('fixture', 'fresh')
  configuring.resolve({ owner: 'fresh', repo: 'fresh' })
  assert.equal(await newPending, true)
  configRead.resolve({ configured: false, owner: 'old', repo: '', autoSync: false })
  assert.equal(await pending, null)
  assert.equal(current.github.configured, true)
  assert.equal(current.github.repo, 'fresh')
})

for (const provider of ['github', 'webdav']) {
  await regression(`${provider} connection preserves a user toggle back to its starting value`, async current => {
    current[provider].configured = true
    if (provider === 'github') {
      current.github.dataSaver = false
      current.github.silentFailures = true
    }
    const delayed = delayInvoke(provider === 'github' ? 'configure_github_sync' : 'configure_webdav_sync')
    const pending = provider === 'github'
      ? current.configureGitHub('fixture', 'fresh')
      : current.configureWebDav('https://fresh.test', 'fixture', 'fixture')
    current[provider].autoSync = true
    current[provider].autoSync = false
    if (provider === 'github') {
      current.github.dataSaver = true
      current.github.dataSaver = false
      current.github.silentFailures = false
      current.github.silentFailures = true
    }
    delayed.resolve({ owner: 'fresh', repo: 'fresh' })
    assert.equal(await pending, true)
    assert.equal(current[provider].autoSync, false)
    if (provider === 'github') {
      assert.equal(current.github.dataSaver, false)
      assert.equal(current.github.silentFailures, true)
    }
  })

  await regression(`${provider} config read preserves a user toggle back to its starting value`, async current => {
    current[provider].configured = true
    if (provider === 'github') {
      current.github.dataSaver = false
      current.github.silentFailures = true
    }
    const gh = delayInvoke('get_github_sync_config')
    const wd = delayInvoke('get_webdav_sync_config')
    const pending = current.loadConfigs()
    current[provider].autoSync = true
    current[provider].autoSync = false
    if (provider === 'github') {
      current.github.dataSaver = true
      current.github.dataSaver = false
      current.github.silentFailures = false
      current.github.silentFailures = true
    }
    gh.resolve({ configured: true, autoSync: true, dataSaver: true, silentFailures: false })
    wd.resolve({ configured: true, autoSync: true })
    await pending
    assert.equal(current[provider].autoSync, false)
    if (provider === 'github') {
      assert.equal(current.github.dataSaver, false)
      assert.equal(current.github.silentFailures, true)
    }
  })
}

await regression('an older import reload cannot supersede a newer token config read', async current => {
  const delayedImport = delayInvoke('import_config')
  const importing = current.importConfig()
  const validation = delayInvoke('validate_github_token')
  const currentRead = delayInvoke('get_github_sync_config')
  const validating = current.validateGitHubToken('fixture')
  validation.resolve({ username: 'fresh' })
  await new Promise(resolve => setImmediate(resolve))
  delayedImport.resolve({ success: true, settings: {} })
  await importing
  currentRead.resolve({ configured: false, owner: 'fresh', repo: '', autoSync: false })
  assert.equal(await validating, 'fresh')
  assert.equal(current.github.owner, 'fresh')
  assert.equal(current.github.configured, false)
})

await regression('a silent-only edit does not mark untouched connection defaults as edited', async current => {
  current.github.configured = true
  const delayed = delayInvoke('configure_github_sync')
  const pending = current.configureGitHub('fixture', 'fresh')
  const before = calls.length
  current.github.silentFailures = true
  await vue.nextTick()
  const updates = calls.slice(before).filter(call => call.command === 'update_github_sync_settings')
  assert.equal(updates.length, 1)
  const update = updates[0]
  delayed.resolve({ owner: 'fresh', repo: 'fresh' })
  assert.equal(await pending, true)
  assert.deepEqual(update.args, { silentFailures: true })
  assert.equal(current.github.autoSync, true)
  assert.equal(current.github.dataSaver, true)
  assert.equal(current.github.silentFailures, true)
})

console.log(`test-sync-protocol-upgrade: existing approval checks and ${cases - failures.length}/${cases} delayed-response regressions passed`)
assert.equal(failures.length, 0, `${failures.length} delayed-response regression(s) failed`)
