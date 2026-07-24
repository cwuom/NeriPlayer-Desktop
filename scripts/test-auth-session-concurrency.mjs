import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const moduleUrl = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`

async function read(relativePath) {
  return readFile(new URL(relativePath, root), 'utf8')
}

function compile(source) {
  return ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
}

async function compileAuthStore() {
  const [authSource, authStatusRequestSource] = await Promise.all([
    read('src/stores/auth.ts'),
    read('src/modules/auth/authStatusRequest.ts'),
  ])
  const authStatusRequestUrl = moduleUrl(compile(authStatusRequestSource))

  let authMutationRequestUrl = moduleUrl(`
    export class AuthMutationRequestCoordinator {
      run() { throw new Error('auth mutation coordinator is missing') }
    }
  `)
  try {
    authMutationRequestUrl = moduleUrl(compile(await read('src/modules/auth/authMutationRequest.ts')))
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error
  }

  const piniaUrl = moduleUrl('export const defineStore = (_id, setup) => setup')
  const vueUrl = moduleUrl(`
    export const ref = value => ({ value })
    export const computed = getter => ({ get value() { return getter() } })
  `)
  const tauriUrl = moduleUrl(`
    export const invoke = (...args) => globalThis.__authSessionConcurrency.invoke(...args)
  `)
  const toastUrl = moduleUrl(`
    export const useToastStore = () => globalThis.__authSessionConcurrency.toast
  `)
  const recommendUrl = moduleUrl(`
    export const useRecommendStore = () => globalThis.__authSessionConcurrency.recommend
  `)
  const likedSongsUrl = moduleUrl(`
    export const useLikedSongsStore = () => globalThis.__authSessionConcurrency.likedSongs
  `)
  const i18nUrl = moduleUrl(`
    export default { global: { t: (key, values) => values ? key + ':' + JSON.stringify(values) : key } }
  `)
  const loggerUrl = moduleUrl(`
    export const createLogger = () => ({
      error: (...args) => globalThis.__authSessionConcurrency.logger.error(...args),
      warn: (...args) => globalThis.__authSessionConcurrency.logger.warn(...args),
    })
  `)

  const rewritten = authSource
    .replace(/from ['"]pinia['"]/, `from ${JSON.stringify(piniaUrl)}`)
    .replace(/from ['"]vue['"]/, `from ${JSON.stringify(vueUrl)}`)
    .replace(/from ['"]@tauri-apps\/api\/core['"]/, `from ${JSON.stringify(tauriUrl)}`)
    .replace(/from ['"]\.\/toast['"]/, `from ${JSON.stringify(toastUrl)}`)
    .replace(/from ['"]\.\/recommend['"]/, `from ${JSON.stringify(recommendUrl)}`)
    .replace(/from ['"]\.\/likedSongs['"]/, `from ${JSON.stringify(likedSongsUrl)}`)
    .replace(/from ['"]@\/i18n['"]/, `from ${JSON.stringify(i18nUrl)}`)
    .replace(/from ['"]@\/utils\/logger['"]/, `from ${JSON.stringify(loggerUrl)}`)
    .replace(/from ['"]@\/modules\/auth\/authStatusRequest['"]/, `from ${JSON.stringify(authStatusRequestUrl)}`)
    .replace(/from ['"]@\/modules\/auth\/authMutationRequest['"]/, `from ${JSON.stringify(authMutationRequestUrl)}`)

  return import(`${moduleUrl(compile(rewritten))}#auth-store`)
}

