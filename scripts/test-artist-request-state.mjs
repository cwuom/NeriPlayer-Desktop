import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import ts from 'typescript'
import { computed, createRenderer, nextTick, onUnmounted, proxyRefs, reactive, ref, watch } from 'vue'
import { parse } from 'vue/compiler-sfc'

function deferred() {
  let resolve, reject
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail })
  return { promise, resolve, reject }
}
async function settle() { await nextTick(); await setImmediate(); await nextTick() }
const renderer = createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }),
  createComment: text => ({ text }), setText: () => {}, setElementText: () => {},
  parentNode: node => node.parent, nextSibling: () => null, patchProp: () => {},
  insert: (node, parent) => { node.parent = parent; parent.children.push(node) },
  remove: node => { node.parent.children = node.parent.children.filter(value => value !== node) },
})

async function mountPage(name, params, cached = null) {
  const source = await readFile(new URL(`../src/views/${name}`, import.meta.url), 'utf8')
  const descriptor = parse(source).descriptor
  const script = ts.createSourceFile('artist.ts', descriptor.scriptSetup.content, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const withoutImports = ts.factory.updateSourceFile(script, script.statements.filter(node => !ts.isImportDeclaration(node)))
  const compiled = ts.transpileModule(ts.createPrinter().printFile(withoutImports), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  const calls = [], pending = [], played = [], errors = []
  const route = reactive({ params, query: { name: 'Artist' } })
  const dependencies = {
    computed, onUnmounted, ref, watch, useRoute: () => route, useRouter: () => ({ push() {}, back() {} }),
    useI18n: () => ({ t: key => key }), usePlayerStore: () => ({ playAll: (...args) => played.push(args) }),
    useToastStore: () => ({ error: message => errors.push(message) }), normalizeTrack: track => track,
    useArtistFavorite: () => ({ following: ref(false), changing: ref(false), toggle: async () => {} }),
    playlistDetailCacheKey: () => 'cache-key',
    // 与 previewCachedDetail 相同的语义：缓存异步读到后只在新数据到达前展示
    previewCachedDetail: (_key, show) => {
      let fresh = false
      const read = Promise.resolve(structuredClone(cached)).then(value => !!value && !fresh && show(value) !== false)
      return { markFresh() { fresh = true }, shown: () => read }
    },
    writePlaylistDetailCache() {}, formatTrackDuration: () => '', recordPlaylistOpen() {},
    parseYouTubeArtistDetail: raw => raw, parseYouTubeArtistItems: raw => raw,
    youtubeArtistItemTrack: item => item.videoId ? { id: `youtube:${item.videoId}` } : null,
    invoke: (command, input) => { const request = deferred(); calls.push({ command, input }); pending.push(request); return request.promise },
    exports: {},
  }
  const names = name.startsWith('Bili')
    ? ['load', 'loadContents', 'detail', 'contents', 'loadingContents', 'error', 'failedLoadMore', 'selectedContent', 'loadCollection']
    : ['load', 'loadSection', 'playSection', 'detail', 'queueLoading', 'sectionLoading', 'sectionPages']
  const setup = new Function(...Object.keys(dependencies), `${compiled}\nreturn { ${names.join(', ')} }`)
  let bindings
  const app = renderer.createApp({ setup() { bindings = setup(...Object.values(dependencies)); return () => null } })
  app.mount({ children: [] })
  const retryExpression = descriptor.template.content.match(/@click="(selectedContent \? [^"]+)"/)?.[1]
  return {
    calls, pending, played, errors, route, bindings, stop: () => app.unmount(),
    retry: () => new Function('scope', `with (scope) { return (${retryExpression}) }`)(proxyRefs(bindings)),
  }
}

function biliDetail(page = 1, hasMore = false) {
  return { header: { name: 'Artist', coverUrl: '', bannerUrl: '', description: '' }, tracks: [{ id: `bilibili:BV${page}` }], page, total: 31, hasMore }
}
function biliContents() { return { collections: [{ id: '10', name: 'Collection' }], series: [], page: 1, hasMore: false } }
const item = { videoId: 'song', kind: 'song', title: 'Song' }
const section = { title: 'Songs', items: [item], moreEndpoint: { browseId: 'UC_artist', params: 'songs' } }
function youtubeDetail() { return { header: { name: 'Artist' }, sections: [section] } }
let failed = 0
let total = 0
async function test(name, run) {
  total++
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('cached Bilibili refresh retry requests the first page even without more pages', async () => {
  const page = await mountPage('BiliArtistView.vue', { mid: '12' }, biliDetail(1, false))
  try {
    page.pending[0].reject(new Error('refresh failed')); page.pending[1].resolve(biliContents())
    await settle()
    const retry = page.retry()
    assert.equal(page.calls.length, 3, 'retry must issue a request')
    assert.equal(page.calls[2].input.page, 1, 'initial refresh must not retry as next page')
    page.pending[2].resolve(biliDetail())
    await retry
  } finally { page.stop() }
})

await test('Bilibili pagination retry repeats the failed page', async () => {
  const page = await mountPage('BiliArtistView.vue', { mid: '12' })
  try {
    page.pending[0].resolve(biliDetail(1, true)); page.pending[1].resolve(biliContents()); await settle()
    const more = page.bindings.load(true)
    page.pending[2].reject(new Error('page failed')); await more
    const retry = page.retry()
    assert.equal(page.calls[3].input.page, 2)
    page.pending[3].resolve(biliDetail(2, false)); await retry
    assert.equal(page.bindings.detail.value.tracks.length, 2)
  } finally { page.stop() }
})

await test('retrying Bilibili videos does not strand an in-flight collection directory', async () => {
  const page = await mountPage('BiliArtistView.vue', { mid: '12' })
  try {
    page.pending[0].reject(new Error('profile failed')); await settle()
    const retry = page.bindings.load()
    page.pending[2].resolve(biliDetail()); await retry
    page.pending[1].resolve(biliContents()); await settle()
    assert.equal(page.bindings.loadingContents.value, false)
    assert.equal(page.bindings.contents.value.collections[0].id, '10')
  } finally { page.stop() }
})

await test('Bilibili directory from an old route cannot replace the new artist', async () => {
  const page = await mountPage('BiliArtistView.vue', { mid: '12' })
  try {
    page.route.params.mid = '13'; await settle()
    page.pending[2].resolve(biliDetail()); page.pending[3].resolve({ ...biliContents(), collections: [{ id: '13' }] }); await settle()
    page.pending[0].resolve(biliDetail()); page.pending[1].resolve(biliContents()); await settle()
    assert.equal(page.bindings.contents.value.collections[0].id, '13')
    assert.equal(page.bindings.loadingContents.value, false)
  } finally { page.stop() }
})

await test('YouTube detail retry releases a cancelled playback queue', async () => {
  const page = await mountPage('YouTubeArtistView.vue', { browseId: 'UC_artist' }, youtubeDetail())
  try {
    page.pending[0].reject(new Error('refresh failed')); await settle()
    const play = page.bindings.playSection(section, item)
    assert.equal(page.bindings.queueLoading.value, true)
    const retry = page.bindings.load()
    page.pending[2].resolve(youtubeDetail()); await retry
    page.pending[1].resolve({ items: [item], continuation: '' }); await play
    assert.equal(page.bindings.queueLoading.value, false)
    assert.equal(page.played.length, 0, 'cancelled queue must not start playback')
  } finally { page.stop() }
})

await test('YouTube playback does not race an in-flight section page', async () => {
  const page = await mountPage('YouTubeArtistView.vue', { browseId: 'UC_artist' })
  try {
    page.pending[0].resolve(youtubeDetail()); await settle()
    const load = page.bindings.loadSection(section)
    const play = page.bindings.playSection(section, item)
    assert.equal(page.calls.length, 2, 'only one request may own the section page')
    page.pending[1].resolve({ items: [item], continuation: '' }); await load; await play
  } finally { page.stop() }
})

await test('an old YouTube queue cannot consume continuation pages loaded after retry', async () => {
  const page = await mountPage('YouTubeArtistView.vue', { browseId: 'UC_artist' }, youtubeDetail())
  try {
    page.pending[0].reject(new Error('refresh failed')); await settle()
    const oldPlay = page.bindings.playSection(section, item)
    const retry = page.bindings.load()
    page.pending[2].resolve(youtubeDetail()); await retry
    const newLoad = page.bindings.loadSection(section)
    page.pending[3].resolve({ items: [item], continuation: 'new-continuation' }); await newLoad
    page.pending[1].resolve({ items: [item], continuation: 'old-continuation' }); await settle()
    assert.equal(page.calls.length, 4, 'cancelled queue must not request the new continuation')
    await oldPlay
    assert.equal(Object.values(page.bindings.sectionPages.value)[0].continuation, 'new-continuation')
    assert.equal(page.played.length, 0)
  } finally { page.stop() }
})

await test('a failed section request from an old YouTube route does not toast on the new page', async () => {
  const page = await mountPage('YouTubeArtistView.vue', { browseId: 'UC_artist' })
  try {
    page.pending[0].resolve(youtubeDetail()); await settle()
    const oldLoad = page.bindings.loadSection(section)
    page.route.params.browseId = 'UC_other'; await settle()
    page.pending[1].reject(new Error('old section failed')); await oldLoad
    assert.equal(page.errors.length, 0)
    page.pending[2].resolve(youtubeDetail()); await settle()
  } finally { page.stop() }
})

if (failed) process.exitCode = 1
console.log(`Artist request-state regressions: ${total - failed} passed, ${failed} failed`)
