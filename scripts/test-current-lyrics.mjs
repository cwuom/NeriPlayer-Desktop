import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import { subtitle } from './bili-subtitle-test-loader.mjs'

async function loadModule(path, dependencies) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const key = `current-lyrics-test-${Math.random()}`
  globalThis[key] = dependencies
  compiled = compiled.replace(/import\s*\{([\s\S]*?)\}\s*from\s*['"]([^'"]+)['"];?/g, (_, bindings, specifier) => {
    assert.ok(specifier in dependencies, `unexpected dependency ${specifier}`)
    return `const { ${bindings.replace(/\bas\b/g, ':')} } = globalThis[${JSON.stringify(key)}][${JSON.stringify(specifier)}];`
  })
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
  } finally {
    delete globalThis[key]
  }
}

async function flush() { for (let i = 0; i < 12; i++) await Promise.resolve() }

function deferred() {
  let resolve, reject
  const promise = new Promise((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

const watchers = []
function trigger() {
  for (const watcher of watchers) {
    const next = watcher.source()
    if (next === watcher.last) continue
    watcher.last = next
    watcher.callback(next)
  }
}
const vue = {
  ref: value => ({ value }),
  shallowRef: value => ({ value }),
  computed: getter => ({ get value() { return getter() } }),
  watch: (source, callback) => {
    watchers.push({ source, callback, last: source() })
    return () => {}
  },
}
const pinia = {
  defineStore: (_, setup) => {
    let store
    return () => {
      if (store) return store
      const raw = setup()
      store = {}
      for (const [name, value] of Object.entries(raw)) {
        if (value && typeof value === 'object' && 'value' in value) {
          Object.defineProperty(store, name, { get: () => value.value, set: next => { value.value = next } })
        } else {
          store[name] = value
        }
      }
      return store
    }
  },
}

const line = (startMs, text, words = []) => ({ startMs, durationMs: 1000, text, words })
const plain = [line(1000, '第一行'), line(2000, '第二行')]
const wordTimed = [
  line(1000, '第一行', [{ startMs: 1000, durationMs: 500, text: '第一' }, { startMs: 1500, durationMs: 500, text: '行' }]),
  line(2000, '第二行', [{ startMs: 2000, durationMs: 1000, text: '第二行' }]),
]

const player = { hasPlaybackSession: true, isLoadingAudio: false, durationMs: 0, loadedPlaybackTrackKey: null,
  currentTrack: { id: 'netease:1', title: '歌', artist: '人', durationMs: 0 },
}
const settings = { defaultLyricSource: 'automatic', preferWordTimedLyrics: true }
const sources = new Map()
let payloadState = { kind: 'absent' }
let synced = null
let cached = null
const cacheWrites = []
let automatic = []
let wordTimedRequests = []
const supplementRequests = []
let syncedSource = 'CLOUD_MUSIC'
const format = await loadModule('../src/modules/lyrics/lyricsFormat.ts', {})
const offset = await loadModule('../src/modules/lyrics/lyricOffset.ts', {})
const supplement = await loadModule('../src/modules/lyrics/lyricsSupplement.ts', {
  './lyricsFormat': format,
  './lyricOffset': offset,
  './lyricMatch': { defaultLyricMatchKeyword: () => '', matchLyrics: async () => [] },
})

const { useCurrentLyricsStore } = await loadModule('../src/stores/currentLyrics.ts', {
  pinia,
  vue,
  '@tauri-apps/api/core': { invoke: async () => [] },
  '@/stores/player': { usePlayerStore: () => player },
  '@/stores/settings': { useSettingsStore: () => settings },
  '@/modules/lyrics/biliSubtitle': subtitle,
  '@/modules/lyrics/biliSubtitleLoader': {
    biliSubtitleRevision: { value: 0 },
    loadBiliSubtitleLyrics: async () => ({ source: 'bilibili_cc', lines: [] }),
  },
  '@/modules/lyrics/lyricsCache': {
    getCachedLyrics: async () => cached,
    saveCachedLyrics: async (track, lines) => { cacheWrites.push([track.id, lines]) },
  },
  '@/modules/lyrics/lyricsRequest': {
    hasWordTimedLyrics: lines => lines.some(entry => entry.words.some(word => word.durationMs > 0)),
    loadLyricsSingleFlight: (_, loader) => loader(),
  },
  '@/modules/lyrics/lyricsFormat': {
    ...await loadModule('../src/modules/lyrics/lyricsFormat.ts', {}),
    mapBackendLyrics: value => value,
    materializeStoredLyrics: async () => synced,
    mergeParsedLyricsWithRomanization: value => value,
    mergeWordTimedLyricsWithBaseline: (_, upgrade) => upgrade,
    resolveKnownNeteaseLyricSongId: () => null,
    resolveStoredLyricStateFromPayload: () => payloadState,
    shouldBackfillNeteaseRomanization: () => false,
  },
  '@/modules/lyrics/lyricOffset': { normalizeLyricSource: offset.normalizeLyricSource, readSyncedLyricSource: () => syncedSource },
  '@/modules/lyrics/lyricsFetch': {
    fetchAutomaticLyrics: async () => {
      const request = deferred()
      automatic.push(request)
      return request.promise
    },
    fetchNeteaseRomanization: async () => null,
    fetchPreferredSourceLyrics: async () => null,
    fetchWordTimedLyrics: async () => {
      const request = deferred()
      wordTimedRequests.push(request)
      return request.promise
    },
    preferredLyricMatchSource: () => null,
  },
  '@/modules/lyrics/lyricsSupplement': {
    ...supplement,
    supplementKugouLyrics: async (track, lines, needs) => {
      if (!(track.durationMs > 0) || !lines.length) return lines
      const request = deferred()
      supplementRequests.push({ ...request, track, lines, needs })
      return request.promise
    },
  },
  '@/modules/lyrics/lyricSource': {
    lyricSourceOf: track => sources.get(track.id) ?? null,
    rememberLyricSource: (track, source) => { if (source) sources.set(track.id, source); else sources.delete(track.id) },
  },
  '@/modules/playback/playbackSource': { getPlaybackSourceKind: track => track.id.split(':')[0] },
  '@/modules/playback/playbackRequest': {
    playbackSessionTrackKey: (has, playlistKey, id) => (has && id ? playlistKey || id : 'empty'),
  },
  '@/utils/logger': { createLogger: () => ({ warn() {}, error() {}, info() {} }) },
  '@/utils/logSanitizer': { summarizeLogError: String },
})

const store = useCurrentLyricsStore()
assert.equal(automatic.length, 0, '没有界面用到时不取词')

// 正在播放页打开：取到逐行歌词后后台换上逐字版本，来源记成 AMLL，偏移按它算
const releasePage = store.acquire()
await flush()
assert.equal(store.loading, true)
automatic[0].resolve({ source: 'netease', lines: plain })
await flush()
assert.deepEqual(store.lines, plain)
assert.equal(sources.get('netease:1'), 'netease')
assert.equal(wordTimedRequests.length, 1, '没有逐字时间轴时后台升级')
// 起播后才补上时长：仍是同一首歌，升级结果不能被丢掉（应用内看不到逐字效果的原因）
player.currentTrack = { ...player.currentTrack, durationMs: 215_000 }
trigger()
wordTimedRequests[0].resolve({ source: 'amll_ttml', lines: wordTimed })
await flush()
assert.deepEqual(store.lines, wordTimed, '逐字升级在时长更新后仍然生效')
assert.equal(sources.get('netease:1'), 'amll_ttml', '默认偏移跟着换成逐字歌词的来源')
assert.equal(store.loading, false)
assert.equal(automatic.length, 1, '同一首歌不重复取词')

// 桌面歌词再打开：直接拿同一份，不另取
const releaseDesktop = store.acquire()
await flush()
assert.equal(automatic.length, 1)
assert.deepEqual(store.lines, wordTimed)

// 用户编辑后，进行中的升级不能把编辑盖回去
player.currentTrack = { id: 'netease:2', title: '另一首', artist: '人', durationMs: 1000 }
trigger()
await flush()
assert.deepEqual(store.lines, [], '换歌立即撤下旧词')
automatic[1].resolve({ source: 'netease', lines: plain })
await flush()
const edited = [line(500, '我改的')]
store.replace(edited)
wordTimedRequests[1].resolve({ source: 'amll_ttml', lines: wordTimed })
await flush()
assert.deepEqual(store.lines, edited, '编辑过的歌词不被逐字升级覆盖')

// 切走后旧请求晚到：不能显示在新歌上
player.currentTrack = { id: 'qq:3', title: '第三首', artist: '人', durationMs: 1000 }
trigger()
await flush()
player.currentTrack = { id: 'qq:4', title: '第四首', artist: '人', durationMs: 1000 }
trigger()
await flush()
automatic[2].resolve({ source: 'qq', lines: [line(0, '旧歌')] })
await flush()
assert.deepEqual(store.lines, [])
automatic[3].resolve({ source: 'qq', lines: [line(0, '新歌')] })
await flush()
assert.equal(store.lines[0].text, '新歌')

// 同步载荷里有意清空：不在线回填
synced = []
player.currentTrack = { id: 'netease:5', title: '清空', artist: '人', durationMs: 1000 }
trigger()
await flush()
assert.equal(automatic.length, 4, '有意清空的歌词不联网')
assert.deepEqual(store.lines, [])
synced = null

// 缓存命中：先显示缓存，再升级
cached = plain
player.currentTrack = { id: 'netease:6', title: '缓存', artist: '人', durationMs: 1000 }
trigger()
await flush()
assert.deepEqual(store.lines, plain)
assert.equal(automatic.length, 4, '缓存命中不联网')
assert.equal(wordTimedRequests.length, 4, '缓存缺逐字时同样后台升级')
cached = null

// 两个界面都关了：换歌只清空，不联网；再打开时按当前曲目取
releasePage()
releaseDesktop()
releaseDesktop()
player.currentTrack = { id: 'netease:7', title: '后台', artist: '人', durationMs: 1000 }
trigger()
await flush()
assert.equal(automatic.length, 4, '没有界面时换歌不取词')
assert.deepEqual(store.lines, [])
const releaseAgain = store.acquire()
await flush()
assert.equal(automatic.length, 5, '重新打开时补取当前曲目')
releaseAgain()

// 旧酷狗缓存立即显示，后台补副轨，两个界面仍然共用酷狗逐字时间轴和来源
settings.preferWordTimedLyrics = false
cached = wordTimed
sources.set('bilibili:8', 'kugou')
player.currentTrack = { id: 'bilibili:8', title: '补全', artist: '人', durationMs: 60_000 }
trigger()
const releaseSupplement = store.acquire()
await flush()
assert.deepEqual(store.lines, wordTimed, '补全不阻塞主歌词显示')
assert.equal(store.loading, false)
assert.equal(supplementRequests.length, 1, '缓存里的酷狗歌词缺翻译和音译时也补全')
assert.deepEqual(supplementRequests[0].needs, { translation: true, romanization: true })
const enriched = wordTimed.map((entry, i) => ({ ...entry, translation: `翻译${i}`, roman: `roman${i}` }))
supplementRequests[0].resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, '翻译0')
assert.equal(store.lines[0].roman, 'roman0')
assert.deepEqual(store.lines[0].words, wordTimed[0].words)
assert.equal(sources.get('bilibili:8'), 'kugou', '补充副轨不改变主歌词来源和默认偏移')
assert.equal(cacheWrites.at(-1)[1][0].translation, '翻译0', '补全结果写回歌词缓存')

// 补全晚到时，换歌或用户编辑均不能被旧结果覆盖
player.currentTrack = { id: 'bilibili:9', title: '旧歌', artist: '人', durationMs: 60_000 }
sources.set('bilibili:9', 'kugou')
trigger()
await flush()
assert.equal(supplementRequests.length, 2)
player.currentTrack = { id: 'bilibili:10', title: '新歌', artist: '人', durationMs: 60_000 }
sources.set('bilibili:10', 'kugou')
trigger()
await flush()
supplementRequests[1].resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, undefined, '旧曲目补全不能写入新曲目')
store.replace(edited)
supplementRequests[2].resolve(enriched)
await flush()
assert.deepEqual(store.lines, edited, '补全不能覆盖用户编辑')

// 同步来的酷狗原词可补全，但用户清空的副轨必须保持为空
cached = null
synced = wordTimed
syncedSource = 'KUGOU'
player.currentTrack = { id: 'bilibili:11', title: '同步', artist: '人', durationMs: 60_000,
  syncPayload: { matchedLyric: '原词', lyricSyncEdited: true, matchedTranslatedLyric: '' },
}
trigger()
await flush()
assert.equal(supplementRequests.length, 4)
assert.deepEqual(supplementRequests[3].needs, { translation: false, romanization: true })
supplementRequests[3].resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, undefined, '有意清空的翻译不被恢复')
assert.equal(store.lines[0].roman, 'roman0')

