import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import ts from 'typescript'
import * as Vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'

const { createRenderer, defineComponent, h, nextTick, reactive, ref } = Vue
const renderer = createRenderer({
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

async function settle() { await nextTick(); await setImmediate(); await nextTick() }
function allNodes(node) { return [node, ...node.children.flatMap(allNodes)] }
function byClass(root, name) {
  return allNodes(root).filter(node => String(node.props?.class || '').split(/\s+/).includes(name))
}
function textContent(node) {
  return (node.type === '#comment' ? '' : node.text || '') + node.children.map(textContent).join('')
}
function song(id, name, artist = 'Artist', album = 'Album') {
  return { id, name, ar: [{ name: artist }], al: { name: album, picUrl: '' }, dt: 60000 }
}
function album(id, name, year) {
  return { id, name, picUrl: '', publishTime: Date.UTC(year, 6, 1), size: 2 }
}
const fixture = {
  songs: [
    ...Array.from({ length: 120 }, (_, index) => song(index + 1, `Song ${index + 1}`)),
    song(501, 'Needle Alpha', 'Guest Vocalist', 'Special Record'),
    song(502, 'Needle Beta'),
  ],
  hotAlbums: [album(31, 'Summer Collection', 2020), album(32, 'Winter Collection', 2024)],
}

async function mountArtist(data = fixture) {
  const source = await readFile(new URL('../src/views/NeteaseArtistView.vue', import.meta.url), 'utf8')
  const descriptor = parse(source).descriptor
  const content = compileScript(descriptor, { id: 'netease-artist-search-test', inlineTemplate: true }).content
  const script = ts.createSourceFile('artist.ts', content, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const route = reactive({ params: { id: '12' }, query: { name: 'Artist' } })
  const played = [], opened = [], calls = []
  const imports = {
    useRoute: () => route,
    useRouter: () => ({ push: location => opened.push(location), back() {} }),
    useI18n: () => ({ t: key => key }),
    usePlayerStore: () => ({ currentTrack: null, isPlaying: false, playAll: (...args) => played.push(args) }),
    useToastStore: () => ({ error() {} }),
    useArtistFavorite: () => ({ following: ref(false), changing: ref(false), toggle: async () => {} }),
    playlistDetailCacheKey: (_kind, id) => String(id),
    previewCachedDetail: () => ({ markFresh() {}, shown: async () => false }),
    writePlaylistDetailCache() {},
    resolveNeteaseCover: () => '',
    formatTrackDuration: () => '1:00',
    createLogger: () => ({ error() {} }),
    BilibiliCoverImage: defineComponent({ setup: () => () => h('img') }),
    invoke: async (command, input) => {
      calls.push({ command, input })
      if (command === 'get_netease_artist_detail') return { data: { artist: { name: 'Artist', alias: [], musicSize: data.songs.length, albumSize: data.hotAlbums.length } } }
      if (command === 'get_netease_artist_songs') return { songs: data.songs }
      if (command === 'get_netease_artist_albums') return { hotAlbums: data.hotAlbums }
      throw new Error(`Unexpected IPC command: ${command}`)
    },
  }
  const dependencies = { exports: {} }
  for (const statement of script.statements) {
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
  // 搜索测试关注组件渲染与事件处理, DOM 指令和过渡由浏览器验证
  dependencies._vModelText = {}
  dependencies._Transition = defineComponent({
    props: ['name', 'mode'],
    setup: (_props, { slots }) => () => slots.default?.(),
  })
  const withoutImports = ts.factory.updateSourceFile(script, script.statements.filter(node => !ts.isImportDeclaration(node)))
  const compiled = ts.transpileModule(ts.createPrinter().printFile(withoutImports), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  new Function(...Object.keys(dependencies), compiled)(...Object.values(dependencies))
  const root = { children: [] }
  const app = renderer.createApp(dependencies.exports.default)
  app.mount(root)
  await settle()
  return {
    root, route, played, opened, calls,
    songTitles: () => byClass(root, 'track-title').map(textContent),
    albumNames: () => byClass(root, 'artist-album-name').map(textContent),
    async search(value) {
      const input = allNodes(root).find(node => node.type === 'input')
      assert.ok(input, 'artist page must expose a search input')
      assert.equal(input.props['aria-label'], 'library.tab_search_hint')
      input.props['onUpdate:modelValue'](value)
      await settle()
    },
    async tab(index) { byClass(root, 'artist-tab')[index].props.onClick(); await settle() },
    async scroll() {
      byClass(root, 'detail-view')[0].props.onScrollPassive({ currentTarget: { scrollTop: 10000, clientHeight: 900, scrollHeight: 10900 } })
      await settle()
    },
    stop: () => app.unmount(),
  }
}

let total = 0, failed = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('song search finds loaded matches beyond the first render window', async () => {
  const page = await mountArtist()
  try {
    assert.equal(page.songTitles().length, 100)
    await page.search('  nEeDlE  ')
    assert.deepEqual(page.songTitles(), ['Needle Alpha', 'Needle Beta'])
    await page.search('guest vocalist')
    assert.deepEqual(page.songTitles(), ['Needle Alpha'])
    await page.search('special record')
    assert.deepEqual(page.songTitles(), ['Needle Alpha'])
  } finally { page.stop() }
})

await test('clicking a filtered song plays the filtered queue and selected track id', async () => {
  const page = await mountArtist()
  try {
    await page.search('needle')
    byClass(page.root, 'track-item')[1].props.onClick()
    assert.deepEqual(page.played[0][0].map(track => track.id), ['netease:501', 'netease:502'])
    assert.equal(page.played[0][1], 'netease:502')
    byClass(page.root, 'play-all-btn')[0].props.onClick()
    assert.equal(page.played[1][0].length, 122, 'artist play-all must retain its complete queue')
  } finally { page.stop() }
})

await test('search and tab changes reset the render window and clear unrelated tab queries', async () => {
  const page = await mountArtist()
  try {
    await page.scroll()
    assert.equal(page.songTitles().length, 122)
    await page.search('needle')
    await page.search('')
    assert.equal(page.songTitles().length, 100)
    await page.search('needle')
    await page.tab(1)
    assert.deepEqual(page.albumNames(), ['Summer Collection', 'Winter Collection'])
    await page.search('WINTER')
    assert.deepEqual(page.albumNames(), ['Winter Collection'])
    await page.search('2020')
    assert.deepEqual(page.albumNames(), ['Summer Collection'])
    byClass(page.root, 'artist-album-item')[0].props.onClick()
    assert.deepEqual(page.opened[0], { name: 'netease-album', params: { id: '31' } })
    await page.tab(0)
    assert.equal(page.songTitles().length, 100)
  } finally { page.stop() }
})

await test('unmatched queries show a search-empty message in songs and albums', async () => {
  const page = await mountArtist()
  try {
    await page.search('not a match')
    assert.deepEqual(page.songTitles(), [])
    assert.ok(byClass(page.root, 'state-center').some(node => textContent(node).includes('player.no_results')))
    await page.tab(1)
    await page.search('not a match')
    assert.deepEqual(page.albumNames(), [])
    assert.ok(byClass(page.root, 'state-center').some(node => textContent(node).includes('player.no_results')))
  } finally { page.stop() }
})

await test('opening another artist resets the query, tab, and render window', async () => {
  const page = await mountArtist()
  try {
    await page.tab(1)
    await page.search('winter')
    page.route.params.id = '13'
    await settle()
    assert.equal(page.songTitles().length, 100)
    assert.equal(page.calls.filter(call => call.input.artistId === 13).length, 3)
  } finally { page.stop() }
})

console.log(`Netease artist-search regressions: ${total - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
