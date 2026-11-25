import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import * as pinia from 'pinia'
import ts from 'typescript'
import * as vue from 'vue'

const source = await readFile(new URL('../src/components/NowPlaying.vue', import.meta.url), 'utf8')
const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const parsed = ts.createSourceFile('NowPlaying.ts', script, ts.ScriptTarget.ES2022, true)
// 歌手跳转与歌词回填已拆到 now-playing/ 下的组合式函数，按声明名在页面脚本和这些文件里查找
const extractedSources = [parsed, ...await Promise.all([
  'useNowPlayingContextMenu.ts', 'useTrackNavigation.ts', 'useLyricsEditing.ts', 'useTrackInfoEditing.ts', 'searchResultLyrics.ts',
].map(async file => ts.createSourceFile(file, await readFile(new URL(`../src/components/now-playing/${file}`, import.meta.url), 'utf8'), ts.ScriptTarget.ES2022, true)))]
function findFunction(name) {
  for (const sourceFile of extractedSources) {
    let found
    const visit = node => {
      if (found) return
      if (ts.isFunctionDeclaration(node) && node.name?.text === name) found = node
      else ts.forEachChild(node, visit)
    }
    visit(sourceFile)
    // 去掉 export 修饰，与原先页面里的局部函数形态一致
    if (found) return found.getText(sourceFile).replace(/^export\s+/, '')
  }
}
// 后端命令统一经页面的 nowPlayingIpc 发出：用页面里的真实命令映射接到各用例的假 invoke 上
const ipcStatement = parsed.statements.find(node => ts.isVariableStatement(node)
  && node.declarationList.declarations.some(declaration => declaration.name.getText(parsed) === 'nowPlayingIpc'))
assert.ok(ipcStatement)
const ipcCode = ts.transpileModule(ipcStatement.getText(parsed), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
const ipcFor = invoke => new Function('invoke', `${ipcCode}\nreturn nowPlayingIpc`)(invoke)
const navigationFunctions = ['openArtistPage', 'openCurrentArtist'].map(name => {
  const declaration = findFunction(name)
  assert.ok(declaration)
  return declaration
}).join('\n')
const compiled = ts.transpileModule(navigationFunctions, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText

function loadModule(source, dependencies = {}) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const exports = {}
  new Function('require', 'exports', compiled)(name => {
    assert.ok(name in dependencies, `unexpected dependency ${name}`)
    return dependencies[name]
  }, exports)
  return exports
}
const videoSkip = loadModule(await readFile(new URL('../src/modules/playback/biliVideoSkip.ts', import.meta.url), 'utf8'))
const covers = loadModule(await readFile(new URL('../src/utils/trackCover.ts', import.meta.url), 'utf8'))
const navigationSource = await readFile(new URL('../src/modules/library/artistTrackNavigation.ts', import.meta.url), 'utf8')
const artistNavigation = loadModule(await readFile(new URL('../src/modules/library/artistNavigation.ts', import.meta.url), 'utf8'))
const playbackSource = await readFile(new URL('../src/modules/playback/playbackSource.ts', import.meta.url), 'utf8')
const playbackParsed = ts.createSourceFile('playbackSource.ts', playbackSource, ts.ScriptTarget.ES2022, true)
const remoteKinds = playbackParsed.statements.find(node => ts.isVariableStatement(node)
  && node.declarationList.declarations.some(declaration => declaration.name.getText(playbackParsed) === 'REMOTE_SOURCE_KINDS'))
assert.ok(remoteKinds)
const playbackIdentityFunctions = ['getPlaybackSourceKind', 'syncPayloadString'].map(name => {
  const declaration = playbackParsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name)
  assert.ok(declaration)
  return declaration.getText(playbackParsed)
}).join('\n')
const playbackIdentity = loadModule(`${remoteKinds.getText(playbackParsed)}\n${playbackIdentityFunctions}`)
const localArtistsSource = await readFile(new URL('../src/modules/library/localArtists.ts', import.meta.url), 'utf8')
const localArtistsParsed = ts.createSourceFile('localArtists.ts', localArtistsSource, ts.ScriptTarget.ES2022, true)
const localArtistDeclarations = localArtistsParsed.statements.filter(node =>
  (ts.isFunctionDeclaration(node) && ['splitArtistNames', 'localArtistStableKey'].includes(node.name?.text))
  || (ts.isVariableStatement(node) && node.declarationList.declarations.some(declaration =>
    declaration.name.getText(localArtistsParsed) === 'ARTIST_SEPARATORS')))