// 初始来源没有副轨，逐字升级为酷狗后仍然触发补全
synced = null
settings.preferWordTimedLyrics = true
player.currentTrack = { id: 'bilibili:12', title: '逐字升级', artist: '人', durationMs: 60_000 }
trigger()
await flush()
automatic.at(-1).resolve({ source: 'netease', lines: plain })
await flush()
wordTimedRequests.at(-1).resolve({ source: 'kugou', lines: wordTimed })
await flush()
assert.equal(supplementRequests.length, 5, '升级成酷狗逐字歌词后也补齐副轨')
supplementRequests[4].resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, '翻译0')
assert.equal(sources.get('bilibili:12'), 'kugou')

// 缓存加载时还没有时长，起播后补齐时长才补全
settings.preferWordTimedLyrics = false
cached = wordTimed
sources.set('bilibili:13', 'kugou')
player.currentTrack = { id: 'bilibili:13', title: '未知时长', artist: '人', durationMs: 0 }
trigger()
await flush()
assert.equal(supplementRequests.length, 5)
player.currentTrack = { ...player.currentTrack, durationMs: 60_000 }
trigger()
await flush()
assert.equal(supplementRequests.length, 6, '时长补齐后重新尝试副轨补全')
// 请求途中收到用户清空音译的载荷，返回时重新检查每条副轨
player.currentTrack = { ...player.currentTrack,
  syncPayload: { lyricSyncEdited: true, matchedRomanizedLyric: '' },
}
supplementRequests[5].resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, '翻译0')
assert.equal(store.lines[0].roman, undefined, '补全过程中新确认清空的音译也受到保护')

