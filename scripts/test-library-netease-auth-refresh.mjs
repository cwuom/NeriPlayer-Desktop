import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/views/LibraryView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const recommendSource = await readFile(new URL('../src/stores/recommend.ts', import.meta.url), 'utf8')
const authSource = await readFile(new URL('../src/stores/auth.ts', import.meta.url), 'utf8')
const authStatusRequestSource = await readFile(new URL('../src/modules/auth/authStatusRequest.ts', import.meta.url), 'utf8')

const compiledAuthStatusRequest = ts.transpileModule(authStatusRequestSource, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const authStatusRequestModuleUrl = `data:text/javascript;base64,${Buffer.from(compiledAuthStatusRequest).toString('base64')}`
const { AuthStatusRequestCoordinator } = await import(authStatusRequestModuleUrl)

function deferred() {
  let resolve
  const promise = new Promise(nextResolve => {
    resolve = nextResolve
  })
  return { promise, resolve }
}

{
  const coordinator = new AuthStatusRequestCoordinator()
  const pending = deferred()
  let firstLoaderCalls = 0
  let secondLoaderCalls = 0

  const firstRequest = coordinator.run(() => {
    firstLoaderCalls += 1
    return pending.promise
  })
  const secondRequest = coordinator.run(() => {
    secondLoaderCalls += 1
    return Promise.reject(new Error('coalesced loader must not run'))
  })
  void (secondRequest.promise ?? secondRequest).catch(() => undefined)

  assert.equal(firstRequest.started, true)
  assert.equal(secondRequest.started, false)
  assert.equal(firstRequest.promise, secondRequest.promise, 'same-generation status checks must share one promise')
  await Promise.resolve()
  assert.equal(firstLoaderCalls, 1)
  assert.equal(secondLoaderCalls, 0)

  pending.resolve('coalesced')
  assert.deepEqual(await firstRequest.promise, { current: true, value: 'coalesced' })
}

{
  const coordinator = new AuthStatusRequestCoordinator()
  const oldPending = deferred()
  const oldRequest = coordinator.run(() => oldPending.promise)

  coordinator.invalidate()

  const freshPending = deferred()
  const freshRequest = coordinator.run(() => freshPending.promise)
  assert.equal(freshRequest.started, true)
  assert.notEqual(oldRequest.promise, freshRequest.promise, 'invalidate must allow a fresh status request')

  oldPending.resolve('old')
  assert.deepEqual(await oldRequest.promise, { current: false, value: 'old' })

  let thirdLoaderCalls = 0
  const thirdRequest = coordinator.run(() => {
    thirdLoaderCalls += 1
    return Promise.reject(new Error('fresh request must survive the old request finally'))
  })
  void (thirdRequest.promise ?? thirdRequest).catch(() => undefined)
  assert.equal(thirdRequest.started, false)
  assert.equal(thirdRequest.promise, freshRequest.promise, 'old request cleanup must not clear the fresh in-flight request')
  assert.equal(thirdLoaderCalls, 0)

  freshPending.resolve('fresh')
  assert.deepEqual(await freshRequest.promise, { current: true, value: 'fresh' })
}

assert.match(
  source,
  /import\s*\{\s*NeteaseLibraryRequestCoordinator\s*\}\s*from\s*['"]@\/modules\/library\/neteaseLibraryRequest['"]/,
  'library must use the shared Netease request coordinator',
)
assert.match(
  source,
  /const\s+neteaseLibraryRequestCoordinator\s*=\s*new\s+NeteaseLibraryRequestCoordinator\(\)/,
  'library must own a coordinator instance',
)
assert.match(
  source,
  /const\s+neteaseSessionFingerprint\s*=\s*computed\(\s*\(\)\s*=>\s*`\$\{auth\.netease\.loggedIn\s*\?\s*'1'\s*:\s*'0'\}:\$\{auth\.neteaseSessionVersion\}`\s*\)/,
  'a single fingerprint must cover login and session changes',
)
assert.match(
  source,
  /watch\(\s*neteaseSessionFingerprint\s*,[\s\S]*?\{\s*immediate:\s*true\s*}\s*\)/,
  'the fingerprint watcher must immediately establish Netease library state',
)
assert.doesNotMatch(source, /watch\(\s*\(\)\s*=>\s*auth\.netease\.nickname/, 'nickname must not have an independent watcher')
assert.doesNotMatch(source, /watch\(\s*\(\)\s*=>\s*auth\.netease\.loggedIn/, 'loggedIn must not have an independent watcher')
assert.doesNotMatch(source, /watch\(\s*\(\)\s*=>\s*auth\.neteaseSessionVersion/, 'sessionVersion must not have an independent watcher')
assert.match(
  source,
  /fetchUserPlaylists\('netease'\)/,
  'auth watcher must refresh Netease playlists',
)
assert.match(
  source,
  /fetchUserAlbums\(\)/,
  'auth watcher must refresh Netease albums',
)
assert.match(
  source,
  /recommend\.userAlbums\s*=\s*\[\]/,
  'logout must clear Netease album data from the view',
)
assert.match(
  source,
  /recommend\.userPlaylists\[['"]netease['"]\]\s*=\s*\[\]/,
  'logout must clear Netease playlist data from the view',
)
assert.match(source, /const\s+neteasePlaylistLoading\s*=\s*ref\(false\)/)
assert.match(source, /const\s+neteaseAlbumLoading\s*=\s*ref\(false\)/)
assert.match(source, /const\s+neteasePlaylistError\s*=\s*ref(?:<[^>]+>)?\(null\)/)
assert.match(source, /const\s+neteaseAlbumError\s*=\s*ref(?:<[^>]+>)?\(null\)/)
assert.doesNotMatch(source, /neteaseLibraryLoading|neteaseLibraryError|neteaseLibraryRequestGeneration/)
assert.match(source, /async\s+function\s+loadNeteaseLibrary\(/)
assert.match(
  source,
  /neteaseLibraryRequestCoordinator\.run\(\s*\(\)\s*=>\s*recommend\.fetchUserPlaylists\('netease'\)\s*,\s*\(\)\s*=>\s*recommend\.fetchUserAlbums\(\)\s*,?\s*\)/,
  'playlist and album requests must be coordinated and coalesced',
)
assert.match(source, /if\s*\(!request\.started\)\s*return\s*request\.promise/, 'coalesced callers must reuse the in-flight promise')
assert.match(source, /result\.playlistsOk/, 'playlist failures must be handled independently')
assert.match(source, /result\.albumsOk/, 'album failures must be handled independently')
assert.match(
  source,
  /neteasePlaylistLoading\s*&&\s*neteasePlaylists\.length\s*===\s*0/,
  'playlist loading must not hide existing playlist rows',
)
assert.match(
  source,
  /neteasePlaylistError\s*&&\s*neteasePlaylists\.length\s*===\s*0[\s\S]*?@click="loadNeteaseLibrary"/,
  'playlist failures must expose a retry action only when there are no rows',
)
assert.match(
  source,
  /neteaseAlbumLoading\s*&&\s*recommend\.userAlbums\.length\s*===\s*0/,
  'album loading must not hide existing album rows',
)
assert.match(
  source,
  /neteaseAlbumError\s*&&\s*recommend\.userAlbums\.length\s*===\s*0[\s\S]*?@click="loadNeteaseLibrary"/,
  'album failures must expose a retry action only when there are no rows',
)
assert.match(
  recommendSource,
  /async\s+function\s+fetchUserPlaylists\(platform:\s*string\):\s*Promise<boolean>/,
  'Netease playlist loading must expose a success result to the view',
)
assert.match(
  recommendSource,
  /async\s+function\s+fetchUserAlbums\(\):\s*Promise<boolean>/,
  'Netease album loading must expose a success result to the view',
)
assert.match(authSource, /const\s+neteaseSessionVersion\s*=\s*ref\(0\)/)
assert.match(
  authSource,
  /import\s*\{\s*AuthStatusRequestCoordinator\s*\}\s*from\s*['"]@\/modules\/auth\/authStatusRequest['"]/,
  'auth store must use the latest-request-wins status coordinator',
)
assert.match(
  authSource,
  /const\s+authStatusRequestCoordinator\s*=\s*new\s+AuthStatusRequestCoordinator(?:<[^>]+>)?\(\)/,
  'auth store must own an auth status request coordinator',
)

const checkStatusSource = authSource.slice(
  authSource.indexOf('async function checkStatus'),
  authSource.indexOf('function needsYoutubeProfileRefresh'),
)
assert.match(
  checkStatusSource,
  /const\s+statusRequest\s*=\s*authStatusRequestCoordinator\.run\(\s*\(\)\s*=>\s*invoke<any>\('check_auth_status'\)\s*,?\s*\)/,
  'status checks must run through the latest-request-wins coordinator',
)
assert.match(
  checkStatusSource,
  /if\s*\(!statusRequest\.started\)\s*\{\s*await\s+statusRequest\.promise\s*return\s*\}/,
  'coalesced status callers must wait for the shared result and return without publishing it',
)
assert.match(
  checkStatusSource,
  /const\s+statusResult\s*=\s*await\s+statusRequest\.promise[\s\S]*if\s*\(!statusResult\.current\)\s*return/,
  'stale status checks must return before mutating auth or dependent stores',
)
assert.match(
  checkStatusSource,
  /const\s+status\s*=\s*statusResult\.value/,
  'current status checks must publish the coordinated result',
)
assert.match(
  checkStatusSource,
  /const\s+nextNetease\s*=\s*mapAuth\(status\.netease\)/,
  'every auth status result must first be mapped as a potential Netease session boundary',
)
assert.match(
  checkStatusSource,
  /const\s+neteaseSessionChanged\s*=\s*hasNeteaseSessionBoundary\(netease\.value,\s*nextNetease\)/,
  'status checks must compare the current and incoming Netease identities',
)
assert.match(
  checkStatusSource,
  /if\s*\(neteaseSessionChanged\s*&&\s*!needsNeteaseVerification\)\s*\{\s*useRecommendStore\(\)\.clearPlatformCache\('netease'\)\s*useLikedSongsStore\(\)\.clearCloudLikes\(\)\s*\}/,
  'only a Netease session boundary may invalidate recommendations and cloud likes',
)
assert.match(
  checkStatusSource,
  /const\s+needsNeteaseVerification\s*=\s*!hasVerifiedNeteaseAuth[\s\S]*if\s*\(needsNeteaseVerification\)\s*\{\s*useRecommendStore\(\)\.clearPlatformCache\('netease'\)\s*useLikedSongsStore\(\)\.clearCloudLikes\(\)\s*\}[\s\S]*await\s+statusRequest\.promise/,
  'unverified Netease cache must be cleared before the initial status result is trusted',
)
assert.match(
  checkStatusSource,
  /netease\.value\s*=\s*nextNetease/,
  'checking status must assign the mapped Netease status after invalidation',
)
const mappedNeteaseIndex = checkStatusSource.indexOf('mapAuth(status.netease)')
const sessionBoundaryIndex = checkStatusSource.indexOf('hasNeteaseSessionBoundary', mappedNeteaseIndex)
const boundaryCacheClearIndex = checkStatusSource.indexOf("clearPlatformCache('netease')", sessionBoundaryIndex)
const boundaryLikesClearIndex = checkStatusSource.indexOf('clearCloudLikes()', boundaryCacheClearIndex)
assert.ok(
  mappedNeteaseIndex < sessionBoundaryIndex &&
    sessionBoundaryIndex < boundaryCacheClearIndex &&
    boundaryCacheClearIndex < boundaryLikesClearIndex &&
    boundaryLikesClearIndex < checkStatusSource.indexOf('netease.value = nextNetease'),
  'Netease status checks must map, compare identities, invalidate old stores, then publish the new status',
)
assert.match(
  checkStatusSource,
  /if\s*\(nextNetease\.loggedIn\s*&&\s*\(neteaseSessionChanged\s*\|\|\s*needsNeteaseVerification\)\)\s*\{\s*neteaseSessionVersion\.value\+\+\s*void\s+useLikedSongsStore\(\)\.refreshCloudLikes\(\)\s*\}/,
  'an authenticated boundary or completed verification must advance the session and reload cloud likes',
)
assert.equal(
  (checkStatusSource.match(/neteaseSessionVersion\.value\+\+/g) ?? []).length,
  1,
  'a status result must advance the Netease session at most once',
)
assert.equal(
  (checkStatusSource.match(/useLikedSongsStore\(\)\.refreshCloudLikes\(\)/g) ?? []).length,
  1,
  'a status result must start at most one cloud-like refresh',
)
assert.match(
  authSource,
  /function\s+hasNeteaseSessionBoundary\(previous:\s*PlatformAuth,\s*next:\s*PlatformAuth\)[\s\S]*previous\.loggedIn\s*!==\s*next\.loggedIn[\s\S]*next\.loggedIn\s*&&\s*previous\.accountId\s*!==\s*next\.accountId/,
  'a Netease session boundary must be a login-state or logged-in account-id change',
)

assert.match(
  authSource,
  /function\s+invalidateAuthStatusRequests\(\)\s*\{\s*authStatusRequestCoordinator\.invalidate\(\)\s*\}/,
  'every auth mutation must invalidate the global status response',
)
assert.match(
  authSource,
  /import\s*\{\s*AuthMutationRequestCoordinator\s*\}\s*from\s*['"]@\/modules\/auth\/authMutationRequest['"]/,
  'auth store must use the per-platform mutation coordinator',
)
assert.match(
  authSource,
  /const\s+authMutationRequestCoordinator\s*=\s*new\s+AuthMutationRequestCoordinator\(\)/,
  'auth store must own one mutation coordinator',
)

function assertMutationInvalidatesStatusTwice(name, mutationSource, runExpression, commitExpression) {
  const invalidation = 'invalidateAuthStatusRequests()'
  const invalidationIndexes = []
  let searchFrom = 0
  while ((searchFrom = mutationSource.indexOf(invalidation, searchFrom)) >= 0) {
    invalidationIndexes.push(searchFrom)
    searchFrom += invalidation.length
  }

  assert.equal(invalidationIndexes.length, 2, `${name} must invalidate status requests at mutation start and commit`)
  const runIndex = runExpression instanceof RegExp
    ? mutationSource.search(runExpression)
    : mutationSource.indexOf(runExpression)
  const currentCheckIndex = mutationSource.indexOf('if (!mutation.isCurrent()) return')
  const commitIndex = mutationSource.indexOf(commitExpression)
  assert.ok(
    invalidationIndexes[0] < runIndex &&
      runIndex < currentCheckIndex &&
      currentCheckIndex < invalidationIndexes[1] &&
      invalidationIndexes[1] < commitIndex,
    `${name} must invalidate before its request and again before publishing its result`,
  )
}

const doLoginSource = authSource.slice(
  authSource.indexOf('async function doLogin'),
  authSource.indexOf('async function loginNetease'),
)
assertMutationInvalidatesStatusTwice('popup login', doLoginSource, 'authMutationRequestCoordinator.run(key', 'clearNeteaseCacheForAccountChange(key, mapped)')

const logoutSource = authSource.slice(
  authSource.indexOf('async function logout'),
  authSource.indexOf('async function loginWithCookies'),
)
assertMutationInvalidatesStatusTwice(
  'logout',
  logoutSource,
  /authMutationRequestCoordinator\.run\(\s*platform\s*,/,
  'netease.value = emptyAuth()',
)

const cookieLoginSource = authSource.slice(
  authSource.indexOf('async function loginWithCookies'),
  authSource.lastIndexOf('return {'),
)
assertMutationInvalidatesStatusTwice(
  'cookie login',
  cookieLoginSource,
  /authMutationRequestCoordinator\.run\(\s*platform\s*,/,
  'clearNeteaseCacheForAccountChange(platform, mapped)',
)

console.log('library netease auth refresh tests passed')
