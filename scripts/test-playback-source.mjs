import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

// 取流失败的重试间隔（250ms × 次数）在这里立即执行，只记录请求的等待时长
const retryDelays = []
const realSetTimeout = globalThis.setTimeout
globalThis.setTimeout = (callback, ms, ...args) => {
  retryDelays.push(ms)
  return realSetTimeout(callback, 0, ...args)
}

const mockModule = Buffer.from(`
  export const invoke = (command, args) => globalThis.__playbackInvoke(command, args)
`).toString('base64')
const mockModuleUrl = `data:text/javascript;base64,${mockModule}`
function transpileUrl(source) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
}
const protocolUrl = transpileUrl(await readFile(new URL('../src/stores/listenTogether/protocol.ts', import.meta.url), 'utf8'))
const queueUrl = transpileUrl(await readFile(new URL('../src/stores/listenTogether/queue.ts', import.meta.url), 'utf8'))
const streamQualityUrl = transpileUrl((await readFile(new URL('../src/stores/listenTogether/streamQuality.ts', import.meta.url), 'utf8'))
  .replace("from './protocol'", `from '${protocolUrl}'`))
const mapperUrl = transpileUrl((await readFile(new URL('../src/stores/listenTogether/mapper.ts', import.meta.url), 'utf8'))
  .replace("from './protocol'", `from '${protocolUrl}'`)
  .replace("from './queue'", `from '${queueUrl}'`)
  .replace("from './streamQuality'", `from '${streamQualityUrl}'`))
const failureUrl = transpileUrl(await readFile(new URL('../src/modules/playback/playbackFailure.ts', import.meta.url), 'utf8'))
const sourceUrl = new URL('../src/modules/playback/playbackSource.ts', import.meta.url)
const source = (await readFile(sourceUrl, 'utf8')).replace(
  "from '@tauri-apps/api/core'",
  `from '${mockModuleUrl}'`,
).replace("from '@/stores/listenTogether/mapper'", `from '${mapperUrl}'`)
  .replace("from './playbackFailure'", `from '${failureUrl}'`)
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const {
  canonicalizePlaybackTrack,
  PlaybackUrlResolver,
  playbackCacheReadCandidates,
  playbackCacheWriteOptions,
  resolvePlaybackResult,
  resolvePlaybackSource,
  resolveDownloadSource,
  selectPlaybackCandidate,
  setDecodableCodecs,
} = await import(moduleUrl)

const settings = {
  neteaseQuality: 'exhigh',
  qqMusicQuality: 'high',
  biliQuality: 'high',
  youtubeQuality: 'high',
}

function track(id) {
  return {
    id: `netease:${id}`,
    title: `song-${id}`,
    artist: 'artist',
    album: 'album',
    durationMs: 180_000,
    audioUrl: '',
    source: 'netease',
  }
}

async function run(name, test) {
  await test()
  console.log(`ok - ${name}`)
}

await run('continues below preview quality and selects the first full resource', async () => {
  const qualities = []
  const responses = [
    {
      url: 'https://music.example/preview.mp3',
      bitrate: 320_000,
      format: 'mp3',
      expected_content_length: 900_000,
      is_preview: true,
      unavailable_reason: null,
    },
    {
      url: 'https://music.example/full.mp3',
      bitrate: 192_000,
      format: 'mp3',
      expected_content_length: 4_800_000,
      is_preview: false,
      unavailable_reason: null,
    },
  ]
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_netease_song_url')
    qualities.push(args.quality)
    return responses.shift()
  }

  const resolved = await resolvePlaybackSource(track(101), settings)

  assert.deepEqual(qualities, ['exhigh', 'higher'])
  assert.equal(resolved?.qualityKey, 'higher')
  assert.equal(resolved?.isPreview, false)
  assert.equal(resolved?.expectedContentLength, 4_800_000)
  // 降级到 higher 播放，但缓存仍按首选 exhigh 建键，下次同样的设置能直接命中（对齐 Android）
  assert.match(resolved?.cacheKey ?? '', /-exhigh$/)
  assert.equal(resolved?.cacheKey, playbackCacheReadCandidates(track(101), settings)[0].cacheKey)
})

