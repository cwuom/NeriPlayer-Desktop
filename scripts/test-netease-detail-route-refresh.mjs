import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/views/NeteasePlaylistView.vue', import.meta.url), 'utf8')
const scopeHelperUrl = new URL(
  '../src/modules/library/neteaseDetailCacheScope.ts',
  import.meta.url,
)
let scopeHelperSource = ''
try {
  scopeHelperSource = await readFile(scopeHelperUrl, 'utf8')
} catch (error) {
  if (error?.code !== 'ENOENT') throw error
}

assert.ok(
  scopeHelperSource,
  'Netease detail caches need a pure account/session scope helper',
)

const compiledScopeHelper = ts.transpileModule(scopeHelperSource, {
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const scopeHelperModuleUrl = `data:text/javascript;base64,${Buffer.from(compiledScopeHelper).toString('base64')}`
const { createNeteaseDetailCacheScope } = await import(scopeHelperModuleUrl)

const accountA = {
  loggedIn: true,
  accountId: '10001',
}
const accountB = {
  loggedIn: true,
  accountId: '10002',
}

const accountASession1 = createNeteaseDetailCacheScope(accountA, 1)
assert.ok(accountASession1, 'a logged-in account ID must produce a cache scope')
assert.equal(
  accountASession1,
  createNeteaseDetailCacheScope({ ...accountA }, 1),
  'the NetEase account ID must produce a stable scope across app restarts',
)
assert.notEqual(
  accountASession1,
  createNeteaseDetailCacheScope(accountB, 1),
  'different NetEase account IDs must not share a persisted detail cache scope',
)
assert.notEqual(
  accountASession1,
  createNeteaseDetailCacheScope(accountA, 2),
  'new auth sessions must not reuse the live session cache scope',
)
assert.equal(
  accountASession1,
  createNeteaseDetailCacheScope({ loggedIn: true, accountId: '10001 ' }, 1),
  'equivalent account ID text must normalize to one stable identity',
)
assert.equal(
  createNeteaseDetailCacheScope(
    { loggedIn: false, accountId: accountA.accountId },
    1,
  ),
  null,
  'logged-out detail views must not receive a cache scope',
)
assert.equal(
  createNeteaseDetailCacheScope(
    { loggedIn: true, accountId: null },
    1,
  ),
  null,
  'authenticated sessions without an account ID must not receive a persisted cache scope',
)
assert.equal(
  createNeteaseDetailCacheScope({ loggedIn: true, accountId: '   ' }, 1),
  null,
  'blank account IDs must not receive a persisted cache scope',
)
assert.doesNotMatch(accountASession1, /10001/)
assert.doesNotMatch(scopeHelperSource, /nickname|avatarUrl/)

assert.match(source, /watch\(\s*\(\)\s*=>\s*\[route\.params\.id,\s*props\.isAlbum\]/)
assert.match(source, /let\s+detailRequestGeneration\s*=\s*0/)
assert.match(source, /const\s+requestGeneration\s*=\s*\+\+detailRequestGeneration/)
assert.match(
  source,
  /requestGeneration\s*!==\s*detailRequestGeneration/,
  'stale playlist responses must not replace the active route',
)
assert.equal(
  source.match(/assertNeteaseDetailResponse\(data\)/g)?.length,
  2,
  'playlist and album responses must both reject non-success business codes',
)
assert.match(
  source,
  /function\s+resetDetailState\(\)[\s\S]*playCount\.value\s*=\s*0[\s\S]*tracks\.value\s*=\s*\[\]/,
  'route changes must clear every field inherited from the previous detail page',
)
assert.match(
  source,
  /const\s+requestGeneration\s*=\s*\+\+detailRequestGeneration[\s\S]*resetDetailState\(\)[\s\S]*const\s+id\s*=\s*Number\(route\.params\.id\)/,
  'detail state must be reset before resolving the next route id',
)
assert.match(
  source,
  /if\s*\(!Number\.isSafeInteger\(id\)\s*\|\|\s*id\s*<=\s*0\)\s*\{[\s\S]*error\.value\s*=\s*t\('player\.load_failed'\)/,
  'invalid route ids must enter a visible error state',
)
assert.match(
  source,
  /if\s*\(props\.isAlbum\)[\s\S]*playCount\.value\s*=\s*0/,
  'album details must not inherit playlist play counts',
)
assert.match(source, /import\s*\{\s*useAuthStore\s*\}\s*from\s*['"]@\/stores\/auth['"]/)
assert.match(source, /const\s+auth\s*=\s*useAuthStore\(\)/)
assert.match(
  source,
  /createNeteaseDetailCacheScope\(\s*auth\.netease\s*,\s*auth\.neteaseSessionVersion\s*\)/,
  'the cache scope must include the active auth identity and session boundary',
)
assert.match(
  source,
  /const\s+neteaseSessionFingerprint\s*=\s*computed\(\s*\(\)\s*=>\s*`\$\{auth\.netease\.loggedIn\s*\?\s*['"]1['"]\s*:\s*['"]0['"]\}:\$\{auth\.neteaseSessionVersion\}`\s*,?\s*\)/,
  'session invalidation must remain observable even when no durable cache identity exists',
)
assert.match(
  source,
  /if\s*\(!auth\.netease\.loggedIn\)\s*\{[\s\S]*?error\.value\s*=[\s\S]*?isLoading\.value\s*=\s*false[\s\S]*?return[\s\S]*?\}\s*const\s+cacheScope/,
  'logged-out detail loads must stop visibly before cache and backend access',
)
assert.match(
  source,
  /const\s+cacheKey\s*=\s*cacheScope\s*\?[\s\S]*?playlistDetailCacheKey\([\s\S]*?cacheScope[\s\S]*?,\s*id\s*,?\s*\)[\s\S]*?:\s*null/,
  'Netease playlist and album keys must be scoped to the active account session',
)
assert.match(
  source,
  /watch\(\s*neteaseSessionFingerprint\s*,[\s\S]*?resetDetailState\(\)[\s\S]*?loadDetail\(\)/,
  'mounted detail views must clear and reload immediately when the auth session changes',
)

const loadDetailSource = source.slice(
  source.indexOf('async function loadDetail()'),
  source.indexOf('function playAll()'),
)
assert.match(
  loadDetailSource,
  /const\s+cached\s*=\s*cacheKey\s*\?\s*readPlaylistDetailCache[^:]*:\s*null/,
  'authenticated profiles without durable identity must not read the persistent cache',
)
assert.match(
  loadDetailSource,
  /if\s*\(cacheKey\)\s+saveDetailCache\(cacheKey\)/,
  'authenticated profiles without durable identity must not write the persistent cache',
)
assert.match(
  loadDetailSource,
  /catch\s*\([^)]*\)\s*\{[\s\S]*?resetDetailState\(\)[\s\S]*?error\.value\s*=/,
  'failed revalidation must remove cached detail UI and expose the failure',
)
assert.doesNotMatch(
  loadDetailSource,
  /requestGeneration\s*===\s*detailRequestGeneration\s*&&\s*!cached/,
  'a cached response must not suppress a revalidation error',
)

console.log('Netease detail route refresh tests passed')
