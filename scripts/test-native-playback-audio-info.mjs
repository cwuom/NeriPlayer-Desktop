import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

let sequence = 0
async function load(source, dependencies = {}) {
  const key = `__nativeAudioInfoTest${++sequence}`
  globalThis[key] = dependencies
  let compiled = ts.transpileModule(source.replaceAll('import.meta.env.DEV', 'false'), {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (ts.isExportDeclaration(node) && node.moduleSpecifier && ts.isNamedExports(node.exportClause)) {
      const name = node.moduleSpecifier.text
      assert.ok(name in dependencies, `missing dependency ${name}`)
      const names = node.exportClause.elements.map(item => item.name.text).join(', ')
      const replacement = `const { ${names} } = deps[${JSON.stringify(name)}]; export { ${names} };`
      compiled = compiled.slice(0, node.getStart(parsed)) + replacement + compiled.slice(node.end)
      continue
    }
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `missing dependency ${name}`)
    const clause = node.importClause
    const assignments = []
    if (clause?.name) assignments.push(`const ${clause.name.text} = deps[${JSON.stringify(name)}].default`)
    if (clause?.namedBindings && ts.isNamedImports(clause.namedBindings)) {
      const names = clause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text)
      assignments.push(`const { ${names.join(', ')} } = deps[${JSON.stringify(name)}]`)
    }
    compiled = compiled.slice(0, node.getStart(parsed)) + assignments.join(';\n') + compiled.slice(node.end)
  }
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`)
  } finally { delete globalThis[key] }
}

const root = new URL('../src/', import.meta.url)
const source = await readFile(new URL('stores/player.ts', root), 'utf8')
const sourceText = await readFile(new URL('modules/playback/playbackSource.ts', root), 'utf8')
const policy = await load(await readFile(new URL('modules/playback/playbackPolicy.ts', root), 'utf8'))
const request = await load(await readFile(new URL('modules/playback/playbackRequest.ts', root), 'utf8'))
const queue = await load(await readFile(new URL('modules/playback/playbackQueue.ts', root), 'utf8'))
const state = await load(await readFile(new URL('modules/playback/playerState.ts', root), 'utf8'))
const display = await load(await readFile(new URL('modules/playback/audioQualityDisplay.ts', root), 'utf8'))
const metadataText = await readFile(new URL('modules/playback/playbackAudioInfo.ts', root), 'utf8')
const localInfoText = await readFile(new URL('modules/playback/localAudioInfo.ts', root), 'utf8')
const failure = await load(await readFile(new URL('modules/playback/playbackFailure.ts', root), 'utf8'))
const longForm = await load(await readFile(new URL('modules/playback/longFormProgress.ts', root), 'utf8'))
const ltProtocol = await load(await readFile(new URL('stores/listenTogether/protocol.ts', root), 'utf8'))
const streamQuality = await load(await readFile(new URL('stores/listenTogether/streamQuality.ts', root), 'utf8'), { './protocol': ltProtocol })
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const flush = async () => { for (let i = 0; i < 20; i++) { await Promise.resolve(); await vue.nextTick() } }
const track = id => ({ id: `netease:${id}`, source: 'netease', title: id, artist: 'Artist', album: 'Album', durationMs: 180000, coverUrl: '', audioUrl: '' })
const properties = { sampleRateHz: 96000, channelCount: 2, bitDepth: 24, codec: 'FLAC', bitrate: 2964 }
const visibility = { showAudioBitrate: true, showAudioFormat: true, showAudioChannels: true, showAudioSampleRate: true, showAudioBitDepth: true }

globalThis.localStorage = { getItem: () => null, setItem: () => {} }
globalThis.window = { setTimeout: () => 0 }
globalThis.requestAnimationFrame = () => 0

async function runtime(options = {}) {
  const calls = [], events = new Map(), metadata = []
  const invoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_playback_audio_info') {
      metadata.push(args)
      return options.metadata ? options.metadata(args) : properties
    }
    if (command === 'get_local_audio_info') return options.localInfo ? options.localInfo() : { format: 'FLAC', codec: 'FLAC', sampleRateHz: 44100, bitDepth: 16, channelCount: 2 }
    if (command === 'get_netease_song_url') return options.resolve ? options.resolve(args) : {
      url: `https://audio.example/${args.songId}.flac`, bitrate: 2964000, format: 'flac', level: 'hires', duration_ms: 180000,
    }
    if (command === 'find_netease_local_sources') return [{ id: 'local:fallback', title: 'Fallback', artist: 'Artist', album: '', duration_ms: 180000, url: 'C:/Music/fallback.flac' }]
    if (command === 'play_cached_audio_candidates') return options.cached ? { durationMs: 180000, source: 'netease', qualityKey: 'hires' } : null
    if (command === 'play_url_streaming' && options.streamingError) throw new Error('streaming unavailable')
    if (['play_url_streaming', 'play_url_fast', 'crossfade_url_streaming', 'play_file', 'crossfade_file'].includes(command)) return options.durationMs ?? 180000
    if (command === 'release_audio_file') return true
    return undefined
  }
  const core = { invoke }
  const playback = await load(sourceText, {
    '@tauri-apps/api/core': core,
    '@/stores/listenTogether/mapper': { trustedInboundStreamUrls: () => [] },
    './playbackFailure': failure,
  })
  const localInfo = await load(localInfoText, { '@tauri-apps/api/core': core })
  const playbackInfo = await load(metadataText, { '@tauri-apps/api/core': core })
  const settings = vue.reactive({
    volume: 0.7, playbackSpeed: 1, loudnessGainMb: 0, equalizerEnabled: false, equalizerPresetId: 'flat', equalizerBands: [0, 0, 0, 0, 0],
    volumeBalance: 0, maxCacheSize: 1024, fadeIn: false, crossfade: !!options.crossfade, crossfadeNext: !!options.crossfade,
    crossfadeInDuration: 100, crossfadeOutDuration: 100, fadeInDuration: 100, fadeOutDuration: 100,
    neteaseQuality: 'hires', qqMusicQuality: 'high', biliQuality: 'high', youtubeQuality: 'high',
    neteaseAutoSourceSwitch: false, neteaseLocalSourceFallback: !!options.localFallback,
    rememberLongFormProgress: options.rememberLongForm !== false,
  })
  const loaded = await load(source, {
    vue, pinia, '@tauri-apps/api/core': core,
    '@tauri-apps/api/event': { listen: async (name, callback) => { events.set(name, callback); return () => events.delete(name) } },
    './history': { useHistoryStore: () => options.history ?? { record: () => {}, rememberedPosition: () => 0, updateResumePosition() {} } },
    './toast': { useToastStore: () => ({ error: message => options.toasts?.push(message) }) },
    './settings': { useSettingsStore: () => settings, MIN_MEDIA_CACHE_SIZE_MB: 128, MAX_MEDIA_CACHE_SIZE_MB: 16384 },
    './download': { useDownloadStore: () => ({ getDownloadedTrack: () => options.downloaded ? { filePath: 'C:/Music/download.flac', durationMs: 180000 } : null }) },
    './listenTogether': { useListenTogetherStore: () => options.listenTogether ?? { isConnected: false } },
    '@/i18n': { default: { global: { t: key => key } } },
    '@/modules/playback/playbackSource': playback,
    '@/modules/playback/playbackFailure': failure,
    '@/modules/playback/playedQualityMemory': { rememberPlayedQuality() {}, recallPlayedQuality: async () => null },
    '@/modules/playback/youtubeSeekRefreshPolicy': { shouldRefreshUrlBeforeSeek: () => false, shouldRefreshUrlBeforeResume: () => false },
    '@/modules/playback/playbackPrefetch': {
      playbackPrefetchManager: { replacePlaybackDemand() {}, take: () => null, prefetchWindow() {} },
      genericUrlPrefetchTtlMs: () => 90_000,
    },
    '@/modules/playback/playbackPolicy': { ...policy, PlaybackStartupWatchdog: class { cancel() {} schedule() {} } },
    '@/modules/playback/playbackQueue': queue,
    '@/modules/playback/longFormProgress': longForm,
    '@/stores/listenTogether/streamQuality': streamQuality,
    '@/modules/playback/playbackRequest': request,
    '@/modules/playback/playerState': state,
    '@/utils/logger': { createLogger: () => ({ info() {}, warn() {}, error() {} }) },
    '@/utils/timeFormat': { formatTimeMs: value => String(value) },
    '@/utils/trackCover': { getTrackCoverUrl: value => value.coverUrl || '' },
    '@/utils/logSanitizer': { summarizeLogError: String },
    '@/modules/playback/localAudioInfo': localInfo,
    '@/modules/playback/playbackAudioInfo': playbackInfo,
    '@/modules/library/albumDisplay': { displayAlbum: album => album },
    '@/modules/persistence/userData': {
      LEGACY_PLAYER_STATE_KEY: 'neri:player-state',
      preloadedUserData: () => null,
      persistUserData: async () => undefined,
      finishLegacyPlayerStateCleanup: () => {},
    },
  })
  pinia.setActivePinia(pinia.createPinia())
  return { store: loaded.usePlayerStore(), calls, metadata, events }
}

