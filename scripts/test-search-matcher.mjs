import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const read = path => readFile(new URL(path, import.meta.url), 'utf8')

function load(source, dependencies = {}) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    if (name in dependencies) return dependencies[name]
    throw new Error(`Unexpected dependency: ${name}`)
  }, exports)
  return exports
}

const matcher = load(await read('../src/modules/search/textMatcher.ts'), { 'pinyin-pro': require('pinyin-pro') })
const { filterAndRank, searchMatches, searchScore, searchValue, trackSearchTokens } = matcher
const bindings = load(await read('../src/modules/shortcuts/bindings.ts'), {
  './platform': load(await read('../src/modules/shortcuts/platform.ts'), {
    '@/utils/platform': load(await read('../src/utils/platform.ts')),
  }),
})

let cases = 0
function check(name, run) {
  run()
  cases++
  console.log(`ok - ${name}`)
}

const tracks = [
  { title: '晴天', artist: '周杰伦', album: '叶惠美' },
  { title: '七里香', artist: '周杰伦', album: '七里香' },
  { title: 'Sunny Day', artist: 'Someone', album: 'Weather' },
  { title: '重庆森林', artist: '王菲', album: '' },
  { title: 'Love Story', artist: 'Taylor Swift', album: 'Fearless' },
  { title: '后来', artist: '刘若英', album: '我等你' },
]
const titles = query => filterAndRank(query, tracks, trackSearchTokens).map(track => track.title)

check('full pinyin, initials and Chinese text all match', () => {
  assert.deepEqual(titles('qingtian'), ['晴天'])
  assert.deepEqual(titles('qt'), ['晴天'])
  assert.deepEqual(titles('晴'), ['晴天'])
  assert.ok(titles('zjl').length === 2, 'artist initials match both Jay Chou songs')
})

check('title hits rank above artist and album hits', () => {
  // 「七里香」既是标题也是专辑，排在只有歌手命中的「晴天」前面
  assert.deepEqual(titles('周杰伦 七里香'), ['七里香'])
  assert.equal(titles('qilixiang')[0], '七里香')
})

check('polyphonic characters use context-aware readings', () => {
  assert.deepEqual(titles('chongqing'), ['重庆森林'])
  assert.deepEqual(titles('cqsl'), ['重庆森林'])
})

check('latin text supports prefix, acronym and loose subsequence', () => {
  assert.deepEqual(titles('sun'), ['Sunny Day'])
  assert.deepEqual(titles('ls'), ['Love Story'])
  assert.deepEqual(titles('lvstry'), ['Love Story'])
  assert.deepEqual(titles('taylor'), ['Love Story'])
  assert.deepEqual(titles('xyz'), [])
})

check('every query word must match and accents are folded', () => {
  assert.deepEqual(titles('love fearless'), ['Love Story'])
  assert.deepEqual(titles('love weather'), [])
  assert.ok(searchMatches('cafe', 'Café del Mar'))
})

check('scores order exact < prefix < contains and empty query keeps everything', () => {
  const exact = searchScore('love', ['love'])
  const prefix = searchScore('love', ['lovely'])
  const contains = searchScore('love', ['glove'])
  assert.ok(exact < prefix && prefix < contains)
  assert.equal(searchScore('  ', ['x']), 0)
  assert.equal(filterAndRank('', tracks, trackSearchTokens).length, tracks.length)
  assert.ok(searchScore('a', [searchValue('a', 10)]) > searchScore('a', ['a']))
})

check('shortcut bindings normalize, override and convert to accelerators', () => {
  const normalized = bindings.normalizeShortcutBindings({
    local: { next: 'shift+mod+arrowright', bogus: 'A', mute: '' },
    global: { search: 'Mod+Alt+F', toggle_play: 'Mod+Alt+Escape', like: 'Alt+Mod+K' },
  })
  assert.deepEqual(normalized.local, { next: 'Mod+Shift+ArrowRight', mute: '' }, 'case-insensitive names, unknown actions dropped')
  assert.equal(normalized.global.search, undefined, 'search cannot be global')
  assert.equal(normalized.global.toggle_play, undefined, 'Escape is reserved')
  assert.equal(normalized.global.like, 'Mod+Alt+K')
  const local = bindings.effectiveShortcuts(normalized, 'local')
  assert.equal(local.mute, '', 'explicit clear survives defaults')
  assert.equal(local.toggle_play, 'Space')
  assert.equal(local.show_window, '', 'global-only actions are never local')
  const reset = bindings.withShortcut(normalized, 'global', 'like', bindings.DEFAULT_GLOBAL_SHORTCUTS.like)
  assert.equal('like' in reset.global, false, 'default values are not stored')
  assert.equal(bindings.toAccelerator('Mod+Alt+ArrowRight'), 'CommandOrControl+Alt+ArrowRight')
  assert.equal(bindings.toAccelerator('Mod+Shift+='), 'CommandOrControl+Shift+Equal')
  assert.equal(bindings.toAccelerator('Mod+MediaPlayPause'), null)
  const conflicts = bindings.conflictingActions({ ...local, like: 'Space' })
  assert.deepEqual(conflicts.get('Space'), ['toggle_play', 'like'])
})

console.log(`Search matcher regressions: ${cases} passed`)
