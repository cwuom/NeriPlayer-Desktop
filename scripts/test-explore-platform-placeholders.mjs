import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const sourceUrl = new URL('../src/views/ExploreView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')

assert.doesNotMatch(source, /class="platform-hero bilibili"/)
assert.doesNotMatch(source, /class="platform-hero youtube"/)
assert.doesNotMatch(source, /explore\.bili_hint/)
assert.doesNotMatch(source, /explore\.yt_hint/)
assert.match(source, /type PlatformTab/)
assert.match(source, /const selectedPlatform = ref<PlatformTab>\(['"]netease['"]\)/)
assert.match(source, /v-for="platform in platformTabs"/)
assert.match(source, /@click="selectPlatform\(platform\.key\)"/)
assert.match(source, /function selectPlatform[\s\S]*?searchStore\.clear\(\)/)
assert.match(source, /searchStore\.search\(q, selectedPlatform\.value\)/)
assert.doesNotMatch(source, /searchStore\.search\(q, ['"]netease['"]\)/)
assert.doesNotMatch(source, /recommend\.fetch.*(?:Bili|Youtube)/i)

console.log('explore platform tab tests passed')