function deferred() {
  let resolve
  let reject
  const promise = new Promise((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

async function flushMicrotasks(count = 8) {
  for (let index = 0; index < count; index += 1) await Promise.resolve()
}

function account(nickname, accountId = nickname) {
  return {
    platform: 'netease',
    logged_in: true,
    nickname,
    avatar_url: `${nickname}-avatar`,
    account_id: accountId,
  }
}

function authStatus(nickname, accountId = nickname) {
  return {
    netease: account(nickname, accountId),
    bilibili: { platform: 'bilibili', logged_in: false, nickname: null, avatar_url: null },
    youtube: { platform: 'youtube', logged_in: false, nickname: null, avatar_url: null },
  }
}

function loggedOutAuthStatus() {
  return {
    netease: { platform: 'netease', logged_in: false, nickname: null, avatar_url: null },
    bilibili: { platform: 'bilibili', logged_in: false, nickname: null, avatar_url: null },
    youtube: { platform: 'youtube', logged_in: false, nickname: null, avatar_url: null },
  }
}

function installHarness(invoke) {
  const effects = {
    cacheClears: [],
    cloudLikesCleared: 0,
    likedPlaylistsLoaded: 0,
    cloudLikesRefreshed: 0,
    successes: [],
    errors: [],
    infos: [],
    logs: [],
  }
  globalThis.__authSessionConcurrency = {
    invoke,
    toast: {
      success: message => effects.successes.push(message),
      error: message => effects.errors.push(message),
      show: (message, kind) => effects.infos.push({ message, kind }),
    },
    recommend: {
      clearPlatformCache: platform => effects.cacheClears.push(platform),
    },
    likedSongs: {
      clearCloudLikes: () => { effects.cloudLikesCleared += 1 },
      loadLikedPlaylist: () => {
        effects.likedPlaylistsLoaded += 1
        return Promise.resolve()
      },
      refreshCloudLikes: () => {
        effects.cloudLikesRefreshed += 1
        return Promise.resolve()
      },
    },
    logger: {
      error: (...args) => effects.logs.push(['error', ...args]),
      warn: (...args) => effects.logs.push(['warn', ...args]),
    },
  }
  return effects
}

async function run(name, test) {
  try {
    await test()
    console.log(`ok - ${name}`)
  } catch (error) {
    console.error(`not ok - ${name}`)
    console.error(error)
    process.exitCode = 1
  }
}

const { useAuthStore } = await compileAuthStore()

await run('coalesced status callers publish one session boundary', async () => {
  const pending = deferred()
  let statusInvocations = 0
  const effects = installHarness(command => {
    assert.equal(command, 'check_auth_status')
    statusInvocations += 1
    return pending.promise
  })
  const store = useAuthStore()

  const first = store.checkStatus()
  const second = store.checkStatus()
  await flushMicrotasks()
  assert.equal(statusInvocations, 1, 'coalesced checks must use one backend request')
  assert.equal(store.canMutateNetease.value, false, 'Netease mutations must remain gated while startup auth is pending')

  pending.resolve(authStatus('restored-account'))
  await Promise.all([first, second])

  assert.equal(store.neteaseSessionVersion.value, 1, 'one status response is one session boundary')
  assert.equal(store.netease.value.accountId, 'restored-account')
  assert.equal(store.canMutateNetease.value, true)
  assert.deepEqual(effects.cacheClears, ['netease'])
  assert.equal(effects.cloudLikesCleared, 1)
  assert.equal(effects.likedPlaylistsLoaded, 0)
  assert.equal(effects.cloudLikesRefreshed, 1)
})

await run('unchanged Netease status does not publish a second session boundary', async () => {
  let statusInvocations = 0
  const effects = installHarness(command => {
    assert.equal(command, 'check_auth_status')
    statusInvocations += 1
    return Promise.resolve(authStatus('same-account', 'same-account-id'))
  })
  const store = useAuthStore()

  await store.checkStatus()
  await store.checkStatus()

  assert.equal(statusInvocations, 2)
  assert.equal(store.neteaseSessionVersion.value, 1)
  assert.deepEqual(effects.cacheClears, ['netease'])
  assert.equal(effects.cloudLikesCleared, 1)
  assert.equal(effects.likedPlaylistsLoaded, 0)
  assert.equal(effects.cloudLikesRefreshed, 1)
})

await run('forced status reconciliation bypasses an older in-flight snapshot', async () => {
  const staleStatus = deferred()
  const importedStatus = deferred()
  let statusInvocations = 0
  installHarness(command => {
    assert.equal(command, 'check_auth_status')
    statusInvocations += 1
    return statusInvocations === 1 ? staleStatus.promise : importedStatus.promise
  })
  const store = useAuthStore()

  const staleRequest = store.checkStatus()
  await flushMicrotasks()
  const reconciliation = store.reconcileStatus()
  await flushMicrotasks()

  assert.equal(statusInvocations, 2, 'forced reconciliation must start a fresh backend request')

  importedStatus.resolve(authStatus('imported-account', 'imported-account-id'))
  await reconciliation
  staleStatus.resolve(authStatus('old-account', 'old-account-id'))
  await staleRequest

  assert.equal(store.netease.value.accountId, 'imported-account-id')
  assert.equal(store.neteaseSessionVersion.value, 1)
})

await run('failed initial Netease status clears unverified personalized cache before retry', async () => {
  let statusInvocations = 0
  const effects = installHarness(command => {
    assert.equal(command, 'check_auth_status')
    statusInvocations += 1
    return Promise.reject(new Error('status offline'))
  })
  const store = useAuthStore()

  await store.checkStatus()
  await store.checkStatus()

  assert.equal(statusInvocations, 2)
  assert.deepEqual(effects.cacheClears, ['netease', 'netease'])
  assert.equal(effects.cloudLikesCleared, 2)
  assert.equal(effects.likedPlaylistsLoaded, 0)
  assert.equal(effects.cloudLikesRefreshed, 0, 'failed auth verification must not refresh cloud likes')
  assert.equal(store.neteaseSessionVersion.value, 0)
  assert.equal(store.netease.value.loggedIn, false)
  assert.equal(store.canMutateNetease.value, false)
})

await run('a legacy auth response without an account id maps to null', async () => {
  const status = authStatus('legacy-account')
  delete status.netease.account_id
  installHarness(command => {
    assert.equal(command, 'check_auth_status')
    return Promise.resolve(status)
  })
  const store = useAuthStore()

  await store.checkStatus()

  assert.equal(store.netease.value.accountId, null)
})

await run('a successful cross-platform login invalidates an older all-platform status response', async () => {
  const status = deferred()
  const bilibiliLogin = deferred()
  const commands = []
  installHarness((command, args) => {
    commands.push({ command, args })
    if (command === 'check_auth_status') return status.promise
    if (command === 'login_bilibili') return bilibiliLogin.promise
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  const statusRequest = store.checkStatus()
  await flushMicrotasks()
  const loginRequest = store.loginBilibili()
  await flushMicrotasks()

  bilibiliLogin.resolve({
    platform: 'bilibili',
    logged_in: true,
    nickname: 'b-account',
    avatar_url: 'b-avatar',
  })
  await loginRequest
  assert.equal(store.bilibili.value.loggedIn, true)

  status.resolve(loggedOutAuthStatus())
  await statusRequest

  assert.deepEqual(commands.map(call => call.command), ['check_auth_status', 'login_bilibili'])
  assert.deepEqual(store.bilibili.value, {
    loggedIn: true,
    nickname: 'b-account',
    avatarUrl: 'b-avatar',
    accountId: null,
  }, 'an older global status response must not overwrite a newer platform mutation')
})

await run('same-platform popup and cookie logins serialize and the newest call wins', async () => {
  const popup = deferred()
  const cookies = deferred()
  const commands = []
  let popupSettled = false
  let cookieStartedBeforePopupSettled = false
  const effects = installHarness((command, args) => {
    commands.push({ command, args })
    if (command === 'login_netease') return popup.promise
    if (command === 'login_with_cookies') {
      cookieStartedBeforePopupSettled = !popupSettled
      return cookies.promise
    }
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  const popupLogin = store.loginNetease()
  await flushMicrotasks()
  const cookieLogin = store.loginWithCookies('netease', 'MUSIC_U=new')
  await flushMicrotasks()

  if (cookieStartedBeforePopupSettled) {
    cookies.resolve(account('new-cookie-account'))
    await cookieLogin
    popupSettled = true
    popup.resolve(account('old-popup-account'))
    await popupLogin
  } else {
    popupSettled = true
    popup.resolve(account('old-popup-account'))
    await popupLogin
    await flushMicrotasks()
    cookies.resolve(account('new-cookie-account'))
    await cookieLogin
  }

  assert.equal(cookieStartedBeforePopupSettled, false, 'same-platform backend mutations must not overlap')
  assert.deepEqual(commands.map(call => call.command), ['login_netease', 'login_with_cookies'])
  assert.equal(store.netease.value.nickname, 'new-cookie-account')
  assert.equal(store.netease.value.accountId, 'new-cookie-account')
  assert.equal(store.neteaseSessionVersion.value, 1, 'the stale popup result must not publish a session')
  assert.deepEqual(effects.cacheClears, ['netease'])
  assert.equal(effects.cloudLikesCleared, 1)
  assert.equal(effects.likedPlaylistsLoaded, 0)
  assert.equal(effects.cloudLikesRefreshed, 1)
  assert.equal(effects.successes.length, 1)
})

await run('a stale login failure does not notify and its queued successor still runs', async () => {
  const popup = deferred()
  const cookies = deferred()
  let cookieStarted = false
  let statusInvocations = 0
  const effects = installHarness(command => {
    if (command === 'login_netease') return popup.promise
    if (command === 'login_with_cookies') {
      cookieStarted = true
      return cookies.promise
    }
    if (command === 'check_auth_status') {
      statusInvocations += 1
      return Promise.resolve(authStatus('unexpected-reconciliation'))
    }
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  const popupLogin = store.loginNetease()
  await flushMicrotasks()
  const cookieLogin = store.loginWithCookies('netease', 'MUSIC_U=new')
  await flushMicrotasks()
  popup.reject(new Error('popup failed'))
  await popupLogin
  await flushMicrotasks()
  assert.equal(cookieStarted, true, 'a rejected queue item must release the next mutation')
  cookies.resolve(account('new-cookie-account'))
  await cookieLogin

  assert.deepEqual(effects.errors, [], 'a superseded failure must not show an error toast')
  assert.deepEqual(effects.infos, [], 'a superseded cancellation must not show an info toast')
  assert.equal(statusInvocations, 0, 'a superseded failure must not reconcile backend status')
  assert.equal(store.netease.value.nickname, 'new-cookie-account')
})

await run('a current cookie failure reconciles an older successful popup backend mutation', async () => {
  const popup = deferred()
  const cookies = deferred()
  const commands = []
  let cookieFailed = false
  let statusInvocations = 0
  let statusObservedReportedFailure = false
  let effects
  effects = installHarness((command, args) => {
    commands.push({ command, args })
    if (command === 'login_netease') return popup.promise
    if (command === 'login_with_cookies') return cookies.promise
    if (command === 'check_auth_status') {
      statusInvocations += 1
      statusObservedReportedFailure = cookieFailed && effects.errors.length === 1 && effects.logs.length === 1
      return Promise.resolve(authStatus('backend-popup-account'))
    }
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  const popupLogin = store.loginNetease()
  await flushMicrotasks()
  const cookieLogin = store.loginWithCookies('netease', 'MUSIC_U=invalid')
  await flushMicrotasks()

  popup.resolve(account('backend-popup-account'))
  await popupLogin
  await flushMicrotasks()
  assert.equal(store.netease.value.nickname, null, 'the stale popup result must not publish directly')

  cookieFailed = true
  cookies.reject(new Error('cookie login failed'))
  await cookieLogin

  assert.equal(statusInvocations, 1, 'the current failure must reconcile backend auth status once')
  assert.equal(statusObservedReportedFailure, true, 'reconciliation must start after reporting the original failure')
  assert.deepEqual(
    commands.map(call => call.command),
    ['login_netease', 'login_with_cookies', 'check_auth_status'],
  )
  assert.equal(store.netease.value.nickname, 'backend-popup-account')
  assert.equal(store.neteaseSessionVersion.value, 1)
  assert.equal(effects.errors.length, 1)
})

await run('failed mutation reconciles to same account and starts exactly one refresh cycle', async () => {
  const cookieLogin = deferred()
  let initializing = true
  let reconciliationInvocations = 0
  const effects = installHarness(command => {
    if (command === 'check_auth_status') {
      if (!initializing) reconciliationInvocations += 1
      return Promise.resolve(authStatus('same-account', 'same-account-id'))
    }
    if (command === 'login_with_cookies') return cookieLogin.promise
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  await store.checkStatus()
  initializing = false
  const baselineSessionVersion = store.neteaseSessionVersion.value
  const baselineCacheClears = effects.cacheClears.length
  const baselineCloudLikesCleared = effects.cloudLikesCleared
  const baselineCloudLikesRefreshed = effects.cloudLikesRefreshed

  const loginRequest = store.loginWithCookies('netease', 'MUSIC_U=invalid')
  await flushMicrotasks()
  cookieLogin.reject(new Error('cookie login failed'))
  await loginRequest

  assert.equal(reconciliationInvocations, 1, 'the failed mutation must reconcile status once')
  assert.equal(store.netease.value.accountId, 'same-account-id')
  assert.equal(store.canMutateNetease.value, true)
  assert.equal(effects.cacheClears.length, baselineCacheClears + 1)
  assert.equal(effects.cloudLikesCleared, baselineCloudLikesCleared + 1)
  assert.equal(store.neteaseSessionVersion.value, baselineSessionVersion + 1)
  assert.equal(effects.cloudLikesRefreshed, baselineCloudLikesRefreshed + 1)
})

await run('a current popup failure reconciles an older successful logout backend mutation', async () => {
  const logout = deferred()
  const popup = deferred()
  const commands = []
  let initializing = true
  let popupFailed = false
  let reconciliationInvocations = 0
  let statusObservedReportedFailure = false
  let effects
  effects = installHarness((command, args) => {
    commands.push({ command, args })
    if (command === 'check_auth_status') {
      if (initializing) return Promise.resolve(authStatus('initial-account'))
      reconciliationInvocations += 1
      statusObservedReportedFailure = popupFailed && effects.errors.length === 1 && effects.logs.length === 1
      return Promise.resolve(loggedOutAuthStatus())
    }
    if (command === 'logout') return logout.promise
    if (command === 'login_netease') return popup.promise
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()
  await store.checkStatus()
  initializing = false
  commands.length = 0
  assert.equal(store.netease.value.nickname, 'initial-account')

  const logoutRequest = store.logout('netease')
  await flushMicrotasks()
  const popupLogin = store.loginNetease()
  await flushMicrotasks()

  logout.resolve()
  await logoutRequest
  await flushMicrotasks()
  assert.equal(store.netease.value.nickname, 'initial-account', 'the stale logout must not publish directly')

  popupFailed = true
  popup.reject(new Error('popup login failed'))
  await popupLogin

  assert.equal(reconciliationInvocations, 1, 'the current failure must reconcile backend auth status once')
  assert.equal(statusObservedReportedFailure, true, 'reconciliation must start after reporting the original failure')
  assert.deepEqual(commands.map(call => call.command), ['logout', 'login_netease', 'check_auth_status'])
  assert.deepEqual(store.netease.value, { loggedIn: false, nickname: null, avatarUrl: null, accountId: null })
  assert.equal(effects.successes.length, 0, 'the stale logout must not show a success toast')
  assert.equal(effects.errors.length, 1)
})

await run('a current logout failure reconciles backend auth status', async () => {
  const logout = deferred()
  let initializing = true
  let logoutFailed = false
  let reconciliationInvocations = 0
  let statusObservedLoggedFailure = false
  let effects
  effects = installHarness(command => {
    if (command === 'check_auth_status') {
      if (initializing) return Promise.resolve(authStatus('initial-account'))
      reconciliationInvocations += 1
      statusObservedLoggedFailure = logoutFailed && effects.logs.length === 1
      return Promise.resolve(loggedOutAuthStatus())
    }
    if (command === 'logout') return logout.promise
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()
  await store.checkStatus()
  initializing = false
  assert.equal(store.netease.value.nickname, 'initial-account')
  assert.equal(store.canMutateNetease.value, true)

  const logoutRequest = store.logout('netease')
  await flushMicrotasks()
  assert.equal(store.canMutateNetease.value, false, 'Netease mutations must close while logout is pending')
  logoutFailed = true
  logout.reject(new Error('logout failed after clearing backend state'))
  await logoutRequest

  assert.equal(reconciliationInvocations, 1)
  assert.equal(statusObservedLoggedFailure, true, 'logout reconciliation must start after logging the failure')
  assert.deepEqual(store.netease.value, { loggedIn: false, nickname: null, avatarUrl: null, accountId: null })
  assert.equal(store.canMutateNetease.value, false)
})

await run('a queued logout supersedes an older same-platform login', async () => {
  const popup = deferred()
  const logout = deferred()
  let popupSettled = false
  let logoutStartedBeforePopupSettled = false
  const commands = []
  installHarness((command, args) => {
    commands.push({ command, args })
    if (command === 'login_netease') return popup.promise
    if (command === 'logout') {
      logoutStartedBeforePopupSettled = !popupSettled
      return logout.promise
    }
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  const popupLogin = store.loginNetease()
  await flushMicrotasks()
  const logoutRequest = store.logout('netease')
  await flushMicrotasks()

  if (logoutStartedBeforePopupSettled) {
    logout.resolve()
    await logoutRequest
    popupSettled = true
    popup.resolve(account('old-popup-account'))
    await popupLogin
  } else {
    popupSettled = true
    popup.resolve(account('old-popup-account'))
    await popupLogin
    await flushMicrotasks()
    logout.resolve()
    await logoutRequest
  }

  assert.equal(logoutStartedBeforePopupSettled, false, 'logout must queue behind an active mutation')
  assert.deepEqual(commands.map(call => call.command), ['login_netease', 'logout'])
  assert.deepEqual(store.netease.value, { loggedIn: false, nickname: null, avatarUrl: null, accountId: null })
})

await run('finishing one platform login preserves another active logging indicator', async () => {
  const netease = deferred()
  const bilibili = deferred()
  installHarness(command => {
    if (command === 'login_netease') return netease.promise
    if (command === 'login_bilibili') return bilibili.promise
    throw new Error(`unexpected command: ${command}`)
  })
  const store = useAuthStore()

  const first = store.loginNetease()
  await flushMicrotasks()
  const second = store.loginBilibili()
  await flushMicrotasks()
  assert.equal(store.loggingIn.value, 'bilibili')

  netease.resolve(account('netease-account'))
  await first
  assert.equal(store.loggingIn.value, 'bilibili', 'an older finally must preserve the active platform')

  bilibili.resolve({ platform: 'bilibili', logged_in: true, nickname: 'b-account', avatar_url: 'b-avatar' })
  await second
  assert.equal(store.loggingIn.value, null)
})

await run('mutation coordination serializes each platform and keeps the newest generation current', async () => {
  const source = await read('src/modules/auth/authMutationRequest.ts')
  const { AuthMutationRequestCoordinator } = await import(`${moduleUrl(compile(source))}#serialization`)
  const coordinator = new AuthMutationRequestCoordinator()
  const firstPending = deferred()
  const secondPending = deferred()
  const thirdPending = deferred()
  const starts = []

  const first = coordinator.run('netease', () => {
    starts.push('first')
    return firstPending.promise
  })
  const second = coordinator.run('netease', () => {
    starts.push('second')
    return secondPending.promise
  })
  await flushMicrotasks()
  assert.deepEqual(starts, ['first'])
  assert.equal(first.isCurrent(), false)
  assert.equal(second.isCurrent(), true)

  firstPending.resolve('old')
  assert.equal(await first.promise, 'old')
  await flushMicrotasks()
  assert.deepEqual(starts, ['first', 'second'])

  const third = coordinator.run('netease', () => {
    starts.push('third')
    return thirdPending.promise
  })
  await flushMicrotasks()
  assert.deepEqual(starts, ['first', 'second'], 'old tail cleanup must not release a newer queue tail')
  assert.equal(second.isCurrent(), false)
  assert.equal(third.isCurrent(), true)

  secondPending.resolve('second')
  assert.equal(await second.promise, 'second')
  await flushMicrotasks()
  assert.deepEqual(starts, ['first', 'second', 'third'])
  thirdPending.resolve('newest')
  assert.equal(await third.promise, 'newest')
})

await run('mutation coordination lets different platforms run independently', async () => {
  const source = await read('src/modules/auth/authMutationRequest.ts')
  const { AuthMutationRequestCoordinator } = await import(`${moduleUrl(compile(source))}#parallel`)
  const coordinator = new AuthMutationRequestCoordinator()
  const neteasePending = deferred()
  const bilibiliPending = deferred()
  const starts = []

  const netease = coordinator.run('netease', () => {
    starts.push('netease')
    return neteasePending.promise
  })
  const bilibili = coordinator.run('bilibili', () => {
    starts.push('bilibili')
    return bilibiliPending.promise
  })
  await flushMicrotasks()
  assert.deepEqual(starts, ['netease', 'bilibili'])
  assert.equal(netease.isCurrent(), true)
  assert.equal(bilibili.isCurrent(), true)

  bilibiliPending.resolve('b')
  neteasePending.resolve('n')
  assert.equal(await bilibili.promise, 'b')
  assert.equal(await netease.promise, 'n')
})

await run('mutation queue continues after synchronous throws and rejections', async () => {
  const source = await read('src/modules/auth/authMutationRequest.ts')
  const { AuthMutationRequestCoordinator } = await import(`${moduleUrl(compile(source))}#errors`)
  const coordinator = new AuthMutationRequestCoordinator()
  const starts = []
  const syncFailure = coordinator.run('netease', () => {
    starts.push('sync')
    throw new Error('synchronous failure')
  })
  const rejection = coordinator.run('netease', () => {
    starts.push('reject')
    return Promise.reject(new Error('asynchronous failure'))
  })
  const success = coordinator.run('netease', () => {
    starts.push('success')
    return Promise.resolve('continued')
  })

  await assert.rejects(syncFailure.promise, /synchronous failure/)
  await assert.rejects(rejection.promise, /asynchronous failure/)
  assert.equal(await success.promise, 'continued')
  assert.deepEqual(starts, ['sync', 'reject', 'success'])
  assert.equal(syncFailure.isCurrent(), false)
  assert.equal(rejection.isCurrent(), false)
  assert.equal(success.isCurrent(), true)
})

if (!process.exitCode) console.log('auth session concurrency tests passed')
