import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const sourceUrl = new URL('../src/views/ExploreView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')

assert.doesNotMatch(source, /class="platform-hero bilibili"/)
assert.doesNotMatch(source, /class="platform-hero youtube"/)
assert.doesNotMatch(source, /explore\.bili_hint/)
assert.doesNotMatch(source, /explore\.yt_hint/)
assert.doesNotMatch(source, /class="platform-tabs"/)
assert.doesNotMatch(source, /type PlatformTab/)
assert.match(source, /searchStore\.search\(q, ['"]netease['"]\)/)

console.log('explore platform placeholder tests passed')
