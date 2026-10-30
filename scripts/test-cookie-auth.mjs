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
  const profileListeners = new Map()
  let subscriptions = 0
  let invokeHandler = null
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
      if (invokeHandler) return invokeHandler(command, args)
      if (rejected) throw rejected
      return resolved
    } },
    '@tauri-apps/api/event': { listen: async (name, callback) => {
      subscriptions++
      profileListeners.set(name, callback)
      return () => profileListeners.delete(name)
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
    handle: callback => { invokeHandler = callback },
    subscriptions: () => subscriptions,
    profileListeners,
    emitProfile: platform => profileListeners.get('auth-profile-updated')?.({ payload: platform }),
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

  await regression('an older Cookie login result cannot restore a newer logout', async current => {
    let finish
    const pending = new Promise(resolve => { finish = resolve })
    current.handle(command => command === 'login_with_cookies' ? pending : undefined)
    const login = current.store.loginWithCookies('netease', 'MUSIC_U=older')
    await current.store.logout('netease')
    finish({ logged_in: true, nickname: 'Older login' })
    assert.equal(await login, false)
    assert.equal(current.store.netease.loggedIn, false)
  })

  await regression('an older successful login cannot replace a newer login or clear its loading state', async current => {
    let finishOld, finishNew
    const old = new Promise(resolve => { finishOld = resolve })
    const latest = new Promise(resolve => { finishNew = resolve })
    current.handle((command, args) => args?.rawCookies === 'MUSIC_U=old' ? old : latest)
    const olderLogin = current.store.loginWithCookies('netease', 'MUSIC_U=old')
    const newerLogin = current.store.loginWithCookies('netease', 'MUSIC_U=new')
    finishOld({ logged_in: true, nickname: 'Old' })
    assert.equal(await olderLogin, false)
    assert.equal(current.store.loggingIn, 'netease')
    finishNew({ logged_in: true, nickname: 'New' })
    assert.equal(await newerLogin, true)
    assert.equal(current.store.netease.nickname, 'New')
    assert.equal(current.store.loggingIn, null)
  })

  await regression('a status response started before login cannot replace the new account summary', async current => {
    let finishStatus
    const pending = new Promise(resolve => { finishStatus = resolve })
    current.handle(command => command === 'check_auth_status' ? pending : { logged_in: true, nickname: 'New' })
    const checking = current.store.checkStatus()
    await Promise.resolve()
    await Promise.resolve()
    await current.store.loginWithCookies('netease', 'MUSIC_U=new')
    finishStatus({ netease: {}, bilibili: {}, youtube: {} })
    await checking
    assert.equal(current.store.netease.nickname, 'New')
  })

  await regression('an external import or debug clear rejects a login response already delayed by IPC', async current => {
    let finishLogin
    const pending = new Promise(resolve => { finishLogin = resolve })
    current.handle(command => command === 'login_with_cookies' ? pending : {
      netease: { logged_in: true, nickname: 'Imported account' }, bilibili: {}, youtube: {},
    })
    const login = current.store.loginWithCookies('netease', 'MUSIC_U=delayed-success')
    await current.store.checkStatus({ invalidatePending: ['netease'] })
    finishLogin({ logged_in: true, nickname: 'Delayed older login' })
    assert.equal(await login, false)
    assert.equal(current.store.netease.nickname, 'Imported account')
    assert.equal(current.store.loggingIn, null)
  })

  await regression('an external update for another platform preserves an unrelated pending login', async current => {
    let finishLogin
    const pending = new Promise(resolve => { finishLogin = resolve })
    current.handle(command => command === 'login_with_cookies' ? pending : { netease: {}, bilibili: {}, youtube: {} })
    const login = current.store.loginWithCookies('netease', 'MUSIC_U=pending')
    await current.store.checkStatus({ invalidatePending: ['bilibili'] })
    assert.equal(current.store.loggingIn, 'netease')
    finishLogin({ logged_in: true, nickname: 'Unrelated login' })
    assert.equal(await login, true)
    assert.equal(current.store.netease.nickname, 'Unrelated login')
  })

  for (const kind of ['cookie', 'web']) {
    await regression(`a status response started during a pending ${kind} login cannot replace its committed account`, async current => {
      let finishLogin, finishStatus
      const pendingLogin = new Promise(resolve => { finishLogin = resolve })
      const pendingStatus = new Promise(resolve => { finishStatus = resolve })
      current.handle(command => command === 'check_auth_status' ? pendingStatus : pendingLogin)
      const login = kind === 'cookie'
        ? current.store.loginWithCookies('netease', 'MUSIC_U=pending')
        : current.store.loginNetease()
      const checking = current.store.checkStatus()
      await Promise.resolve()
      await Promise.resolve()
      finishLogin({ logged_in: true, nickname: 'Committed account' })
      await login
      finishStatus({ netease: { logged_in: true, nickname: 'Old status' }, bilibili: {}, youtube: {} })
      await checking
      assert.equal(current.store.netease.nickname, 'Committed account')
    })
  }

  await regression('a status response started during pending logout cannot restore the cleared account', async current => {
    let finishLogout, finishStatus
    const pendingLogout = new Promise(resolve => { finishLogout = resolve })
    const pendingStatus = new Promise(resolve => { finishStatus = resolve })
    current.store.netease = { loggedIn: true, nickname: 'Old', avatarUrl: null }
    current.handle(command => command === 'check_auth_status' ? pendingStatus : pendingLogout)
    const logout = current.store.logout('netease')
    const checking = current.store.checkStatus()
    await Promise.resolve()
    await Promise.resolve()
    finishLogout()
    await logout
    finishStatus({ netease: { logged_in: true, nickname: 'Old' }, bilibili: {}, youtube: {} })
    await checking
    assert.equal(current.store.netease.loggedIn, false)
  })

  await regression('YouTube profile refresh uses the committed login revision', async current => {
    let finishProfile
    const pending = new Promise(resolve => { finishProfile = resolve })
    current.handle(command => command === 'refresh_youtube_profile' ? pending : { logged_in: true })
    assert.equal(await current.store.loginWithCookies('youtube', 'SAPISID=fixture'), true)
    finishProfile({ logged_in: true, nickname: 'YouTube profile', avatar_url: 'fixture.png' })
    await Promise.resolve()
    await Promise.resolve()
    await vue.nextTick()
    assert.equal(current.store.youtube.nickname, 'YouTube profile')
  })

  await regression('background ID completion refreshes consumers once without duplicate listeners', async current => {
    current.resolve({ netease: { logged_in: true }, bilibili: {}, youtube: {} })
    await current.store.checkStatus()
    await current.store.checkStatus()
    assert.equal(current.subscriptions(), 1)
    assert.equal(current.store.statusChecked, true)
    await current.emitProfile('netease')
    assert.deepEqual(current.invalidated, ['netease'], 'ID-only profile updates must reload libraries even when summaries match')
    assert.deepEqual(current.events.map(event => event.detail), [{ platform: 'netease' }])
    current.dispose()
    assert.equal(current.profileListeners.size, 0)
  })

  console.log(`Cookie authentication: ${cases} cases passed`)
} finally {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow)
  else delete globalThis.window
  if (originalCustomEvent) Object.defineProperty(globalThis, 'CustomEvent', originalCustomEvent)
  else delete globalThis.CustomEvent
}
