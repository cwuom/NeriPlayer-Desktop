import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const read = async relative => readFile(new URL(relative, root), 'utf8')

const playlistSource = await read('src/views/NeteasePlaylistView.vue')
assert.match(
  playlistSource,
  /id:\s*'download'[\s\S]*?handleTrackDownload\(track\)/,
  'NetEase track menu must retain its download action',
)
assert.match(
  playlistSource,
  /async function handleTrackDownload\(track: TrackInfo\)[\s\S]*?downloadStore\.downloadTrack\(track\)/,
  'NetEase track download action must delegate to the download store',
)
assert.match(
  playlistSource,
  /const isPlaylistFavorited\s*=\s*computed\(/,
  'playlist detail must expose reactive favorite state',
)
assert.match(
  playlistSource,
  /<button\b[\s\S]*?isPlaylistFavorited[\s\S]*?favoritePlaylistLoading[\s\S]*?togglePlaylistFavorite/,
  'playlist detail must expose a disabled heart toggle',
)
assert.match(
  playlistSource,
  /invoke\(['"]add_favorite_playlist['"][\s\S]*?tracks:/,
  'favoriting must send the playlist metadata and tracks to Rust',
)
assert.match(
  playlistSource,
  /invoke\(['"]remove_favorite_playlist['"][\s\S]*?source:\s*['"]netease['"]/,
  'unfavoriting must identify the NetEase source',
)
assert.match(
  playlistSource,
  /function toBackendTrack\(track: TrackInfo\)[\s\S]*?duration_ms:[\s\S]*?source:[\s\S]*?cover_url:/,
  'favorite tracks must use the backend TrackInfo shape',
)
assert.match(
  playlistSource,
  /v-if="!props\.isAlbum"[\s\S]*?togglePlaylistFavorite/,
  'albums must not display the playlist favorite control',
)
assert.match(
  playlistSource,
  /async function loadPlaylistFavorites\([\s\S]*?list_favorite_playlists[\s\S]*?normalizeFavoritePlaylist/,
  'playlist detail must load its initial favorite state from the shared DTO normalizer',
)
assert.match(
  playlistSource,
  /onMounted\([\s\S]*?loadPlaylistFavorites\(/,
  'playlist detail must load favorite state when mounted',
)

const libraryCommandSource = await read('src-tauri/src/commands/library_cmd.rs')
assert.match(libraryCommandSource, /struct FavoritePlaylistInput/)
assert.match(libraryCommandSource, /pub async fn add_favorite_playlist\(/)
assert.match(libraryCommandSource, /pub async fn remove_favorite_playlist\(/)
assert.match(libraryCommandSource, /tracks_to_sync_songs_pub/)
assert.match(libraryCommandSource, /favorite-playlists-changed/)

const managerSource = await read('src-tauri/src/sync/manager.rs')
assert.match(
  managerSource,
  /pub fn save_favorite_playlists_pub\(favorites: Vec<SyncFavoritePlaylist>\)/,
  'favorite commands must use a shared persistence helper',
)

function functionSection(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(start, -1, `missing ${startMarker}`)
  assert.notEqual(end, -1, `missing ${endMarker}`)
  return source.slice(start, end)
}

for (const [startMarker, endMarker, label] of [
  ['pub async fn sync_github(', 'struct GitHubRemoteVersion', 'GitHub'],
  ['pub async fn sync_webdav(', 'pub fn build_local_sync_data(', 'WebDAV'],
]) {
  const section = functionSection(managerSource, startMarker, endMarker)
  const lockIndex = section.indexOf('acquire_sync_lock().await')
  const snapshotIndex = section.indexOf('build_local_sync_data(')
  assert.ok(lockIndex >= 0, `${label} sync must acquire the shared sync lock`)
  assert.ok(snapshotIndex > lockIndex, `${label} sync must build its local snapshot after acquiring the lock`)
}

const favoriteMutationSection = functionSection(
  managerSource,
  'pub async fn mutate_favorite_playlists_pub<T>(',
  'pub fn load_favorite_playlists_strict_pub(',
)
assert.ok(
  favoriteMutationSection.indexOf('acquire_sync_lock().await') < favoriteMutationSection.indexOf('FAVORITES_IO_LOCK'),
  'favorite mutations must share the sync lock before touching the favorites file',
)

const mainSource = await read('src-tauri/src/main.rs')
assert.match(mainSource, /library_cmd::add_favorite_playlist/)
assert.match(mainSource, /library_cmd::remove_favorite_playlist/)

const syncCommandSource = await read('src-tauri/src/commands/sync_cmd.rs')
for (const [startMarker, endMarker, label] of [
  ['pub async fn sync_github(', 'pub async fn disconnect_github_sync(', 'GitHub'],
  ['pub async fn sync_webdav(', 'pub async fn update_github_sync_settings(', 'WebDAV'],
]) {
  const section = functionSection(syncCommandSource, startMarker, endMarker)
  assert.match(section, /favorite-playlists-changed/, `${label} sync must refresh favorite playlists`)
}

const librarySource = await read('src/views/LibraryView.vue')
assert.match(
  librarySource,
  /normalizeFavoritePlaylist/,
  'LibraryView must normalize the Rust DTO through the shared helper',
)
assert.match(librarySource, /listen\(['"]favorite-playlists-changed['"]/, 'LibraryView must refresh favorite rows after mutations')
assert.match(librarySource, /unlistenFavoritePlaylistsChanged/, 'LibraryView must clean up the favorite event listener')
assert.doesNotMatch(
  librarySource,
  /onMounted\(loadFavorites\)/,
  'LibraryView must not start the initial favorite request before listeners are registered',
)
assert.match(
  librarySource,
  /listen\(['"]favorite-playlists-changed['"][\s\S]*?catch[\s\S]*?void loadFavorites\(\)/,
  'LibraryView must register the favorite listener before its initial favorite request',
)

const normalizerSource = await read('src/modules/library/favoritePlaylists.ts')
const compiledNormalizer = ts.transpileModule(normalizerSource, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const normalizerModule = await import(
  'data:text/javascript;base64,' + Buffer.from(compiledNormalizer).toString('base64'),
)

const camel = normalizerModule.normalizeFavoritePlaylist({
  id: 123,
  name: 'Cloud mix',
  coverUrl: 'https://cover.example/camel.jpg',
  trackCount: 4,
  source: 'netease',
  addedTime: 11,
  modifiedAt: 12,
  isDeleted: false,
  songs: [{ id: 'netease:1' }],
})
assert.deepEqual(camel, {
  id: '123',
  name: 'Cloud mix',
  coverUrl: 'https://cover.example/camel.jpg',
  trackCount: 4,
  source: 'netease',
  browseId: '',
  playlistId: '',
  subtitle: '',
  songs: [{ id: 'netease:1' }],
  addedTime: 11,
  modifiedAt: 12,
  isDeleted: false,
})

const snake = normalizerModule.normalizeFavoritePlaylist({
  id: '456',
  name: 'Legacy mix',
  cover_url: 'https://cover.example/snake.jpg',
  track_count: 2,
  source: 'netease',
  browse_id: 'VLPLlegacy',
  playlist_id: 'PLlegacy',
  subtitle: 'Legacy subtitle',
  added_time: 21,
  modified_at: 22,
  is_deleted: false,
  songs: [],
})
assert.equal(snake.coverUrl, 'https://cover.example/snake.jpg')
assert.equal(snake.trackCount, 2)
assert.equal(snake.browseId, 'VLPLlegacy')
assert.equal(snake.playlistId, 'PLlegacy')
assert.equal(snake.subtitle, 'Legacy subtitle')
assert.equal(snake.addedTime, 21)
assert.equal(snake.modifiedAt, 22)

console.log('netease playlist action tests passed')
