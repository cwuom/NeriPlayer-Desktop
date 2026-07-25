import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const read = relative => readFile(new URL(relative, root), 'utf8')

function functionSection(source, signature) {
  const start = source.indexOf(signature)
  assert.notEqual(start, -1, `missing ${signature}`)

  const openingBrace = source.indexOf('{', start + signature.length)
  assert.notEqual(openingBrace, -1, `missing body for ${signature}`)

  let depth = 0
  for (let index = openingBrace; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1
    if (source[index] === '}') depth -= 1
    if (depth === 0) return source.slice(start, index + 1)
  }

  assert.fail(`unterminated body for ${signature}`)
}

const syncCommandSource = await read('src-tauri/src/commands/sync_cmd.rs')
const importPlaylistsSection = functionSection(
  syncCommandSource,
  'pub async fn import_playlists(',
)
assert.match(
  importPlaylistsSection,
  /save_imported_playlist_backup\(&sync_data\)\.await/,
  'full sync snapshots must import ordinary playlists and online favorites together',
)
assert.equal(
  importPlaylistsSection.match(/save_imported_playlist_backup\(&sync_data\)\.await/g)?.length,
  2,
  'binary and wrapped JSON snapshots must use transactional backup persistence',
)
assert.doesNotMatch(
  importPlaylistsSection,
  /save_synced_playlists\(&sync_data\)/,
  'playlist-only imports must not invoke full-sync persistence',
)
assert.match(
  importPlaylistsSection,
  /save_imported_playlists\(&sync_data\)\.await/,
  'bare SyncPlaylist arrays must use locked ordinary-playlist persistence',
)
assert.match(
  importPlaylistsSection,
  /favorite-playlists-changed/,
  'favorite playlist imports must refresh the library favorite tab',
)

const managerSource = await read('src-tauri/src/sync/manager.rs')
const playlistSaveSection = functionSection(managerSource, 'fn save_playlists(')
assert.doesNotMatch(
  playlistSaveSection,
  /favorite_playlists|save_favorite_playlists_pub/,
  'shared ordinary-playlist persistence must not read or write favorite playlists',
)
assert.match(
  playlistSaveSection,
  /acquire_playlist_io_lock\(\)/,
  'sync publication must share the ordinary-playlist mutation lock with user actions',
)
const importedPlaylistSaveSection = functionSection(
  managerSource,
  'pub async fn save_imported_playlists(',
)
assert.doesNotMatch(
  importedPlaylistSaveSection,
  /favorite_playlists|save_favorite_playlists_pub/,
  'playlist-only persistence must not read or write favorite playlists',
)
assert.match(
  importedPlaylistSaveSection,
  /acquire_sync_lock\(\)\.await[\s\S]*?acquire_playlist_io_lock\(\)[\s\S]*?merge_imported_playlists_unlocked\(imported\)/,
  'playlist-only import must hold the sync and playlist locks in order',
)

const importedDesktopPlaylistSaveSection = functionSection(
  managerSource,
  'pub async fn save_imported_desktop_playlists(',
)
assert.match(
  importedDesktopPlaylistSaveSection,
  /acquire_sync_lock\(\)\.await[\s\S]*?acquire_playlist_io_lock\(\)[\s\S]*?PlaylistStore::load_strict/,
  'legacy desktop playlist imports must use the same ordered locks',
)

const importedPlaylistPersistSection = functionSection(
  managerSource,
  'fn merge_imported_playlists_unlocked(',
)
assert.match(
  importedPlaylistPersistSection,
  /PlaylistStore::load_strict\(&path\)[\s\S]*?merge_imported_playlists_into_store\(&mut store, imported\)[\s\S]*?store\.save\(&path\)/,
  'playlist import must reject a damaged local store and atomically persist the merge',
)

