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
  /save_imported_playlists\(&sync_data\)/,
  'playlist-only imports must use persistence that preserves favorite playlists',
)
assert.equal(
  importPlaylistsSection.match(/save_imported_playlists\(&sync_data\)/g)?.length,
  2,
  'wrapped and bare SyncPlaylist imports must both preserve favorite playlists',
)
assert.doesNotMatch(
  importPlaylistsSection,
  /save_synced_playlists\(&sync_data\)/,
  'playlist-only imports must not invoke full-sync persistence',
)

const managerSource = await read('src-tauri/src/sync/manager.rs')
const playlistSaveSection = functionSection(managerSource, 'fn save_playlists(')
assert.doesNotMatch(
  playlistSaveSection,
  /favorite_playlists|save_favorite_playlists_pub/,
  'shared ordinary-playlist persistence must not read or write favorite playlists',
)
const importedPlaylistSaveSection = functionSection(
  managerSource,
  'pub fn save_imported_playlists(',
)
assert.doesNotMatch(
  importedPlaylistSaveSection,
  /favorite_playlists|save_favorite_playlists_pub/,
  'playlist-only persistence must not read or write favorite playlists',
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

console.log('favorite playlist tests passed')
