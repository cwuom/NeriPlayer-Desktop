import assert from 'node:assert/strict'
import * as vue from 'vue'
import * as pinia from 'pinia'
import { loadModule, subtitle, format, deferred, flush } from './bili-subtitle-test-loader.mjs'

const plain = text => [{ startMs: 1000, durationMs: 2000, text, words: [] }]
const cc = (language = 'zh-CN') => ({ source: 'bilibili_cc', lines: [{ ...plain(language)[0], translation: `${language} translation` }], track: { language, label: language } })
const bili = id => ({ id: `bilibili:${id}:22`, title: 'Video', artist: 'Uploader', album: 'Bilibili|22|BV1xx411c7mD', durationMs: 60000, audioUrl: '', source: 'bilibili' })

async function runtime() {
  const instance = pinia.createPinia()
  pinia.setActivePinia(instance)
  const settings = vue.reactive({ defaultLyricSource: 'automatic', preferWordTimedLyrics: true, biliCcSubtitleEnabled: true, biliCcSubtitleLanguage: '', biliCcSubtitleAiFallback: true })
  const player = vue.reactive({ hasPlaybackSession: true, currentTrack: bili(1), durationMs: 60000, isLoadingAudio: false, loadedPlaybackTrackKey: null })
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
      if (command === 'load_local_lyrics') { calls.local.push(args); return handlers.local() }
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
  return { settings, player, revision, sources, cache, entries, calls, handlers, store, stop: () => pinia.disposePinia(instance) }
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