await run('keeps only the final preview fallback and forbids formal cache writes', async () => {
  const qualities = []
  globalThis.__playbackInvoke = async (_command, args) => {
    qualities.push(args.quality)
    return {
      url: `https://music.example/${args.quality}-preview.mp3`,
      bitrate: 128_000,
      format: 'mp3',
      expected_content_length: 800_000,
      is_preview: true,
      unavailable_reason: null,
    }
  }

  const resolved = await resolvePlaybackSource(track(102), settings)

  assert.deepEqual(qualities, ['exhigh', 'higher', 'standard'])
  assert.equal(resolved?.qualityKey, 'standard')
  assert.equal(resolved?.isPreview, true)
  assert.deepEqual(playbackCacheWriteOptions(resolved, 0), {})
})

await run('candidate streams share the preferred-quality cache key', async () => {
  const resolved = {
    type: 'success',
    url: 'https://audio.example/primary',
    candidateUrls: ['https://audio.example/fallback'],
    cacheKey: 'primary-cache',
    cacheKeyOverride: 'resolved-cache',
    expectedContentLength: 12_345,
    source: 'bilibili',
    qualityKey: 'lossless',
  }

  assert.deepEqual(playbackCacheWriteOptions(resolved, 0), {
    cacheKey: 'resolved-cache',
    expectedContentLength: 12_345,
  })
  assert.deepEqual(
    playbackCacheWriteOptions(resolved, 1, resolved.candidateUrls[0]),
    {
      cacheKey: 'resolved-cache',
    },
  )
})

await run('cache-first reads only the preferred-quality key', async () => {
  const neteaseCandidates = playbackCacheReadCandidates(track(105), settings)
  // 不再沿音质阶梯往下读：调高音质后不能继续命中旧的低音质副本
  assert.deepEqual(
    neteaseCandidates.map(candidate => candidate.qualityKey),
    ['exhigh'],
  )

  const biliTrack = {
    id: 'bilibili:BV1cache',
    title: 'cached-video',
    artist: 'artist',
    album: 'Bilibili|987654',
    durationMs: 180_000,
    audioUrl: '',
    source: 'bilibili',
  }
  const [biliCandidate] = playbackCacheReadCandidates(biliTrack, settings)
  globalThis.__playbackInvoke = async (command) => {
    assert.equal(command, 'get_bili_audio_url')
    return {
      url: 'https://audio.example/bili-primary',
      bandwidth: 320_000,
      codecs: 'mp4a.40.2',
      candidates: [],
    }
  }

  const resolved = await resolvePlaybackSource(biliTrack, settings)
  assert.equal(biliCandidate.cacheKey, resolved?.cacheKey)
})

await run('uses Android sync subAudioId as the Bilibili CID', async () => {
  const syncedTrack = {
    id: 'bilibili:BV1sync',
    title: 'synced-video',
    artist: 'artist',
    album: 'Synced album',
    durationMs: 180_000,
    audioUrl: '',
    source: 'bilibili',
    syncPayload: { subAudioId: '7654321' },
  }
  const [cacheCandidate] = playbackCacheReadCandidates(syncedTrack, settings)
  let receivedArgs
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_bili_audio_url')
    receivedArgs = args
    return {
      url: 'https://audio.example/bili-synced',
      bandwidth: 192_000,
      codecs: 'mp4a.40.2',
      candidates: [],
    }
  }

  const resolved = await resolvePlaybackSource(syncedTrack, settings)

  assert.equal(receivedArgs.cid, 7_654_321)
  assert.match(cacheCandidate.cacheKey, /-7654321-high$/)
  assert.equal(cacheCandidate.cacheKey, resolved?.cacheKey)
})

