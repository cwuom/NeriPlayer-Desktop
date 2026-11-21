import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import ts from 'typescript'

const root = new URL('../src/', import.meta.url)

function evaluate(source, dependencies = {}, context = {}) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  const exports = {}
  new Function('require', 'exports', ...Object.keys(context), compiled)(
    name => { assert.ok(name in dependencies, `missing dependency ${name}`); return dependencies[name] },
    exports, ...Object.values(context),
  )
  return exports
}

const localTrack = evaluate(await readFile(new URL('modules/library/localTrack.ts', root), 'utf8'))
const volume = evaluate(await readFile(new URL('utils/volume.ts', root), 'utf8'))
const contextMenu = evaluate(await readFile(new URL('utils/contextMenu.ts', root), 'utf8'))
const biliVideoSkip = evaluate(await readFile(new URL('modules/playback/biliVideoSkip.ts', root), 'utf8'))
const clipboardSource = await readFile(new URL('utils/clipboard.ts', root), 'utf8').catch(error => {
  if (error.code === 'ENOENT') return null
  throw error
})
const trackSource = (await readFile(new URL('components/TrackContextMenu.vue', root), 'utf8')).match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const collectionSource = (await readFile(new URL('components/CollectionContextMenu.vue', root), 'utf8')).match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]

function menuRuntime(native, { nativeError = false, browserError = false } = {}) {
  const nativeWrites = [], browserWrites = [], messages = []
  const navigator = { clipboard: { writeText: async text => {
    if (native || browserError) throw new Error('fixture browser clipboard unavailable')
    browserWrites.push(text)
  } } }
  const clipboard = clipboardSource ? evaluate(clipboardSource, {
    '@tauri-apps/api/core': { isTauri: () => native },
    '@tauri-apps/plugin-clipboard-manager': { writeText: async text => {
      nativeWrites.push(text)
      if (nativeError) throw new Error('fixture native clipboard unavailable')
    } },
  }, { navigator }) : { writeClipboardText: () => assert.fail('missing shared clipboard helper') }
  const dependencies = {
    vue,
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    'vue-router': { useRouter: () => ({ push: async () => {} }) },
    '@tauri-apps/api/core': { invoke: async () => {} },
    '@tauri-apps/plugin-opener': { openUrl: async () => {} },
    '@/stores/player': { usePlayerStore: () => ({ addToQueueNext() {}, addToQueueEnd() {} }) },
    '@/stores/toast': { useToastStore: () => ({
      success: message => messages.push(['success', message]),
      error: message => messages.push(['error', message]),
    }) },
    '@/stores/download': { useDownloadStore: () => ({ getDownloadedTrack: () => null }) },
    '@/composables/useTrackDownloadMenu': { useTrackDownloadMenu: () => ({
      downloadMenuItem: vue.computed(() => ({ id: 'download', label: 'download' })),
      downloadFromMenu: async () => {},
    }) },
    '@/utils/contextMenu': contextMenu,
    '@/modules/playback/biliVideoSkip': biliVideoSkip,
    '@/modules/library/localTrack': localTrack,
    '@/utils/clipboard': clipboard,
    '@/utils/logger': { createLogger: () => ({ warn() {} }) },
  }
  const context = {
    navigator,
    defineProps: () => ({}), withDefaults: (_props, defaults) => defaults,
    defineEmits: () => () => {}, defineExpose() {},
  }
  const track = evaluate(`${trackSource}\nexport { menu, handleClick, items, videoSkipDialogOpen, videoSkipTarget }`, dependencies, context)
  const collection = evaluate(`${collectionSource}\nexport { menu, handleClick }`, dependencies, context)
  return { track, collection, nativeWrites, browserWrites, messages }
}

async function flush() { for (let index = 0; index < 8; index++) await Promise.resolve() }
let failures = 0
async function test(name, run) {
  try { await run(); console.log(`passed: ${name}`) }
  catch (error) { failures++; console.error(`failed: ${name}`, error.message) }
}

await test('local file URIs preserve hosts and decode paths once', () => {
  for (const [url, path] of [
    ['file:///C:/Music/a%20%231.flac', 'C:/Music/a #1.flac'],
    ['file:///home/me/音乐/a%20b.flac', '/home/me/音乐/a b.flac'],
    ['file://localhost/C:/Music/a.flac', 'C:/Music/a.flac'],
    ['file://server/share/Album%20one/a.flac', '//server/share/Album one/a.flac'],
    ['file:///home/me/a%2520b.flac', '/home/me/a%20b.flac'],
    ['C:\\Music\\a.flac', 'C:\\Music\\a.flac'],
    ['\\\\server\\share\\a.flac', '\\\\server\\share\\a.flac'],
  ]) {
    assert.equal(localTrack.localTrackFilePath({ id: 'local:fixture', source: 'local', audioUrl: url }), path)
  }
  assert.equal(localTrack.localTrackFilePath({ id: 'local:fixture', source: 'local', audioUrl: 'file:///broken%ZZ.flac' }), '')
})

