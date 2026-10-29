import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as Vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
function load(source, dependencies = {}) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `missing dependency ${name}`)
    return dependencies[name]
  }, exports)
  return exports
}

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
const allNodes = node => [node, ...node.children.flatMap(allNodes)]
const byClass = (root, name) => allNodes(root).filter(node => String(node.props?.class || '').split(/\s+/).includes(name))
const textContent = node => (node.type === '#comment' ? '' : node.text || '') + node.children.map(textContent).join('')
const settle = () => Vue.nextTick()
const stub = Vue.defineComponent({ inheritAttrs: false, setup: () => () => null })
const data = [
  { id: 'old', title: 'Song 10', artist: 'Artist', album: 'Album', durationMs: 1000, audioUrl: 'C:/Music/old.mp3', fileModifiedMs: 100 },
  { id: 'new', title: 'Song 2', artist: 'Artist', album: 'Album', durationMs: 2000, audioUrl: 'C:/Music/new.mp3', fileModifiedMs: 300 },
  { id: 'middle', title: 'Other', artist: 'Guest', album: 'Album', durationMs: 3000, audioUrl: 'C:/Music/middle.mp3', fileModifiedMs: 200 },
]

async function mountPage(options = {}) {
  const settings = Vue.reactive({ doubleClickPlay: false, localLibrarySortBy: 'default', localLibrarySortDescending: false, ...options })
  const library = Vue.reactive({
    tracks: data.map(track => ({ ...track })), folders: [], playlistTracks: [], isLoaded: true,
    isScanning: false, scanError: null, scanSkipped: [], playlistIndexError: null,
    ensureStarted() {}, refreshPlaylistIndex() {},
  })
  const played = []
  const player = Vue.reactive({ currentTrack: { ...data[1] }, isPlaying: false, playAll: (...args) => played.push(args) })
  const dependencies = {
    vue: { ...Vue, vModelText: {}, vModelCheckbox: {} }, pinia: { storeToRefs: Vue.toRefs },
    '@tauri-apps/api/core': { invoke() {} }, '@tauri-apps/plugin-dialog': { open() {} },
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@/stores/library': { useLibraryStore: () => library },
    '@/stores/player': { usePlayerStore: () => player },
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/stores/toast': { useToastStore: () => ({ error() {} }) },
    '@/utils/contextMenu': { createContextMenuItem: () => ({}), createContextMenuSeparator: () => ({}) },
    '@/composables/useTrackSelection': load(await read('../src/composables/useTrackSelection.ts'), { vue: Vue }),
    '@/modules/library/localScanPreview': load(await read('../src/modules/library/localScanPreview.ts')),
  }
  const source = await read('../src/views/LocalFilesView.vue')
  if (source.includes("@/modules/library/localTrackSort")) {
    dependencies['@/modules/library/localTrackSort'] = load(await read('../src/modules/library/localTrackSort.ts'))
  }
  if (source.includes("@/composables/useTrackActivation")) {
    dependencies['@/composables/useTrackActivation'] = load(await read('../src/composables/useTrackActivation.ts'), {
      '@/stores/settings': dependencies['@/stores/settings'],
    })
  }
  const select = Vue.defineComponent({
    props: ['modelValue', 'options', 'label'], emits: ['update:modelValue'],
    setup: (props, { emit }) => () => Vue.h('select', {
      'aria-label': props.label, onChange: value => emit('update:modelValue', value),
    }),
  })
  const toolbar = Vue.defineComponent({
    emits: ['select-all', 'play'],
    setup: (_, { emit }) => () => Vue.h('div', [
      Vue.h('button', { class: 'test-select-all', onClick: () => emit('select-all') }),
      Vue.h('button', { class: 'test-play-selected', onClick: () => emit('play') }),
    ]),
  })
  for (const name of ['AddToPlaylistDialog', 'TrackContextMenu', 'M3Dialog', 'M3Input', 'ContextMenu', 'CustomSelect', 'TrackSelectionToolbar']) {
    const component = name === 'CustomSelect' ? select : name === 'TrackSelectionToolbar' ? toolbar : stub
    dependencies[`@/components/${['M3Dialog', 'M3Input', 'ContextMenu', 'CustomSelect'].includes(name) ? 'ui/' : ''}${name}.vue`] = { default: component }
  }
  const content = compileScript(parse(source).descriptor, { id: 'local-files-interactions', inlineTemplate: true }).content
  const module = load(content, dependencies)
  const root = { children: [] }
  const app = renderer.createApp(module.default)
  app.mount(root)
  await settle()
  return { root, settings, library, played, stop: () => app.unmount() }
}

