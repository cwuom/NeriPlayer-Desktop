import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/neteaseLibraryFilter.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const {
  normalizeNeteaseLibrarySearch,
  filterNeteasePlaylists,
  filterNeteaseAlbums,
} = await import(moduleUrl)

const playlists = [
  { id: 101, name: 'Morning Focus', playCount: 4200, trackCount: 38 },
  { id: 'cloud-202', name: 'Cloud Songs', playCount: 0, trackCount: 12 },
]
const albums = [
  { id: 301, name: 'Sunset', artist: 'Luna Gray', trackCount: 10 },
  { id: 'album-404', name: 'Ambient Field', artist: 'North Wind', trackCount: 24 },
]
const unicodePlaylists = [
  { id: 505, name: 'Caf\u00e9', playCount: 1, trackCount: 1 },
]

assert.equal(normalizeNeteaseLibrarySearch('  MoRnInG  '), 'morning')
assert.deepEqual(filterNeteasePlaylists(playlists, 'FOCUS'), [playlists[0]])
assert.deepEqual(filterNeteasePlaylists(playlists, 'cloud-202'), [playlists[1]])
assert.deepEqual(filterNeteasePlaylists(playlists, '4200'), [playlists[0]])
assert.deepEqual(filterNeteasePlaylists(playlists, '12'), [playlists[1]])
assert.deepEqual(filterNeteasePlaylists(unicodePlaylists, 'Cafe\u0301'), unicodePlaylists)
assert.equal(filterNeteasePlaylists(playlists, ''), playlists)
assert.deepEqual(filterNeteasePlaylists(playlists, 'missing'), [])
assert.deepEqual(filterNeteaseAlbums(albums, 'LUNA'), [albums[0]])
assert.deepEqual(filterNeteaseAlbums(albums, 'album-404'), [albums[1]])
assert.deepEqual(filterNeteaseAlbums(albums, '24'), [albums[1]])
assert.equal(filterNeteaseAlbums(albums, '   '), albums)
assert.deepEqual(filterNeteaseAlbums(albums, 'no match'), [])

console.log('library netease filter tests passed')