await run('restores a remote source from legacy local-playlist sync payload', async () => {
  const syncedTrack = {
    id: '-8837200123',
    title: 'synced-song',
    artist: 'artist',
    album: 'album',
    durationMs: 180_000,
    audioUrl: '',
    source: 'local',
    syncPayload: {
      channelId: 'netease',
      audioId: '1973665667',
    },
  }
  const [cacheCandidate] = playbackCacheReadCandidates(syncedTrack, settings)
  let receivedArgs
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_netease_song_url')
    receivedArgs = args
    return {
      url: 'https://music.example/synced.flac',
      bitrate: 999_000,
      format: 'flac',
      is_preview: false,
      unavailable_reason: null,
    }
  }

  const resolved = await resolvePlaybackSource(syncedTrack, settings)
  const canonical = canonicalizePlaybackTrack(syncedTrack)

  assert.equal(receivedArgs.songId, 1_973_665_667)
  assert.equal(canonical.id, 'netease:1973665667')
  assert.equal(canonical.source, 'netease')
  assert.match(cacheCandidate.cacheKey, /^netease-1973665667-exhigh$/)
  assert.equal(cacheCandidate.cacheKey, resolved?.cacheKey)
})

await run('accepts Android YouTube channel aliases and media URI fallback', async () => {
  const channelTrack = {
    id: '-100',
    title: 'synced-youtube',
    artist: 'artist',
    album: '',
    durationMs: 180_000,
    audioUrl: '',
    source: 'local',
    syncPayload: {
      channelId: 'youtubeMusic',
      audioId: 'channel-video-id',
    },
  }
  const mediaUriTrack = {
    ...channelTrack,
    id: '-101',
    syncPayload: {
      mediaUri: 'ytmusic://video/media-uri-video-id?playlistId=test',
    },
  }
  const receivedVideoIds = []
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_youtube_audio_url')
    receivedVideoIds.push(args.videoId)
    return [{
      url: 'https://audio.example/youtube',
      bitrate: 128_000,
      mime_type: 'audio/webm; codecs="opus"',
      content_length: 1_000_000,
    }]
  }

  await resolvePlaybackSource(channelTrack, settings)
  await resolvePlaybackSource(mediaUriTrack, settings)

  assert.deepEqual(receivedVideoIds, ['channel-video-id', 'media-uri-video-id'])
  assert.equal(canonicalizePlaybackTrack(channelTrack).id, 'youtube:channel-video-id')
  assert.equal(canonicalizePlaybackTrack(mediaUriTrack).id, 'youtube:media-uri-video-id')
})

await run('without FFmpeg a higher-bitrate webm/opus stream is ordered after m4a/aac', async () => {
  // 解码能力未知（或 FFmpeg 没加载上）时只认内置解码器，极高档也不能选 Opus
  setDecodableCodecs([])
  globalThis.__playbackInvoke = async (command) => {
    assert.equal(command, 'get_youtube_audio_url')
    return [
      {
        url: 'https://audio.example/youtube-opus',
        bitrate: 160_000,
        mime_type: 'audio/webm; codecs="opus"',
        content_length: 2_000_000,
      },
      {
        url: 'https://audio.example/youtube-aac',
        bitrate: 128_000,
        mime_type: 'audio/mp4; codecs="mp4a.40.2"',
        content_length: 1_800_000,
      },
    ]
  }

  const resolved = await resolvePlaybackSource({
    id: 'youtube:prefer-m4a',
    title: 'prefer-m4a',
    artist: 'tester',
    album: '',
    durationMs: 180_000,
    coverUrl: '',
    source: 'youtube',
    syncPayload: { mediaUri: 'ytmusic://video/prefer-m4a' },
  }, { ...settings, youtubeQuality: 'very_high' })

  assert.ok(resolved)
  assert.equal(resolved.url, 'https://audio.example/youtube-aac')
  assert.equal(resolved.candidateUrls?.[0], 'https://audio.example/youtube-opus')
})