let failures = 0
async function run(name, test) {
  try { await test(); console.log(`ok - ${name}`) }
  catch (error) { failures++; console.error(`not ok - ${name}\n${error.stack.replace(/data:text\/javascript;base64,[A-Za-z0-9+/=]+/g, 'production-module')}`) }
}

await run('real online resolver without decoder parameters gains native IPC properties and display labels', async () => {
  const pending = deferred(), r = await runtime({ metadata: () => pending.promise })
  await r.store.play(track('101'))
  assert.equal(r.store.isPlaying, true, 'metadata must not block playback startup')
  assert.equal(r.store.audioInfo.sampleRateHz, undefined, 'the real URL resolver does not invent decoded parameters')
  assert.equal(r.metadata.length, 1, 'successful playback must query the active decoder')
  const generation = r.calls.find(call => call.command === 'play_url_streaming').args.requestGeneration
  assert.deepEqual(r.metadata[0], { requestGeneration: generation })
  pending.resolve(properties); await flush()
  assert.equal(r.store.audioInfo.qualityKey, 'hires')
  assert.equal(r.store.audioInfo.source, 'netease')
  assert.ok(r.store.audioInfo.qualityOptions.length > 0)
  assert.deepEqual(display.actualAudioParameterLabels(r.store.audioInfo, visibility), ['2964 kbps', 'FLAC', '2 ch', '96 kHz', '24 bit'])
})

