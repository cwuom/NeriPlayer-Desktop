import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/homeContinue.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { normalizeContinuePlaylists, continuePlaylistRoute } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

const entry = (overrides = {}) => ({
  id: '1', source: 'netease', name: '歌单', coverUrl: '', trackCount: 3,
  lastOpenedAt: 100, openCount: 1, ...overrides,
})
const local = [{ id: 8, name: '当前歌单', cover_url: 'current.jpg', track_count: 7 }]
const items = normalizeContinuePlaylists([
  entry({ id: '4', lastOpenedAt: 80 }),
  entry({ id: '3', openCount: 2 }),
  entry({ id: '2', openCount: 2 }),
  entry({ id: '2', name: '更新的歌单', lastOpenedAt: 200 }),
  entry({ id: '8', source: 'local', name: '旧名字', trackCount: 1, lastOpenedAt: 300 }),
  entry({ id: '9', source: 'local' }),
  entry({ id: '5', trackCount: 0 }),
  entry({ id: '6', source: 'unknown' }),
  entry({ id: '7', source: 'youtubeMusic' }),
  null,
], local)
assert.deepEqual(items.map(item => item.id), ['8', '2', '3', '4'])
assert.equal(items[0].name, '当前歌单')
assert.equal(items[0].trackCount, 7)
assert.equal(items[0].coverUrl, 'current.jpg')
assert.equal(items[1].name, '更新的歌单')
assert.equal(normalizeContinuePlaylists(Array.from({ length: 20 }, (_, id) => entry({ id: String(id + 1) }))).length, 20)
assert.deepEqual(normalizeContinuePlaylists(null), [])
assert.deepEqual(normalizeContinuePlaylists([entry({ id: 'invalid', source: 'localArtist' })]), [])
assert.deepEqual(normalizeContinuePlaylists([entry({ id: '8', source: 'local' })], [{ ...local[0], track_count: 0 }]), [])

const largeIds = normalizeContinuePlaylists([
  entry({ id: '-9007199254740993', source: 'youtubeMusic', browseId: 'VL_one' }),
  entry({ id: '-9007199254740994', source: 'youtubeMusic', browseId: 'VL_two' }),
])
assert.deepEqual(largeIds.map(item => item.id), ['-9007199254740994', '-9007199254740993'])
assert.deepEqual(continuePlaylistRoute(largeIds[0]), { name: 'youtube-playlist', params: { browseId: 'VL_two' } })
assert.deepEqual(continuePlaylistRoute(entry({ source: 'youtubeMusic', playlistId: 'PL_original' })), {
  name: 'youtube-playlist', params: { browseId: 'VLPL_original' },
})
assert.deepEqual(continuePlaylistRoute(entry({ source: 'neteaseAlbum' })), { name: 'netease-album', params: { id: '1' } })
assert.deepEqual(continuePlaylistRoute(entry({ source: 'local', id: '8' })), { name: 'local-playlist', params: { id: '8' } })
assert.deepEqual(continuePlaylistRoute(entry({ source: 'localArtist', name: '歌手' })), { name: 'local-artist', params: { name: '歌手' } })
assert.deepEqual(continuePlaylistRoute(entry({ source: 'bili', id: '12', fid: '99' })), { name: 'bili-playlist', params: { mediaId: '12' } })
assert.equal(continuePlaylistRoute(entry({ source: 'youtubeMusic' })), null)
assert.equal(continuePlaylistRoute(entry({ source: 'bili', subtype: 'COLLECTION' })), null)
assert.deepEqual(continuePlaylistRoute(entry({ source: 'bili', id: '12', subtype: 'COLLECTION', mid: '42' })), {
  name: 'bili-artist', params: { mid: '42' }, query: { contentId: '12', kind: 'collection', name: '歌单', cover: '', count: '3' },
})
assert.deepEqual(continuePlaylistRoute(entry({ source: 'bili', id: '13', subtype: 'SERIES', mid: '42' })), {
  name: 'bili-artist', params: { mid: '42' }, query: { contentId: '13', kind: 'series', name: '歌单', cover: '', count: '3' },
})
assert.equal(normalizeContinuePlaylists([entry({ id: '9007199254740993', source: 'local' })], [
  { id: '9007199254740993', name: '大 ID 歌单', track_count: 5 },
])[0].id, '9007199254740993')
console.log('Home continue playlist ordering, hydration, filtering and navigation fixtures passed')