// YouTube 常见的几条音频流：Opus 251/250/249 与 AAC 140/139
const youtubeLadder = [
  { url: 'https://audio.example/yt-139', bitrate: 48_000, mime_type: 'audio/mp4; codecs="mp4a.40.5"' },
  { url: 'https://audio.example/yt-249', bitrate: 50_000, mime_type: 'audio/webm; codecs="opus"' },
  { url: 'https://audio.example/yt-250', bitrate: 70_000, mime_type: 'audio/webm; codecs="opus"' },
  { url: 'https://audio.example/yt-140', bitrate: 128_000, mime_type: 'audio/mp4; codecs="mp4a.40.2"' },
  { url: 'https://audio.example/yt-251', bitrate: 160_000, mime_type: 'audio/webm; codecs="opus"' },
]

function youtubeTrack(id) {
  return {
    id: `youtube:${id}`, title: id, artist: 'tester', album: '', durationMs: 180_000, coverUrl: '',
    source: 'youtube', syncPayload: { mediaUri: `ytmusic://video/${id}` },
  }
}

await run('with FFmpeg YouTube quality tiers follow Android', async () => {
  setDecodableCodecs(['opus', 'e-ac-3', 'ac-3', 'alac'])
  globalThis.__playbackInvoke = async () => youtubeLadder
  const pick = async (quality) =>
    (await resolvePlaybackSource(youtubeTrack(`tier-${quality}`), { ...settings, youtubeQuality: quality })).url

  // 极高档取最高码率（Opus 251），高档取刚过 128k 的那条，中档取刚过 96k 的那条，低档取最低
  assert.equal(await pick('very_high'), 'https://audio.example/yt-251')
  assert.equal(await pick('high'), 'https://audio.example/yt-140')
  assert.equal(await pick('medium'), 'https://audio.example/yt-140')
  assert.equal(await pick('low'), 'https://audio.example/yt-139')
})

await run('YouTube prefers direct streams unless only HLS meets the quality', async () => {
  setDecodableCodecs(['opus'])
  globalThis.__playbackInvoke = async () => [
    { url: 'https://audio.example/hls-high.m3u8', bitrate: 160_000, mime_type: 'audio/mp4', stream_type: 'hls' },
    { url: 'https://audio.example/direct-high', bitrate: 160_000, mime_type: 'audio/webm; codecs="opus"' },
  ]
  assert.equal((await resolvePlaybackSource(youtubeTrack('direct-first'), { ...settings, youtubeQuality: 'very_high' })).url,
    'https://audio.example/direct-high')

  globalThis.__playbackInvoke = async () => [
    { url: 'https://audio.example/hls-high.m3u8', bitrate: 160_000, mime_type: 'audio/mp4', stream_type: 'hls' },
    { url: 'https://audio.example/direct-low', bitrate: 48_000, mime_type: 'audio/mp4' },
  ]
  assert.equal((await resolvePlaybackSource(youtubeTrack('hls-meets'), { ...settings, youtubeQuality: 'very_high' })).url,
    'https://audio.example/hls-high.m3u8')
})

await run('YouTube downloads prefer m4a even when Opus is decodable', async () => {
  setDecodableCodecs(['opus'])
  globalThis.__playbackInvoke = async () => youtubeLadder
  const download = await resolveDownloadSource(youtubeTrack('download-m4a'), { ...settings, youtubeQuality: 'very_high' })
  // 下载落盘要写标签、要用内置解码器校验，WebM/Opus 写不了标签（对齐 Android preferM4a）
  assert.equal(download.url, 'https://audio.example/yt-140')
})