const localArtists = loadModule(localArtistDeclarations.map(node => node.getText(localArtistsParsed)).join('\n'))

function artistPage(track, options = {}) {
  const routes = []
  const messages = []
  const commands = []
  const invoke = async (command, args) => {
    commands.push({ command, args })
    if (command === 'resolve_bili_uploader') return { id: '1', name: '原UP主', cover_url: null }
    if (command === 'get_bili_video_info') return options.videoInfo ? options.videoInfo() : {
      bvid: 'BV1xx411c7mD', aid: 123, title: '视频', owner: '原UP主', owner_mid: 1,
      owner_face: 'http://i0.hdslb.com/face.jpg', cover: '', cid: 456, duration: 60, pages: [],
    }
    if (command === 'get_netease_song_detail') return options.songDetail ? options.songDetail() : { songs: [{ id: 77, ar: [{ id: 10, name: '原歌手' }] }] }
    throw new Error(`unexpected command ${command}`)
  }
  const dependencies = {
    ...artistNavigation,
    ...loadModule(navigationSource, {
      '@tauri-apps/api/core': { invoke },
      '@/modules/playback/biliVideoSkip': videoSkip,
      '@/modules/playback/playbackSource': playbackIdentity,
      '@/utils/trackCover': covers,
    }),
    player: { currentTrack: track },
    artistLinksLoading: { value: false },
    artistLinks: { value: [] },
    artistLinkTrackId: { value: '' },
    contextMenu: { value: {} },
    getPlaybackSourceKind: playbackIdentity.getPlaybackSourceKind,
    router: { push: async route => {
      if (options.navigate) await options.navigate(route)
      routes.push(route)
    } },
    invoke,
    ipc: ipcFor(invoke),
    emit() {},
    toast: { show: message => messages.push(message), error: message => messages.push(message) },
    t: key => key,
    log: { warn() {}, error() {} },
    splitArtistNames: localArtists.splitArtistNames,
    currentNeteaseSongNumericId: { value: track.id.startsWith('netease:') ? Number(track.id.slice(8)) : null },
    primaryArtistName: { value: track.artist },
    hideMoreSheet() {},
  }
  const actions = new Function(...Object.keys(dependencies), `${compiled}\nreturn { openArtistPage, openCurrentArtist }`)(...Object.values(dependencies))
  return {
    open: () => actions.openArtistPage({ clientX: 0, clientY: 0 }), openMenu: actions.openCurrentArtist,
    routes, commands, messages, player: dependencies.player,
    artistLinks: dependencies.artistLinks, contextMenu: dependencies.contextMenu,
  }
}

await test('explicitly replaced artist searches its displayed name', async () => {
  const matchedArtist = artistPage({
    id: 'bilibili:BV1xx411c7mD', source: 'bilibili', title: '演唱片段', artist: '匹配歌手', album: '',
    syncPayload: { originalArtist: '原UP主', customArtist: '匹配歌手', matchedSongId: 'netease:123' },
  })
  await matchedArtist.open()
  assert.deepEqual(matchedArtist.routes, [{
    name: 'explore', query: { q: '匹配歌手', platform: 'netease', kind: 'artists' },
  }], 'an explicitly replaced artist must search the displayed artist instead of opening the audio uploader')
  assert.deepEqual(matchedArtist.commands, [], 'replaced artist navigation must not resolve the original audio identity')
})

for (const source of ['netease', 'qq', 'youtube']) {
  await test(`${source} audio searches explicitly replaced artist independently of lyric source`, async () => {
    const page = artistPage({
      id: `${source}:77`, source, title: '歌曲', artist: '替换歌手', album: '',
      syncPayload: { custom_artist: '替换歌手', original_artist: '原歌手', matchedSongId: 'qq:lyric-id', matchedLyricSource: 'CLOUD_MUSIC' },
    })
    await page.open()
    assert.deepEqual(page.routes, [{ name: 'explore', query: { q: '替换歌手', platform: 'netease', kind: 'artists' } }])
    assert.deepEqual(page.commands, [])
  })
}

