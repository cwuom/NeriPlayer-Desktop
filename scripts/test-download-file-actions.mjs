import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const downloadsSource = await readFile(new URL('src/views/DownloadsView.vue', root), 'utf8')

function evaluate(source, names, context, exported = names) {
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
  const parsed = ts.createSourceFile('component.ts', script, ts.ScriptTarget.ES2022, true)
  const statements = names.flatMap(name => {
    const node = parsed.statements.find(item =>
      ts.isFunctionDeclaration(item) ? item.name?.text === name :
      ts.isVariableStatement(item) && item.declarationList.declarations.some(declaration => declaration.name.getText(parsed) === name))
    if (!node && ['isTrackInUse', 'selectableVisibleDownloads'].includes(name)) return []
    assert.ok(node, `missing production declaration: ${name}`)
    return [node.getText(parsed)]
  }).join('\n')
  const compiled = ts.transpileModule(statements, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  return new Function(...Object.keys(context), `${compiled}\nreturn { ${exported.join(',')} }`)(...Object.values(context))
}

const playing = { id: 'netease:playing', title: 'Playing', artist: 'Artist', album: 'NeteaseRaw album', filePath: 'E:/Music/playing.mp3' }
const other = { id: 'netease:other', title: 'Other', artist: 'Artist', album: 'Album', filePath: 'E:/Music/other.mp3' }
const hidden = { id: 'netease:hidden', title: 'Hidden', artist: 'Artist', album: '', filePath: 'E:/Music/hidden.mp3' }

function downloadsRuntime({ failingDelete, failingRedownload = false } = {}) {
  const calls = [], notices = [], invalidations = []
  const running = vue.reactive(new Set())
  const downloadStore = vue.reactive({
    downloads: [playing, other, hidden],
    isDownloading: id => running.has(id),
    deleteDownload: async id => {
      calls.push({ operation: 'delete', id })
      if (id === failingDelete) throw new Error('fixture delete failed')
      downloadStore.downloads = downloadStore.downloads.filter(track => track.id !== id)
    },
    redownloadTrack: async track => {
      calls.push({ operation: 'redownload', track })
      if (failingRedownload) throw new Error('fixture redownload failed')
    },
  })
  const context = {
    computed: vue.computed,
    downloadStore,
    player: vue.reactive({ currentTrack: playing, isPlayingFromDownload: true, handleDownloadedFileRemoved: (...args) => invalidations.push(args) }),
    toast: Object.fromEntries(['show', 'success', 'error'].map(method => [method, (...args) => notices.push({ method, args })])),
    log: { error() {} },
    t: key => key,
    createContextMenuItem: (label, options) => ({ label, ...options }),
    createContextMenuSeparator: id => ({ id, separator: true }),
    filteredDownloads: vue.computed(() => downloadStore.downloads.filter(track => track.id !== hidden.id)),
    selectionMode: vue.ref(false), selectedIds: vue.ref(new Set()),
    showDeleteDialog: vue.ref(false), deleteTarget: vue.ref(null), batchDeleting: vue.ref(false),
    deletionProgress: vue.ref({ done: 0, total: 0, failed: 0 }),
    downloadContextMenuTarget: vue.ref({ kind: 'downloaded', track: playing }),
  }
  const functions = ['downloadToTrack', 'enterSelectionMode', 'leaveSelectionMode', 'toggleSelected', 'toggleSelectAllVisible', 'requestDelete', 'requestBatchDelete', 'confirmDelete', 'redownloadTrack']
  const computed = ['selectedCount', 'visibleSelectedCount', 'allVisibleSelected', 'downloadContextMenuItems']
  const actual = evaluate(downloadsSource, ['isTrackInUse', 'selectableVisibleDownloads', ...computed, ...functions], context, [...computed, ...functions])
  return { ...context, ...actual, calls, notices, invalidations, running }
}

let failures = 0
async function test(name, run) {
  try { await run(); console.log(`passed: ${name}`) }
  catch (error) { failures++; console.error(`failed: ${name}: ${error.message}`) }
}

await test('playing downloaded song can enter selection and toggle its selected state', () => {
  const runtime = downloadsRuntime()
  runtime.enterSelectionMode(playing)
  assert.equal(runtime.selectionMode.value, true)
  assert.equal(runtime.selectedIds.value.has(playing.id), true)
  runtime.toggleSelected(playing.id)
  assert.equal(runtime.selectedIds.value.has(playing.id), false)
  runtime.toggleSelected(playing.id)
  assert.equal(runtime.selectedIds.value.has(playing.id), true)
})

await test('select all includes playing song and respects the visible search scope', () => {
  const runtime = downloadsRuntime()
  runtime.toggleSelectAllVisible()
  assert.deepEqual([...runtime.selectedIds.value], [playing.id, other.id])
  assert.equal(runtime.visibleSelectedCount.value, 2)
  assert.equal(runtime.allVisibleSelected.value, true)
  runtime.toggleSelectAllVisible()
  assert.equal(runtime.selectedIds.value.size, 0)
  assert.equal(runtime.selectionMode.value, false)
})

await test('playing song delete opens confirmation and delegates removal to the store', async () => {
  const runtime = downloadsRuntime()
  runtime.requestDelete(playing)
  assert.equal(runtime.showDeleteDialog.value, true)
  assert.equal(runtime.calls.length, 0, 'opening confirmation must not delete the file')
  await runtime.confirmDelete()
  assert.deepEqual(runtime.calls, [{ operation: 'delete', id: playing.id }])
  assert.equal(runtime.showDeleteDialog.value, false)
  assert.deepEqual(runtime.invalidations, [], 'the view must not mutate playback state outside the file operation')
})

await test('batch deletion attempts playing song and retains failure accounting', async () => {
  const runtime = downloadsRuntime({ failingDelete: other.id })
  runtime.enterSelectionMode()
  runtime.selectedIds.value = new Set([playing.id, other.id])
  runtime.requestBatchDelete()
  await runtime.confirmDelete()
  assert.deepEqual(runtime.calls.map(call => call.id), [playing.id, other.id])
  assert.deepEqual(runtime.deletionProgress.value, { done: 2, total: 2, failed: 1 })
  assert.equal(runtime.selectionMode.value, true, 'failed deletions remain available to retry')
  assert.ok(runtime.notices.some(notice => notice.method === 'error' && notice.args[0] === 'download.deletion_failed'))
  assert.deepEqual(runtime.invalidations, [])
})

await test('playing downloaded song can be redownloaded without modifying raw album identity', async () => {
  const runtime = downloadsRuntime()
  assert.notEqual(runtime.downloadContextMenuItems.value.find(item => item.id === 'redownload').disabled, true)
  assert.notEqual(runtime.downloadContextMenuItems.value.find(item => item.id === 'delete').disabled, true)
  await runtime.redownloadTrack(playing)
  assert.equal(runtime.calls[0]?.operation, 'redownload')
  assert.equal(runtime.calls[0].track.album, playing.album)
  assert.equal(runtime.calls[0].track.audioUrl, playing.filePath)
  assert.deepEqual(runtime.invalidations, [])
})

await test('failed redownload keeps playback state for the store operation to restore', async () => {
  const runtime = downloadsRuntime({ failingRedownload: true })
  runtime.player.isPlayingFromDownload = false
  await runtime.redownloadTrack(playing)
  assert.equal(runtime.calls.length, 1)
  assert.deepEqual(runtime.invalidations, [])
  assert.ok(runtime.notices.some(notice => notice.method === 'error'))
})

await test('redownload menu and direct action reject an already running download', async () => {
  const runtime = downloadsRuntime()
  runtime.player.isPlayingFromDownload = false
  runtime.running.add(playing.id)
  assert.equal(runtime.downloadContextMenuItems.value.find(item => item.id === 'redownload').disabled, true)
  await runtime.redownloadTrack(playing)
  assert.deepEqual(runtime.calls, [])
})

for (const file of ['NeteasePlaylistView.vue', 'BiliPlaylistView.vue', 'YouTubePlaylistView.vue']) {
  await test(`${file} allows current downloaded song to redownload and rejects duplicate work`, async () => {
    const source = await readFile(new URL(`src/views/${file}`, root), 'utf8')
    const calls = []
    const running = vue.reactive(new Set())
    const context = {
      computed: vue.computed, t: key => key,
      trackMenu: vue.ref({ show: true, track: playing }),
      player: { currentTrack: playing, isPlayingFromDownload: true },
      createContextMenuItem: (label, options) => ({ label, ...options }),
      downloadStore: {
        downloading: new Map(), isDownloading: id => running.has(id), isDownloaded: () => true,
        redownloadTrack: async track => calls.push(track.id), downloadTrack: async () => assert.fail('saved song should use redownload'),
      },
    }
    const actual = evaluate(source, ['closeTrackMenu', 'downloadTaskStatusText', 'trackDownloadLabel', 'isTrackDownloadDisabled', 'handleTrackDownload', 'trackMenuItems'], context)
    assert.equal(actual.isTrackDownloadDisabled(playing), false)
    assert.equal(actual.trackMenuItems.value.find(item => item.id === 'download').disabled, false)
    await actual.handleTrackDownload(playing)
    assert.deepEqual(calls, [playing.id])
    assert.equal(context.trackMenu.value.show, false)
    running.add(playing.id)
    assert.equal(actual.isTrackDownloadDisabled(playing), true)
    assert.equal(actual.trackMenuItems.value.find(item => item.id === 'download').disabled, true)
    await actual.handleTrackDownload(playing)
    assert.deepEqual(calls, [playing.id], 'repeat action must not enqueue the same running track')
  })
}

if (failures) throw new Error(`${failures} download file action regression(s) failed`)
console.log('download file action tests passed')