for (const [name, options, item] of [
  ['playback cache', { cached: true }, track('201')],
  ['fast playback fallback', { streamingError: true }, track('202')],
  ['downloaded file', { downloaded: true }, track('203')],
  ['local file', {}, { ...track('204'), id: 'local:204', source: 'local', audioUrl: 'C:/Music/local.flac' }],
  ['online local-source fallback', { localFallback: true, resolve: () => ({ url: null, bitrate: 0, format: '', unavailable_reason: 'no_permission' }) }, track('205')],
]) {
  await run(`${name} also gains active decoder properties`, async () => {
    const r = await runtime(options); await r.store.play(item); await flush()
    assert.equal(r.store.playError, null)
    assert.equal(r.metadata.length, 1)
    assert.equal(r.store.audioInfo.sampleRateHz, 96000)
    assert.equal(r.store.audioInfo.bitDepth, 24)
    assert.equal(r.store.audioInfo.channelCount, 2)
    if (options.cached) assert.equal(r.store.audioInfo.qualityKey, 'hires')
  })
}

await run('crossfade uses the new decoded source metadata', async () => {
  let index = 0
  const r = await runtime({ crossfade: true, metadata: () => ({ ...properties, sampleRateHz: ++index === 1 ? 96000 : 48000 }) })
  await r.store.play(track('301')); await flush()
  await r.store.play(track('302')); await flush()
  assert.ok(r.calls.some(call => call.command === 'crossfade_url_streaming'))
  assert.equal(r.store.audioInfo.sampleRateHz, 48000)
})

for (const sameTrack of [false, true]) {
  await run(`late decoder metadata cannot overwrite ${sameTrack ? 'same-track replay' : 'a newer track'}`, async () => {
    const old = deferred(); let index = 0
    const r = await runtime({ metadata: () => ++index === 1 ? old.promise : { ...properties, sampleRateHz: 48000, bitDepth: 16 } })
    await r.store.play(track('401'))
    await r.store.play(track(sameTrack ? '401' : '402'), 'local', 0, true); await flush()
    old.resolve(properties); await flush()
    assert.equal(r.store.audioInfo.sampleRateHz, 48000)
    assert.equal(r.store.audioInfo.bitDepth, 16)
  })
}

