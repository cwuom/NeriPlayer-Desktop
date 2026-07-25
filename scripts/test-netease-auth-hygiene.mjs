import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const root = new URL('../', import.meta.url)

async function read(relativePath) {
  return readFile(new URL(relativePath, root), 'utf8')
}

async function compileRecommendStore() {
  const source = await read('src/stores/recommend.ts')

  const moduleUrl = value => `data:text/javascript;base64,${Buffer.from(value).toString('base64')}`
  const piniaUrl = moduleUrl('export const defineStore = (_id, setup) => setup')
  const vueUrl = moduleUrl(`export const ref = value => ({ value })`)
  const tauriUrl = moduleUrl('export const invoke = async () => ({})')
  const loggerUrl = moduleUrl(`export const createLogger = () => ({ error() {}, warn() {} })`)
  const parserUrl = moduleUrl(`export const parseYouTubeLibraryPlaylists = () => []`)

  const rewritten = source
    .replace(/from ['"]pinia['"]/, `from ${JSON.stringify(piniaUrl)}`)
    .replace(/from ['"]vue['"]/, `from ${JSON.stringify(vueUrl)}`)
    .replace(/from ['"]@tauri-apps\/api\/core['"]/, `from ${JSON.stringify(tauriUrl)}`)
    .replace(/from ['"]@\/utils\/logger['"]/, `from ${JSON.stringify(loggerUrl)}`)
    .replace(/from ['"]@\/modules\/youtube\/youtubePlaylistParse['"]/, `from ${JSON.stringify(parserUrl)}`)

  const compiled = ts.transpileModule(rewritten, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText

  return import(moduleUrl(compiled))
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

await run('Netease account validation is wired through the login path', async () => {
  const [client, authCommand, state] = await Promise.all([
    read('src-tauri/src/api/netease/client.rs'),
    read('src-tauri/src/commands/auth_cmd.rs'),
    read('src-tauri/src/auth/state.rs'),
  ])

  assert.match(client, /parse_netease_account_profile/)
  assert.match(authCommand, /parse_netease_account_profile/)
  assert.match(state, /user_id[\s\S]*has_login/)
})

await run('stable backend account ids reach every frontend auth response', async () => {
  const [authState, authCommand, authStore] = await Promise.all([
    read('src-tauri/src/auth/state.rs'),
    read('src-tauri/src/commands/auth_cmd.rs'),
    read('src/stores/auth.ts'),
  ])

  assert.match(authState, /pub\s+account_id:\s*Option<String>/)
  assert.match(authState, /account_id:\s*a\.user_id\.map\(\|id\|\s*id\.to_string\(\)\)/)
  assert.match(authState, /account_id:\s*a\.mid\.map\(\|id\|\s*id\.to_string\(\)\)/)

  const neteasePopup = authCommand.slice(
    authCommand.indexOf('pub async fn login_netease'),
    authCommand.indexOf('pub async fn login_bilibili'),
  )
  const cookieLogin = authCommand.slice(
    authCommand.indexOf('pub async fn login_with_cookies'),
    authCommand.indexOf('pub async fn refresh_youtube_profile'),
  )
  for (const source of [neteasePopup, cookieLogin]) {
    assert.match(source, /let\s+account_id\s*=\s*Some\(profile\.user_id\.to_string\(\)\)/)
    assert.match(source, /AuthInfo\s*\{[\s\S]*account_id/)
  }

  assert.match(authStore, /accountId:\s*string\s*\|\s*null/)
  assert.match(authStore, /accountId:\s*raw\?\.account_id\s*\?\?\s*null/)
})

await run('Netease candidate validation preserves domain cookies in an isolated Jar', async () => {
  const [client, authCommand, cookies, appState] = await Promise.all([
    read('src-tauri/src/api/netease/client.rs'),
    read('src-tauri/src/commands/auth_cmd.rs'),
    read('src-tauri/src/auth/cookies.rs'),
    read('src-tauri/src/state.rs'),
  ])

  assert.doesNotMatch(client, /get_user_account_with_cookie_header/)
  assert.doesNotMatch(client, /\.header\(\s*["']Cookie["']/)
  assert.doesNotMatch(authCommand, /fn build_cookie_header/)
  assert.doesNotMatch(client, /with_isolated_cookie_jar/)

  const isolatedClient = appState.slice(
    appState.indexOf('pub fn http_with_cookie_jar'),
    appState.indexOf('pub fn http(&self)'),
  )
  assert.match(isolatedClient, /Client::builder\(\)/)
  assert.match(isolatedClient, /\.cookie_provider\(cookie_jar\)/)
  assert.match(isolatedClient, /let\s+bypass_proxy\s*=\s*self\.http\.read\(\)\.bypass_proxy/)
  assert.match(isolatedClient, /if\s+bypass_proxy/)
  assert.match(isolatedClient, /builder\s*=\s*builder\.no_proxy\(\)/)

  const candidateValidation = authCommand.slice(
    authCommand.indexOf('async fn validate_netease_account'),
    authCommand.indexOf('#[cfg(test)]'),
  )
  assert.match(candidateValidation, /Arc::new\(reqwest::cookie::Jar::default\(\)\)/)
  assert.match(candidateValidation, /cookies::inject_cookies\(&candidate_jar, entries\)/)
  assert.match(candidateValidation, /state\.http_with_cookie_jar\(candidate_jar\)/)
  assert.match(candidateValidation, /NeteaseClient::new\(&http\)/)
  assert.match(candidateValidation, /client\.get_user_account\(\)\.await/)
  assert.doesNotMatch(candidateValidation, /state\.http\(\)/)

  const cookieInjection = cookies.slice(
    cookies.indexOf('pub fn inject_cookies'),
    cookies.indexOf('/// 登出时过期指定平台'),
  )
  assert.match(cookieInjection, /for entry in entries/)
  assert.match(cookieInjection, /Domain=\{\}; Path=\/[\s\S]*entry\.domain/)
  assert.match(authCommand, /entry\.name == name && entry\.domain == domain/)

  const popupLogin = authCommand.slice(
    authCommand.indexOf('pub async fn login_netease'),
    authCommand.indexOf('pub async fn login_bilibili'),
  )
  const cookieLogin = authCommand.slice(
    authCommand.indexOf('pub async fn login_with_cookies'),
    authCommand.indexOf('pub async fn refresh_youtube_profile'),
  )

  for (const source of [popupLogin, cookieLogin]) {
    const validation = source.indexOf('validate_netease_account(&state, &entries)')
    const injection = source.indexOf('cookies::inject_cookies')
    assert.ok(validation >= 0, 'Netease login must validate with an isolated candidate Jar')
    assert.ok(injection > validation, 'shared Jar must only receive validated Netease cookies')
  }
})

await run('failed Netease popup validation clears the stale browser session', async () => {
  const authCommand = await read('src-tauri/src/commands/auth_cmd.rs')
  const popupLogin = authCommand.slice(
    authCommand.indexOf('pub async fn login_netease'),
    authCommand.indexOf('pub async fn login_bilibili'),
  )

  const validation = popupLogin.indexOf('match validate_netease_account(&state, &entries).await')
  const clearPersistedAuth = popupLogin.indexOf('auth_state.netease = None')
  const expireJar = popupLogin.indexOf(
    'cookies::expire_platform_cookies(&state.cookie_jar, &previous_auth, "netease")',
  )
  const clearWebView = popupLogin.indexOf(
    'clear_and_reinject_webview_cookies(&app, &state).await',
  )
  const persist = popupLogin.lastIndexOf('commit_auth_update(&app, &state')
  const sharedJarInjection = popupLogin.indexOf(
    'cookies::inject_cookies(&state.cookie_jar, &auth.cookies)',
  )

  assert.ok(validation >= 0, 'popup login must handle candidate validation failures')
  assert.ok(clearPersistedAuth > validation, 'failed validation must remove stale persisted auth')
  assert.ok(expireJar > validation, 'failed validation must expire the stale shared Jar session')
  assert.ok(clearWebView > expireJar, 'failed validation must clear stale WebView cookies')
  assert.ok(persist > clearWebView, 'validated auth must persist before shared cookies are published')
  assert.ok(
    sharedJarInjection > persist,
    'candidate cookies must only reach the shared Jar after validation and persistence succeed',
  )
})

await run('HTTP client and bypass policy publish as one locked snapshot', async () => {
  const appState = await read('src-tauri/src/state.rs')
  const stateFields = appState.slice(
    appState.indexOf('pub struct AppState'),
    appState.indexOf('impl AppState'),
  )
  const constructor = appState.slice(
    appState.indexOf('pub fn new()'),
    appState.indexOf('pub fn rebuild_http'),
  )
  const rebuild = appState.slice(
    appState.indexOf('pub fn rebuild_http'),
    appState.indexOf('pub fn http_with_cookie_jar'),
  )
  const normalAccessor = appState.slice(appState.indexOf('pub fn http(&self)'))

  assert.match(
    appState,
    /struct\s+HttpClientSnapshot\s*\{[\s\S]*?client:\s*reqwest::Client,[\s\S]*?bypass_proxy:\s*bool,[\s\S]*?\}/,
  )
  assert.match(stateFields, /http:\s*parking_lot::RwLock<HttpClientSnapshot>/)
  assert.doesNotMatch(stateFields, /bypass_proxy:\s*AtomicBool/)
  assert.match(
    constructor,
    /http:\s*parking_lot::RwLock::new\(\s*HttpClientSnapshot\s*\{[\s\S]*?client:\s*http,[\s\S]*?bypass_proxy:\s*true,[\s\S]*?\}\s*\)/,
  )
  assert.match(constructor, /\.no_proxy\(\)[\s\S]*?\.build\(\)/)

  assert.match(
    rebuild,
    /if\s+let\s+Ok\(client\)\s*=\s*builder\.build\(\)\s*\{\s*\*self\.http\.write\(\)\s*=\s*HttpClientSnapshot\s*\{\s*client,\s*bypass_proxy,\s*\};\s*\}/,
  )
  assert.equal((rebuild.match(/self\.http\.write\(\)/g) ?? []).length, 1)
  assert.doesNotMatch(rebuild, /\.store\(|Ordering::/)
  assert.match(normalAccessor, /self\.http\.read\(\)\.client\.clone\(\)/)
})

await run('logout serializes shared cookie cleanup without stale auth reinjection', async () => {
  const [authCommand, appState] = await Promise.all([
    read('src-tauri/src/commands/auth_cmd.rs'),
    read('src-tauri/src/state.rs'),
  ])

  const logout = authCommand.slice(
    authCommand.indexOf('pub async fn logout'),
    authCommand.indexOf('async fn clear_and_reinject_webview_cookies'),
  )
  const cleaner = authCommand.slice(
    authCommand.indexOf('async fn clear_and_reinject_webview_cookies'),
  )
  const popupLogins = [
    authCommand.slice(
      authCommand.indexOf('pub async fn login_netease'),
      authCommand.indexOf('pub async fn login_bilibili'),
    ),
    authCommand.slice(
      authCommand.indexOf('pub async fn login_bilibili'),
      authCommand.indexOf('pub async fn login_youtube'),
    ),
    authCommand.slice(
      authCommand.indexOf('pub async fn login_youtube'),
      authCommand.indexOf('pub async fn login_with_cookies'),
    ),
  ]
  const cookieLogin = authCommand.slice(
    authCommand.indexOf('pub async fn login_with_cookies'),
    authCommand.indexOf('pub async fn refresh_youtube_profile'),
  )

  assert.match(appState, /pub auth_cookie_gate:\s*tokio::sync::Mutex<\(\)>/)
  assert.match(logout, /let _cookie_guard = state\.auth_cookie_gate\.lock\(\)\.await;/)
  assert.doesNotMatch(logout, /tokio::task::spawn/)
  assert.doesNotMatch(logout, /remaining_auth/)
  assert.match(logout, /clear_and_reinject_webview_cookies\(&app, &state\)\.await\?;/)
  assert.match(cleaner, /let current_auth = state\.auth\.lock\(\)\.clone\(\);/)
  assert.match(cleaner, /cookies::inject_all\(&state\.cookie_jar, &current_auth\);/)
  assert.doesNotMatch(cleaner, /let label = "cookie-cleaner";/)
  assert.match(cleaner, /format!\(\s*"cookie-cleaner-\{\}"/)

  for (const login of popupLogins) {
    const gate = login.indexOf('let _cookie_guard = state.auth_cookie_gate.lock().await;')
    const window = login.indexOf('WebviewWindowBuilder::new')
    assert.ok(gate >= 0, 'popup login must join the shared cookie mutation gate')
    assert.ok(gate < window, 'popup login must lock before touching WebView cookies')
  }

  const cookieGate = cookieLogin.indexOf('let _cookie_guard = state.auth_cookie_gate.lock().await;')
  const injection = cookieLogin.indexOf('cookies::inject_cookies')
  assert.ok(cookieGate >= 0, 'cookie login must join the shared cookie mutation gate')
  assert.ok(cookieGate < injection, 'cookie login must lock before touching the shared Jar')
})

await run('debug cookie cleanup shares the auth cookie gate and current-state cleaner', async () => {
  const authCommand = await read('src-tauri/src/commands/auth_cmd.rs')
  const debugClear = authCommand.slice(
    authCommand.indexOf('pub async fn clear_debug_cookie_storage'),
    authCommand.indexOf('pub async fn logout'),
  )

  assert.match(debugClear, /let _cookie_guard = state\.auth_cookie_gate\.lock\(\)\.await;/)
  assert.match(debugClear, /clear_and_reinject_webview_cookies\(&app, &state\)\.await/)
  assert.doesNotMatch(
    debugClear,
    /clear_and_reinject_webview_cookies\(\s*&app,\s*&state\.cookie_jar/,
  )

  const cleaner = authCommand.slice(
    authCommand.indexOf('async fn clear_and_reinject_webview_cookies'),
  )
  assert.doesNotMatch(cleaner, /let\s+_\s*=\s*window\.(?:clear_all_browsing_data|close)\(\)/)
  assert.match(cleaner, /let\s+clear_result\s*=\s*window\.clear_all_browsing_data\(\)/)
  assert.match(cleaner, /let\s+close_result\s*=\s*window\.close\(\)/)
  assert.match(cleaner, /match\s*\(clear_result,\s*close_result\)/)
})

await run('YouTube auth refresh writers join the shared auth cookie gate', async () => {
  const authCommand = await read('src-tauri/src/commands/auth_cmd.rs')
  const profileRefresh = authCommand.slice(
    authCommand.indexOf('pub async fn refresh_youtube_profile'),
    authCommand.indexOf('pub async fn maybe_refresh_youtube_session'),
  )
  const sessionRefresh = authCommand.slice(
    authCommand.indexOf('pub async fn maybe_refresh_youtube_session'),
    authCommand.indexOf('pub async fn check_auth_status'),
  )

  for (const [name, source] of [
    ['profile refresh', profileRefresh],
    ['session refresh', sessionRefresh],
  ]) {
    const gate = source.lastIndexOf('auth_cookie_gate.lock().await')
    const write = source.lastIndexOf('cookies::save_auth')
    assert.ok(gate >= 0, `${name} must acquire the shared auth cookie gate before committing`)
    assert.ok(gate < write, `${name} must acquire the gate before persisting auth`)
  }
})

await run('Netease download parsing rejects failed and preview-only responses', async () => {
  const client = await read('src-tauri/src/api/netease/client.rs')
  assert.match(client, /parse_download_url_response/)
  assert.match(client, /root_code[\s\S]*301[\s\S]*RequiresLogin/)
  assert.match(client, /data_code[\s\S]*404[\s\S]*NoPermission/)
  assert.match(client, /freeTrialInfo[\s\S]*is_preview[\s\S]*url: None/)
})

await run('Netease logout removes only Netease personalized cache', async () => {
  const { clearPlatformRecommendCache, useRecommendStore } = await compileRecommendStore()
  assert.equal(typeof clearPlatformRecommendCache, 'function')

  const cache = {
    recommendedPlaylists: [{ id: 1 }],
    userPlaylists: {
      netease: [{ id: 'old-account' }],
      bilibili: [{ id: 'b-account' }],
      youtube: [{ id: 'y-account' }],
    },
    homeHotSongs: { items: [{ id: 'hot-old' }], loading: false, error: null },
    homeRadarSongs: { items: [{ id: 'radar-old' }], loading: false, error: null },
    homeFeedShelves: [{ title: 'local' }],
    timestamp: 123,
  }

  const cleared = clearPlatformRecommendCache(cache, 'netease')
  assert.deepEqual(cleared.userPlaylists, {
    bilibili: [{ id: 'b-account' }],
    youtube: [{ id: 'y-account' }],
  })
  assert.deepEqual(cleared.recommendedPlaylists, [])
  assert.deepEqual(cleared.homeHotSongs, { items: [], loading: false, error: null })
  assert.deepEqual(cleared.homeRadarSongs, { items: [], loading: false, error: null })
  assert.deepEqual(cleared.homeFeedShelves, cache.homeFeedShelves)
  assert.deepEqual(cache.userPlaylists.netease, [{ id: 'old-account' }])

  const storage = new Map()
  globalThis.localStorage = {
    getItem: key => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
  }
  const store = useRecommendStore()
  store.recommendedPlaylists.value = cache.recommendedPlaylists
  store.recommendedSongs.value = [{ id: 'daily' }]
  store.userPlaylists.value = cache.userPlaylists
  store.userAlbums.value = [{ id: 'album' }]
  store.likedSongIds.value = new Set([42])
  store.homeHotSongs.value = cache.homeHotSongs
  store.homeRadarSongs.value = cache.homeRadarSongs
  store.homeFeedShelves.value = cache.homeFeedShelves

  store.clearPlatformCache('netease')

  assert.deepEqual(store.recommendedPlaylists.value, [])
  assert.deepEqual(store.recommendedSongs.value, [])
  assert.deepEqual(store.userAlbums.value, [])
  assert.deepEqual([...store.likedSongIds.value], [])
  assert.deepEqual(store.userPlaylists.value, cleared.userPlaylists)
  assert.deepEqual(store.homeHotSongs.value, { items: [], loading: false, error: null })
  assert.deepEqual(store.homeRadarSongs.value, { items: [], loading: false, error: null })
  assert.deepEqual(store.homeFeedShelves.value, cache.homeFeedShelves)

  const persisted = JSON.parse(storage.get('neri:recommend:cache'))
  assert.deepEqual(persisted.userPlaylists, cleared.userPlaylists)
  assert.deepEqual(persisted.recommendedPlaylists, [])
  assert.deepEqual(persisted.homeHotSongs, { items: [], loading: false, error: null })
  assert.deepEqual(persisted.homeRadarSongs, { items: [], loading: false, error: null })
})

await run('successful logout delegates cache cleanup to the recommend store', async () => {
  const authSource = await read('src/stores/auth.ts')
  assert.match(authSource, /useRecommendStore/)
  assert.match(authSource, /clearPlatformCache\(platform\)/)
})

await run('unverified and changed restored Netease sessions clear the previous account cache', async () => {
  const authSource = await read('src/stores/auth.ts')
  const checkStatus = authSource.slice(
    authSource.indexOf('async function checkStatus'),
    authSource.indexOf('function needsYoutubeProfileRefresh'),
  )
  assert.match(
    checkStatus,
    /const\s+needsNeteaseVerification\s*=\s*!hasVerifiedNeteaseAuth[\s\S]*if\s*\(needsNeteaseVerification\)\s*\{[\s\S]*clearPlatformCache\('netease'\)[\s\S]*const\s+statusResult\s*=\s*await\s+statusRequest\.promise/,
  )
  assert.match(
    checkStatus,
    /const\s+nextNetease\s*=\s*mapAuth\(status\.netease\)[\s\S]*const\s+neteaseSessionChanged\s*=\s*hasNeteaseSessionBoundary\(netease\.value,\s*nextNetease\)[\s\S]*if\s*\(neteaseSessionChanged\s*&&\s*!needsNeteaseVerification\)\s*\{[\s\S]*clearPlatformCache\('netease'\)[\s\S]*netease\.value\s*=\s*nextNetease/,
  )
})

await run('Netease status checks invalidate cloud likes and reload only active sessions', async () => {
  const authSource = await read('src/stores/auth.ts')
  const checkStatus = authSource.slice(
    authSource.indexOf('async function checkStatus'),
    authSource.indexOf('function needsYoutubeProfileRefresh'),
  )
  assert.match(authSource, /useLikedSongsStore/)
  assert.match(
    checkStatus,
    /hasNeteaseSessionBoundary\(netease\.value,\s*nextNetease\)[\s\S]*if\s*\(neteaseSessionChanged\s*&&\s*!needsNeteaseVerification\)\s*\{[\s\S]*clearCloudLikes\(\)[\s\S]*netease\.value\s*=\s*nextNetease/,
  )
  assert.match(
    checkStatus,
    /const\s+advanceNeteaseSession\s*=\s*\([\s\S]*invalidatePlatformSessions[\s\S]*nextNetease\.loggedIn[\s\S]*\)[\s\S]*if\s*\(advanceNeteaseSession\)\s*\{\s*neteaseSessionVersion\.value\+\+\s*if\s*\(nextNetease\.loggedIn\)\s*void\s+useLikedSongsStore\(\)\.refreshCloudLikes\(\)\s*\}/,
  )
  assert.equal((checkStatus.match(/neteaseSessionVersion\.value\+\+/g) ?? []).length, 1)
  assert.equal((checkStatus.match(/refreshCloudLikes\(\)/g) ?? []).length, 1)
  assert.match(
    authSource,
    /clearNeteaseCacheForAccountChange[\s\S]*clearCloudLikes\(\)/,
  )
  assert.match(
    authSource,
    /case ['"]netease['"]:[\s\S]*clearCloudLikes\(\)/,
  )
})

await run('both Netease login flows clear the previous account cache', async () => {
  const authSource = await read('src/stores/auth.ts')
  const popupLogin = authSource.slice(
    authSource.indexOf('async function doLogin'),
    authSource.indexOf('async function loginNetease'),
  )
  const cookieLogin = authSource.slice(
    authSource.indexOf('async function loginWithCookies'),
    authSource.lastIndexOf('return {'),
  )

  assert.match(popupLogin, /clearNeteaseCacheForAccountChange\(key, mapped\)/)
  assert.match(cookieLogin, /clearNeteaseCacheForAccountChange\(platform, mapped\)/)
})