await run('downloads skip codecs that only FFmpeg can decode', async () => {
  setDecodableCodecs(['opus', 'e-ac-3'])
  globalThis.__playbackInvoke = async () => ({
    url: 'https://audio.example/bili-dolby',
    bandwidth: 448_000,
    codecs: 'ec-3',
    quality_key: 'dolby',
    mime_type: 'audio/eac3',
    candidates: [
      { url: 'https://audio.example/bili-dolby', bandwidth: 448_000, codecs: 'ec-3', quality_key: 'dolby', mime_type: 'audio/eac3' },
      { url: 'https://audio.example/bili-aac', bandwidth: 320_000, codecs: 'mp4a.40.2', quality_key: 'high', mime_type: 'audio/mp4' },
    ],
  })
  const biliTrack = { ...track(906), id: 'bilibili:BV1download', source: 'bilibili', album: 'Bilibili|1' }
  const download = await resolveDownloadSource(biliTrack, { ...settings, biliQuality: 'dolby' })
  assert.equal(download.url, 'https://audio.example/bili-aac')
})

await run('Bilibili Dolby plays directly with FFmpeg and falls back up front without it', async () => {
  const dolby = async () => ({
    url: 'https://audio.example/bili-dolby',
    bandwidth: 448_000,
    codecs: 'ec-3',
    quality_key: 'dolby',
    mime_type: 'audio/eac3',
    candidates: [
      { url: 'https://audio.example/bili-dolby', bandwidth: 448_000, codecs: 'ec-3', quality_key: 'dolby', mime_type: 'audio/eac3' },
      { url: 'https://audio.example/bili-flac', bandwidth: 1_411_000, codecs: 'fLaC', quality_key: 'hires', mime_type: 'audio/flac' },
    ],
  })
  globalThis.__playbackInvoke = dolby
  setDecodableCodecs(['opus', 'e-ac-3', 'ac-3'])
  const withFfmpeg = await resolvePlaybackSource(
    { ...track(907), id: 'bilibili:BV1dolby', source: 'bilibili', album: 'Bilibili|2' },
    { ...settings, biliQuality: 'dolby' },
  )
  assert.equal(withFfmpeg.url, 'https://audio.example/bili-dolby')
  assert.equal(withFfmpeg.codec, 'E-AC-3')

  setDecodableCodecs([])
  const withoutFfmpeg = await resolvePlaybackSource(
    { ...track(908), id: 'bilibili:BV1dolby2', source: 'bilibili', album: 'Bilibili|3' },
    { ...settings, biliQuality: 'dolby' },
  )
  assert.equal(withoutFfmpeg.url, 'https://audio.example/bili-flac')
  assert.equal(withoutFfmpeg.qualityKey, 'hires')
})
setDecodableCodecs([])

await run('surfaces the Android-aligned login requirement', async () => {
  globalThis.__playbackInvoke = async () => ({
    url: null,
    bitrate: 0,
    format: 'mp3',
    is_preview: false,
    unavailable_reason: 'requires_login',
  })

  const resolution = await resolvePlaybackResult(track(103), settings)

  assert.equal(resolution.type, 'requires_login')
})

await run('does not retry lower qualities after an unknown response failure', async () => {
  const qualities = []
  globalThis.__playbackInvoke = async (_command, args) => {
    qualities.push(args.quality)
    return {
      url: null,
      bitrate: 0,
      format: 'mp3',
      is_preview: false,
      unavailable_reason: 'unknown',
    }
  }

  const resolved = await resolvePlaybackSource(track(104), settings)

  assert.equal(resolved, null)
  // 整次解析按 Android 重试 5 次，但每次都停在首选音质，不往下降
  assert.deepEqual(qualities, Array(6).fill('exhigh'))
})

await run('retries a transient resolution failure with Android backoff', async () => {
  retryDelays.length = 0
  let calls = 0
  globalThis.__playbackInvoke = async (_command, args) => {
    calls++
    if (calls <= 2) throw new Error('Network error: error sending request')
    return { url: `https://audio.example/${args.songId}.mp3`, bitrate: 320_000, format: 'mp3', level: 'exhigh' }
  }
  const resolution = await resolvePlaybackResult(track(105), settings)
  assert.equal(resolution.type, 'success')
  assert.equal(calls, 3)
  assert.deepEqual(retryDelays, [250, 500])
})

