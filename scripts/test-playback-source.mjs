import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

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
const mapperUrl = transpileUrl((await readFile(new URL('../src/stores/listenTogether/mapper.ts', import.meta.url), 'utf8'))
  .replace("from './protocol'", `from '${protocolUrl}'`)
  .replace("from './queue'", `from '${queueUrl}'`))
const sourceUrl = new URL('../src/modules/playback/playbackSource.ts', import.meta.url)
const source = (await readFile(sourceUrl, 'utf8')).replace(
  "from '@tauri-apps/api/core'",
  `from '${mockModuleUrl}'`,
).replace("from '@/stores/listenTogether/mapper'", `from '${mapperUrl}'`)
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const {
  canonicalizePlaybackTrack,
  playbackCacheReadCandidates,
  playbackCacheWriteOptions,
  resolvePlaybackResult,
  resolvePlaybackSource,
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
  assert.match(resolved?.cacheKey ?? '', /-higher$/)
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

await run('candidate streams use isolated formal cache keys', async () => {
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
      cacheKey: 'resolved-cache|candidate:1',
    },
  )
})

await run('cache-first keys match resolution keys and include NetEase fallbacks', async () => {
  const neteaseCandidates = playbackCacheReadCandidates(track(105), settings)
  assert.deepEqual(
    neteaseCandidates.map(candidate => candidate.qualityKey),
    ['exhigh', 'higher', 'standard'],
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

await run('prefers youtube m4a/aac over higher-bitrate webm/opus', async () => {
  // 桌面 symphonia 未启 opus; 即使 opus 码率更高也必须优先 mp4/AAC
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
  }, settings)

  assert.ok(resolved)
  assert.equal(resolved.url, 'https://audio.example/youtube-aac')
  assert.equal(resolved.candidateUrls?.[0], 'https://audio.example/youtube-opus')
})

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
  let calls = 0
  globalThis.__playbackInvoke = async () => {
    calls++
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
  assert.equal(calls, 1)
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

console.log('playback source tests passed')
