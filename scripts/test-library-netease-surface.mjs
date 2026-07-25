import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const localeFiles = ['en', 'ja', 'zh-CN', 'zh-TW']
const requiredLocaleKeys = [
  'tab_netease',
  'netease_category_playlists',
  'netease_category_albums',
  'netease_search_placeholder',
  'netease_search_empty_title',
  'netease_search_empty_desc',
  'netease_playlist_empty',
  'netease_album_empty',
  'netease_play_count',
  'netease_login',
]

for (const locale of localeFiles) {
  const localeUrl = new URL(`../src/i18n/${locale}.json`, import.meta.url)
  const messages = JSON.parse(await readFile(localeUrl, 'utf8'))

  for (const key of requiredLocaleKeys) {
    const value = messages.library?.[key]
    assert.equal(
      typeof value,
      'string',
      `${locale} is missing library.${key}`,
    )
    assert.notEqual(value.trim(), '', `${locale} has an empty library.${key}`)
  }
}

const sourceUrl = new URL('../src/views/LibraryView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')

function sliceBetween(value, startMarker, endMarker, label) {
  const start = value.indexOf(startMarker)
  assert.notEqual(start, -1, 'missing ' + label + ' start')
  const end = value.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(end, -1, 'missing ' + label + ' end')
  return value.slice(start, end)
}

function requiredMatch(value, pattern, message) {
  const match = value.match(pattern)
  assert.ok(match, message)
  return match[0]
}