await test('local artist edits retain aggregation by the displayed artist name', async () => {
  const page = artistPage({
    id: 'local:77', source: 'local', title: '歌曲', artist: '替换歌手', album: '',
    syncPayload: { customArtist: '替换歌手', originalArtist: '原歌手' },
  })
  await page.open()
  assert.deepEqual(page.routes, [{ name: 'local-artist', params: { name: '替换歌手' } }])
  assert.deepEqual(page.commands, [])
})

await test('multiple local artists offer a separate target for each displayed name', async () => {
  const page = artistPage({ id: 'local:77', source: 'local', artist: ' Alice / Bob / alice ', album: '' })
  await page.open()
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.artistLinks.value, [
    { id: 'Alice', name: 'Alice', route: { name: 'local-artist', params: { name: 'Alice' } } },
    { id: 'Bob', name: 'Bob', route: { name: 'local-artist', params: { name: 'Bob' } } },
  ])
  assert.equal(page.contextMenu.value.show, true)
  assert.equal(page.contextMenu.value.type, 'artist-page')
})

for (const [label, payload] of [
  ['unchanged override', { customArtist: '原UP主', originalArtist: '原UP主' }],
  ['unchanged stored artist', { customArtist: '原UP主', artist: '原UP主' }],
  ['empty override', { customArtist: '', originalArtist: '原UP主' }],
  ['missing override', { originalArtist: '原UP主', matchedSongId: 'netease:123', matchedLyricSource: 'CLOUD_MUSIC' }],
]) {
  await test(`Bilibili ${label} retains uploader navigation`, async () => {
    const page = artistPage({ id: 'bilibili:BV1xx411c7mD', source: 'bilibili', artist: '原UP主', album: '', syncPayload: payload })
    await page.open()
    assert.equal(page.routes[0]?.name, 'bili-artist')
    assert.equal(page.routes[0]?.params.mid, '1')
  })
}

await test('missing original artist still searches an explicit nonempty override', async () => {
  const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: '替换歌手', album: '', syncPayload: { customArtist: '替换歌手', originalArtist: '' } })
  await page.open()
  assert.deepEqual(page.routes, [{ name: 'explore', query: { q: '替换歌手', platform: 'netease', kind: 'artists' } }])
  assert.deepEqual(page.commands, [])
})

await test('a shared Bilibili track searches a displayed artist that differs from its uploader', async () => {
  const page = artistPage({
    id: 'bilibili:123', source: 'bilibili', artist: '房主选择的歌手', album: '',
    syncPayload: { listenTogetherDisplayArtist: '房主选择的歌手' },
  })
  await page.open()
  assert.deepEqual(page.routes, [{ name: 'explore', query: { q: '房主选择的歌手', platform: 'netease', kind: 'artists' } }])
  assert.deepEqual(page.commands, [{ command: 'get_bili_video_info', args: { bvid: null, avid: 123 } }])
})

await test('a shared Bilibili track with its original uploader still opens the uploader page', async () => {
  const page = artistPage({
    id: 'bilibili:123', source: 'bilibili', artist: '原UP主', album: '',
    syncPayload: { listenTogetherDisplayArtist: '原UP主' },
  })
  await page.open()
  assert.equal(page.routes[0]?.name, 'bili-artist')
  assert.equal(page.routes[0]?.params.mid, '1')
  assert.deepEqual(page.commands, [{ command: 'get_bili_video_info', args: { bvid: null, avid: 123 } }])
})

await test('unchanged NetEase artist still uses its exact platform identity', async () => {
  const page = artistPage({ id: 'netease:77', source: 'netease', artist: '原歌手', album: '', syncPayload: { customArtist: '原歌手', originalArtist: '原歌手' } })
  await page.open()
  assert.deepEqual(page.commands, [{ command: 'get_netease_song_detail', args: { songId: 77 } }])
  assert.deepEqual(page.routes, [{ name: 'netease-artist', params: { id: '10' } }])
})

await test('NetEase menu entry searches an explicitly replaced artist', async () => {
  const page = artistPage({ id: 'netease:77', source: 'netease', artist: '匹配歌手', album: '', syncPayload: { customArtist: '匹配歌手', originalArtist: '原歌手' } })
  await page.openMenu()
  assert.deepEqual(page.routes, [{ name: 'explore', query: { q: '匹配歌手', platform: 'netease', kind: 'artists' } }])
  assert.deepEqual(page.commands, [])
})

