// 回归：歌单/歌手/最近播放页里先搜索再点歌，队列只剩搜索结果。
// 搜索只用来定位：点击后应按完整列表排队、从所点歌曲开始
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const views = {
  LocalPlaylistView: /const playlistTracks = tracks\.value\b/,
  NeteasePlaylistView: /player\.playAll\(tracks\.value, track\.id\)/,
  YouTubePlaylistView: /player\.playAll\(tracks\.value, track\.id\)/,
  BiliPlaylistView: /player\.playAll\(tracks\.value, track\.id\)/,
  LocalArtistView: /player\.playAll\(tracks\.value, track\.id\)/,
  NeteaseArtistView: /player\.playAll\(tracks\.value, track\.id\)/,
  RecentView: /const tracks = history\.entries\.map\(e => e\.track\)/,
}

for (const [name, expected] of Object.entries(views)) {
  const source = await readFile(new URL(`../src/views/${name}.vue`, import.meta.url), 'utf8')
  const playTrack = /function play(?:Track|Entry)\([^)]*\)[^{]*\{[\s\S]*?\n\}/.exec(source)?.[0]
  assert.ok(playTrack, `${name}: click handler not found`)
  assert.match(playTrack, expected, `${name}: clicking a track must queue the whole list`)
  assert.doesNotMatch(playTrack, /playAll\(filtered|= filtered(?:Tracks|Entries)\.value\s*$|filteredEntries\.value\.map/m, `${name}: queue must not be built from search results`)
}

console.log('search keeps full queue: ok')