await run('gives up after five retries and reports a localized reason', async () => {
  let calls = 0
  globalThis.__playbackInvoke = async () => { calls++; throw new Error('Network error: timed out') }
  const resolution = await resolvePlaybackResult(track(106), settings)
  assert.equal(resolution.type, 'failure')
  assert.equal(resolution.reason, 'url_error')
  assert.equal(calls, 6)
})

await run('does not retry superseded requests or restricted tracks', async () => {
  let calls = 0
  globalThis.__playbackInvoke = async () => { calls++; throw new Error('Audio error: Playback request superseded') }
  assert.equal((await resolvePlaybackResult(track(107), settings)).type, 'failure')
  assert.equal(calls, 1)

  const restricted = []
  globalThis.__playbackInvoke = async (_command, args) => {
    restricted.push(args.quality)
    return { url: null, bitrate: 0, format: 'mp3', unavailable_reason: 'no_permission' }
  }
  const resolution = await resolvePlaybackResult(track(108), settings)
  assert.equal(resolution.type, 'failure')
  assert.equal(resolution.reason, 'no_permission')
  assert.deepEqual(restricted, ['exhigh', 'higher', 'standard'], 'one pass down the ladder, no retries')
})

await run('a NetEase transport error is retried at the preferred quality instead of downgrading', async () => {
  const qualities = []
  globalThis.__playbackInvoke = async (_command, args) => {
    qualities.push(args.quality)
    if (qualities.length === 1) throw new Error('Network error: error sending request')
    return { url: `https://audio.example/${args.songId}.flac`, bitrate: 900_000, format: 'flac', level: args.quality }
  }
  const resolution = await resolvePlaybackResult(track(109), { ...settings, neteaseQuality: 'lossless' })
  assert.equal(resolution.type, 'success')
  assert.equal(resolution.qualityKey, 'lossless')
  assert.deepEqual(qualities, ['lossless', 'lossless'])
})

await run('a YouTube LOGIN_REQUIRED stream error is a retryable failure, not a login prompt', async () => {
  let calls = 0
  globalThis.__playbackInvoke = async () => {
    calls++
    throw new Error('API error: YouTube playback failed: VISIONOS:LOGIN_REQUIRED | ANDROID_VR:UNPLAYABLE')
  }
  const resolution = await resolvePlaybackResult({ ...track(110), id: 'youtube:dQw4w9WgXcQ', source: 'youtube' }, settings)
  assert.equal(resolution.type, 'failure')
  assert.equal(calls, 6)
})

await run('Bilibili video info failures map to the video-info reason', async () => {
  globalThis.__playbackInvoke = async () => {
    throw new Error('API error: Bilibili video info unavailable: Network error: timed out')
  }
  const resolution = await resolvePlaybackResult({ ...track(111), id: 'bilibili:BV1xx411c7mD', source: 'bilibili' }, settings)
  assert.equal(resolution.type, 'failure')
  assert.equal(resolution.reason, 'video_info_unavailable')
  assert.equal(resolution.retryable, true, '网络超时可能是瞬时的，照常重试')
})

await run('a removed or hidden Bilibili video fails without retries', async () => {
  let calls = 0
  globalThis.__playbackInvoke = async () => {
    calls++
    throw new Error('API error: Bilibili video info unavailable: API error: Bili API error: code=62002, message="稿件不可见"')
  }
  const resolution = await resolvePlaybackResult({ ...track(112), id: 'bilibili:116933185832925', source: 'bilibili' }, settings)
  assert.equal(resolution.type, 'failure')
  assert.equal(resolution.reason, 'video_info_unavailable')
  assert.equal(resolution.retryable, false)
  assert.equal(calls, 1, '稿件不可见是明确答复，不再重试 5 次')
})