sources.set('bilibili:14', 'kugou')
player.currentTrack = { id: 'bilibili:14', title: '补全失败', artist: '人', durationMs: 60_000 }
trigger()
await flush()
supplementRequests.at(-1).reject(new Error('offline'))
await flush()
assert.deepEqual(store.lines, wordTimed, '补全失败保留酷狗主歌词')

// 时长在主词加载期间补齐，完成加载后必须采用最新曲目信息
sources.set('bilibili:15', 'kugou')
player.currentTrack = { id: 'bilibili:15', title: '加载期间补齐时长', artist: '人', durationMs: 0 }
trigger()
await Promise.resolve()
assert.equal(store.loading, true)
player.currentTrack = { ...player.currentTrack, durationMs: 60_000 }
trigger()
await flush()
assert.equal(supplementRequests.length, 8, '加载期间补齐时长也能补全副轨')
assert.equal(supplementRequests.at(-1).track.durationMs, 60_000)
supplementRequests.at(-1).resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, '翻译0')

// 后端时长只更新播放器，音频加载完成之前不能借用上一首的残留时长
sources.set('bilibili:16', 'kugou')
player.isLoadingAudio = true
player.durationMs = 215_000
player.currentTrack = { id: 'bilibili:16', title: '后端时长', artist: '人', durationMs: 0 }
trigger()
await flush()
assert.equal(supplementRequests.length, 8, '音频仍在加载时不使用上首时长')
player.durationMs = 60_000
trigger()
await flush()
assert.equal(supplementRequests.length, 8, '后端时长提交但音频仍在加载时继续等待')
player.loadedPlaybackTrackKey = JSON.stringify([null, 'bilibili:16'])
player.isLoadingAudio = false
trigger()
await flush()
assert.equal(supplementRequests.length, 9, '音频加载完成后使用当前后端时长补全')
assert.equal(supplementRequests.at(-1).track.durationMs, 60_000)
supplementRequests.at(-1).resolve(enriched)
await flush()
assert.equal(store.lines[0].roman, 'roman0')
assert.equal(player.currentTrack.durationMs, 0, '补全不会改写曲目元数据')

