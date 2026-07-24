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
  const importedAuth = sliceBetween(source, 'if let Some(imported_auth) = payload.auth', 'let legacy_history_mode')

  const gate = importedAuth.indexOf('state.auth_cookie_gate.lock().await')
  const authLock = importedAuth.indexOf('state.auth.lock()')
  const expire = importedAuth.indexOf('expire_platform_cookies')
  const replace = importedAuth.indexOf('*auth = imported_auth')
  const persist = importedAuth.indexOf('save_auth')
  const unlock = importedAuth.indexOf('drop(auth)')
  const cleanup = importedAuth.indexOf('clear_and_reinject_webview_cookies')
  assert.ok(gate >= 0, 'config import must acquire the shared cookie gate')
  assert.ok(authLock > gate, 'config import must lock auth after acquiring the cookie gate')
  assert.ok(expire > authLock, 'config import must expire old cookies under the auth lock')
  assert.ok(replace > expire, 'config import must replace auth after expiring old cookies')
  assert.ok(persist > replace, 'config import must persist the imported auth under the auth lock')
  assert.ok(cleanup > persist, 'config import must synchronously clear shared WebView cookies')
  assert.ok(unlock > persist && unlock < cleanup, 'config import must release auth before the cleaner re-reads it')
  assert.equal(
    importedAuth.indexOf('inject_all'),
    -1,
    'config import must leave imported auth injection to the post-cleanup cleaner',
  )
  assert.match(
    importedAuth,
    /clear_and_reinject_webview_cookies\(&app,\s*&state\)\.await\?;/,
    'config import must propagate WebView cleanup errors while still holding the cookie gate',
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

await run('auth cookie writer contract is part of the Netease regression suite', async () => {
  const packageJson = JSON.parse(await read('package.json'))
  assert.equal(packageJson.scripts['test:auth-cookie-writers'], 'node scripts/test-auth-cookie-writers.mjs')
  assert.match(packageJson.scripts['test:netease'], /pnpm test:auth-cookie-writers/)
})

if (!process.exitCode) console.log('auth cookie writer tests passed')
