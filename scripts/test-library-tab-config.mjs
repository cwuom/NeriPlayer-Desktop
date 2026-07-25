import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const sourceUrl = new URL('../src/views/LibraryView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')

for (const key of ['local', 'favorites', 'downloads', 'netease']) {
  assert.match(source, new RegExp(`key: ['"]${key}['"]`), `missing public library tab: ${key}`)
}

assert.doesNotMatch(source, /key:\s*['"]netease_playlists['"]/, 'legacy NetEase playlist tab must not remain public')
assert.doesNotMatch(source, /key:\s*['"]netease_albums['"]/, 'legacy NetEase album tab must not remain public')
assert.doesNotMatch(source, /key:\s*['"]bilibili_favorites['"]/)
assert.doesNotMatch(source, /key:\s*['"]youtube_playlists['"]/)
for (const key of ['playlists', 'albums']) {
  assert.match(source, new RegExp(`key: ['"]${key}['"]`), `missing NetEase library category: ${key}`)
}
assert.match(source, /buildLibraryQuery/)
assert.match(source, /resolveLibraryLocation/)
assert.match(source, /const activeTab\s*=\s*ref<LibraryTabKey>/)
assert.doesNotMatch(source, /activeTab\s*===\s*['"]netease_playlists['"]/, 'legacy NetEase playlist branch must not remain')
assert.doesNotMatch(source, /activeTab\s*===\s*['"]netease_albums['"]/, 'legacy NetEase album branch must not remain')
assert.match(source, /grid-template-columns:\s*repeat\(4,\s*minmax\(0,\s*1fr\)\)/)
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
