import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import ts from 'typescript'
import * as Vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'

const source = await readFile(new URL('../src/modules/library/localArtists.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  transformers: {
    before: [context => file => ts.visitNode(file, function visit(node) {
      if (ts.isImportDeclaration(node)) return undefined
      return ts.visitEachChild(node, visit, context)
    })],
  },
}).outputText
const { splitArtistNames, groupLocalArtists, sortLocalArtists, filterLocalArtists, localArtistStableKey } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

assert.deepEqual(splitArtistNames(' Artist / artist; Guest、GUEST & Artist '), ['Artist', 'Guest'])
for (const separator of ['feat.', 'feat', 'ft.', 'FT', 'vs.', 'vs']) {
  assert.deepEqual(splitArtistNames(`Artist ${separator} Guest`), ['Artist', 'Guest'])
}
assert.deepEqual(splitArtistNames('Feather / Daft Punk'), ['Feather', 'Daft Punk'])
assert.deepEqual(splitArtistNames('  '), [])

const tracks = [
  { id: '1', title: 'First', artist: 'Zulu / zulu feat. Guest', album: 'First Album', coverUrl: '', addedAt: 1 },
  { id: '2', title: 'Second', artist: 'Zulu', album: 'Second Album', coverUrl: 'cover-zulu', addedAt: 3 },
  { id: '3', title: 'Third', artist: 'Alpha / Beta', album: 'Shared Album', coverUrl: 'cover-alpha', addedAt: 2 },
  { id: '4', title: 'Unknown song', artist: '', album: '', coverUrl: '', addedAt: 4 },
]
const artists = groupLocalArtists(tracks, '未知歌手')
const zulu = artists.find(artist => artist.key === 'zulu')
assert.deepEqual(zulu.tracks.map(track => track.id), ['1', '2'], 'one song counts once for the same artist')
assert.equal(zulu.coverUrl, 'cover-zulu')
assert.equal(zulu.latestAddedAt, 3)
assert.deepEqual(artists.find(artist => artist.key === 'guest').tracks.map(track => track.id), ['1'])
assert.deepEqual(artists.find(artist => artist.key === '未知歌手').tracks.map(track => track.id), ['4'])

const originalOrder = artists.map(artist => artist.key)
const countSorted = sortLocalArtists(artists, 'song_count')
assert.equal(countSorted[0].key, 'zulu', 'artists with more songs appear first')
assert.deepEqual(countSorted.slice(1).map(artist => artist.key),
  sortLocalArtists(artists.filter(artist => artist.key !== 'zulu'), 'name').map(artist => artist.key),
  'same counts are ordered by name')
assert.deepEqual(artists.map(artist => artist.key), originalOrder, 'sorting leaves the source array intact')
assert.equal(sortLocalArtists(artists, 'recent')[0].key, '未知歌手')
assert.equal(sortLocalArtists(artists.filter(artist => artist.key !== '未知歌手'), 'name')[0].key, 'alpha')
assert.deepEqual(filterLocalArtists(countSorted, ' SHARED ALBUM ').map(artist => artist.key), ['alpha', 'beta'])
assert.deepEqual(filterLocalArtists(countSorted, 'first').map(artist => artist.key), ['zulu', 'guest'])
assert.deepEqual(filterLocalArtists(countSorted, '').map(artist => artist.key), countSorted.map(artist => artist.key))
console.log('local artist grouping and sorting tests passed')

