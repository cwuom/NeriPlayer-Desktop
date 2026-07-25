import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const sourceUrl = new URL('../src/views/SettingsView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')

for (const key of ['personal', 'playback', 'listen_together', 'lyrics', 'effects', 'quality', 'storage', 'backup']) {
  assert.match(
    source,
    new RegExp(`is-collapsed.*isExpanded\\('${key}'\\)|isExpanded\\('${key}'\\).*is-collapsed`),
    `missing collapse state binding for ${key}`,
  )
}

assert.match(source, /\.settings-section-panel\.is-collapsed\s*>\s*\.section-label\.clickable\s*~/)
assert.match(source, /toggleSection\('effects'\)/)
assert.match(source, /is-collapsed.*isExpanded\('downloads'\)|isExpanded\('downloads'\).*is-collapsed/)
assert.match(source, /toggleSection\('downloads'\)/)

console.log('settings collapse structure tests passed')