const importedPlaylistMergeSection = functionSection(
  managerSource,
  'fn merge_imported_playlists_into_store(',
)
assert.match(
  importedPlaylistMergeSection,
  /for imported_playlist in imported\s*\.playlists/,
  'playlist import must process only playlists supplied by the selected backup',
)
assert.match(
  importedPlaylistMergeSection,
  /store\.playlists\.iter\(\)\.position[\s\S]*?store\.playlists\.push\(/,
  'playlist import must update matches or append new playlists to the existing store',
)
assert.doesNotMatch(
  importedPlaylistMergeSection,
  /store\.playlists\s*=|std::mem::take\(&mut store\.playlists\)/,
  'playlist import must not replace or drain desktop-only playlists',
)

const desktopOnlyFixture = [
  { id: 10, name: 'Desktop only' },
  { id: 20, name: 'Shared' },
]
const phoneFixture = [
  { id: 20, name: 'Shared' },
  { id: 30, name: 'Phone only' },
]
const mergedFixture = [...desktopOnlyFixture]
for (const imported of phoneFixture) {
  if (!mergedFixture.some(saved => saved.id === imported.id || saved.name === imported.name)) {
    mergedFixture.push(imported)
  }
}
assert.deepEqual(
  mergedFixture.map(playlist => playlist.name),
  ['Desktop only', 'Shared', 'Phone only'],
  'the import merge contract must retain playlists that exist only on desktop',
)

const importedFavoriteMergeSection = functionSection(
  managerSource,
  'fn merge_imported_favorite_playlists(',
)
assert.match(
  importedFavoriteMergeSection,
  /filter\(\|favorite\| !favorite\.is_deleted\)/,
  'manual favorite import must ignore phone tombstones',
)
assert.match(
  importedFavoriteMergeSection,
  /deleted_local[\s\S]*?saturating_add\(1\)[\s\S]*?favorite\.is_deleted = false/,
  'an active imported favorite must be able to restore a local tombstone',
)
assert.match(
  importedFavoriteMergeSection,
  /three_way_merge/,
  'favorite imports must merge with existing favorites instead of replacing them',
)

const importedBackupSection = functionSection(
  managerSource,
  'pub async fn save_imported_playlist_backup(',
)
assert.match(
  importedBackupSection,
  /acquire_sync_lock\(\)\.await[\s\S]*?acquire_playlist_io_lock\(\)[\s\S]*?FAVORITES_IO_LOCK/,
  'full backup import must hold sync, playlist, and favorites locks in order',
)
assert.match(
  importedBackupSection,
  /let original_store = PlaylistStore::load_strict\(&path\)[\s\S]*?merged_store\.save\(&path\)[\s\S]*?original_store\.save\(&path\)/,
  'full backup import must roll ordinary playlists back if favorite persistence fails',
)
assert.match(
  importedBackupSection,
  /let original_favorites = load_favorite_playlists_unlocked\(\)\?[\s\S]*?original_favorites\.clone\(\)[\s\S]*?save_favorite_playlists_unlocked\(original_favorites\)/,
  'full backup import must retain and restore the original favorites snapshot on publication failure',
)

const syncedPlaylistSaveSection = functionSection(
  managerSource,
  'pub fn save_synced_playlists(',
)
assert.match(
  syncedPlaylistSaveSection,
  /save_favorite_playlists_pub\(merged\.favorite_playlists\.clone\(\)\)/,
  'full sync must continue to persist its merged favorite snapshot',
)

const atomicFavoriteWriteSection = functionSection(
  managerSource,
  'fn atomic_write_favorites_file_with_publish(',
)
assert.match(
  atomicFavoriteWriteSection,
  /tempfile::Builder::new\(\)[\s\S]*?\.tempfile_in\(parent\)/,
  'favorite writes must stage in a sibling temporary file so publication stays on one filesystem',
)
const favoriteTempSync = atomicFavoriteWriteSection.indexOf(
  'temporary.as_file().sync_all()',
)
const favoritePublish = atomicFavoriteWriteSection.indexOf(
  'publish(temporary, path)',
)
assert.notEqual(
  favoriteTempSync,
  -1,
  'favorite writes must fsync the complete temporary file before publication',
)
assert.ok(
  favoritePublish > favoriteTempSync,
  'favorite writes must publish only after the temporary file is fsynced',
)

const atomicFavoritePublishSection = functionSection(
  managerSource,
  'fn atomic_write_favorites_file(',
)
assert.match(
  atomicFavoritePublishSection,
  /temporary[\s\S]*?\.persist\(path\)/,
  'favorite writes must atomically persist the prepared sibling over the destination',
)

const favoriteSaveSection = functionSection(
  managerSource,
  'fn save_favorite_playlists_unlocked(',
)
assert.match(
  favoriteSaveSection,
  /atomic_write_favorites_file\(&path, content\.as_bytes\(\)\)/,
  'locked favorite persistence must use the crash-safe atomic writer',
)
assert.doesNotMatch(
  favoriteSaveSection,
  /std::fs::write\(&path/,
  'favorite persistence must not truncate the last valid snapshot in place',
)

const favoriteModuleSource = await read('src/modules/library/favoritePlaylists.ts')
const playlistStoreSource = await read('src-tauri/src/library/playlist.rs')
const libraryCommandSource = await read('src-tauri/src/commands/library_cmd.rs')
assert.match(
  playlistStoreSource,
  /static PLAYLIST_IO_LOCK:\s*OnceLock<Mutex<\(\)>>/,
  'ordinary playlist mutations must share one process-wide storage lock',
)
const playlistLockSection = functionSection(
  playlistStoreSource,
  'pub fn acquire_playlist_io_lock(',
)
assert.match(
  playlistLockSection,
  /PLAYLIST_IO_LOCK[\s\S]*?\.lock\(\)[\s\S]*?Playlist storage lock poisoned/,
  'ordinary playlist storage lock poisoning must be returned as an application error',
)
for (const signature of [
  'fn list_playlists_blocking(',
  'pub async fn create_playlist(',
  'pub async fn delete_playlist(',
  'pub async fn rename_playlist(',
  'pub async fn add_to_playlist(',
  'pub async fn add_tracks_to_playlist(',
  'pub async fn update_playlist_track(',
  'pub async fn remove_from_playlist(',
  'pub async fn remove_tracks_from_playlist(',
  'pub async fn reorder_playlist_tracks(',
]) {
  const mutationSection = functionSection(libraryCommandSource, signature)
  assert.match(
    mutationSection,
    /acquire_playlist_io_lock\(\)\?[\s\S]*?PlaylistStore::load\(&path\)[\s\S]*?store\.save\(&path\)/,
    `${signature} must hold the ordinary playlist lock across load and save`,
  )
}
const strictPlaylistLoad = functionSection(playlistStoreSource, 'pub fn load_strict(')
assert.match(
  strictPlaylistLoad,
  /ErrorKind::NotFound[\s\S]*?Self::default\(\)/,
  'strict playlist loading may default only when the file does not exist',
)
const atomicPlaylistSave = functionSection(playlistStoreSource, 'pub fn save(')
assert.match(
  atomicPlaylistSave,
  /tempfile::NamedTempFile::new_in\(parent\)[\s\S]*?sync_all\(\)[\s\S]*?persist\(path\)/,
  'ordinary playlist persistence must publish a synced sibling temporary file atomically',
)
const compiledFavoriteModule = ts.transpileModule(favoriteModuleSource, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const favoriteModule = await import(
  `data:text/javascript;base64,${Buffer.from(compiledFavoriteModule).toString('base64')}`
)

assert.equal(
  typeof favoriteModule.favoritePlaylistLocation,
  'function',
  'favorite rows must expose a route resolver',
)
assert.equal(
  typeof favoriteModule.favoritePlaylistKey,
  'function',
  'favorite rows must expose a source-aware key',
)

const favorite = (source, id = '42', fields = {}) => ({
  id,
  name: 'Favorite',
  coverUrl: '',
  trackCount: 0,
  source,
  browseId: '',
  playlistId: '',
  subtitle: '',
  songs: [],
  addedTime: 0,
  modifiedAt: 0,
  isDeleted: false,
  ...fields,
})

assert.notEqual(
  favoriteModule.favoritePlaylistKey(favorite('netease')),
  favoriteModule.favoritePlaylistKey(favorite('bilibili')),
  'favorite row keys must distinguish equal IDs from different sources',
)
assert.deepEqual(
  favoriteModule.favoritePlaylistLocation(favorite('netease')),
  { name: 'netease-playlist', params: { id: '42' } },
)
assert.deepEqual(
  favoriteModule.favoritePlaylistLocation(favorite('bilibili')),
  { name: 'bili-playlist', params: { mediaId: '42' } },
)
assert.deepEqual(
  favoriteModule.favoritePlaylistLocation(favorite('bili')),
  { name: 'bili-playlist', params: { mediaId: '42' } },
  'Android Bilibili favorites must use the existing desktop detail route',
)
assert.deepEqual(
  favoriteModule.favoritePlaylistLocation(favorite('youtube', '42', { browseId: 'VLPLabc' })),
  { name: 'youtube-playlist', params: { browseId: 'VLPLabc' } },
)
assert.deepEqual(
  favoriteModule.favoritePlaylistLocation(favorite('youtubeMusic', '42', { playlistId: 'PLabc' })),
  { name: 'youtube-playlist', params: { browseId: 'VLPLabc' } },
  'Android YouTube Music favorites must derive a browse ID from playlistId',
)
assert.equal(
  favoriteModule.favoritePlaylistLocation(favorite('youtube')),
  null,
  'a hashed YouTube favorite ID is not a usable browse ID',
)
assert.equal(
  favoriteModule.favoritePlaylistLocation(favorite('unknown')),
  null,
  'unknown sources must not invent a detail page',
)

const normalizedYoutube = favoriteModule.normalizeFavoritePlaylist({
  id: 42,
  source: 'youtubeMusic',
  browseId: 'VLPLabc',
  playlistId: 'PLabc',
  subtitle: 'Mix',
})
assert.equal(normalizedYoutube.browseId, 'VLPLabc')
assert.equal(normalizedYoutube.playlistId, 'PLabc')
assert.equal(normalizedYoutube.subtitle, 'Mix')

const mainSource = await read('src/main.ts')
for (const route of [
  /path:\s*'\/playlist\/netease\/:id',\s*name:\s*'netease-playlist'/,
  /path:\s*'\/playlist\/bilibili\/:mediaId',\s*name:\s*'bili-playlist'/,
  /path:\s*'\/playlist\/youtube\/:browseId',\s*name:\s*'youtube-playlist'/,
]) {
  assert.match(
    mainSource,
    route,
    'favorite locations must target an existing detail route and parameter',
  )
}

const librarySource = await read('src/views/LibraryView.vue')
const likedSongsSource = await read('src/stores/likedSongs.ts')
assert.match(
  librarySource,
  /v-for="row in favoritePlaylistRows"[\s\S]*?:key="row\.key"/,
  'favorite rows must render from source-aware row metadata',
)
assert.match(
  librarySource,
  /:is="row\.location \? RouterLink : 'div'"[\s\S]*?:to="row\.location \?\? undefined"/,
  'known favorite sources must render as links while unknown sources remain plain rows',
)
assert.match(
  librarySource,
  /SYSTEM_LIKED_PLAYLIST_ID\s*=\s*-1001[\s\S]*?SYSTEM_LOCAL_PLAYLIST_ID\s*=\s*-1002/,
  'the desktop library must recognize Android system playlist IDs',
)
assert.match(
  librarySource,
  /function isProtectedPlaylist\([\s\S]*?isLikedPlaylist\(pl\)\s*\|\|\s*isLocalFilesPlaylist\(pl\)/,
  'Android system playlists must remain protected from desktop playlist actions',
)
assert.match(
  librarySource,
  /const localPlaylist = raw\.find\(isLocalFilesPlaylist\)/,
  'local scanning must reuse an imported Android local-files playlist',
)
assert.match(
  likedSongsSource,
  /p\.id === SYSTEM_LIKED_PLAYLIST_ID \|\| LIKED_PLAYLIST_NAMES\.includes\(p\.name\)/,
  'liked songs must hydrate from an imported Android favorites playlist by stable ID',
)
assert.match(
  libraryCommandSource,
  /fn is_system_playlist_id\([\s\S]*?SYSTEM_FAVORITES_PLAYLIST_ID \| SYSTEM_LOCAL_FILES_PLAYLIST_ID/,
  'the backend must identify Android system playlists before rename or deletion',
)

console.log('favorite playlist tests passed')