// 删除当前队列项仅改变选中曲目时，旧音频时长不属于新曲目
sources.set('bilibili:17', 'kugou')
player.currentTrack = { id: 'bilibili:17', title: '队列中的下一首', artist: '人', durationMs: 0 }
trigger()
await flush()
assert.equal(supplementRequests.length, 9, '未经历音频加载的新曲目不能借用旧音频时长')

// 旧歌加载期间删除当前项，加载完成的时长仍然属于旧音频
sources.set('bilibili:18', 'kugou')
sources.set('bilibili:19', 'kugou')
player.isLoadingAudio = true
player.currentTrack = { id: 'bilibili:18', title: '正在加载的旧歌', artist: '人', durationMs: 0 }
trigger()
await flush()
player.currentTrack = { id: 'bilibili:19', title: '仅选中的下一首', artist: '人', durationMs: 0 }
trigger()
await flush()
player.loadedPlaybackTrackKey = JSON.stringify([null, 'bilibili:18'])
player.durationMs = 215_000
player.isLoadingAudio = false
trigger()
await flush()
assert.equal(supplementRequests.length, 9, '加载期间仅更改选中项也不能借用旧音频时长')

// 真实快速切歌的新音频完成后，仍然能够使用它的后端时长
sources.set('bilibili:20', 'kugou')
sources.set('bilibili:21', 'kugou')
player.isLoadingAudio = true
player.currentTrack = { id: 'bilibili:20', title: '快速切歌之前', artist: '人', durationMs: 0 }
trigger()
await flush()
player.currentTrack = { id: 'bilibili:21', title: '快速切歌之后', artist: '人', durationMs: 0 }
trigger()
await flush()
player.loadedPlaybackTrackKey = JSON.stringify([null, 'bilibili:21'])
player.durationMs = 60_000
player.isLoadingAudio = false
trigger()
await flush()
assert.equal(supplementRequests.length, 10, '真实快速切歌完成后仍补全当前曲目')
supplementRequests.at(-1).resolve(enriched)
await flush()
assert.equal(store.lines[0].translation, '翻译0')
releaseSupplement()

