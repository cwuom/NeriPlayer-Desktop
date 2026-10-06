import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/youtubeArtistDetail.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText
const { parseYouTubeArtistDetail, parseYouTubeArtistItems, youtubeArtistItemTrack } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
const responsive = { musicResponsiveListItemRenderer: {
  playlistItemData: { videoId: 'song1' },
  flexColumns: [
    { musicResponsiveListItemFlexColumnRenderer: { text: { runs: [{ text: 'Song title' }] } } },
    { musicResponsiveListItemFlexColumnRenderer: { text: { runs: [{ text: 'Singer', navigationEndpoint: { browseEndpoint: { browseId: 'UC_singer' } } }, { text: ' • ' }, { text: 'Album', navigationEndpoint: { browseEndpoint: { browseId: 'MPREb_album' } } }, { text: ' • ' }, { text: '3:45' }] } } },
  ], thumbnail: { musicThumbnailRenderer: { thumbnail: { thumbnails: [{ url: '//cover/song' }] } } },
} }
function card(title, browseId) { return { musicTwoRowItemRenderer: { title: { simpleText: title }, subtitle: { simpleText: 'Subtitle' }, navigationEndpoint: { browseEndpoint: { browseId } } } } }
const response = {
  header: { musicImmersiveHeaderRenderer: { title: { simpleText: 'Creator' }, description: { simpleText: 'Description' }, shortSubscriberCountText: { simpleText: '3M subscribers' } } },
  contents: { singleColumnBrowseResultsRenderer: { tabs: [{ tabRenderer: { content: { sectionListRenderer: { contents: [
    { musicShelfRenderer: { title: { simpleText: 'TOP SONGS' }, contents: [responsive], bottomEndpoint: { browseEndpoint: { browseId: 'UC_demo', params: 'songs-params' } } } },
    { musicCarouselShelfRenderer: { header: { musicCarouselShelfBasicHeaderRenderer: { title: { simpleText: 'Albums' } } }, contents: [card('Album', 'MPREb_album')] } },
    { musicCarouselShelfRenderer: { header: { musicCarouselShelfBasicHeaderRenderer: { title: { simpleText: 'Fans might also like' } } }, contents: [card('Related artist', 'UC_related')] } },
    { musicCarouselShelfRenderer: { header: { musicCarouselShelfBasicHeaderRenderer: { title: { simpleText: 'Featured on' } } }, contents: [card('Playlist', 'VLPLdemo')] } },
  ] } } } }] } },
}
const detail = parseYouTubeArtistDetail(response, { name: 'Fallback', coverUrl: 'fallback.jpg', subtitle: 'Fallback subtitle' })
assert.equal(detail.header.name, 'Creator')
assert.equal(detail.header.coverUrl, 'fallback.jpg')
assert.deepEqual(detail.sections.map(section => section.title), ['TOP SONGS', 'Albums', 'Fans might also like', 'Featured on'])
assert.deepEqual(detail.sections[0].moreEndpoint, { browseId: 'UC_demo', params: 'songs-params' })
assert.deepEqual(detail.sections.map(section => section.items[0].kind), ['song', 'album', 'artist', 'playlist'])
const track = youtubeArtistItemTrack(detail.sections[0].items[0], detail.header.name)
assert.equal(track.id, 'youtube:song1')
assert.equal(track.artist, 'Singer')
assert.equal(track.album, 'Album')
assert.equal(track.durationMs, 225000)
assert.equal(track.coverUrl, 'https://cover/song')
assert.equal(track.syncPayload.audioId, 'song1')
assert.equal(youtubeArtistItemTrack(detail.sections[1].items[0], 'Creator'), null)
const continuation = parseYouTubeArtistItems({ continuationContents: { musicShelfContinuation: { contents: [responsive], continuations: [{ nextContinuationData: { continuation: 'next-page' } }] } } })
assert.equal(continuation.items.length, 1)
assert.equal(continuation.continuation, 'next-page')
const allSongsFirstPage = parseYouTubeArtistItems({ contents: { singleColumnBrowseResultsRenderer: { tabs: [
  { tabRenderer: { content: { sectionListRenderer: { contents: [{ musicPlaylistShelfRenderer: {
    title: { simpleText: 'All songs' }, contents: [responsive],
    continuations: [{ nextContinuationData: { continuation: 'all-songs-next' } }],
  } }] } } } },
] } } })
assert.equal(allSongsFirstPage.items.length, 1)
assert.equal(allSongsFirstPage.items[0].videoId, 'song1')
assert.equal(allSongsFirstPage.continuation, 'all-songs-next')
const allSongsContinuation = parseYouTubeArtistItems({ continuationContents: { musicPlaylistShelfContinuation: {
  contents: [responsive], continuations: [{ nextContinuationData: { continuation: 'all-songs-last' } }],
} } })
assert.equal(allSongsContinuation.items.length, 1)
assert.equal(allSongsContinuation.continuation, 'all-songs-last')
const twoRowVideo = { musicTwoRowItemRenderer: { title: { simpleText: 'Video' }, navigationEndpoint: { watchEndpoint: { videoId: 'video1' } }, subtitle: { runs: [{ text: 'Creator' }, { text: ' • ' }, { text: '4:01' }] } } }
assert.equal(parseYouTubeArtistItems({ onResponseReceivedActions: [{ appendContinuationItemsAction: { continuationItems: [twoRowVideo] } }] }).items[0].kind, 'video')
assert.equal(parseYouTubeArtistDetail({}, { name: 'Fallback' }).header.name, 'Fallback')
assert.deepEqual(parseYouTubeArtistItems(null), { items: [], continuation: '' })
const pages = await Promise.all(['BiliArtistView.vue', 'YouTubeArtistView.vue'].map(name =>
  readFile(new URL(`../src/views/${name}`, import.meta.url), 'utf8')))
const keys = new Set(pages.flatMap(page => [...page.matchAll(/['"]((?:common|library|player)\.[a-z_]+)['"]/g)].map(match => match[1])))
for (const key of ['player.artist_videos', 'player.artist_collections', 'player.artist_series']) keys.add(key)
for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) {
  const messages = JSON.parse(await readFile(new URL(`../src/i18n/${locale}.json`, import.meta.url), 'utf8'))
  for (const key of keys) {
    const value = key.split('.').reduce((node, part) => node?.[part], messages)
    assert.equal(typeof value, 'string', `${locale} missing ${key}`)
    assert.ok(value.trim(), `${locale} empty ${key}`)
  }
}
console.log('YouTube artist headers, sections, route kinds, pagination and four-language page labels passed')
