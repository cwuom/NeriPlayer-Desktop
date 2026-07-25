import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const root = new URL('..', import.meta.url)

async function read(path) {
  return readFile(new URL(path, root), 'utf8')
}

function sliceBetween(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker)
  assert.ok(start >= 0, `missing ${startMarker}`)
  const end = endMarker ? source.indexOf(endMarker, start) : source.length
  assert.ok(end >= 0, `missing ${endMarker}`)
  return source.slice(start, end)
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

await run('YouTube playlist refresh commits only under the auth cookie gate', async () => {
  const [recommend, auth] = await Promise.all([
    read('src-tauri/src/commands/recommend_cmd.rs'),
    read('src-tauri/src/commands/auth_cmd.rs'),
  ])
  const source = sliceBetween(
    recommend,
    'pub async fn get_youtube_playlist_detail',
    '/// 验证平台登录状态',
  )
  const refreshCommit = sliceBetween(source, 'if let Some(updated) = refreshed_auth', 'Ok(detail)')

  const gate = refreshCommit.indexOf('state.auth_cookie_gate.lock().await')
  const authLock = refreshCommit.indexOf('state.auth.lock()')
  const assignment = refreshCommit.indexOf('*saved = updated')
  const persist = refreshCommit.indexOf('cookies::save_auth')
  const inject = refreshCommit.indexOf('cookies::inject_cookies')
  assert.ok(gate >= 0, 'playlist refresh must acquire the shared cookie gate')
  assert.ok(authLock > gate, 'playlist refresh must lock auth after acquiring the cookie gate')
  assert.ok(assignment > authLock, 'playlist refresh must only assign after locking current auth')
  assert.ok(persist > assignment, 'playlist refresh must persist after the guarded assignment')
  assert.ok(inject > persist, 'playlist refresh must inject cookies after persistence')
  assert.match(
    refreshCommit,
    /if\s*!?\s*crate::commands::auth_cmd::youtube_auth_matches\(saved,\s*&updated\)/,
    'playlist refresh must reject an account-switched response before replacing auth',
  )
  assert.match(auth, /pub\s*\(crate\)\s+fn\s+youtube_auth_matches/)
})

await run('auth-bearing config import clears WebView cookies before injecting imported auth', async () => {
  const [sync, auth] = await Promise.all([
    read('src-tauri/src/commands/sync_cmd.rs'),
    read('src-tauri/src/commands/auth_cmd.rs'),
  ])
  const source = sliceBetween(sync, 'pub async fn import_config', '/// 断开 WebDAV 同步')

  const gate = source.indexOf('state.auth_cookie_gate.lock().await')
  const authLock = source.indexOf('let previous_auth = state.auth.lock().clone()', gate)
  const merge = source.indexOf('merge_imported_auth_platforms', authLock)
  const persist = source.indexOf('save_auth_strict(&app, auth)', merge)
  const expire = source.indexOf('expire_platform_cookies', persist)
  const replace = source.indexOf('*state.auth.lock() = imported_auth.clone()', expire)
  const cleanup = source.indexOf('clear_and_reinject_webview_cookies(&app, &state).await', replace)
  const restore = source.indexOf('*state.auth.lock() = previous_auth.clone()', cleanup)
  const rollback = source.indexOf('rollback_config_import_persistence', restore)
  assert.ok(gate >= 0, 'config import must acquire the shared cookie gate')
  assert.ok(authLock > gate, 'config import must lock auth after acquiring the cookie gate')
  assert.ok(merge > authLock, 'config import must merge phone auth with the latest guarded desktop state')
  assert.ok(persist > merge, 'config import must strictly persist auth before publishing it in memory')
  assert.ok(expire > persist, 'config import must expire replaced platform cookies after persistence succeeds')
  assert.ok(replace > expire, 'config import must publish imported auth after expiring old cookies')
  assert.ok(cleanup > replace, 'config import must synchronously clear shared WebView cookies')
  assert.ok(restore > cleanup, 'a WebView cleanup failure must restore the previous in-memory auth')
  assert.ok(rollback > restore, 'a WebView cleanup failure must roll persisted config back')
  assert.doesNotMatch(source, /drop\(auth\)/)
  assert.doesNotMatch(
    source,
    /let mut auth = state\.auth\.lock\(\)[\s\S]*?clear_and_reinject_webview_cookies/,
    'config import must not hold a non-Send auth guard across WebView cleanup',
  )
  assert.match(
    source,
    /if let Err\(error\) =[\s\S]*?clear_and_reinject_webview_cookies\(&app,\s*&state\)\.await[\s\S]*?return Err\(error\)/,
    'config import must roll back and propagate WebView cleanup errors while holding the cookie gate',
  )

  const cleaner = sliceBetween(auth, 'async fn clear_and_reinject_webview_cookies', null)
  assert.match(
    auth,
    /pub\(crate\)\s+async\s+fn\s+clear_and_reinject_webview_cookies/,
    'config import must reuse the shared auth cleaner',
  )
  const clear = cleaner.indexOf('clear_all_browsing_data')
  const currentAuth = cleaner.indexOf('state.auth.lock().clone()')
  const inject = cleaner.indexOf('cookies::inject_all')
  assert.ok(clear >= 0, 'shared cleaner must attempt to clear WebView browsing data')
  assert.ok(currentAuth > clear, 'shared cleaner must read current auth after the destructive clear')
  assert.ok(inject > currentAuth, 'shared cleaner must inject current auth into reqwest Jar after cleanup')
})

await run('logout releases its non-Send auth guard before WebView cleanup', async () => {
  const auth = await read('src-tauri/src/commands/auth_cmd.rs')
  const source = sliceBetween(
    auth,
    'pub async fn logout',
    'pub(crate) async fn clear_and_reinject_webview_cookies',
  )

  assert.doesNotMatch(source, /drop\(auth\)/)
  assert.match(
    source,
    /let _cookie_guard = state\.auth_cookie_gate\.lock\(\)\.await;\s*\{\s*let mut auth = state\.auth\.lock\(\);[\s\S]*?cookies::save_auth\(&app,\s*&auth\);\s*\}\s*clear_and_reinject_webview_cookies\(&app,\s*&state\)\.await\?;/,
    'logout must end the non-Send auth guard scope before awaiting WebView cleanup',
  )
})

await run('auth cookie writer contract is part of the Netease regression suite', async () => {
  const packageJson = JSON.parse(await read('package.json'))
  assert.equal(packageJson.scripts['test:auth-cookie-writers'], 'node scripts/test-auth-cookie-writers.mjs')
  assert.match(packageJson.scripts['test:netease'], /pnpm test:auth-cookie-writers/)
})

if (!process.exitCode) console.log('auth cookie writer tests passed')