await run('a listener without control is stopped before the player changes, but safety pauses still work', async () => {
  const toasts = []
  const listenTogether = { isConnected: true, roomId: null, isController: false, localControlRestriction: null }
  const r = await runtime({ toasts, listenTogether })
  await r.store.play(track('601')); await flush()
  assert.equal(r.store.isPlaying, true)
  listenTogether.roomId = 'ABC234'
  listenTogether.localControlRestriction = 'member_control_disabled'
  const commands = r.calls.length
  await r.store.pause()
  await r.store.next()
  await r.store.seekTo(30000)
  assert.equal(r.store.isPlaying, true, 'the blocked pause leaves the player in step with the room')
  assert.ok(!r.calls.slice(commands).some(call => ['pause', 'seek'].includes(call.command)))
  assert.deepEqual(toasts, Array(3).fill('listen_together.control_blocked_member_control'))
  await r.store.pause('local_safety')
  assert.equal(r.store.isPlaying, false, 'a sleep-timer pause is not a room control')
})

await run('local files cannot be started while in a Listen Together room', async () => {
  const toasts = []
  const listenTogether = { isConnected: true, roomId: 'ABC234', isController: true, localControlRestriction: null }
  const r = await runtime({ toasts, listenTogether })
  await r.store.play({ ...track('602'), id: 'local:602', source: 'local', audioUrl: 'C:/Music/local.flac' })
  assert.equal(r.store.currentTrack, null)
  assert.deepEqual(toasts, ['listen_together.local_playback_blocked'])
  await r.store.play(track('603'))
  assert.equal(r.store.currentTrack?.id, 'netease:603', 'online tracks are still allowed')
})

await run('seeking the same decoded source does not discard its pending properties', async () => {
  const pending = deferred(), r = await runtime({ metadata: () => pending.promise })
  await r.store.play(track('451'))
  await r.store.seekTo(80000)
  pending.resolve(properties); await flush()
  assert.equal(r.store.audioInfo.sampleRateHz, 96000)
  assert.equal(r.store.audioInfo.bitDepth, 24)
  assert.equal(r.store.positionMs, 80000)
})

await run('same-generation session removal blocks delayed metadata', async () => {
  const pending = deferred(), r = await runtime({ metadata: () => pending.promise })
  await r.store.play(track('501'))
  r.store.hasPlaybackSession = false
  pending.resolve(properties); await flush()
  assert.equal(r.store.audioInfo.sampleRateHz, undefined)
})

await run('file release blocks delayed metadata while preserving pause state', async () => {
  const pending = deferred(), changed = deferred(), r = await runtime({ downloaded: true, metadata: () => pending.promise })
  await r.store.play(track('601')); await r.store.pause()
  const operation = r.store.withReleasedAudioFile('C:/Music/download.flac', () => changed.promise)
  await flush(); pending.resolve(properties); await flush()
  assert.equal(r.store.audioInfo, null)
  changed.resolve(); await operation
  assert.equal(r.store.isPlaying, false)
})

await run('late local file probe cannot replace already merged decoder properties', async () => {
  const pending = deferred(), r = await runtime({ downloaded: true, localInfo: () => pending.promise })
  await r.store.play(track('701')); await flush()
  assert.equal(r.store.audioInfo.sampleRateHz, 96000)
  pending.resolve({ format: 'FLAC', sampleRateHz: 44100, bitDepth: 16, channelCount: 1 }); await flush()
  assert.equal(r.store.audioInfo.sampleRateHz, 96000)
  assert.equal(r.store.audioInfo.bitDepth, 24)
  assert.equal(r.store.audioInfo.channelCount, 2)
})

await run('late local probe cannot restore metadata after file release', async () => {
  const local = deferred(), changed = deferred(), r = await runtime({ downloaded: true, localInfo: () => local.promise })
  await r.store.play(track('711')); await r.store.pause()
  const operation = r.store.withReleasedAudioFile('C:/Music/download.flac', () => changed.promise)
  await flush()
  assert.equal(r.store.audioInfo, null)
  local.resolve({ format: 'FLAC', bitrate: 2800, sampleRateHz: 44100 }); await flush()
  assert.equal(r.store.audioInfo, null, 'session visibility alone must not let a previous generation restore released file info')
  changed.resolve(); await operation
})

