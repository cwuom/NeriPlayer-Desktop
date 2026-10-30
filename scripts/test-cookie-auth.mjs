import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as pinia from 'pinia'
import * as vue from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/auth.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const originalCustomEvent = Object.getOwnPropertyDescriptor(globalThis, 'CustomEvent')
let cases = 0

function runtime() {
  const requests = [], notifications = [], invalidated = [], events = []
  let resolved = { platform: 'netease', logged_in: true, nickname: 'Fixture', avatar_url: null }
  let rejected = null
  let cleared = 0
  globalThis.window = { dispatchEvent: event => events.push(event) }
  globalThis.CustomEvent = class {
    constructor(type, options) { this.type = type; this.detail = options.detail }
  }
  const deps = {
    pinia, vue,
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      requests.push({ command, args })
      if (rejected) throw rejected
      return resolved
    } },
    './toast': { useToastStore: () => ({
      success: value => notifications.push(['success', value]),
      error: value => notifications.push(['error', value]),
      show: value => notifications.push(['info', value]),
    }) },
    './recommend': { useRecommendStore: () => ({ invalidatePlatform: value => invalidated.push(value) }) },
    '@/i18n': { default: { global: { t: key => key } } },
    '@/utils/logger': { createLogger: () => ({ error() {}, warn() {} }) },
    '@/modules/playback/playbackSource': { playbackUrlResolver: { clear: () => cleared++ } },
    '@/modules/playback/playbackPrefetch': { playbackPrefetchManager: { clear: () => cleared++ } },
  }
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in deps, `Unexpected dependency: ${name}`)
    return deps[name]
  }, exports)
  const instance = pinia.createPinia()
  pinia.setActivePinia(instance)
  return {
    store: exports.useAuthStore(), requests, notifications, invalidated, events,
    resolve: value => { resolved = value },
    reject: value => { rejected = value },
    cleared: () => cleared,
    dispose: () => pinia.disposePinia(instance),
  }
}

async function regression(name, run) {
  const current = runtime()
  try {
    await run(current)
    cases++
    console.log(`ok - ${name}`)
  } finally { current.dispose() }
}

try {
  await regression('successful Cookie login returns true and refreshes account consumers', async current => {
    assert.equal(await current.store.loginWithCookies('netease', 'MUSIC_U=fixture'), true)
    assert.deepEqual(current.requests[0], {
      command: 'login_with_cookies', args: { platform: 'netease', rawCookies: 'MUSIC_U=fixture' },
    })
    assert.equal(current.store.netease.loggedIn, true)
    assert.equal(current.store.netease.nickname, 'Fixture')
    assert.deepEqual(current.invalidated, ['netease'])
    assert.equal(current.cleared(), 2)
    assert.equal(current.events[0].type, 'neri:auth-changed')
    assert.deepEqual(current.events[0].detail, { platform: 'netease' })
    assert.equal(current.store.loggingIn, null)
  })

  await regression('failed Cookie login returns false and preserves the existing account', async current => {
    current.store.bilibili = { loggedIn: true, nickname: 'Existing', avatarUrl: 'existing.png' }
    current.reject(new Error('Cookie validation failed'))
    assert.equal(await current.store.loginWithCookies('bilibili', 'SESSDATA=invalid'), false)
    assert.deepEqual(current.store.bilibili, { loggedIn: true, nickname: 'Existing', avatarUrl: 'existing.png' })
    assert.deepEqual(current.invalidated, [])
    assert.deepEqual(current.events, [])
    assert.equal(current.store.loggingIn, null)
  })

  await regression('Cookie login cannot report success for an unauthenticated response', async current => {
    current.store.netease = { loggedIn: true, nickname: 'Existing', avatarUrl: null }
    current.resolve({ logged_in: false })
    assert.equal(await current.store.loginWithCookies('netease', 'MUSIC_U=invalid'), false)
    assert.equal(current.store.netease.nickname, 'Existing')
    assert.deepEqual(current.events, [])
  })

  await regression('imported accounts can invalidate cached data even when summaries match', async current => {
    assert.equal(typeof current.store.notifyAuthChanged, 'function')
    current.store.notifyAuthChanged('bilibili')
    assert.deepEqual(current.invalidated, ['bilibili'])
    assert.deepEqual(current.events.map(event => event.detail), [{ platform: 'bilibili' }])
  })

  await regression('unknown Cookie login platform is rejected without IPC or account changes', async current => {
    assert.equal(await current.store.loginWithCookies('unknown', 'SID=fixture'), false)
    assert.deepEqual(current.requests, [])
    assert.equal(current.store.youtube.loggedIn, false)
  })

  console.log(`Cookie authentication: ${cases} cases passed`)
} finally {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow)
  else delete globalThis.window
  if (originalCustomEvent) Object.defineProperty(globalThis, 'CustomEvent', originalCustomEvent)
  else delete globalThis.CustomEvent
}
