import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function loadModule(path, dependencies = {}) {
  let source = await readFile(new URL(path, import.meta.url), 'utf8')
  for (const [name, url] of Object.entries(dependencies)) {
    source = source.replace(`from '${name}'`, `from ${JSON.stringify(url)}`)
  }
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const url = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
  return { module: await import(url), url }
}

const layoutPath = '../src/modules/youtube/youtubeHomeLayout.ts'
assert.ok(existsSync(new URL(layoutPath, import.meta.url)), 'YouTube 首页分类尚未实现')
const { module: { buildYoutubeHomeSections } } = await loadModule(layoutPath)
const { module: { resolvePlaybackQueueStartIndex } } = await loadModule('../src/modules/playback/playbackQueue.ts')
const { url: coverUrl } = await loadModule('../src/utils/trackCover.ts')
const { module: { parseYouTubeHomeFeed } } = await loadModule('../src/modules/youtube/youtubePlaylistParse.ts', {
  '@/utils/trackCover': coverUrl,
})

const song = (videoId, overrides = {}) => ({ title: `歌曲 ${videoId}`, subtitle: '歌曲 • 歌手 • 专辑 • 3:15 • 42 views', coverUrl: 'https://cover.test/song.jpg', videoId, durationMs: 195000, ...overrides })
const playlist = (browseId, overrides = {}) => ({ title: `歌单 ${browseId}`, subtitle: '12 songs', coverUrl: 'https://cover.test/playlist.jpg', browseId, ...overrides })
const shelf = (title, items) => ({ title, items })

const feed = [
  shelf('猜你喜欢但不是歌曲', [playlist('VL-first')]),
  shelf('每日 / 发现', [song('daily')]),
  shelf('Recommended_for-you', [song('guess')]),
  shelf('Oldies', [song('old'), playlist('VL-mixed')]),
  shelf('API 原序歌单', [playlist('VL-next')]),
  shelf('Guess you like later', [song('later')]),
  shelf('普通全歌曲', [song('plain')]),
  shelf('专辑', [playlist('VL-album', { pageType: 'MUSIC_PAGE_TYPE_ALBUM' })]),
]
const original = structuredClone(feed)
const sections = buildYoutubeHomeSections(feed)
assert.deepEqual(sections.map(section => section.titleKey || section.title), [
  'home.youtube_guess', 'home.youtube_daily', 'home.youtube_more',
  '猜你喜欢但不是歌曲', 'Oldies', 'API 原序歌单', 'Guess you like later', '普通全歌曲',
])
assert.deepEqual(sections.slice(0, 3).map(section => section.icon), ['radar', 'explore', 'star'])
assert.equal(new Set(sections.map(section => section.key)).size, sections.length)
assert.deepEqual(sections[0].songs.map(track => track.id), ['youtube:guess'])
assert.deepEqual(sections[1].songs.map(track => track.id), ['youtube:daily'])
assert.deepEqual(sections[2].playlists.map(item => item.id), ['VL-first', 'VL-mixed', 'VL-next'])
assert.equal(sections.find(section => section.title === 'Oldies').kind, 'songs')
assert.deepEqual(sections.find(section => section.title === 'Oldies').songs.map(track => track.id), ['youtube:old'])
assert.deepEqual(sections[0].songs[0], {
  id: 'youtube:guess', title: '歌曲 guess', artist: '歌手', album: '专辑', durationMs: 195000,
  coverUrl: 'https://cover.test/song.jpg', audioUrl: '', source: 'youtube',
  playlistKey: 'youtube-home:youtube_guess:0:youtube:guess',
  syncPayload: {
    id: 'guess', name: '歌曲 guess', artist: '歌手', album: '专辑', durationMs: 195000,
    coverUrl: 'https://cover.test/song.jpg', mediaUri: 'ytmusic://video/guess',
    channelId: 'youtube_music', audioId: 'guess',
  },
})
assert.deepEqual(feed, original, '转换不能修改推荐缓存')

for (const title of ['再听一遍', '老歌重温', '翻唱与混音', '每日发现', '猜你喜欢', 'listen-again', 'oldies', 'Covers / and Remixes', 'daily_discover']) {
  const result = buildYoutubeHomeSections([shelf(title, [song('one'), playlist('VL-two')])])
  assert.equal(result.find(section => section.songs.length)?.songs[0].id, 'youtube:one', title)
}
for (const title of ['guess.you.like', 'discover.daily']) {
  const result = buildYoutubeHomeSections([shelf(title, [song('one')])])
  assert.ok(result.every(section => !['home.youtube_guess', 'home.youtube_daily'].includes(section.titleKey)), '只规范化 Android 指定的分隔符')
}
const ordinaryMixed = buildYoutubeHomeSections([shelf('普通混合', [song('hidden'), playlist('VL-visible')])])
assert.ok(ordinaryMixed.every(section => section.songs.length === 0))
assert.deepEqual(ordinaryMixed.at(-1).playlists.map(item => item.id), ['VL-visible'])