await run('sparse native average bitrate fills missing cache metadata without inventing parameters', async () => {
  const r = await runtime({ cached: true, metadata: () => ({ bitrate: 3000 }) })
  await r.store.play(track('721')); await flush()
  assert.equal(r.store.audioInfo.bitrate, 3000)
  assert.equal(r.store.audioInfo.codec, undefined)
  assert.equal(r.store.audioInfo.sampleRateHz, undefined)
  assert.equal(r.store.audioInfo.bitDepth, undefined)
  assert.equal(r.store.audioInfo.channelCount, undefined)
  assert.equal(r.store.audioInfo.qualityKey, 'hires')
  assert.equal(r.store.audioInfo.source, 'netease')
})

await run('platform bitrate remains authoritative when decoder reports a different average', async () => {
  const r = await runtime({ metadata: () => ({ ...properties, bitrate: 3000 }) })
  await r.store.play(track('731')); await flush()
  assert.equal(r.store.audioInfo.bitrate, 2964)
  assert.equal(r.store.audioInfo.sampleRateHz, 96000)
})

for (const late of [false, true]) {
  await run(`local file bitrate wins over decoder average when its probe ${late ? 'finishes late' : 'finishes first'}`, async () => {
    const local = deferred(), r = await runtime({ downloaded: true, metadata: () => ({ ...properties, bitrate: 3000 }), localInfo: () => late ? local.promise : { format: 'FLAC', bitrate: 2800 } })
    await r.store.play(track('741')); await flush()
    if (late) {
      assert.equal(r.store.audioInfo.bitrate, 3000)
      local.resolve({ format: 'FLAC', bitrate: 2800 }); await flush()
    }
    assert.equal(r.store.audioInfo.bitrate, 2800)
    assert.equal(r.store.audioInfo.sampleRateHz, 96000)
  })
}

await run('invalid native fields cannot erase platform metadata or leak quality/source changes', async () => {
  const r = await runtime({ metadata: () => ({ sampleRateHz: NaN, bitDepth: 0, channelCount: 2.5, bitrate: -1, codec: '   ', qualityKey: 'unknown', source: 'local' }) })
  await r.store.play(track('801')); await flush()
  assert.equal(r.store.audioInfo.bitrate, 2964)
  assert.equal(r.store.audioInfo.codec, 'FLAC')
  assert.equal(r.store.audioInfo.sampleRateHz, undefined)
  assert.equal(r.store.audioInfo.channelCount, undefined)
  assert.equal(r.store.audioInfo.bitDepth, undefined)
  assert.equal(r.store.audioInfo.source, 'netease')
  assert.equal(r.store.audioInfo.qualityKey, 'hires')
})

for (const [name, metadata] of [['missing metadata', () => null], ['metadata failure', () => { throw new Error('decoder properties unavailable') }]]) {
  await run(`${name} leaves successful playback and platform info intact`, async () => {
    const r = await runtime({ metadata }); await r.store.play(track('901')); await flush()
    assert.equal(r.metadata.length, 1)
    assert.equal(r.store.isPlaying, true)
    assert.equal(r.store.playError, null)
    assert.equal(r.store.audioInfo.bitrate, 2964)
    assert.equal(r.store.audioInfo.codec, 'FLAC')
  })
}

await run('native property loader accepts fractional kbps and trimmed codec without inventing absent fields', async () => {
  const calls = [], updates = []
  const module = await load(metadataText, { '@tauri-apps/api/core': { invoke: async (command, args) => {
    calls.push({ command, args }); return { bitrate: 193.5, codec: ' Opus ', bitDepth: null }
  } } })
  await module.loadPlaybackAudioInfo(42, () => false, info => updates.push(info))
  assert.equal(calls.length, 0)
  await module.loadPlaybackAudioInfo(42, () => true, info => updates.push(info))
  assert.deepEqual(calls, [{ command: 'get_playback_audio_info', args: { requestGeneration: 42 } }])
  assert.deepEqual(updates, [{ bitrate: 193.5, codec: 'Opus' }])
})