for (const [entry, expectedMessage] of [['open', 'player.artist_load_failed'], ['openMenu', 'Error: route load failed']]) {
  await test(`${entry} reports a rejected artist search without an unhandled rejection`, async () => {
    const page = artistPage({
      id: 'netease:77', source: 'netease', artist: '匹配歌手', album: '',
      syncPayload: { customArtist: '匹配歌手', originalArtist: '原歌手' },
    }, { navigate: async () => { throw new Error('route load failed') } })
    await assert.doesNotReject(page[entry]())
    assert.deepEqual(page.routes, [])
    assert.deepEqual(page.messages, [expectedMessage])
  })
}

await test('NetEase menu entry retains the original exact artist link', async () => {
  const page = artistPage({ id: 'netease:77', source: 'netease', artist: '原歌手', album: '' })
  await page.openMenu()
  assert.deepEqual(page.routes, [{ name: 'netease-artist', params: { id: '10' }, query: { name: '原歌手' } }])
})

await test('NetEase menu lookup discards a same-track artist edit', async () => {
  let resolveDetail
  const pending = new Promise(resolve => { resolveDetail = resolve })
  const page = artistPage({ id: 'netease:77', source: 'netease', artist: '原歌手', album: '' }, { songDetail: () => pending })
  const opening = page.openMenu()
  page.player.currentTrack = { ...page.player.currentTrack, artist: '新歌手', syncPayload: { customArtist: '新歌手', originalArtist: '原歌手' } }
  resolveDetail({ songs: [{ id: 77, ar: [{ id: 10, name: '原歌手' }] }] })
  await opening
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.messages, [])
})

for (const [label, track, expected] of [
  ['stored BV', { syncPayload: { bvid: 'BV1xx411c7mD', audioId: '123', subAudioId: '456' } }, { bvid: 'BV1xx411c7mD', avid: null }],
  ['album BV', { album: 'Bilibili|456|BV1xx411c7mD' }, { bvid: 'BV1xx411c7mD', avid: null }],
  ['Android AV', { syncPayload: { audioId: '123', subAudioId: '456' } }, { bvid: null, avid: 123 }],
  ['part AV', {}, { bvid: null, avid: 123 }],
  ['legacy audio BV', { syncPayload: { audio_id: 'BV1xx411c7mD' } }, { bvid: 'BV1xx411c7mD', avid: null }],
]) {
  await test(`Bilibili artist resolves ${label} identity`, async () => {
    const page = artistPage({
      id: 'bilibili:123:456', source: 'bilibili', title: '视频', artist: '原UP主', album: '', ...track,
    })
    await page.open()
    assert.deepEqual(page.commands, [{ command: 'get_bili_video_info', args: expected }])
    assert.deepEqual(page.routes, [{ name: 'bili-artist', params: { mid: '1' }, query: { name: '原UP主', cover: 'https://i0.hdslb.com/face.jpg' } }])
    assert.deepEqual(page.messages, [])
  })
}

await test('invalid Bilibili identity reports unavailable without a video request', async () => {
  const page = artistPage({ id: 'bilibili:invalid', source: 'bilibili', artist: '原UP主', album: '', syncPayload: { audioId: '9007199254740992' } })
  await page.open()
  assert.deepEqual(page.commands, [])
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.messages, ['player.bili_uploader_unavailable'])
})

await test('same-track artist edits discard an earlier pending uploader result', async () => {
  let resolveInfo
  const pending = new Promise(resolve => { resolveInfo = resolve })
  const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: '原UP主', album: '' }, { videoInfo: () => pending })
  const opening = page.open()
  page.player.currentTrack = { ...page.player.currentTrack, artist: '新歌手', syncPayload: { customArtist: '新歌手', originalArtist: '原UP主' } }
  resolveInfo({ owner_mid: 1, owner: '原UP主', owner_face: '' })
  await opening
  assert.deepEqual(page.routes, [], 'editing the artist while its lookup is pending must not navigate to the previous artist')
  assert.deepEqual(page.messages, [])
})