await test('network and resource addresses never become local file actions', () => {
  for (const audioUrl of ['https://example.test/a.flac', 'http://example.test/a.flac', 'ftp://server/a.flac', 'smb://server/share/a.flac', 'rtsp://server/a', 'asset://localhost/a', 'blob:fixture', 'data:audio/flac;base64,AA==', 'tauri://localhost/a', '']) {
    assert.equal(localTrack.localTrackFilePath({ id: 'local:fixture', source: 'local', audioUrl }), '', audioUrl)
  }
  assert.equal(localTrack.localTrackFilePath({ id: 'netease:1', source: 'netease', audioUrl: 'C:/Music/a.flac' }), '')
  assert.equal(localTrack.isLocalTrack({ id: 'netease:1', source: 'local' }), true)
  assert.equal(localTrack.isLocalTrack({ id: 'local:1', source: 'netease' }), true)
  assert.equal(localTrack.isLocalTrack({ id: 'netease:1', source: 'netease' }), false)
})

await test('volume wheel changes five percent and clamps at both ends', () => {
  for (const [current, deltaY, expected] of [[0.5, -100, 0.55], [0.5, 100, 0.45], [0.98, -1, 1], [0.02, 1, 0], [0.3, 0, 0.3]]) {
    assert.equal(volume.wheelAdjustedVolume(current, deltaY), expected)
  }
  let current = 0
  for (let index = 0; index < 6; index++) current = volume.wheelAdjustedVolume(current, -1)
  assert.equal(current, 0.3, 'repeated steps retain a whole percent value')
})

await test('skip interval actions are limited to Bilibili tracks and known local origins', () => {
  const r = menuRuntime(false)
  for (const [track, expected] of [
    [{ id: 'netease:1', title: 'song', artist: 'artist', source: 'netease' }, false],
    [{ id: 'local:1', title: 'song', artist: 'artist', source: 'local' }, false],
    [{ id: 'bilibili:BV1fixture', title: 'song', artist: 'artist', source: 'bilibili' }, true],
    [{ id: 'local:1', title: 'song', artist: 'artist', source: 'local', syncPayload: { channelId: 'bilibili', audioId: 'BV1fixture', subAudioId: '11' } }, true],
  ]) {
    r.track.menu.value = { open: true, x: 0, y: 0, track }
    assert.equal(r.track.items.value.some(item => item.id === 'bili-video-skip'), expected)
    if (expected) {
      r.track.handleClick({ id: 'bili-video-skip', label: '' })
      assert.equal(r.track.videoSkipDialogOpen.value, true)
      assert.equal(r.track.videoSkipTarget.value.id, track.id)
      assert.notEqual(r.track.videoSkipTarget.value, track, 'the editor captures the menu target')
      assert.equal(r.track.menu.value.open, false)
    }
  }
})

for (const native of [true, false]) {
  await test(`${native ? 'native' : 'browser'} track and collection menus copy through the available clipboard`, async () => {
    const r = menuRuntime(native)
    r.track.menu.value.track = { id: 'netease:1', title: '歌曲', artist: '歌手' }
    r.track.handleClick({ id: 'copy-title', label: '' })
    await flush()
    r.track.handleClick({ id: 'copy-info', label: '' })
    await flush()
    r.collection.menu.value.target = { route: '/fixture', webUrl: 'https://music.163.com/playlist?id=1' }
    await r.collection.handleClick({ id: 'copy-link', label: '' })
    assert.deepEqual(native ? r.nativeWrites : r.browserWrites, ['歌曲', '歌曲 - 歌手', 'https://music.163.com/playlist?id=1'])
    assert.equal(native ? r.browserWrites.length : r.nativeWrites.length, 0)
    assert.deepEqual(r.messages, [['success', 'player.copied'], ['success', 'player.copied'], ['success', 'player.copied']])
  })
}

for (const native of [true, false]) {
  await test(`${native ? 'native' : 'browser'} clipboard failures are reported by both menus`, async () => {
    const r = menuRuntime(native, { nativeError: native, browserError: !native })
    r.track.menu.value.track = { id: 'netease:1', title: '歌曲', artist: '歌手' }
    r.track.handleClick({ id: 'copy-title', label: '' })
    await flush()
    r.collection.menu.value.target = { route: '/fixture', webUrl: 'https://music.163.com/playlist?id=1' }
    await r.collection.handleClick({ id: 'copy-link', label: '' })
    assert.deepEqual(r.messages, [['error', 'player.copy_failed'], ['error', 'player.copy_failed']])
    if (native) assert.deepEqual(r.nativeWrites, ['歌曲', 'https://music.163.com/playlist?id=1'], 'native errors must not switch to browser clipboard permissions')
  })
}

assert.equal(failures, 0, `${failures} context menu utility regressions failed`)