function sortControl(page) {
  const control = allNodes(page.root).find(node => node.type === 'select' && node.props['aria-label'] === 'library.local_sort')
  assert.ok(control, 'local files expose a sort selector')
  return control
}
const ids = queue => queue.map(track => track.id)
let failed = 0
async function test(name, run) {
  try { await run(); console.log(`PASS ${name}`) }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack}`) }
}

await test('modification sorting drives rendered rows, play-all and selected queues', async () => {
  const page = await mountPage()
  try {
    sortControl(page).props.onChange('modified')
    await settle()
    allNodes(page.root).find(node => node.type === 'button' && node.props['aria-label'] === 'download.sort_descending').props.onClick()
    await settle()
    assert.equal(page.settings.localLibrarySortDescending, true, 'direction changes update the saved preference')
    assert.deepEqual(byClass(page.root, 'scan-track-info').map(node => textContent(node.children[0])), ['Song 2', 'Other', 'Song 10'])
    const playAll = byClass(page.root, 'scan-actions')[0].children.find(node => node.type === 'button')
    playAll.props.onClick()
    assert.deepEqual(ids(page.played.at(-1)[0]), ['new', 'middle', 'old'])
    byClass(page.root, 'test-select-all')[0].props.onClick()
    await settle()
    byClass(page.root, 'test-play-selected')[0].props.onClick()
    assert.deepEqual(ids(page.played.at(-1)[0]), ['new', 'middle', 'old'])
    assert.deepEqual(ids(page.library.tracks), ['old', 'new', 'middle'], 'sorting preserves the stored index order')
  } finally { page.stop() }
})

await test('double-click preference gates row playback while explicit play and keyboard stay direct', async () => {
  const page = await mountPage({ doubleClickPlay: true })
  try {
    const info = byClass(page.root, 'scan-track-info')[0]
    info.props.onClick({ detail: 1 })
    assert.equal(page.played.length, 0, 'single pointer click must not start playback')
    info.props.onDblclick({ detail: 2 })
    assert.equal(page.played.length, 1)
    info.props.onClick({ detail: 0 })
    assert.equal(page.played.length, 2, 'keyboard activation remains accessible')
    const row = byClass(page.root, 'scan-track')[0]
    row.children.filter(node => node.type === 'button').at(-1).props.onClick()
    assert.equal(page.played.length, 3, 'explicit play remains immediate')
    byClass(page.root, 'test-select-all')[0].props.onClick()
    await settle()
    info.props.onClick({ detail: 1 })
    info.props.onClick({ detail: 2 })
    info.props.onDblclick({ detail: 2 })
    assert.equal(page.played.length, 3, 'multi-selection never starts playback')
  } finally { page.stop() }
})

await test('paused current track has a full-row highlight independent of selection', async () => {
  const page = await mountPage()
  try {
    assert.ok(String(byClass(page.root, 'scan-track')[1].props.class).split(/\s+/).includes('playing'))
    byClass(page.root, 'test-select-all')[0].props.onClick()
    await settle()
    const row = byClass(page.root, 'scan-track')[1]
    assert.ok(row.props.class.includes('selected'))
    assert.ok(row.props.class.includes('playing'))
  } finally { page.stop() }
})

await test('double-clicking the last selected track clears selection without starting playback', async () => {
  const page = await mountPage({ doubleClickPlay: true })
  try {
    const row = byClass(page.root, 'scan-track')[0]
    row.children.find(node => node.type === 'label').children[0].props.onChange()
    await settle()
    assert.ok(byClass(page.root, 'scan-track')[0].props.class.includes('selected'))
    const info = byClass(page.root, 'scan-track-info')[0]
    info.props.onClick({ detail: 1 })
    await settle()
    assert.ok(!byClass(page.root, 'scan-track')[0].props.class.includes('selected'))
    const updatedInfo = byClass(page.root, 'scan-track-info')[0]
    updatedInfo.props.onClick({ detail: 2 })
    updatedInfo.props.onDblclick({ detail: 2 })
    assert.equal(page.played.length, 0)
  } finally { page.stop() }
})

if (failed) process.exitCode = 1