await run('retains only trusted room stream candidates associated with the primary URL', async () => {
  globalThis.__playbackInvoke = async () => { throw new Error('Direct room streams must not resolve again') }
  const primary = 'https://m801.music.126.net/room-primary'
  const backup = 'https://m802.music.126.net/room-backup'
  const roomTrack = {
    ...track(200), audioUrl: primary,
    syncPayload: { streamUrls: [primary, 'https://evil.test/audio', backup, backup, 'file:///private.mp3'] },
  }
  const resolved = await resolvePlaybackSource(roomTrack, settings)
  assert.equal(resolved.url, primary)
  assert.deepEqual(resolved.candidateUrls, [backup])
  assert.deepEqual(playbackCacheWriteOptions(resolved, 0), {})

  const unrelated = await resolvePlaybackSource({ ...roomTrack,
    syncPayload: { streamUrls: [backup] },
  }, settings)
  assert.deepEqual(unrelated.candidateUrls, [])

  const maliciousPrimary = await resolvePlaybackSource({ ...roomTrack,
    audioUrl: 'https://evil.test/audio',
    syncPayload: { streamUrls: ['https://evil.test/audio', backup] },
  }, settings)
  assert.deepEqual(maliciousPrimary.candidateUrls, [])

  const bili = await resolvePlaybackSource({ ...roomTrack, id: 'bilibili:BV1room', source: 'bilibili',
    audioUrl: 'https://a.bilivideo.com/audio',
    syncPayload: { streamUrls: ['https://a.bilivideo.com/audio',
      'https://b.mountaintoys.cn/audio', 'https://c.bilivideo.cn/audio'] },
  }, settings)
  assert.deepEqual(bili.candidateUrls, ['https://b.mountaintoys.cn/audio'])
})

await run('changing YouTube source invalidates stream resolution and inflight reuse', async () => {
  const resolver = new PlaybackUrlResolver()
  const sources = []
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_youtube_audio_url')
    sources.push(args.playbackSource)
    return [{ url: `https://audio.example/${args.playbackSource}.m4a`, bitrate: 128_000, mime_type: 'audio/mp4' }]
  }
  const youtube = { ...track(900), id: 'youtube:source-preference', source: 'youtube' }
  const first = await resolver.resolve(youtube, { ...settings, youtubePlaybackSource: 'automatic' })
  const second = await resolver.resolve(youtube, { ...settings, youtubePlaybackSource: 'android_vr' })
  assert.notEqual(first.url, second.url)
  assert.equal((await resolver.resolve(youtube, { ...settings, youtubePlaybackSource: 'automatic' })).url, first.url)
  assert.deepEqual(sources, ['automatic', 'android_vr'])
  let releaseFirst
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_youtube_audio_url')
    sources.push(args.playbackSource)
    if (args.playbackSource === 'visionos') return new Promise(resolve => { releaseFirst = resolve })
    return [{ url: 'https://audio.example/new-preference.m4a', bitrate: 128_000, mime_type: 'audio/mp4' }]
  }
  const pending = resolver.resolve(youtube, { ...settings, youtubePlaybackSource: 'visionos' })
  const latest = await resolver.resolve(youtube, { ...settings, youtubePlaybackSource: 'web_creator' })
  assert.equal(latest.url, 'https://audio.example/new-preference.m4a')
  releaseFirst([{ url: 'https://audio.example/old-preference.m4a', bitrate: 128_000, mime_type: 'audio/mp4' }])
  await pending
  assert.equal((await resolver.resolve(youtube, { ...settings, youtubePlaybackSource: 'web_creator' })).url, latest.url)
})

const unavailableResponse = reason => ({ url: null, bitrate: 0, format: 'mp3', unavailable_reason: reason })
const fallbackSettings = { ...settings, neteaseLocalSourceFallback: true, neteaseAutoSourceSwitch: true }

