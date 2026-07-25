import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const sourceUrl = new URL('../src/views/LibraryView.vue', import.meta.url)
const [source, ...translations] = await Promise.all([
  readFile(sourceUrl, 'utf8'),
  ...['en', 'ja', 'zh-CN', 'zh-TW'].map(locale => (
    readFile(new URL(`../src/i18n/${locale}.json`, import.meta.url), 'utf8').then(JSON.parse)
  )),
])

for (const messages of translations) {
  assert.equal(typeof messages.common?.refresh, 'string', 'cloud refresh controls need a localized label')
  assert.ok(messages.common.refresh.length > 0)
}

for (const key of ['local', 'favorites', 'downloads', 'netease', 'bilibili', 'youtube']) {
  assert.match(source, new RegExp(`key: ['"]${key}['"]`), `missing public library tab: ${key}`)
}

assert.doesNotMatch(source, /key:\s*['"]netease_playlists['"]/, 'legacy NetEase playlist tab must not remain public')
assert.doesNotMatch(source, /key:\s*['"]netease_albums['"]/, 'legacy NetEase album tab must not remain public')
assert.match(source, /recommend\.fetchUserPlaylists\(platform\)/)
assert.match(source, /loadCloudLibrary\(['"]bilibili['"]\)/)
assert.match(source, /loadCloudLibrary\(['"]youtube['"]\)/)
assert.match(source, /activeTab\s*===\s*['"]bilibili['"]/)
assert.match(source, /activeTab\s*===\s*['"]youtube['"]/)
assert.match(source, /name:\s*['"]bili-playlist['"]\s*,\s*params:\s*\{\s*mediaId:/)
assert.match(source, /name:\s*['"]youtube-playlist['"]\s*,\s*params:\s*\{\s*browseId:/)
assert.match(
  source,
  /if \(!loaded && \(recommend\.userPlaylists\[platform\]\?\.length \|\| 0\) > 0\) \{[\s\S]*?toast\.error\(t\('player\.load_failed'\)\)/,
  'a failed refresh must remain visible while stale cloud rows are still shown',
)
for (const key of ['playlists', 'albums']) {
  assert.match(source, new RegExp(`key: ['"]${key}['"]`), `missing NetEase library category: ${key}`)
}
assert.match(source, /buildLibraryQuery/)
assert.match(source, /resolveLibraryLocation/)
assert.match(source, /const activeTab\s*=\s*ref<LibraryTabKey>/)
assert.doesNotMatch(source, /activeTab\s*===\s*['"]netease_playlists['"]/, 'legacy NetEase playlist branch must not remain')
assert.doesNotMatch(source, /activeTab\s*===\s*['"]netease_albums['"]/, 'legacy NetEase album branch must not remain')
assert.match(source, /grid-template-columns:\s*repeat\(6,\s*minmax\(0,\s*1fr\)\)/)
assert.match(
  source,
  /\.pl-count\s*\{[\s\S]*?overflow:\s*hidden;[\s\S]*?text-overflow:\s*ellipsis;[\s\S]*?white-space:\s*nowrap;/,
  'long cloud playlist descriptions must stay inside narrow rows',
)
assert.match(source, /role="tablist"\s*:aria-label="t\('library\.tab_netease'\)"/)
assert.match(source, /role="tab"[\s\S]*:aria-selected="neteaseCategory === category\.key"/)
assert.match(source, /:id="`netease-category-\$\{category\.key\}`"/)
assert.match(source, /aria-controls="netease-category-panel"/)
assert.match(source, /:tabindex="neteaseCategory === category\.key \? 0 : -1"/)
assert.match(source, /@keydown="handleNeteaseCategoryKeydown\(\$event, category\.key\)"/)
assert.match(source, /<template v-else>\s*<template v-if="neteaseCategory === 'playlists'">/)
assert.match(source, /<template v-else-if="neteaseCategory === 'albums'">/)
assert.doesNotMatch(source, /<div v-if="neteaseCategory === 'albums'"/, 'album content must remain in the shared NetEase state chain')
assert.match(
  source,
  /id="netease-category-panel"\s*role="tabpanel"\s*:aria-labelledby="`netease-category-\$\{neteaseCategory\}`"\s*tabindex="0"\s*class="netease-content"/,
)
for (const key of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End']) {
  assert.match(source, new RegExp(`['"]${key}['"]`), `missing NetEase category keyboard key: ${key}`)
}
assert.match(source, /function handleNeteaseCategoryKeydown\(/)
assert.match(source, /event\.preventDefault\(\)/)
assert.match(source, /activateNeteaseCategory\(nextCategory\)/)
assert.match(source, /document\.getElementById\(`netease-category-\$\{nextCategory\}`\)\?\.focus\(\)/)
assert.match(
  source,
  /activeTab\.value = location\.tab[\s\S]*if \(location\.tab === 'netease'\) \{\s*neteaseCategory\.value = location\.category/,
)

console.log('library tab config tests passed')