await run('a picked long-form track resumes where it was left and navigation starts over', async () => {
  const writes = []
  const history = { record() {}, rememberedPosition: () => 600_000, updateResumePosition: (item, position) => writes.push([item.id, position]) }
  const toasts = []
  const r = await runtime({
    history,
    toasts,
    durationMs: 3_600_000,
    resolve: args => ({ url: `https://audio.example/${args.songId}.mp3`, bitrate: 320000, format: 'mp3', level: 'exhigh', duration_ms: 3_600_000 }),
  })
  const episode = id => ({ ...track(id), durationMs: 3_600_000 })
  const starts = () => r.calls.filter(call => call.command === 'play_url_streaming').map(call => call.args.startPositionMs)
  r.store.playAll([episode('301'), episode('302')]); await flush()
  assert.deepEqual(toasts, [])
  assert.deepEqual(starts(), [600_000])
  await r.store.next(); await flush()
  assert.deepEqual(starts(), [600_000, 0], 'next must not jump into a remembered position')
  assert.deepEqual(writes, [['netease:301', 600_000]], 'switching away remembers where the previous episode was left')
})

await run('the remembered position is ignored when the setting is off or the track is short', async () => {
  const history = { record() {}, rememberedPosition: () => 600_000, updateResumePosition() {} }
  const off = await runtime({ history, rememberLongForm: false })
  await off.store.play({ ...track('311'), durationMs: 3_600_000 }); await flush()
  const short = await runtime({ history })
  await short.store.play({ ...track('312'), durationMs: 14 * 60_000 }); await flush()
  for (const r of [off, short]) {
    assert.equal(r.calls.find(call => call.command === 'play_url_streaming').args.startPositionMs, 0)
  }
})

await run('a host shares NetEase links per quality group and tags what it plays', async () => {
  const levels = []
  const r = await runtime({
    resolve: args => {
      levels.push(args.quality)
      // 没有超清母带：请求 sky 时平台退回无损
      const level = args.quality === 'sky' ? 'lossless' : args.quality
      return { url: `https://m701.music.126.net/${level}.flac`, bitrate: 1000000, format: 'flac', level, duration_ms: 180000 }
    },
  })
  const song = track('401')
  assert.deepEqual(await r.store.resolveShareableStreamUrls(song), [
    'https://m701.music.126.net/hires.flac#neriplayer-ltw-quality=netease:hires',
    'https://m701.music.126.net/exhigh.flac#neriplayer-ltw-quality=netease:exhigh',
    'https://m701.music.126.net/lossless.flac#neriplayer-ltw-quality=netease:lossless',
  ], 'the preferred quality first, then exhigh and lossless (Android takes three groups)')

  await r.store.play(song); await flush()
  assert.deepEqual(r.store.getCurrentStreamUrls(song.id), ['https://m701.music.126.net/hires.flac#neriplayer-ltw-quality=netease:hires'])
  assert.equal(r.store.getCurrentStreamUrl(song.id), 'https://m701.music.126.net/hires.flac#neriplayer-ltw-quality=netease:hires')
})

await run('a link received from the room keeps the host tag and an untagged one gets none', async () => {
  const r = await runtime()
  const tagged = 'https://m701.music.126.net/a.flac#neriplayer-ltw-quality=netease:lossless'
  await r.store.play({ ...track('402'), audioUrl: tagged, syncPayload: { channelId: 'netease', streamUrls: [tagged] } }); await flush()
  assert.equal(r.store.getCurrentStreamUrl('netease:402'), tagged)
  const untagged = 'https://m701.music.126.net/b.flac'
  await r.store.play({ ...track('403'), audioUrl: untagged }); await flush()
  assert.equal(r.store.getCurrentStreamUrl('netease:403'), untagged, 'the quality of a direct link is unknown')
})

await run('plays count toward the local playlist the queue was started from', async () => {
  const r = await runtime()
  const member = { ...track('501'), playlistKey: 'netease:501|Album' }
  const other = track('502')
  r.store.playAll([member], member.id, member.playlistKey, '42'); await flush()
  assert.equal(r.store.localPlaylistIdFor(member), '42')
  r.store.addToQueueEnd(other)
  assert.equal(r.store.localPlaylistIdFor(other), null, 'a song queued from elsewhere is not a playlist play')
  r.store.playAll([member]); await flush()
  assert.equal(r.store.localPlaylistIdFor(member), null, 'another queue clears the source')
  r.store.shufflePlay([member], '43'); await flush()
  assert.equal(r.store.localPlaylistIdFor(member), '43')
})

if (failures) process.exitCode = 1
else console.log('native playback audio info store regressions passed')
