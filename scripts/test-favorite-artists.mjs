import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/favoriteArtists.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { parseFavoritePlaylists, isArtistFavoriteSource, filterFavoriteArtists, favoriteKey, favoriteArtistRoute } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

const favorites = parseFavoritePlaylists([
  { id: 39, name: '陈奕迅', source: 'neteaseArtist', subtitle: 'Eason Chan', sortOrder: 20 },
  { id: 39, name: '音乐 UP', source: 'biliArtist', coverUrl: 'https://i.example/avatar.jpg', sortOrder: 30 },
  { id: '-9007199254740993', name: 'Artist', source: 'youtubeMusicArtist', browseId: 'UC_artist', subtitle: '频道', sortOrder: 10 },
  { id: 39, name: '歌单', source: 'netease', songs: [{ id: 1 }] },
  { id: 40, name: '已取消', source: 'neteaseArtist', isDeleted: true },
  { id: 41, name: '旧歌手', source: 'neteaseArtist', subtitle: 'Other', cover_url: 'old.jpg', track_count: 12, added_time: 5 },
  null,
])
assert.equal(favorites.length, 5)
assert.equal(favorites[0].source, 'biliArtist')
assert.equal(new Set(favorites.map(favoriteKey)).size, 5)
assert.deepEqual(['neteaseArtist', 'biliArtist', 'youtubeMusicArtist'].map(isArtistFavoriteSource), [true, true, true])
assert.equal(isArtistFavoriteSource('youtubeMusic'), false)
assert.equal(filterFavoriteArtists(favorites, 'neteaseArtist').length, 2)
assert.equal(filterFavoriteArtists(favorites, 'neteaseArtist', ' EASON ')[0].name, '陈奕迅')
assert.equal(filterFavoriteArtists(favorites, 'biliArtist', 'UP')[0].id, '39')
assert.equal(filterFavoriteArtists(favorites, 'youtubeMusicArtist', '频道')[0].browseId, 'UC_artist')
assert.equal(favorites.find(f => f.name === '旧歌手').coverUrl, 'old.jpg')
assert.deepEqual(favoriteArtistRoute(favorites.find(f => f.source === 'neteaseArtist' && f.id === '39')), {
  name: 'netease-artist', params: { id: '39' }, query: { name: '陈奕迅', subtitle: 'Eason Chan' },
})
assert.deepEqual(favoriteArtistRoute(favorites[0]), {
  name: 'bili-artist', params: { mid: '39' }, query: { name: '音乐 UP', cover: 'https://i.example/avatar.jpg' },
})
assert.equal(favoriteArtistRoute(favorites.find(f => f.source === 'youtubeMusicArtist')).params.browseId, 'UC_artist')
assert.equal(favoriteArtistRoute({ source: 'youtubeMusicArtist', id: '123', name: '无频道 ID', browseId: '' }), null)
assert.equal(favoriteArtistRoute({ source: 'biliArtist', id: '0', name: '无效 ID' }), null)
assert.deepEqual(parseFavoritePlaylists(null), [])
console.log('Favorite artist sync, filtering and navigation fixtures passed')
