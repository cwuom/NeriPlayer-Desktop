import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'
import * as vue from 'vue'
import * as pinia from 'pinia'
import { loadModule, subtitle, format, deferred, flush } from './bili-subtitle-test-loader.mjs'

const plain = text => [{ startMs: 1000, durationMs: 2000, text, words: [] }]
const cc = (language = 'zh-CN') => ({ source: 'bilibili_cc', lines: [{ ...plain(language)[0], translation: `${language} translation` }], track: { language, label: language } })
const bili = id => ({ id: `bilibili:${id}:22`, title: 'Video', artist: 'Uploader', album: 'Bilibili|22|BV1xx411c7mD', durationMs: 60000, audioUrl: '', source: 'bilibili' })
const playerSource = ts.createSourceFile('player.ts', readFileSync(new URL('../src/stores/player.ts', import.meta.url), 'utf8'), ts.ScriptTarget.ES2022, true)
let downloadResolver
function findDownloadResolver(node) {
  if (ts.isFunctionDeclaration(node) && node.name?.text === 'downloadedTrackForPlayback') downloadResolver = node.getText(playerSource)
  ts.forEachChild(node, findDownloadResolver)
}
findDownloadResolver(playerSource)
assert.ok(downloadResolver)
const compiledDownloadResolver = ts.transpileModule(downloadResolver, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const downloadPolicy = loadModule('../src/modules/playback/biliVideoSkip.ts')

async function runtime() {
  const instance = pinia.createPinia()
  pinia.setActivePinia(instance)
  const settings = vue.reactive({ defaultLyricSource: 'automatic', preferWordTimedLyrics: true, biliCcSubtitleEnabled: true, biliCcSubtitleLanguage: '', biliCcSubtitleAiFallback: true })
  const player = vue.reactive({ hasPlaybackSession: true, currentTrack: bili(1), durationMs: 60000, isLoadingAudio: false, loadedPlaybackTrackKey: null })
  const downloads = vue.reactive(new Map())
  const resolverDependencies = { ...downloadPolicy, useDownloadStore: () => ({ getDownloadedTrack: id => downloads.get(id) }) }
  player.downloadedTrackForPlayback = new Function(...Object.keys(resolverDependencies), `${compiledDownloadResolver}\nreturn downloadedTrackForPlayback`)(...Object.values(resolverDependencies))
  const revision = vue.ref(0)
  const sources = loadModule('../src/modules/lyrics/lyricSource.ts', { vue })
  const offset = loadModule('../src/modules/lyrics/lyricOffset.ts')
  const entries = new Map()
  const calls = { cc: [], automatic: [], preferred: [], word: [], roman: [], local: [] }
  const handlers = {
    cc: async () => cc(), automatic: async () => ({ source: 'netease', lines: plain('music') }),
    preferred: async () => null, local: async () => [], cacheRead: async key => entries.get(key) ?? null,
  }
  const cache = loadModule('../src/modules/lyrics/lyricsCache.ts', {
    '@/utils/persistentCache': {
      getCachedValue: async (_, key) => handlers.cacheRead(key),
      setCachedValue: async (_, key, value) => entries.set(key, value),
      removeCachedValue: async (_, key) => entries.delete(key),
    },
    './lyricsRequest': loadModule('../src/modules/lyrics/lyricsRequest.ts'), './lyricSource': sources,
  })
  const { useCurrentLyricsStore } = loadModule('../src/stores/currentLyrics.ts', {
    vue, pinia, '@/stores/player': { usePlayerStore: () => player }, '@/stores/settings': { useSettingsStore: () => settings },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      if (command === 'load_local_lyrics') { calls.local.push(args); return handlers.local(args) }
      if (command === 'parse_lrc_content') return plain(args.content)
      throw new Error(command)
    } },
    '@/modules/lyrics/biliSubtitle': subtitle,
    '@/modules/lyrics/biliSubtitleLoader': { biliSubtitleRevision: revision, loadBiliSubtitleLyrics: async (track, preferences) => {
      if (!preferences.enabled || !subtitle.biliSubtitleTarget(track)) return { source: 'bilibili_cc', lines: [] }
      calls.cc.push([track, preferences]); return handlers.cc(track, preferences)
    } },
    '@/modules/lyrics/lyricsCache': cache,
    '@/modules/lyrics/lyricsRequest': loadModule('../src/modules/lyrics/lyricsRequest.ts'),
    '@/modules/lyrics/lyricsFormat': { ...format, shouldBackfillNeteaseRomanization: () => false },
    '@/modules/lyrics/lyricOffset': offset,
    '@/modules/lyrics/lyricSource': sources,
    '@/modules/lyrics/lyricsFetch': {
      preferredLyricMatchSource: (_, preference) => preference === 'automatic' ? null : preference,
      fetchPreferredSourceLyrics: async (...args) => { calls.preferred.push(args); return handlers.preferred(...args) },
      fetchAutomaticLyrics: async (...args) => { calls.automatic.push(args); return handlers.automatic(...args) },
      fetchWordTimedLyrics: async (...args) => { calls.word.push(args); return { source: null, lines: [] } },
      fetchNeteaseRomanization: async (...args) => { calls.roman.push(args); return null },
    },
    '@/modules/lyrics/lyricsSupplement': { kugouSecondaryLyricNeeds: () => ({ translation: false, romanization: false }) },
    '@/modules/playback/playbackSource': { getPlaybackSourceKind: track => track.source ?? track.id.split(':')[0] },
    '@/modules/playback/playbackRequest': { playbackSessionTrackKey: (has, key, id) => has ? key || id : 'empty' },
    '@/utils/logger': { createLogger: () => ({ warn() {}, error() {} }) }, '@/utils/logSanitizer': { summarizeLogError: String },
  })
  const store = useCurrentLyricsStore()
  return { settings, player, downloads, revision, sources, cache, entries, calls, handlers, store, stop: () => pinia.disposePinia(instance) }
}

