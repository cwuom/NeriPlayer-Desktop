import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function plainModule(file) {
  const source = await readFile(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
  }).outputText
  return import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
}

async function functions(file, names, context) {
  const component = await readFile(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const script = file.endsWith('.vue') ? component.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1] : component
  const parsed = ts.createSourceFile('view.ts', script, ts.ScriptTarget.ES2022, true)
  const source = names.map(name => {
    const node = parsed.statements.find(item => ts.isFunctionDeclaration(item) && item.name?.text === name)
    assert.ok(node, `missing actual menu handler ${file}:${name}`)
    // 纯 TS 模块里的处理函数是导出声明，去掉 export 后与组件内函数同样求值
    return node.getText(parsed).replace(/^export\s+/, '')
  }).join('\n')
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
  return new Function(...Object.keys(context), `${compiled}; return { ${names.join(',')} }`)(...Object.values(context))
}

const { collectionWebUrl } = await plainModule('utils/collectionLinks.ts')
assert.equal(collectionWebUrl('youtube', 'album', 'MPREb_demo'), '', 'an album browseId is not a web playlistId')
const bili = await plainModule('modules/library/biliPlaylistReference.ts')
const routed = [], menus = [], played = []
const event = { type: 'contextmenu' }
const router = { push: route => routed.push(route) }
const collectionMenuRef = { value: { open: (event, target) => menus.push(target) } }
const favoriteLinks = await functions('components/library/favoriteLinks.ts', ['favoriteRoute', 'favoriteWebUrl'], {
  collectionWebUrl, ...bili,
  isArtistFavoriteSource: source => ['neteaseArtist', 'biliArtist', 'youtubeMusicArtist'].includes(source),
  favoriteArtistRoute: favorite => ({ name: 'fixture-artist', params: { id: favorite.browseId || favorite.id } }),
})
const library = await functions('views/LibraryView.vue', ['openFavorite', 'openFavoriteMenu'], {
  router, collectionMenuRef, ...favoriteLinks,
  toast: { show() { assert.fail('valid favorite routes must not produce an error') } }, t: key => key,
})
for (const favorite of [
  { source: 'netease', id: '123' }, { source: 'neteaseAlbum', id: '234' },
  { source: 'youtubeMusic', id: 'hash', browseId: 'VLPLdemo' },
  { source: 'bili', id: '345' },
  { source: 'bili', id: '456', browseId: 'bili-playlist/v1/COLLECTION/456/567' },
  { source: 'neteaseArtist', id: '678' },
  { source: 'youtubeMusicArtist', id: 'hash', browseId: 'UCdemo' },
  { source: 'unknown', id: '789' },
]) {
  library.openFavorite(favorite)
  library.openFavoriteMenu(event, favorite)
  assert.deepEqual(menus.at(-1).route, routed.at(-1), `right click preserves ${favorite.source} routing`)
}
assert.equal(menus[2].webUrl, 'https://music.youtube.com/playlist?list=PLdemo')
assert.equal(menus[4].webUrl, 'https://space.bilibili.com/567/lists/456?type=season')
assert.equal(menus[6].webUrl, 'https://music.youtube.com/channel/UCdemo')
assert.equal(menus[7].webUrl, undefined, 'local fallback collections do not invent external URLs')

const resolvedAlbum = { source: 'youtubeMusic', id: 'album-hash', browseId: 'MPREb_demo', playlistId: 'OLAK5uy_demo' }
library.openFavorite(resolvedAlbum)
library.openFavoriteMenu(event, resolvedAlbum)
assert.deepEqual(menus.at(-1).route, routed.at(-1), 'resolved albums keep the browse route')
assert.equal(menus.at(-1).webUrl, 'https://music.youtube.com/playlist?list=OLAK5uy_demo', 'resolved album links use the available playlist identity')

const artist = await functions('views/YouTubeArtistView.vue', ['collectionRoute', 'openItem', 'openCollectionMenu'], {
  router, collectionMenuRef, collectionWebUrl,
  playSection: (section, item) => played.push(item),
  openTrackMenu: (event, section, item) => played.push(item),
})
for (const kind of ['artist', 'playlist', 'album']) {
  const item = { kind, browseId: kind === 'artist' ? 'UCdemo' : kind === 'album' ? 'MPREb_demo' : 'VLPLdemo', title: 'Title', coverUrl: '', subtitle: '' }
  artist.openItem({}, item)
  artist.openCollectionMenu(event, {}, item)
  assert.deepEqual(menus.at(-1).route, routed.at(-1))
}
const video = { kind: 'video', videoId: 'song1' }
artist.openItem({}, video)
artist.openCollectionMenu(event, {}, video)
assert.deepEqual(played, [video, video], 'videos use track operations instead of collection routes')

const shelf = await functions('views/ExploreView.vue', ['youtubeShelfTrack', 'youtubeShelfTarget', 'openYoutubeShelfMenu', 'goToYoutubeShelfItem'], {
  router, collectionMenuRef, collectionWebUrl,
  trackMenuRef: { value: { open: (event, track) => played.push(track) } },
  player: { play: track => played.push(track) },
})
const song = { videoId: 'loaded', title: 'Song', subtitle: 'Artist', durationMs: 9000, coverUrl: '' }
shelf.goToYoutubeShelfItem(song)
shelf.openYoutubeShelfMenu(event, song)
assert.deepEqual(played.at(-1), played.at(-2), 'shelf clicks and menus share complete track metadata')
assert.equal(played.at(-1).durationMs, 9000)
const shelfArtist = { browseId: 'UCdemo', pageType: 'MUSIC_PAGE_TYPE_ARTIST', title: 'Artist', subtitle: '', coverUrl: '' }
shelf.goToYoutubeShelfItem(shelfArtist)
shelf.openYoutubeShelfMenu(event, shelfArtist)
assert.equal(routed.at(-1).name, 'youtube-artist', 'artist shelf cards open the artist page')
assert.deepEqual(menus.at(-1).route, routed.at(-1), 'artist shelf clicks and menus share their destination')
assert.equal(menus.at(-1).webUrl, 'https://music.youtube.com/channel/UCdemo')
console.log('favorite, creator and discovery collection menu routing tests passed')