for (const [label, update] of [
  ['lyric offset', track => ({ ...track, syncPayload: { ...track.syncPayload, userLyricOffsetMs: 500 } })],
  ['cover', track => ({ ...track, coverUrl: 'https://example.com/new-cover.jpg' })],
  ['lyrics', track => ({ ...track, syncPayload: { ...track.syncPayload, matchedLyric: '[00:01.00]新歌词' } })],
]) {
  await test(`Bilibili ${label} updates retain a pending same-artist navigation`, async () => {
    let resolveInfo
    const pending = new Promise(resolve => { resolveInfo = resolve })
    const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: '原UP主', album: '' }, { videoInfo: () => pending })
    const opening = page.open()
    page.player.currentTrack = update(page.player.currentTrack)
    resolveInfo({ owner_mid: 1, owner: '原UP主', owner_face: '' })
    await opening
    assert.deepEqual(page.routes, [{ name: 'bili-artist', params: { mid: '1' }, query: { name: '原UP主', cover: '' } }])
    assert.deepEqual(page.messages, [])
  })
  await test(`NetEase menu ${label} updates retain a pending same-artist navigation`, async () => {
    let resolveDetail
    const pending = new Promise(resolve => { resolveDetail = resolve })
    const page = artistPage({ id: 'netease:77', source: 'netease', artist: '原歌手', album: '' }, { songDetail: () => pending })
    const opening = page.openMenu()
    page.player.currentTrack = update(page.player.currentTrack)
    resolveDetail({ songs: [{ id: 77, ar: [{ id: 10, name: '原歌手' }] }] })
    await opening
    assert.deepEqual(page.routes, [{ name: 'netease-artist', params: { id: '10' }, query: { name: '原歌手' } }])
    assert.deepEqual(page.messages, [])
  })
}

for (const [label, update] of [
  ['video BV', track => ({ ...track, syncPayload: { bvid: 'BV1xx411c7mD' } })],
  ['video AV', track => ({ ...track, syncPayload: { audioId: '456' } })],
  ['artist navigation mode', track => ({ ...track, syncPayload: { customArtist: '原UP主', originalArtist: '另一位作者' } })],
  ['cleared current track', () => null],
]) {
  await test(`changed ${label} discards an earlier Bilibili artist lookup`, async () => {
    let resolveInfo
    const pending = new Promise(resolve => { resolveInfo = resolve })
    const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: '原UP主', album: '' }, { videoInfo: () => pending })
    const opening = page.open()
    page.player.currentTrack = update(page.player.currentTrack)
    resolveInfo({ owner_mid: 1, owner: '原UP主', owner_face: '' })
    await opening
    assert.deepEqual(page.routes, [])
    assert.deepEqual(page.messages, [])
  })
}

await test('changed NetEase audio identity discards an earlier artist lookup', async () => {
  let resolveDetail
  const pending = new Promise(resolve => { resolveDetail = resolve })
  const page = artistPage({ id: 'netease:77', source: 'netease', artist: '原歌手', album: '' }, { songDetail: () => pending })
  const opening = page.open()
  page.player.currentTrack = { ...page.player.currentTrack, syncPayload: { audioId: '88' } }
  resolveDetail({ songs: [{ id: 77, ar: [{ id: 10, name: '原歌手' }] }] })
  await opening
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.messages, [])
})

for (const info of [null, { owner_mid: 0 }, { owner_mid: -1 }, { owner_mid: Number.MAX_SAFE_INTEGER + 1 }]) {
  await test(`unavailable Bilibili owner ${JSON.stringify(info)} reports no uploader`, async () => {
    const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: 'UP主', album: '' }, { videoInfo: async () => info })
    await page.open()
    assert.deepEqual(page.routes, [])
    assert.deepEqual(page.messages, ['player.bili_uploader_unavailable'])
  })
}

await test('current Bilibili request failures remain visible', async () => {
  const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: 'UP主', album: '' }, {
    videoInfo: async () => { throw new Error('network unavailable') },
  })
  await page.open()
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.messages, ['player.artist_load_failed'])
})

await test('stale Bilibili request failures do not show an error for the edited artist', async () => {
  let rejectInfo
  const pending = new Promise((_resolve, reject) => { rejectInfo = reject })
  const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: '原UP主', album: '' }, { videoInfo: () => pending })
  const opening = page.open()
  page.player.currentTrack = { ...page.player.currentTrack, artist: '新歌手' }
  rejectInfo(new Error('network unavailable'))
  await opening
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.messages, [])
})

await test('stale Bilibili owner absence does not report unavailable for the edited artist', async () => {
  let resolveInfo
  const pending = new Promise(resolve => { resolveInfo = resolve })
  const page = artistPage({ id: 'bilibili:123', source: 'bilibili', artist: '原UP主', album: '' }, { videoInfo: () => pending })
  const opening = page.open()
  page.player.currentTrack = { ...page.player.currentTrack, artist: '新歌手' }
  resolveInfo(null)
  await opening
  assert.deepEqual(page.routes, [])
  assert.deepEqual(page.messages, [])
})

