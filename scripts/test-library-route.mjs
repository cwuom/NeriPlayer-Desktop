import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/libraryRoute.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { resolveLibraryLocation, buildLibraryQuery, isCanonicalLibraryLocation } = await import(moduleUrl)

assert.deepEqual(resolveLibraryLocation('local', undefined), { tab: 'local', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation('netease', 'albums'), { tab: 'netease', category: 'albums' })
assert.deepEqual(resolveLibraryLocation(['netease'], ['playlists']), { tab: 'netease', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation(null, null), { tab: 'local', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation([null, 'netease'], ['albums']), { tab: 'local', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation(['netease', null], [null, 'albums']), { tab: 'netease', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation('netease_playlists', undefined), { tab: 'netease', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation('netease_albums', undefined), { tab: 'netease', category: 'albums' })
assert.deepEqual(resolveLibraryLocation('unknown', 'albums'), { tab: 'local', category: 'playlists' })
assert.deepEqual(buildLibraryQuery({ tab: 'netease', category: 'albums' }), { tab: 'netease', category: 'albums' })
assert.deepEqual(buildLibraryQuery({ tab: 'downloads', category: 'albums' }), { tab: 'downloads' })
assert.equal(isCanonicalLibraryLocation('netease', 'albums'), true)
assert.equal(isCanonicalLibraryLocation('local', undefined), true)
assert.equal(isCanonicalLibraryLocation(['netease', 'netease'], ['albums', 'albums']), false)
assert.equal(isCanonicalLibraryLocation(['netease'], 'albums'), false)
assert.equal(isCanonicalLibraryLocation('netease', ['albums']), false)
assert.equal(isCanonicalLibraryLocation(null, undefined), false)
assert.equal(isCanonicalLibraryLocation('local', null), false)
assert.equal(isCanonicalLibraryLocation('netease_albums', undefined), false)
assert.equal(isCanonicalLibraryLocation('favorites', 'albums'), false)

console.log('library route tests passed')