const many = Array.from({ length: 30 }, (_, index) => playlist(`VL-${index}`))
const limited = buildYoutubeHomeSections([
  shelf('第一页', [playlist('VL-0'), playlist('VL-0'), playlist('VL-explicit-album', { pageType: 'music_page_type_album' }), playlist('PL-no-type')]),
  shelf('第二页', many),
])
assert.deepEqual(limited.find(section => section.titleKey === 'home.youtube_more').playlists.map(item => item.id), many.slice(0, 24).map(item => item.browseId))
assert.deepEqual(limited.find(section => section.titleKey === 'home.youtube_more').playlists[0], {
  id: 'VL-0', name: '歌单 VL-0', coverUrl: 'https://cover.test/playlist.jpg', trackCount: 0, playCount: 0,
})
assert.equal(limited.find(section => section.title === '第二页').playlists.length, 30, '24 只限制全局推荐歌单区')
const typed = buildYoutubeHomeSections([shelf('类型', [
  playlist('non-VL', { pageType: 'MUSIC_PAGE_TYPE_PLAYLIST' }),
  playlist('VL-artist', { pageType: 'MUSIC_PAGE_TYPE_ARTIST' }),
  playlist('VL-audio', { pageType: 'MUSIC_PAGE_TYPE_AUDIOBOOK' }),
  playlist('VL-fallback'),
  playlist('   ', { pageType: 'MUSIC_PAGE_TYPE_PLAYLIST' }),
])])
assert.deepEqual(typed[0].playlists.map(item => item.id), ['non-VL', 'VL-fallback'])
assert.deepEqual(buildYoutubeHomeSections([]), [])
assert.deepEqual(buildYoutubeHomeSections([shelf('空', [])]), [])

const allSongs = buildYoutubeHomeSections([shelf('完整列表', Array.from({ length: 35 }, (_, index) => song(String(index))))])
assert.equal(allSongs[0].songs.length, 35, 'Android YouTube 歌曲区映射整列表')
const duplicateFeed = [shelf('猜你喜欢', [song('repeated'), song('between'), song('repeated')])]
const occurrences = buildYoutubeHomeSections(duplicateFeed)[0].songs
assert.equal(new Set(occurrences.map(track => track.playlistKey)).size, 3, '同一 video 的各出现位置必须不同')
assert.deepEqual(occurrences.map(track => track.playlistKey), buildYoutubeHomeSections(duplicateFeed)[0].songs.map(track => track.playlistKey), '相同 feed 的位置标识应稳定')
assert.equal(resolvePlaybackQueueStartIndex(occurrences, occurrences[2].id, occurrences[2].playlistKey), 2, '选择第二次出现必须从该位置开始播放')
const fallback = buildYoutubeHomeSections([shelf('回退专辑', [song('fallback', { subtitle: '视频 | 3:01 | 订阅者', durationMs: undefined })])])[0].songs[0]
assert.equal(fallback.artist, 'YouTube Music')
assert.equal(fallback.album, '回退专辑')
assert.equal(fallback.durationMs, 0)

const browseEndpoint = (browseId, pageType) => ({ browseId, browseEndpointContextSupportedConfigs: { browseEndpointContextMusicConfig: { pageType } } })
const parsed = parseYouTubeHomeFeed({ contents: { sectionListRenderer: { contents: [{ musicCarouselShelfRenderer: {
  header: { musicCarouselShelfBasicHeaderRenderer: { title: { simpleText: '解析字段' } } },
  contents: [
    { musicTwoRowItemRenderer: {
      title: { simpleText: '专辑' }, navigationEndpoint: { browseEndpoint: browseEndpoint('VL-not-playlist', 'MUSIC_PAGE_TYPE_ALBUM') },
    } },
    { musicTwoRowItemRenderer: {
      title: { simpleText: '双行歌曲' }, navigationEndpoint: { watchEndpoint: { videoId: 'two-row' } },
      subtitle: { runs: [{ text: 'Artist' }, { text: ' • ' }, { text: 'Album' }, { text: ' • ' }, { text: '4:05' }] },
    } },
    { musicResponsiveListItemRenderer: {
      playlistItemData: { videoId: 'responsive' },
      navigationEndpoint: { browseEndpoint: browseEndpoint('VL-responsive', 'MUSIC_PAGE_TYPE_PLAYLIST') },
      flexColumns: [{ musicResponsiveListItemFlexColumnRenderer: { text: { simpleText: '列表歌曲' } } }],
      fixedColumns: [{ musicResponsiveListItemFixedColumnRenderer: { text: { simpleText: '2:30' } } }],
    } },
  ],
} }] } } })
assert.equal(parsed[0].items[0].pageType, 'MUSIC_PAGE_TYPE_ALBUM')
assert.equal(parsed[0].items[1].durationMs, 245000)
assert.equal(parsed[0].items[2].durationMs, 150000)
assert.equal(parsed[0].items[2].pageType, 'MUSIC_PAGE_TYPE_PLAYLIST')
assert.ok(buildYoutubeHomeSections(parsed).every(section => section.playlists.every(item => item.id !== 'VL-not-playlist')))

console.log('youtube home layout tests passed')