const renderer = Vue.createRenderer({
  createElement: type => ({ type, children: [], props: {}, text: '' }),
  createText: text => ({ type: '#text', children: [], text }),
  createComment: text => ({ type: '#comment', children: [], text }),
  setText: (node, text) => { node.text = text },
  setElementText: (node, text) => { node.children = []; node.text = text },
  parentNode: node => node.parent,
  nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] || null,
  patchProp: (node, key, _previous, value) => { node.props[key] = value },
  insert(node, parent, anchor = null) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    parent.children.splice(index < 0 ? parent.children.length : index, 0, node)
  },
  remove(node) {
    node.parent?.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = null
  },
})
function allNodes(node) { return [node, ...node.children.flatMap(allNodes)] }
function byClass(root, name) {
  return allNodes(root).filter(node => String(node.props?.class || '').split(/\s+/).includes(name))
}
function textContent(node) {
  return (node.type === '#comment' ? '' : node.text || '') + node.children.map(textContent).join('')
}
async function settle() { await Vue.nextTick(); await setImmediate(); await Vue.nextTick() }
const artistTracks = [
  ...Array.from({ length: 120 }, (_, index) => ({ id: `song-${index}`, title: `Song ${index}`, artist: '鹿乃', album: 'Album', durationMs: 60000 })),
  { id: 'alpha', title: 'Needle Alpha', artist: '鹿乃 / Guest Vocalist', album: 'Special Record', durationMs: 60000 },
  { id: 'beta', title: 'Needle Beta', artist: '鹿乃', album: '', durationMs: 60000 },
  { id: 'japanese', title: 'ピエロ', artist: '鹿乃', album: '和音', durationMs: 60000 },
]
const viewSource = await readFile(new URL('../src/views/LocalArtistView.vue', import.meta.url), 'utf8')
const viewContent = compileScript(parse(viewSource).descriptor, { id: 'local-artist-search-test', inlineTemplate: true }).content
const viewScript = ts.createSourceFile('artist.ts', viewContent, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
const withoutImports = ts.factory.updateSourceFile(viewScript, viewScript.statements.filter(node => !ts.isImportDeclaration(node)))
const viewCompiled = ts.transpileModule(ts.createPrinter().printFile(withoutImports), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText

async function mountArtist(data = artistTracks) {
  const played = [], shuffled = [], opened = []
  const imports = {
    useRoute: () => ({ params: { name: '鹿乃' } }), useRouter: () => ({ back() {} }),
    useI18n: () => ({ t: key => key }),
    usePlayerStore: () => ({ currentTrack: null, isPlaying: false, playAll: (...args) => played.push(args), shufflePlay: queue => shuffled.push(queue) }),
    groupLocalArtists, localArtistStableKey, loadArtistSourceTracks: async () => data,
    createLogger: () => ({ error() {} }), formatTrackDuration: () => '1:00',
    BilibiliCoverImage: Vue.defineComponent({ setup: () => () => Vue.h('img') }),
    recordPlaylistOpen: open => opened.push(open),
  }
  const dependencies = { exports: {} }
  for (const statement of viewScript.statements) {
    if (!ts.isImportDeclaration(statement)) continue
    const clause = statement.importClause
    if (clause?.isTypeOnly) continue
    const fromVue = statement.moduleSpecifier.text === 'vue'
    if (clause?.name) dependencies[clause.name.text] = imports[clause.name.text]
    for (const item of clause?.namedBindings?.elements || []) {
      if (item.isTypeOnly) continue
      const name = item.propertyName?.text || item.name.text
      dependencies[item.name.text] = fromVue ? Vue[name] : imports[name]
    }
  }
  // DOM 输入指令由浏览器验证，这里执行真实组件渲染和事件处理
  dependencies._vModelText = {}
  new Function(...Object.keys(dependencies), viewCompiled)(...Object.values(dependencies))
  const root = { children: [] }
  const app = renderer.createApp(dependencies.exports.default)
  app.mount(root)
  await settle()
  return {
    root, played, shuffled, opened,
    titles: () => byClass(root, 'track-title').map(textContent),
    async search(value) {
      const input = allNodes(root).find(node => node.type === 'input')
      assert.ok(input, 'local artist page must expose a search input')
      assert.equal(input.props['aria-label'], 'player.search_tracks')
      input.props['onUpdate:modelValue'](value)
      await settle()
    },
    stop: () => app.unmount(),
  }
}
let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}

await test('local artist search matches all loaded titles, artists and albums', async () => {
  const page = await mountArtist()
  try {
    await page.search('  nEeDlE  ')
    assert.deepEqual(page.titles(), ['Needle Alpha', 'Needle Beta'])
    await page.search('GUEST VOCALIST')
    assert.deepEqual(page.titles(), ['Needle Alpha'])
    await page.search('special record')
    assert.deepEqual(page.titles(), ['Needle Alpha'])
    await page.search('ピエロ')
    assert.deepEqual(page.titles(), ['ピエロ'])
    await page.search('  ')
    assert.equal(page.titles().length, 123)
  } finally { page.stop() }
})

await test('opening a local artist records one open for Continue playing', async () => {
  const page = await mountArtist()
  try {
    await page.search('needle')
    assert.equal(page.opened.length, 1, 'searching is not another open')
    assert.equal(page.opened[0].source, 'localArtist')
    assert.equal(page.opened[0].name, '鹿乃')
    assert.equal(page.opened[0].trackCount, 123)
  } finally { page.stop() }
})
await test('filtered rows play the selected song while bulk actions retain the complete artist queue', async () => {
  const page = await mountArtist()
  try {
    await page.search('needle')
    byClass(page.root, 'track-item')[1].props.onClick()
    assert.deepEqual(page.played[0][0].map(track => track.id), ['alpha', 'beta'])
    assert.equal(page.played[0][1], 'beta', 'filtered index must not select an unrelated original track')
    byClass(page.root, 'primary-action')[0].props.onClick()
    byClass(page.root, 'secondary-action')[0].props.onClick()
    assert.equal(page.played[1][0].length, 123)
    assert.equal(page.shuffled[0].length, 123)
  } finally { page.stop() }
})
await test('unmatched search shows a search-empty state and clearing or Escape restores all songs', async () => {
  const page = await mountArtist()
  try {
    await page.search('no matching song')
    assert.deepEqual(page.titles(), [])
    assert.ok(byClass(page.root, 'empty-state').some(node => textContent(node).includes('player.no_results')))
    const clear = allNodes(page.root).find(node => node.type === 'button' && node.props['aria-label'] === 'common.clear')
    assert.ok(clear, 'search must offer a clear button')
    clear.props.onClick()
    await settle()
    assert.equal(page.titles().length, 123)
    await page.search('needle')
    allNodes(page.root).find(node => node.type === 'input').props.onKeydown({ key: 'Escape' })
    await settle()
    assert.equal(page.titles().length, 123)
  } finally { page.stop() }
})
await test('artists without tracks retain their original empty state', async () => {
  const page = await mountArtist([])
  try {
    assert.ok(byClass(page.root, 'empty-state').some(node => textContent(node).includes('library.local_artist_empty')))
    assert.equal(allNodes(page.root).filter(node => node.type === 'input').length, 0)
  } finally { page.stop() }
})
console.log(`Local artist-search regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