// 所有显示入口过滤同一份歌词，原始数组和真实歌词时间轴不被改写
const creditVerse = { ...wordTimed[0], translation: '真正的翻译', roman: 'roman' }
const dirtyLyrics = [line(0, '键盘：韦伟'), creditVerse, line(3000, 'Backing Vocals: Jane')]
store.replace(dirtyLyrics)
assert.deepEqual(store.lines.map(entry => entry.text), ['第一行'], '编辑替换也不能显示制作署名')
assert.deepEqual(dirtyLyrics.map(entry => entry.text), ['键盘：韦伟', '第一行', 'Backing Vocals: Jane'])
assert.deepEqual(store.lines[0], creditVerse, '正文、逐字时间轴、翻译与音译保留')

player.currentTrack = { id: 'local:credits.flac', source: 'local', title: '本地歌曲', artist: '人' }
trigger()
store.replace(dirtyLyrics)
assert.deepEqual(store.lines.map(entry => entry.text), ['第一行'], '本地歌词只在显示时过滤署名')

store.replace([line(0, 'MIDI程序：韦伟')])
assert.deepEqual(store.lines, [], '只有明确制作信息时不显示牛皮癣')

const dirtySecondary = line(1000, 'Hello')
dirtySecondary.translation = 'Mixing Engineer: John Smith'
dirtySecondary.roman = 'Guitar: John Smith'
store.replace([dirtySecondary])
assert.deepEqual(store.lines, [{ ...line(1000, 'Hello'), translation: undefined, roman: undefined }], '残留在副轨的署名不显示')
assert.equal(dirtySecondary.translation, 'Mixing Engineer: John Smith', '显示过滤不改原始副轨')