await run('Netease fallback prioritizes local and keeps candidate format and duration', async () => {
  const original = track(901)
  const before = structuredClone(original)
  const commands = []
  globalThis.__playbackInvoke = async (command, args) => {
    commands.push(command)
    if (command === 'get_netease_song_url') return unavailableResponse('no_permission')
    assert.equal(command, 'find_netease_local_sources')
    assert.equal(args.songId, '901')
    return [
      { id: 'local:first', url: 'C:/fixture/first.flac', duration_ms: 180_000 },
      { id: 'local:second', url: 'C:/fixture/second.mp3', duration_ms: 179_000 },
    ]
  }
  const resolved = await new PlaybackUrlResolver().resolve(original, fallbackSettings)
  assert.equal(resolved.source, 'local')
  assert.deepEqual(original, before)
  assert.ok(!commands.includes('find_netease_bili_sources'))
  assert.deepEqual(playbackCacheWriteOptions(resolved, 0), {})
  const second = selectPlaybackCandidate(resolved, 1)
  assert.equal(second.url, 'C:/fixture/second.mp3')
  assert.equal(second.format, 'mp3')
  assert.equal(second.durationMs, 179_000)
  assert.equal(second.audioInfo.mimeType, 'audio/mpeg')
})

await run('Bili fallback keeps other matching videos and their cache identities', async () => {
  const original = track(902)
  globalThis.__playbackInvoke = async (command, args) => {
    if (command === 'get_netease_song_url') return unavailableResponse('no_permission')
    if (command === 'find_netease_local_sources') return []
    if (command === 'find_netease_bili_sources') return [
      { id: 'bilibili:BVfirst', album: 'Bilibili|12', duration_ms: 181_000 },
      { id: 'bilibili:BVsecond', album: 'Bilibili|34', duration_ms: 179_000 },
    ]
    assert.equal(command, 'get_bili_audio_url')
    assert.equal(args.cid, args.bvid === 'BVfirst' ? 12 : 34)
    return { url: `https://a.bilivideo.com/${args.bvid}.m4a`, bandwidth: 128_000, codecs: 'mp4a.40.2', quality_key: 'high', candidate_urls: [] }
  }
  const resolved = await new PlaybackUrlResolver().resolve(original, fallbackSettings)
  assert.equal(resolved.source, 'bilibili')
  assert.match(resolved.cacheKey, /^bili-BVfirst-12-/)
  const second = selectPlaybackCandidate(resolved, 1)
  assert.equal(second.url, 'https://a.bilivideo.com/BVsecond.m4a')
  assert.match(playbackCacheWriteOptions(resolved, 1).cacheKey, /^bili-BVsecond-34-/)
  assert.equal(second.durationMs, 179_000)
  assert.equal(original.id, 'netease:902')
})

await run('disabled fallback and unknown failures never search alternate sources', async () => {
  for (const [reason, configured] of [['no_permission', settings], ['unknown', fallbackSettings], ['requires_login', fallbackSettings]]) {
    globalThis.__playbackInvoke = async command => {
      assert.equal(command, 'get_netease_song_url')
      return unavailableResponse(reason)
    }
    const result = await new PlaybackUrlResolver().resolve(track(903), configured)
    assert.notEqual(result.type, 'success')
  }
})

await run('login gated high quality still tries an accessible lower quality', async () => {
  const qualities = []
  globalThis.__playbackInvoke = async (command, args) => {
    assert.equal(command, 'get_netease_song_url')
    qualities.push(args.quality)
    return args.quality === 'exhigh' ? unavailableResponse('requires_login')
      : { url: 'https://music.example/lower.mp3', bitrate: 192_000, format: 'mp3', is_preview: false }
  }
  const resolved = await new PlaybackUrlResolver().resolve(track(904), fallbackSettings)
  assert.equal(resolved.type, 'success')
  assert.deepEqual(qualities, ['exhigh', 'higher'])
})

await run('downloads reject previews without consuming playback fallback settings', async () => {
  globalThis.__playbackInvoke = async command => {
    assert.equal(command, 'get_netease_song_url')
    return { url: 'https://music.example/preview.mp3', bitrate: 128_000, format: 'mp3', is_preview: true }
  }
  await assert.rejects(() => resolveDownloadSource(track(905), fallbackSettings), /Preview audio/)
})

console.log('playback source tests passed')