let cases = 0
async function test(name, run) {
  const r = await runtime()
  try { await run(r); cases++; console.log(`PASS ${name}`) } finally { r.stop() }
}

await test('saved edited lyrics take priority and intentional clearing remains effective', async r => {
  r.player.currentTrack.syncPayload = { matchedLyric: '[00:01]saved', matchedLyricSource: 'LOCAL_EDIT', lyricSyncEdited: true }
  r.store.acquire(); await flush()
  assert.equal(r.store.lines[0].text, '[00:01]saved')
  assert.equal(r.calls.cc.length, 0)
  r.player.currentTrack = { ...bili(2), syncPayload: { matchedLyric: '', lyricSyncEdited: true } }
  await flush()
  assert.deepEqual(r.store.lines, [])
  assert.equal(r.calls.cc.length, 0)
})
await test('downloaded sidecars and explicitly preferred sources win over captions', async r => {
  r.player.currentTrack.audioUrl = 'C:\\Music\\video.m4a'
  r.handlers.local = async () => plain('sidecar')
  r.store.acquire(); await flush()
  assert.equal(r.store.lines[0].text, 'sidecar')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'local')
  assert.equal(r.calls.cc.length, 0)
  r.player.currentTrack = bili(2)
  r.settings.defaultLyricSource = 'qq'
  r.handlers.preferred = async () => ({ source: 'qq', lines: plain('preferred') })
  await flush()
  assert.equal(r.store.lines[0].text, 'preferred')
  assert.equal(r.calls.cc.length, 0)
})
for (const audioUrl of ['', 'https://example.test/video.m4a']) {
  await test(`online queue entries with ${audioUrl ? 'HTTP' : 'empty'} audio URLs prefer downloaded lyrics`, async r => {
    r.player.currentTrack.audioUrl = audioUrl
    r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/download.m4a' })
    r.handlers.local = async () => plain('edited sidecar')
    r.settings.defaultLyricSource = 'qq'
    await r.cache.saveCachedLyrics(r.player.currentTrack, plain('old music'), 'netease')
    r.store.acquire(); await flush()
    assert.deepEqual(r.calls.local, [{ audioPath: 'C:/Music/download.m4a' }])
    assert.equal(r.store.lines[0].text, 'edited sidecar')
    assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'local')
    assert.equal(r.store.subtitle, null)
    assert.equal(r.calls.cc.length + r.calls.preferred.length + r.calls.word.length, 0)
    assert.equal(r.player.currentTrack.audioUrl, audioUrl)
  })
}
for (const legacyId of ['bilibili:1', 'bilibili:BV1xx411c7mD']) {
  await test(`legacy download ${legacyId} supplies sidecars for the matching part`, async r => {
    r.downloads.set(legacyId, { ...bili(1), id: legacyId, filePath: 'C:/Music/legacy.m4a' })
    r.handlers.local = async () => plain('legacy sidecar')
    r.store.acquire(); await flush()
    assert.deepEqual(r.calls.local, [{ audioPath: 'C:/Music/legacy.m4a' }])
    assert.equal(r.store.lines[0].text, 'legacy sidecar')
    assert.equal(r.calls.cc.length, 0)
  })
}
await test('a legacy download from another part cannot supply local lyrics', async r => {
  r.downloads.set('bilibili:1', { ...bili(1), id: 'bilibili:1', album: 'Bilibili|33|BV1xx411c7mD', filePath: 'C:/Music/other-part.m4a' })
  r.handlers.local = async () => plain('wrong part')
  r.store.acquire(); await flush()
  assert.deepEqual(r.calls.local, [])
  assert.equal(r.store.lines[0].text, 'zh-CN')
})
await test('a download discovered during CC loading replaces the pending subtitle request', async r => {
  const late = deferred()
  r.handlers.cc = () => late.promise
  r.handlers.local = async () => plain('downloaded')
  r.store.acquire(); await flush()
  r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/discovered.m4a' }); await flush()
  assert.equal(r.store.lines[0].text, 'downloaded')
  late.resolve(cc('obsolete')); await flush()
  assert.equal(r.store.lines[0].text, 'downloaded')
  assert.equal(r.store.subtitle, null)
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'local')
})
await test('changing the downloaded file for the same queue entry invalidates pending sidecars', async r => {
  const late = deferred()
  r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/old.m4a' })
  r.handlers.local = ({ audioPath }) => audioPath.endsWith('/old.m4a') ? late.promise : plain('new sidecar')
  r.store.acquire(); await flush()
  r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/new.m4a' }); await flush()
  late.resolve(plain('old sidecar')); await flush()
  assert.equal(r.store.lines[0].text, 'new sidecar')
  assert.equal(r.calls.cc.length, 0)
})
await test('missing or unreadable sidecars allow subtitle fallback', async r => {
  r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/empty.m4a' })
  r.store.acquire(); await flush()
  assert.equal(r.calls.local.length, 1)
  assert.equal(r.store.lines[0].text, 'zh-CN')
  r.handlers.local = async () => { throw new Error('unavailable file') }
  r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/missing.m4a' }); await flush()
  assert.equal(r.calls.local.length, 2)
  assert.equal(r.store.lines[0].text, 'zh-CN')
  assert.equal(r.store.loading, false)
})
await test('a previous song sidecar cannot overwrite the new song after switching', async r => {
  const late = deferred()
  r.downloads.set(bili(1).id, { ...bili(1), filePath: 'C:/Music/previous.m4a' })
  r.handlers.local = () => late.promise
  r.store.acquire(); await flush()
  r.player.currentTrack = bili(2); await flush()
  late.resolve(plain('previous song')); await flush()
  assert.equal(r.store.lines[0].text, 'zh-CN')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'bilibili_cc')
})
await test('CC replaces old automatic music caches and retains its own translation and source', async r => {
  await r.cache.saveCachedLyrics(r.player.currentTrack, plain('old matched song'), 'netease')
  r.store.acquire(); await flush()
  assert.equal(r.store.lines[0].text, 'zh-CN')
  assert.equal(r.store.lines[0].translation, 'zh-CN translation')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'bilibili_cc')
  assert.equal(r.calls.word.length, 0)
  assert.equal(r.calls.roman.length, 0)
  assert.equal(r.calls.automatic.length, 0)
  assert.equal([...r.entries.values()][0].source, 'netease', 'captions never pollute the persistent music cache')
  const release = r.store.acquire(); await flush(); release()
  assert.equal(r.calls.cc.length, 1, 'desktop and player consumers share the same original and translation')
})
await test('language changes refresh both tracks and reject earlier responses', async r => {
  const first = deferred(), second = deferred()
  r.handlers.cc = (_, preferences) => preferences.language === 'en' ? first.promise : preferences.language === 'ja' ? second.promise : cc()
  r.store.acquire(); await flush()
  r.settings.biliCcSubtitleLanguage = 'en'; await flush()
  assert.deepEqual(r.store.lines, [])
  r.settings.biliCcSubtitleLanguage = 'ja'; await flush()
  second.resolve(cc('ja')); await flush()
  first.resolve(cc('en')); await flush()
  assert.equal(r.store.lines[0].text, 'ja')
  assert.equal(r.store.lines[0].translation, 'ja translation')
  assert.equal(r.store.subtitle.track.language, 'ja')
})
await test('disabling CC clears original, translation and source before using music fallback', async r => {
  const later = deferred()
  r.handlers.automatic = () => later.promise
  r.store.acquire(); await flush()
  r.settings.biliCcSubtitleEnabled = false; await flush()
  assert.deepEqual(r.store.lines, [])
  assert.equal(r.store.subtitle, null)
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), null)
  later.resolve({ source: 'qq', lines: plain('fallback') }); await flush()
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'qq')
  assert.equal(r.store.lines[0].translation, undefined)
})
await test('a late caption request cannot replace an edit or republish its source', async r => {
  const late = deferred()
  r.handlers.cc = () => late.promise
  r.store.acquire(); await flush()
  r.sources.rememberLyricSource(r.player.currentTrack, 'LOCAL_EDIT')
  r.store.replace(plain('edited'))
  late.resolve(cc()); await flush()
  assert.equal(r.store.lines[0].text, 'edited')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'LOCAL_EDIT')
})
await test('cache clearing and song switches reject late subtitle results', async r => {
  const late = deferred()
  r.handlers.cc = () => late.promise
  r.store.acquire(); await flush()
  const old = r.player.currentTrack
  r.handlers.cc = async () => cc('fresh')
  r.revision.value++; await flush()
  r.player.currentTrack = bili(2); await flush()
  late.resolve(cc('obsolete')); await flush()
  assert.equal(r.store.lines[0].text, 'fresh')
  assert.equal(r.sources.lyricSourceOf(old), 'bilibili_cc')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'bilibili_cc')
})
await test('switching from captions to a video without captions clears both tracks and falls back automatically', async r => {
  const empty = deferred(), fallback = deferred()
  r.settings.biliCcSubtitleLanguage = 'zh'
  r.handlers.cc = track => track.id === bili(1).id ? cc('zh') : empty.promise
  r.handlers.automatic = () => fallback.promise
  r.store.acquire(); await flush()
  assert.equal(r.store.subtitle.track.language, 'zh')
  r.player.currentTrack = { ...bili(2), album: 'Bilibili|33|BV1xx411c7mD' }; await flush()
  assert.deepEqual(r.store.lines, [])
  assert.equal(r.store.subtitle, null)
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), null)
  empty.resolve({ source: 'bilibili_cc', lines: [] }); await flush()
  assert.equal(r.calls.automatic.length, 1)
  assert.equal(r.calls.automatic[0][0].id, bili(2).id)
  fallback.resolve({ source: null, lines: [] }); await flush()
  assert.deepEqual(r.store.lines, [])
  assert.equal(r.store.subtitle, null)
  assert.equal(r.store.loading, false)
  assert.equal(r.settings.biliCcSubtitleLanguage, 'zh')
  r.player.currentTrack = bili(1); await flush()
  assert.equal(r.store.subtitle.track.language, 'zh')
})
await test('old caption results cannot restore subtitles after switching to a video without captions', async r => {
  const old = deferred()
  r.handlers.cc = track => track.id === bili(1).id ? old.promise : { source: 'bilibili_cc', lines: [] }
  r.store.acquire(); await flush()
  r.player.currentTrack = bili(2); await flush()
  assert.equal(r.store.lines[0].text, 'music')
  old.resolve(cc('zh')); await flush()
  assert.equal(r.store.subtitle, null)
  assert.equal(r.store.lines[0].text, 'music')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'netease')
})
await test('a stale persistent cache read cannot overwrite a newly published subtitle source', async r => {
  const late = deferred()
  r.handlers.cacheRead = () => late.promise
  r.store.acquire(); await flush()
  r.handlers.cacheRead = async () => null
  r.settings.biliCcSubtitleLanguage = 'en'; await flush()
  late.resolve({ source: 'netease', lines: plain('old cache') }); await flush()
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'bilibili_cc')
  assert.equal(r.store.lines[0].text, 'zh-CN')
})
await test('subtitle errors retain the existing automatic lyric fallback', async r => {
  r.handlers.cc = async () => { throw new Error('offline') }
  r.store.acquire(); await flush()
  assert.equal(r.store.lines[0].text, 'music')
  assert.equal(r.sources.lyricSourceOf(r.player.currentTrack), 'netease')
})
await test('preference changes while all lyric views are closed take effect on re-entry', async r => {
  const release = r.store.acquire(); await flush(); release()
  r.settings.biliCcSubtitleAiFallback = false; await flush()
  assert.equal(r.calls.cc.length, 1)
  assert.deepEqual(r.store.lines, [])
  r.store.acquire(); await flush()
  assert.equal(r.calls.cc.at(-1)[1].aiFallback, false)
  assert.equal(r.store.lines[0].text, 'zh-CN')
})
console.log(`Bilibili current lyric regressions: ${cases} passed`)