const attachedSecondary = [
  { ...line(1000, 'Hello'), translation: '编曲：张三', roman: 'Producer: John' },
  { ...line(2000, 'World'), translation: '词：徐若瑄', roman: 'Lyrics by Alice' },
  line(3000, 'Break'),
  { ...line(4000, 'After'), translation: '词：徐若瑄', roman: 'Lyrics by Alice' },
]
store.replace(attachedSecondary)
assert.deepEqual(store.lines.map(entry => [entry.translation, entry.roman]), [
  [undefined, undefined], [undefined, undefined], [undefined, undefined], ['词：徐若瑄', 'Lyrics by Alice'],
], '已有副轨也按连续制作信息块过滤，缺失副轨作为边界')
assert.equal(attachedSecondary[1].translation, '词：徐若瑄', '副轨上下文过滤保留原始数据')

const screenshotVerse = {
  ...line(14000, 'I miss the days when life was so simple', [{ startMs: 14000, durationMs: 1000, text: 'I miss the days when life was so simple' }]),
  translation: '我想念那段时光 生活简单无忧', roman: 'verse roman',
}
const screenshotLyrics = [line(0, 'Hayd - Head The Clouds'), line(1000, 'by: Robert Hubers、'), line(2000, 'by: Robert Hubers'), screenshotVerse]
const screenshotOriginal = structuredClone(screenshotLyrics)
synced = null
cached = screenshotLyrics
player.currentTrack = { id: 'netease:screenshot', title: 'Head In The Clouds', artist: 'Hayd', album: 'Changes - EP', durationMs: 0 }
trigger()
const releaseScreenshot = store.acquire()
await flush()
assert.deepEqual(store.lines, [screenshotVerse], '已有缓存也根据当前曲目清理漏出的标题和署名')
assert.deepEqual(cached, screenshotOriginal, '过滤旧缓存不改写缓存数据')
synced = screenshotLyrics
player.currentTrack = { ...player.currentTrack, id: 'netease:screenshot-synced' }
trigger()
await flush()
assert.deepEqual(store.lines, [screenshotVerse], '同步歌词也按相同规则显示')
assert.deepEqual(cacheWrites.at(-1)[1], screenshotOriginal, '显示过滤不改写 store 收到的解析结果')
player.currentTrack = { id: 'local:screenshot.flac', source: 'local', title: 'Head In The Clouds', artist: 'Hayd', album: 'Changes - EP' }
trigger()
await flush()
store.replace(screenshotLyrics)
assert.deepEqual(store.lines, [screenshotVerse], '本地及编辑替换的歌词也清理同一组杂项')
assert.deepEqual(screenshotLyrics, screenshotOriginal, '本地及编辑源数据保留')
releaseScreenshot()

console.log('current lyrics store tests passed')
