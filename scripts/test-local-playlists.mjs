import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/localPlaylists.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const {
  LEGACY_PLAYLIST_ORDER_KEY,
  isEmptyLocalFilesPlaylist,
  isFavoritesPlaylist,
  isSystemPlaylist,
  localPlaylistDisplayName,
  playlistOrderIds,
  readLegacyPlaylistOrder,
  visibleSelection,
} = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

assert.equal(isFavoritesPlaylist({ id: -1001, name: 'Renamed elsewhere' }), true)
assert.equal(isFavoritesPlaylist({ id: '-3', name: 'my favorite music' }), true, 'names match case-insensitively on negative ids')
// 对齐 Android：用户自建的同名歌单（正数 id）不是系统歌单，可以删除、改名，也不会被当成收藏
assert.equal(isFavoritesPlaylist({ id: '42', name: 'My Favorite Music' }), false)
assert.equal(isSystemPlaylist({ id: 7, name: 'Local Files' }), false)
assert.equal(isSystemPlaylist({ id: -2, name: 'Local Files' }), true)
assert.equal(isSystemPlaylist({ id: 7, name: 'Road trip' }), false)
const labels = { favorites: 'Liked Songs', localFiles: 'Local Files' }
assert.equal(localPlaylistDisplayName({ id: -1001, name: '我喜欢的音乐' }, labels), 'Liked Songs')
assert.equal(localPlaylistDisplayName({ id: 1, name: '我喜欢的音乐' }, labels), '我喜欢的音乐')
assert.equal(localPlaylistDisplayName({ id: -1002, name: '本地文件' }, labels), 'Local Files')
assert.equal(localPlaylistDisplayName({ id: 2, name: 'Road trip' }, labels), 'Road trip')
assert.deepEqual([...visibleSelection(new Set([1, 2, 3]), [{ id: 3 }, { id: 1 }])].sort(), [1, 3])

assert.equal(isEmptyLocalFilesPlaylist({ id: -1002, name: '本地文件', track_count: 0 }), true)
assert.equal(isEmptyLocalFilesPlaylist({ id: 5, name: 'Local Music', track_count: 0 }), false)
assert.equal(isEmptyLocalFilesPlaylist({ id: -1002, name: '本地文件', track_count: 1 }), false)
assert.equal(isEmptyLocalFilesPlaylist({ id: 6, name: '自己的空歌单', track_count: 0 }), false)

const playlists = [
  { id: -1001, name: '我喜欢的音乐' },
  { id: 9007199254740991, name: 'Large' },
  { id: 3, name: 'Small' },
  { id: -1002, name: '本地音乐' },
]
const isProtected = (playlist) => playlist.id < 0
assert.deepEqual(playlistOrderIds(playlists, isProtected), ['9007199254740991', '3'])

const storage = (value) => ({ getItem: (key) => (key === LEGACY_PLAYLIST_ORDER_KEY ? value : null) })
assert.equal(readLegacyPlaylistOrder(undefined), null)
assert.equal(readLegacyPlaylistOrder(storage(null)), null)
assert.deepEqual(readLegacyPlaylistOrder(storage('[3, "7", 1.5, "x", -4]')), ['3', '7', '-4'])
assert.deepEqual(readLegacyPlaylistOrder(storage('{broken')), [])
assert.deepEqual(readLegacyPlaylistOrder(storage('{"a":1}')), [])

async function readPlaylistHandlers(view, names, bindings) {
  const source = await readFile(new URL(`../src/views/${view}.vue`, import.meta.url), 'utf8')
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
  const parsed = ts.createSourceFile(`${view}.ts`, script, ts.ScriptTarget.ES2022, true)
  const handlers = parsed.statements.filter(node => ts.isFunctionDeclaration(node) && names.includes(node.name?.text))
  const { outputText } = ts.transpileModule(handlers.map(node => node.getText(parsed)).join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  })
  return new Function(...Object.keys(bindings), `${outputText}\nreturn { ${names.join(', ')} }`)(...Object.values(bindings))
}

