import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as vue from 'vue'
const { computed, ref } = vue

function declarations(source, names) {
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
  const parsed = ts.createSourceFile('component.ts', script, ts.ScriptTarget.ES2022, true)
  return names.map(name => {
    const node = parsed.statements.find(item =>
      ts.isFunctionDeclaration(item) ? item.name?.text === name :
      ts.isVariableStatement(item) && item.declarationList.declarations.some(decl => decl.name.getText(parsed) === name))
    assert.ok(node, `missing actual component declaration: ${name}`)
    return node.getText(parsed)
  }).join('\n')
}

function evaluate(source, context, exported) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  return new Function(...Object.keys(context), `${compiled}\nreturn { ${exported.join(',')} }`)(...Object.values(context))
}

const root = new URL('../', import.meta.url)
const local = await readFile(new URL('src/views/LocalPlaylistView.vue', root), 'utf8')
let failures = 0
async function test(name, run) {
  try { await run(); console.log(`passed: ${name}`) }
  catch (error) { failures++; console.error(`failed: ${name}`, error.message) }
}

await test('local playlist right-click offers download', () => {
  const { trackMenuItems } = evaluate(declarations(local, ['trackMenuItems']), {
    computed, t: key => key,
    createContextMenuItem: (label, options) => ({ label, ...options }),
    downloadMenuItem: computed(() => ({ id: 'download', label: 'download.download', icon: 'download' })),
  }, ['trackMenuItems'])
  assert.ok(trackMenuItems.value.some(item => item.id === 'download'))
})

await test('batch download snapshots songs and exits multi-select before enqueue', () => {
  const tracks = ref([
    { id: 'netease:1', album: 'A', playlistKey: 'first' },
    { id: 'netease:1', album: 'A', playlistKey: 'second' },
    { id: 'netease:2', album: 'B', playlistKey: 'third' },
  ])
  const selectionMode = ref(true)
  const selectedIds = ref(new Set(['first', 'second']))
  const enqueued = []
  let dragCancelled = false
  const { downloadSelected } = evaluate(declarations(local, [
    'trackSelectionKey', 'selectedTracks', 'leaveSelectionMode', 'downloadSelected',
  ]), {
    tracks, computed, selectionMode, selectedIds,
    cancelTrackDrag: () => { dragCancelled = true },
    downloadStore: { downloadTrack: track => {
      assert.equal(selectionMode.value, false, 'selection must exit before download is queued')
      enqueued.push(track.playlistKey)
    } },
  }, ['downloadSelected'])
  downloadSelected()
  assert.deepEqual(enqueued, ['first', 'second'], 'clearing selection must not lose songs or repeated occurrences')
  assert.equal(selectionMode.value, false)
  assert.equal(selectedIds.value.size, 0)
  assert.equal(dragCancelled, true)
})

for (const file of ['src/views/RecentView.vue', 'src/views/FavoritePlaylistView.vue', 'src/components/QueuePanel.vue']) {
  await test(`${file} offers and dispatches right-click download`, async () => {
    const source = await readFile(new URL(file, root), 'utf8')
    const queue = file.endsWith('QueuePanel.vue')
    const menuName = queue ? 'queueContextMenuItems' : 'trackMenuItems'
    const handlerName = queue ? 'handleQueueContextMenuClick' : 'handleTrackMenuClick'
    const item = { id: 'download', label: 'download.download', icon: 'download' }
    let downloads = 0
    const track = { id: 'netease:1' }
    const context = {
      computed, t: key => key,
      createContextMenuItem: (label, options) => ({ label, ...options }),
      createContextMenuSeparator: () => ({ separator: true }),
      downloadMenuItem: computed(() => item), downloadFromMenu: () => { downloads++ },
      trackMenu: ref({ track }), closeTrackMenu: () => {},
      player: { queue: [track] }, queueContextMenuIndex: ref(0), closeQueueContextMenu: () => {},
    }
    const actual = evaluate(declarations(source, [menuName, handlerName]), context, [menuName, handlerName])
    assert.ok(actual[menuName].value.some(entry => entry.id === 'download'))
    actual[handlerName](item)
    assert.equal(downloads, 1)
  })
}

await test('shared download action captures target, allows current download and prevents duplicate work', async () => {
  const hooks = []
  const queued = [], redownloaded = [], errors = []
  const state = vue.reactive({ running: false, saved: false })
  const store = {
    downloading: new Map(), isDownloading: () => state.running, isDownloaded: () => state.saved,
    downloadTrack: async track => { queued.push(track.id) },
    redownloadTrack: async track => { redownloaded.push(track.id); if (track.id === 'netease:fail') throw new Error('fixture') },
    initEvents: async () => {}, loadDownloads: async () => {},
  }
  const dependencies = {
    vue: { ...vue, onMounted: callback => hooks.push(callback) },
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@/stores/download': { useDownloadStore: () => store },
    '@/utils/contextMenu': { createContextMenuItem: (label, options) => ({ label, ...options }) },
    '@/utils/logger': { createLogger: () => ({ error: (...args) => errors.push(args) }) },
  }
  const compiled = ts.transpileModule(await readFile(new URL('src/composables/useTrackDownloadMenu.ts', root), 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `unmocked dependency ${name}`)
    return dependencies[name]
  }, exports)
  const target = ref(null)
  const menu = exports.useTrackDownloadMenu(() => target.value, () => { target.value = null })
  assert.equal(menu.downloadMenuItem.value.disabled, true)
  for (const platform of ['netease', 'qq', 'bilibili', 'youtube']) {
    target.value = { id: `${platform}:1` }
    assert.equal(menu.downloadMenuItem.value.disabled, false)
    await menu.downloadFromMenu()
    assert.equal(target.value, null)
    assert.equal(queued.at(-1), `${platform}:1`)
  }
  target.value = { id: 'local:1' }
  assert.equal(menu.downloadMenuItem.value.disabled, true)
  await menu.downloadFromMenu()
  assert.equal(queued.length, 4)
  target.value = { id: 'netease:1' }; state.running = true
  assert.equal(menu.downloadMenuItem.value.disabled, true)
  await menu.downloadFromMenu()
  assert.equal(queued.length, 4)
  state.running = false; state.saved = true; target.value = { id: 'netease:1' }
  assert.equal(menu.downloadMenuItem.value.disabled, false, 'playing downloaded files must remain operable')
  assert.equal(menu.downloadMenuItem.value.label, 'download.redownload')
  await menu.downloadFromMenu()
  assert.deepEqual(redownloaded, ['netease:1'])
  target.value = { id: 'netease:fail' }
  await menu.downloadFromMenu()
  assert.equal(errors.length, 1, 'context menu errors must be caught')
  for (const hook of hooks) hook()
})

assert.equal(failures, 0, `${failures} download menu regressions failed`)