const format = loadModule(await readFile(new URL('../src/modules/lyrics/lyricsFormat.ts', import.meta.url), 'utf8'))
const matchSource = await readFile(new URL('../src/modules/lyrics/lyricMatch.ts', import.meta.url), 'utf8')
const searchTitle = loadModule(await readFile(new URL('../src/modules/lyrics/lyricSearchTitle.ts', import.meta.url), 'utf8'))
const syncSource = await readFile(new URL('../src/modules/lyrics/syncTrackPayload.ts', import.meta.url), 'utf8')
const offsetSource = await readFile(new URL('../src/modules/lyrics/lyricOffset.ts', import.meta.url), 'utf8')
const offsetStoreSource = await readFile(new URL('../src/stores/lyricOffset.ts', import.meta.url), 'utf8')
const userDataSource = await readFile(new URL('../src/modules/persistence/userData.ts', import.meta.url), 'utf8')
const lyricSource = loadModule(await readFile(new URL('../src/modules/lyrics/lyricSource.ts', import.meta.url), 'utf8'), { vue })
const offset = loadModule(offsetSource)
// mapBackendLyrics 不再有页面内的包装，直接用 lyricsFormat 的同名导出（已随 ...format 注入）
const lyricFunctions = ['onLyricMatchPicked', 'commitLyricsToTrack', 'confirmApplySearchResult', 'parseLyricsFromSearchResult',
  'lyricSourceForPlatform'].map(name => {
  const declaration = findFunction(name)
  assert.ok(declaration)
  return declaration
}).join('\n')
const compiledLyrics = ts.transpileModule(lyricFunctions, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
async function lyricActions(fields, { beforeTrackWrite = async () => {} } = {}) {
  const stored = []
  const player = {
    currentTrack: {
      id: 'bilibili:123', source: 'bilibili', title: '原歌曲', artist: '原UP主', album: 'Bilibili',
      coverUrl: 'https://i0.hdslb.com/cover.jpg', durationMs: 60000, audioUrl: '',
      syncPayload: { channelId: 'bilibili', audioId: '123', matchedLyric: '[00:01.00]旧歌词' },
    },
    patchCurrentTrackSyncPayload(payload) { this.currentTrack = { ...this.currentTrack, syncPayload: payload } },
    updateCurrentTrackInfo(patch) { this.currentTrack = { ...this.currentTrack, ...patch } },
  }
  const invoke = async (command, args) => {
    if (command === 'load_user_data_snapshot') {
      return { lyricOffsets: {}, migrated: { lyricOffsets: true, playbackState: true, history: true } }
    }
    if (command === 'set_lyric_offset') {
      stored.push({ command, ...args })
      return
    }
    if (command === 'parse_lrc_content') return [{ start_ms: 1000, duration_ms: 1000, text: '新歌词', words: [] }]
    if (command === 'update_playlist_track' || command === 'record_lyric_override') {
      if (command === 'update_playlist_track') await beforeTrackWrite(args.track)
      stored.push({ command, track: args.track })
      return 1
    }
    throw new Error(`unexpected command ${command}`)
  }
  const log = { warn() {}, error() {} }
  const userData = loadModule(userDataSource, { '@tauri-apps/api/core': { invoke }, '@/utils/logger': { createLogger: () => log } })
  await userData.preloadUserData({ getItem: () => null, removeItem() {} })
  const sync = loadModule(syncSource, {
    '@tauri-apps/api/core': { invoke }, '@/utils/logger': { createLogger: () => log },
    '@/modules/persistence/userData': userData,
  })
  const match = loadModule(matchSource, { '@tauri-apps/api/core': { invoke }, './lyricsFormat': format, './lyricSearchTitle': searchTitle })
  const settings = {
    isHydrated: true,
    cloudMusicOffset: 1000, qqMusicOffset: 500, kugouOffset: 150, lrclibOffset: 0,
    amllTtmlOffset: 0, biliCcSubtitleOffset: 0,
  }
  const offsetStore = loadModule(offsetStoreSource, {
    pinia, vue,
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/modules/lyrics/lyricOffset': offset,
    '@/modules/lyrics/lyricSource': lyricSource,
    '@/modules/lyrics/syncTrackPayload': sync,
    '@/stores/player': { usePlayerStore: () => player },
    '@/modules/persistence/userData': userData,
    '@/utils/logger': { createLogger: () => log },
  }).useLyricOffsetStore(pinia.createPinia())
  lyricSource.rememberLyricSource(player.currentTrack, null)
  let displayedLyrics = []
  const result = {
    id: 'netease:77', source: 'netease', title: '匹配歌曲', artist: '匹配歌手', cover_url: 'https://p1.music.126.net/match.jpg',
    synced_lyrics: '[00:01.00]新歌词',
    lines: [{ startMs: 1000, durationMs: 1000, text: '新歌词', words: [] }],
  }
  const dependencies = {
    ...format, ...sync, ...match, ...offset, ...lyricSource, player, invoke, ipc: ipcFor(invoke), log, lyricOffsetStore: offsetStore,
    mapBackendLyricsShared: format.mapBackendLyrics,
    currentLyrics: { replace(lines) { displayedLyrics = lines } },
    cacheLyricsForTrack() {},
    toast: { success() {}, error(message) { throw new Error(message) } }, t: key => key,
    lyricMatchTarget: { value: 'track' },
    infoApplyCandidate: { value: result }, applyInfoFields: { value: fields },
    goBackToMain() {}, goBackTo() {},
  }
  const actions = new Function(...Object.keys(dependencies), `${compiledLyrics}\nreturn { onLyricMatchPicked, confirmApplySearchResult }`)(...Object.values(dependencies))
  return { ...actions, result, player, stored, offsetStore, get displayedLyrics() { return displayedLyrics } }
}

for (const path of ['lyric matcher', 'song information with only lyrics selected']) {
  await test(`${path} preserves the original audio metadata`, async () => {
    const actions = await lyricActions({ title: false, artist: false, cover: false, lyrics: true })
    if (path === 'lyric matcher') await actions.onLyricMatchPicked(actions.result)
    else await actions.confirmApplySearchResult()
    const track = actions.player.currentTrack
    assert.deepEqual({ id: track.id, source: track.source, title: track.title, artist: track.artist, cover: track.coverUrl }, {
      id: 'bilibili:123', source: 'bilibili', title: '原歌曲', artist: '原UP主', cover: 'https://i0.hdslb.com/cover.jpg',
    })
    assert.equal(track.syncPayload.matchedLyric, '[00:01.00]新歌词')
    assert.equal(track.syncPayload.matchedLyricSource, 'CLOUD_MUSIC')
    assert.equal(track.syncPayload.customArtist, undefined)
    assert.equal(track.syncPayload.customCoverUrl, undefined)
    assert.equal(actions.stored.find(value => value.command === 'update_playlist_track').track.artist, '原UP主')
  })
}

await test('selecting Kugou after adjusting NetEase lyrics resets to the configured Kugou default', async () => {
  const actions = await lyricActions({})
  const { player, offsetStore, stored, result } = actions
  await actions.onLyricMatchPicked(result)
  offsetStore.setEffectiveOffsetMs(player.currentTrack, -200)
  assert.equal(offsetStore.effectiveOffsetMs(player.currentTrack), -200)

  const kugou = { ...result, source: 'kugou', lines: [{ ...result.lines[0], text: '酷狗歌词' }] }
  await actions.onLyricMatchPicked(kugou)

  assert.deepEqual(actions.displayedLyrics, kugou.lines)
  assert.equal(offsetStore.offsetSourceFor(player.currentTrack), 'kugou')
  assert.equal(offsetStore.effectiveOffsetMs(player.currentTrack), 150)
  assert.equal(offsetStore.getUserOffsetMs(player.currentTrack), 0)
  assert.equal(offsetStore.offsets[player.currentTrack.id], undefined)
  assert.equal(player.currentTrack.syncPayload.matchedLyricSource, 'KUGOU')
  assert.equal(player.currentTrack.syncPayload.userLyricOffsetMs, 0)
  assert.deepEqual(stored.filter(entry => entry.command === 'set_lyric_offset').map(entry => entry.offsetMs), [-1200, 0])
  for (const command of ['update_playlist_track', 'record_lyric_override']) {
    const { sync_payload } = stored.findLast(entry => entry.command === command).track
    assert.equal(sync_payload.matchedLyric, '[00:01.00]酷狗歌词')
    assert.equal(sync_payload.matchedLyricSource, 'KUGOU')
    assert.equal(sync_payload.userLyricOffsetMs, 0)
  }
})

await test('matching from the same provider keeps its manual offset', async () => {
  const { player, offsetStore, result, onLyricMatchPicked } = await lyricActions({})
  await onLyricMatchPicked(result)
  offsetStore.setEffectiveOffsetMs(player.currentTrack, -200)
  await onLyricMatchPicked(result)
  assert.equal(offsetStore.offsetSourceFor(player.currentTrack), 'netease')
  assert.equal(offsetStore.effectiveOffsetMs(player.currentTrack), -200)
})

await test('matching the saved provider preserves a synced offset without local calibration metadata', async () => {
  const { player, offsetStore, result, onLyricMatchPicked } = await lyricActions({})
  player.patchCurrentTrackSyncPayload({
    matched_lyric_source: 'KUGOU',
    user_lyric_offset_ms: -200,
  })
  await onLyricMatchPicked({ ...result, source: 'kugou' })

  assert.equal(offsetStore.getUserOffsetMs(player.currentTrack), -200)
})

await test('reselecting the displayed provider clears an offset still tied to the old provider', async () => {
  const { player, offsetStore, result, onLyricMatchPicked } = await lyricActions({})
  await onLyricMatchPicked(result)
  offsetStore.setEffectiveOffsetMs(player.currentTrack, -200)
  player.patchCurrentTrackSyncPayload({ ...player.currentTrack.syncPayload, matchedLyricSource: 'KUGOU' })
  lyricSource.rememberLyricSource(player.currentTrack, 'KUGOU')
  assert.equal(offsetStore.offsetSourceFor(player.currentTrack), 'netease')

  await onLyricMatchPicked({ ...result, source: 'kugou' })
  assert.equal(offsetStore.offsetSourceFor(player.currentTrack), 'kugou')
  assert.equal(offsetStore.effectiveOffsetMs(player.currentTrack), 150)
})

await test('a pending offset write cannot overwrite the newly selected lyrics', async () => {
  let markOldWriteStarted
  let releaseOldWrite
  const oldWriteStarted = new Promise(resolve => { markOldWriteStarted = resolve })
  const oldWritePending = new Promise(resolve => { releaseOldWrite = resolve })
  const { player, offsetStore, stored, result, onLyricMatchPicked } = await lyricActions({}, {
    async beforeTrackWrite(track) {
      if (track.sync_payload.matchedLyricSource !== 'CLOUD_MUSIC' || track.sync_payload.userLyricOffsetMs !== -1200) return
      markOldWriteStarted()
      await oldWritePending
    },
  })
  await onLyricMatchPicked(result)
  offsetStore.setEffectiveOffsetMs(player.currentTrack, -200)
  await oldWriteStarted
  const apply = onLyricMatchPicked({ ...result, source: 'kugou' })
  await new Promise(resolve => setImmediate(resolve))
  const wroteKugouEarly = stored.some(entry => entry.track?.sync_payload.matchedLyricSource === 'KUGOU')
  releaseOldWrite()
  await apply
  assert.equal(wroteKugouEarly, false)
  assert.deepEqual(stored.filter(entry => entry.command === 'update_playlist_track').slice(1).map(entry => [
    entry.track.sync_payload.matchedLyricSource,
    entry.track.sync_payload.userLyricOffsetMs,
  ]), [['CLOUD_MUSIC', -1200], ['KUGOU', 0]])
})

await test('explicit artist and cover selection still persists custom display metadata', async () => {
  const actions = await lyricActions({ title: false, artist: true, cover: true, lyrics: false })
  await actions.confirmApplySearchResult()
  const track = actions.player.currentTrack
  assert.equal(track.id, 'bilibili:123')
  assert.equal(track.source, 'bilibili')
  assert.equal(track.title, '原歌曲')
  assert.equal(track.artist, '匹配歌手')
  assert.equal(track.coverUrl, 'https://p1.music.126.net/match.jpg')
  assert.equal(track.syncPayload.customArtist, '匹配歌手')
  assert.equal(track.syncPayload.originalArtist, '原UP主')
  assert.equal(track.syncPayload.customCoverUrl, 'https://p1.music.126.net/match.jpg')
  assert.equal(track.syncPayload.matchedLyric, '[00:01.00]旧歌词')
  assert.equal(actions.stored[0].track.artist, '匹配歌手')
})