const parent = { id: 'bilibili:BV123', playlistKey: 'bili-parent', album: 'Bilibili||BV123' }
const part = { id: 'bilibili:BV123:456', playlistKey: 'bili-part', album: 'Bilibili|456|BV123' }
const other = { id: 'netease:1', playlistKey: 'song', album: 'Album' }
const tracks = { value: [parent, part, other] }
const played = []
const queued = []
const selections = []
const selectionMode = { value: false }
let pending
const bindings = {
  player: {
    playAll: (...args) => played.push(args),
    shufflePlay: (...args) => played.push(args),
    addToQueueNext: track => queued.push(track),
    addToQueueEnd: track => queued.push(track),
  },
  biliParts: { request: (track, play, playParts) => { pending = { track, play, playParts } } },
  closeTrackMenu() {},
  tracks,
}
const local = await readPlaylistHandlers('LocalPlaylistView', [
  'playTrack', 'playAll', 'shufflePlay', 'playSelected', 'addToQueueNext', 'addToQueueEnd', 'addSelectedToQueueEnd',
], {
  ...bindings,
  filteredTracks: tracks,
  selectedTracks: { value: [parent] },
  route: { params: { id: '13' } },
  selectionMode,
  toggleSelected: key => selections.push(key),
  trackSelectionKey: track => track.playlistKey,
  tracePlaybackUi() {},
  toast: { success() {} },
  t: key => key,
})
local.playTrack(parent)
assert.deepEqual(played, [], 'activating a Bilibili collection waits for part selection')
assert.equal(pending.track, parent)
pending.playParts([part], part.id)
assert.deepEqual(played.pop(), [[part], part.id, undefined, '13'], 'chosen parts retain the originating playlist for playback statistics')

local.playTrack(part)
assert.equal(pending.track, part)
pending.play()
assert.deepEqual(played.pop(), [tracks.value, part.id, part.playlistKey, '13'], 'an individually saved part can play from the original queue')

pending = null
selectionMode.value = true
local.playTrack(parent)
assert.equal(pending, null, 'selection mode toggles the row without requesting playback')
assert.deepEqual(selections, [parent.playlistKey])

const favorite = await readPlaylistHandlers('FavoritePlaylistView', ['playTrack', 'playAll', 'shufflePlay', 'handleTrackMenuClick'], {
  ...bindings,
  trackMenu: { value: { track: parent } },
})
favorite.playTrack(0)
assert.equal(pending.track, parent, 'Bilibili collections in saved playlist snapshots open the picker as well')
assert.deepEqual(played, [])
favorite.playTrack(2)
assert.equal(pending.track, other)
pending.play()
assert.deepEqual(played.at(-1), [tracks.value, other.id, other.playlistKey])

// 播放解析会在队列条目上补 CID，资料库行仍需保留完整视频身份
function resolveQueueCid(track) {
  track.album = 'Bilibili|456|BV123'
  assert.equal(parent.album, 'Bilibili||BV123', 'resolving a queued CID must not pin the library collection to one part')
}
resolveQueueCid(played.pop()[0][0])
selectionMode.value = false
local.playTrack(other)
pending.play()
resolveQueueCid(played.pop()[0][0])
for (const playCollection of [local.playAll, local.shufflePlay, local.playSelected, favorite.playAll, favorite.shufflePlay]) {
  playCollection()
  resolveQueueCid(played.pop()[0][0])
}
local.addToQueueNext(parent)
local.addToQueueEnd(parent)
local.addSelectedToQueueEnd()
favorite.handleTrackMenuClick({ id: 'play-next' })
favorite.handleTrackMenuClick({ id: 'add-to-queue' })
assert.equal(queued.length, 5)
for (const track of queued) resolveQueueCid(track)

console.log('local playlist visibility, order and Bilibili playback tests passed')
