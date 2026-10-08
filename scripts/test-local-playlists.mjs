import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/localPlaylists.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const {
  LEGACY_PLAYLIST_ORDER_KEY,
  isEmptyLocalFilesPlaylist,
  isFavoritesPlaylist,
  isSystemPlaylist,
  localPlaylistDisplayName,
  playlistOrderIds,
  readLegacyPlaylistOrder,
  visibleSelection,
} = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

assert.equal(isFavoritesPlaylist({ id: -1001, name: 'Renamed elsewhere' }), true)
assert.equal(isFavoritesPlaylist({ id: '-3', name: 'my favorite music' }), true, 'names match case-insensitively on negative ids')
// 对齐 Android：用户自建的同名歌单（正数 id）不是系统歌单，可以删除、改名，也不会被当成收藏
assert.equal(isFavoritesPlaylist({ id: '42', name: 'My Favorite Music' }), false)
assert.equal(isSystemPlaylist({ id: 7, name: 'Local Files' }), false)
assert.equal(isSystemPlaylist({ id: -2, name: 'Local Files' }), true)
assert.equal(isSystemPlaylist({ id: 7, name: 'Road trip' }), false)
const labels = { favorites: 'Liked Songs', localFiles: 'Local Files' }
assert.equal(localPlaylistDisplayName({ id: -1001, name: '我喜欢的音乐' }, labels), 'Liked Songs')
assert.equal(localPlaylistDisplayName({ id: 1, name: '我喜欢的音乐' }, labels), '我喜欢的音乐')
assert.equal(localPlaylistDisplayName({ id: -1002, name: '本地文件' }, labels), 'Local Files')
assert.equal(localPlaylistDisplayName({ id: 2, name: 'Road trip' }, labels), 'Road trip')
assert.deepEqual([...visibleSelection(new Set([1, 2, 3]), [{ id: 3 }, { id: 1 }])].sort(), [1, 3])

assert.equal(isEmptyLocalFilesPlaylist({ id: -1002, name: '本地文件', track_count: 0 }), true)
assert.equal(isEmptyLocalFilesPlaylist({ id: 5, name: 'Local Music', track_count: 0 }), false)
assert.equal(isEmptyLocalFilesPlaylist({ id: -1002, name: '本地文件', track_count: 1 }), false)
assert.equal(isEmptyLocalFilesPlaylist({ id: 6, name: '自己的空歌单', track_count: 0 }), false)

const playlists = [
  { id: -1001, name: '我喜欢的音乐' },
  { id: 9007199254740991, name: 'Large' },
  { id: 3, name: 'Small' },
  { id: -1002, name: '本地音乐' },
]
const isProtected = (playlist) => playlist.id < 0
assert.deepEqual(playlistOrderIds(playlists, isProtected), ['9007199254740991', '3'])

const storage = (value) => ({ getItem: (key) => (key === LEGACY_PLAYLIST_ORDER_KEY ? value : null) })
assert.equal(readLegacyPlaylistOrder(undefined), null)
assert.equal(readLegacyPlaylistOrder(storage(null)), null)
assert.deepEqual(readLegacyPlaylistOrder(storage('[3, "7", 1.5, "x", -4]')), ['3', '7', '-4'])
assert.deepEqual(readLegacyPlaylistOrder(storage('{broken')), [])
assert.deepEqual(readLegacyPlaylistOrder(storage('{"a":1}')), [])
console.log('local playlist visibility and order tests passed')
