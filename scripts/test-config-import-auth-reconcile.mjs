import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const moduleUrl = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`

function compile(source) {
  return ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
}

async function compileSyncStore() {
  const piniaUrl = moduleUrl(`
    export const defineStore = (_id, setup) => {
      let instance
      return () => (instance ??= setup())
    }
  `)
  const vueUrl = moduleUrl(`
    export const ref = value => ({ value })
    export const watch = () => {}
  `)
  const tauriUrl = moduleUrl(`
    export const invoke = (...args) => globalThis.__configImportAuth.invoke(...args)
  `)
  const toastUrl = moduleUrl(`
    export const useToastStore = () => globalThis.__configImportAuth.toast
  `)
  const historyUrl = moduleUrl(`
    export const useHistoryStore = () => globalThis.__configImportAuth.history
  `)
  const settingsUrl = moduleUrl(`
    export const useSettingsStore = () => globalThis.__configImportAuth.settings
  `)
  const authUrl = moduleUrl(`
    export const useAuthStore = () => globalThis.__configImportAuth.auth
  `)
  const i18nUrl = moduleUrl(`
    export default { global: { t: key => key } }
    export const setLocale = () => {}
  `)
  const loggerUrl = moduleUrl(`
    export const createLogger = () => ({ error() {}, warn() {}, info() {} })
  `)

  const source = await readFile(new URL('src/stores/sync.ts', root), 'utf8')
  const rewritten = source
    .replace(/from ['"]pinia['"]/, `from ${JSON.stringify(piniaUrl)}`)
    .replace(/from ['"]vue['"]/, `from ${JSON.stringify(vueUrl)}`)
    .replace(/from ['"]@tauri-apps\/api\/core['"]/, `from ${JSON.stringify(tauriUrl)}`)
    .replace(/from ['"]\.\/toast['"]/, `from ${JSON.stringify(toastUrl)}`)
    .replace(/from ['"]\.\/history['"]/, `from ${JSON.stringify(historyUrl)}`)
    .replace(/from ['"]\.\/settings['"]/, `from ${JSON.stringify(settingsUrl)}`)
    .replace(/from ['"]\.\/auth['"]/, `from ${JSON.stringify(authUrl)}`)
    .replace(/from ['"]@\/i18n['"]/g, `from ${JSON.stringify(i18nUrl)}`)
    .replace(/from ['"]@\/utils\/logger['"]/, `from ${JSON.stringify(loggerUrl)}`)

  return import(moduleUrl(compile(rewritten)))
}

const effects = {
  commands: [],
  authReconciliations: 0,
  errors: [],
}

globalThis.__configImportAuth = {
  invoke: async command => {
    effects.commands.push(command)
    if (command === 'import_config') {
      throw new Error('cookie cleaner failed after auth commit')
    }
    throw new Error(`unexpected command: ${command}`)
  },
  toast: {
    success() {},
    error: message => effects.errors.push(message),
  },
  history: {},
  settings: {
    snapshot: () => ({}),
    applySnapshot() {},
  },
  auth: {
    async reconcileStatus() {
      effects.authReconciliations += 1
    },
  },
}

const { useSyncStore } = await compileSyncStore()
const result = await useSyncStore().importConfig()

assert.deepEqual(result, { success: false })
assert.deepEqual(effects.commands, ['import_config'])
assert.equal(effects.errors.length, 1, 'the original import failure must remain visible')
assert.equal(
  effects.authReconciliations,
  1,
  'a rejected config import may have committed auth and must force a fresh backend status',
)

console.log('config import auth reconciliation tests passed')