assert.match(
  source,
  /import\s*\{\s*filterNeteaseAlbums,\s*filterNeteasePlaylists\s*\}\s*from\s*['"]@\/modules\/library\/neteaseLibraryFilter['"]/,
)
assert.match(source, /const neteasePlaylistSearchQuery = ref\(''\)/)
assert.match(source, /const neteaseAlbumSearchQuery = ref\(''\)/)
assert.match(
  source,
  /const neteaseFilteredPlaylists = computed\(\(\) => filterNeteasePlaylists\(neteasePlaylists\.value, neteasePlaylistSearchQuery\.value\)\)/,
)
assert.match(
  source,
  /const neteaseFilteredAlbums = computed\(\(\) => filterNeteaseAlbums\(recommend\.userAlbums, neteaseAlbumSearchQuery\.value\)\)/,
)

const neteasePanel = sliceBetween(
  source,
  'id="netease-category-panel"',
  '\n    <ContextMenu',
  'NetEase category panel',
)
const playlistBranch = sliceBetween(
  neteasePanel,
  '<template v-if="neteaseCategory === \'playlists\'">',
  '<!-- Tab: 网易云-专辑 -->',
  'NetEase playlist branch',
)
const albumBranchStart = neteasePanel.indexOf('<template v-else-if="neteaseCategory === \'albums\'">')
assert.notEqual(albumBranchStart, -1, 'missing NetEase album branch')
const albumBranch = neteasePanel.slice(albumBranchStart)

const playlistLink = requiredMatch(
  playlistBranch,
  /<RouterLink\b(?=[^>]*v-for="npl in neteaseFilteredPlaylists")(?=[^>]*class="playlist-item netease-result-link")(?=[^>]*:to="\{ name: 'netease-playlist', params: \{ id: npl\.id \} \}")[^>]*>[\s\S]*?<\/RouterLink>/,
  'NetEase playlist results must be semantic detail links',
)
const albumLink = requiredMatch(
  albumBranch,
  /<RouterLink\b(?=[^>]*v-for="album in neteaseFilteredAlbums")(?=[^>]*class="playlist-item netease-result-link")(?=[^>]*:to="\{ name: 'netease-album', params: \{ id: album\.id \} \}")[^>]*>[\s\S]*?<\/RouterLink>/,
  'NetEase album results must be semantic detail links',
)
assert.doesNotMatch(neteasePanel, /@click="router\.push\(\{ name: 'netease-(?:playlist|album)'/)

const tools = sliceBetween(
  neteasePanel,
  '<div v-if="auth.netease.loggedIn" class="netease-tools">',
  '<div v-if="!auth.netease.loggedIn"',
  'logged-in NetEase tools',
)
assert.match(
  tools,
  /<input(?=[^>]*v-model="activeNeteaseSearchQuery")(?=[^>]*type="search")(?=[^>]*:placeholder="t\('library\.netease_search_placeholder'\)")[^>]*\/>/,
)
const refreshButton = requiredMatch(
  tools,
  /<button\b(?=[^>]*class="netease-refresh")(?=[^>]*:disabled="neteasePlaylistLoading \|\| neteaseAlbumLoading")(?=[^>]*:aria-label="t\('common\.refresh'\)")(?=[^>]*:title="t\('common\.refresh'\)")(?=[^>]*@click="loadNeteaseLibrary")[^>]*>[\s\S]*?<\/button>/,
  'NetEase refresh must expose coordinated loading and accessible refresh behavior',
)
assert.match(
  refreshButton,
  /<span\b(?=[^>]*class="material-symbols-rounded")(?=[^>]*:class="\{ spinning: neteasePlaylistLoading \|\| neteaseAlbumLoading \}")(?=[^>]*aria-hidden="true")[^>]*>refresh<\/span>/,
)

const loggedOut = sliceBetween(
  neteasePanel,
  '<div v-if="!auth.netease.loggedIn"',
  '<template v-else>',
  'logged-out NetEase state',
)
assert.match(loggedOut, /<p class="empty-title">\{\{ t\('library\.netease_login'\) \}\}<\/p>/)
assert.match(loggedOut, /<p class="empty-desc">\{\{ t\('explore\.login_for_recommend'\) \}\}<\/p>/)
const loginButton = requiredMatch(
  loggedOut,
  /<button\b(?=[^>]*:disabled="auth\.loggingIn === 'netease'")(?=[^>]*@click="auth\.loginNetease")[^>]*>[\s\S]*?<\/button>/,
  'logged-out NetEase state must provide the real login action',
)
assert.match(loginButton, /<span\b(?=[^>]*class="material-symbols-rounded")(?=[^>]*aria-hidden="true")[^>]*>login<\/span>/)

assert.match(
  playlistBranch,
  /v-else-if="neteasePlaylists\.length === 0"[\s\S]*?library\.netease_playlist_empty[\s\S]*?v-else-if="neteasePlaylists\.length > 0 && neteaseFilteredPlaylists\.length === 0"[\s\S]*?library\.netease_search_empty_title[\s\S]*?library\.netease_search_empty_desc/,
)
assert.match(
  albumBranch,
  /v-else-if="recommend\.userAlbums\.length === 0"[\s\S]*?library\.netease_album_empty[\s\S]*?v-else-if="recommend\.userAlbums\.length > 0 && neteaseFilteredAlbums\.length === 0"[\s\S]*?library\.netease_search_empty_title[\s\S]*?library\.netease_search_empty_desc/,
)
assert.match(
  playlistLink,
  /<div class="pl-count">[\s\S]*?library\.netease_play_count[\s\S]*?npl\.playCount \|\| 0[\s\S]*?aria-hidden="true">·<\/span>[\s\S]*?library\.track_count[\s\S]*?npl\.trackCount \|\| 0[\s\S]*?<\/div>/,
)
assert.match(
  albumLink,
  /<div class="pl-count">[\s\S]*?<template v-if="album\.artist">[\s\S]*?album\.artist[\s\S]*?aria-hidden="true">·<\/span>[\s\S]*?<\/template>[\s\S]*?player\.track_count[\s\S]*?album\.trackCount \|\| 0[\s\S]*?<\/div>/,
)

const decorativeIcons = neteasePanel.match(/<span\b(?=[^>]*class="material-symbols-rounded[^"]*")[^>]*>/g) || []
assert.ok(decorativeIcons.length > 0, 'NetEase panel must contain its expected Material Symbols')
for (const icon of decorativeIcons) {
  assert.match(icon, /aria-hidden="true"/, 'decorative NetEase Material Symbol must be hidden from assistive technology')
}
assert.match(
  source,
  /\.netease-result-link\s*\{[\s\S]*?&:focus-visible\s*\{[\s\S]*?outline:/,
  'NetEase result links need an explicit keyboard focus outline',
)

console.log('library NetEase surface tests passed')
