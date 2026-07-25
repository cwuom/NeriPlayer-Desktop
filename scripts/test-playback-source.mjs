import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const mockModule = Buffer.from(`
  export const invoke = (command, args) => globalThis.__playbackInvoke(command, args)
`).toString('base64')
const mockModuleUrl = `data:text/javascript;base64,${mockModule}`
const loggerMockModule = Buffer.from(`
  const write = (scope, level, args) => {
    globalThis.__playbackLogs ??= []
    globalThis.__playbackLogs.push({ scope, level, args })
  }
  export const createLogger = scope => ({
    trace: (...args) => write(scope, 'trace', args),
    debug: (...args) => write(scope, 'debug', args),
    info: (...args) => write(scope, 'info', args),
    warn: (...args) => write(scope, 'warn', args),
    error: (...args) => write(scope, 'error', args),
  })
`).toString('base64')
const loggerMockModuleUrl = `data:text/javascript;base64,${loggerMockModule}`
const sourceUrl = new URL('../src/modules/playback/playbackSource.ts', import.meta.url)
const source = (await readFile(sourceUrl, 'utf8'))
  .replace(
    "from '@tauri-apps/api/core'",
    `from '${mockModuleUrl}'`,
  )
  .replace(
    "from '@/utils/logger'",
    `from '${loggerMockModuleUrl}'`,
  )
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const {
  bilibiliAutoSourceDurationFilter,
  canonicalizePlaybackTrack,
  PlaybackUrlResolver,
  playbackSourceCandidates,
  playbackCacheReadCandidates,
  playbackCacheWriteOptions,
  resolvePlaybackResult,
  resolvePlaybackSource,
  shouldAutoSwitchNeteaseSource,
} = await import(moduleUrl)

const settings = {
  neteaseQuality: 'exhigh',
  qqMusicQuality: 'high',
  biliQuality: 'high',
  youtubeQuality: 'high',
  neteaseAutoSourceSwitch: false,
}

const autoSourceSettings = {
  ...settings,
  neteaseAutoSourceSwitch: true,
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

function resetPlaybackLogs() {
  globalThis.__playbackLogs = []
}

function playbackLogMessages() {
  return globalThis.__playbackLogs.map(entry => String(entry.args[0] ?? ''))
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
      cacheKey: 'resolved-cache|candidate:1|https://audio.example/fallback',
    },
  )
})

await run('limits playback to one primary source and two backup sources', async () => {
  const source = key => ({
    type: 'success',
    url: `https://audio.example/${key}`,
    candidateUrls: [`https://audio.example/${key}-alternate`],
    cacheKey: key,
    source: 'bilibili',
    qualityKey: 'high',
  })
  const primary = {
    ...source('primary'),
    fallbackSources: [source('backup-1'), source('backup-2'), source('backup-3')],
  }

  const candidates = playbackSourceCandidates(primary)

  assert.deepEqual(candidates.map(candidate => candidate.cacheKey), [
    'primary',
    'backup-1',
    'backup-2',
  ])
  assert.deepEqual(candidates[0].candidateUrls, [
    'https://audio.example/primary-alternate',
  ])
})

await run('cache-first keys match resolution keys and include NetEase fallbacks', async () => {
  const neteaseCandidates = playbackCacheReadCandidates(track(105), settings)
  assert.deepEqual(
    neteaseCandidates.map(candidate => candidate.qualityKey),
    ['exhigh', 'higher', 'standard'],
  )
  assert.equal(neteaseCandidates.some(candidate => candidate.source === 'bilibili'), false)

  const autoSourceCandidates = playbackCacheReadCandidates(track(105), autoSourceSettings)
  assert.deepEqual(autoSourceCandidates.at(-1), {
    cacheKey: 'bili-auto-105-high',
    source: 'bilibili',
    qualityKey: 'high',
  })

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

await run('switches a no-permission NetEase track to the best Bilibili match', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      assert.equal(args.platform, 'bilibili')
      return [
        {
          id: 'bilibili:BVweak',
          title: 'unrelated live clip',
          artist: 'someone else',
          duration_ms: 420_000,
          source: 'bilibili',
        },
        {
          id: 'bilibili:BVbest',
          title: 'song-106 official audio',
          artist: 'artist',
          duration_ms: 181_000,
          source: 'bilibili',
        },
        {
          id: 'bilibili:BVbackup',
          title: 'song-106 lyric video',
          artist: 'artist',
          duration_ms: 183_000,
          source: 'bilibili',
        },
      ]
    }
    if (command === 'get_bili_video_pages') {
      const pages = {
        BVweak: [
          { cid: 1_060, title: 'unrelated live clip', duration_seconds: 420 },
        ],
        BVbest: [
          { cid: 1_061, title: 'song-106 official audio', duration_seconds: 181 },
        ],
        BVbackup: [
          { cid: 1_062, title: 'song-106 lyric video', duration_seconds: 183 },
        ],
      }
      return pages[args.bvid] ?? []
    }
    if (command === 'get_bili_audio_url') {
      if (args.bvid === 'BVbackup') {
        assert.equal(args.cid, 1_062)
        return {
          url: 'https://audio.example/bili-auto-secondary',
          bandwidth: 192_000,
          codecs: 'mp4a.40.2',
          candidates: [],
        }
      }
      assert.equal(args.bvid, 'BVbest')
      assert.equal(args.cid, 1_061)
      return {
        url: 'https://audio.example/bili-auto-primary',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [{
          url: 'https://audio.example/bili-auto-backup',
          bandwidth: 192_000,
          codecs: 'mp4a.40.2',
        }],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const originalTrack = {
    ...track(106),
    coverUrl: 'https://image.example/netease-106.jpg',
    playlistKey: 'netease:playlist:42',
    syncPayload: {
      channelId: 'netease',
      audioId: '106',
      lyric: '[00:00.00]original lyric',
      translatedLyric: '[00:00.00]translated lyric',
    },
  }
  const originalSnapshot = structuredClone(originalTrack)
  const resolved = await resolvePlaybackSource(originalTrack, autoSourceSettings)

  assert.ok(resolved)
  assert.equal(resolved.source, 'bilibili')
  assert.equal(resolved.audioInfo?.source, 'bilibili')
  assert.equal(resolved.url, 'https://audio.example/bili-auto-primary')
  assert.deepEqual(resolved.candidateUrls, ['https://audio.example/bili-auto-backup'])
  assert.match(resolved.cacheKey, /^bili-auto-106-BVbest-1061-high$/i)
  assert.equal(resolved.cacheKeyOverride, 'bili-auto-106-high')
  assert.deepEqual(
    playbackSourceCandidates(resolved).map(candidate => candidate.cacheKeyOverride),
    [
      'bili-auto-106-high',
      'bili-auto-106-high',
    ],
  )
  assert.deepEqual(
    calls.filter(call => call.command === 'get_netease_song_url').map(call => call.args.quality),
    ['exhigh', 'higher', 'standard'],
  )
  assert.ok(calls.some(call => call.command === 'search'))
  assert.deepEqual(originalTrack, originalSnapshot)
})

await run('selects the matching Bilibili page CID instead of defaulting to the first page', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVmulti',
        title: 'song-109 complete collection',
        artist: 'artist',
        duration_ms: 0,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      assert.equal(args.bvid, 'BVmulti')
      return [
        { cid: 1_091, title: 'opening theme', duration_seconds: 0 },
        { cid: 1_092, title: 'song-109', duration_seconds: 0 },
      ]
    }
    if (command === 'get_bili_audio_url') {
      assert.equal(args.bvid, 'BVmulti')
      assert.equal(args.avid, null)
      assert.equal(args.cid, 1_092)
      return {
        url: 'https://audio.example/bili-auto-multipage',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolved = await resolvePlaybackSource(track(109), autoSourceSettings)

  assert.equal(resolved?.url, 'https://audio.example/bili-auto-multipage')
  assert.equal(resolved?.cacheKey, 'bili-auto-109-BVmulti-1092-high')
  assert.equal(resolved?.cacheKeyOverride, 'bili-auto-109-high')
  assert.equal(
    calls.filter(call => call.command === 'get_bili_video_pages').length,
    1,
  )
})

await run('ranks the complete Bilibili result set before applying the Android search limit', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [
        ...Array.from({ length: 6 }, (_, index) => ({
          id: `bilibili:BVweak${index}`,
          title: `unrelated clip ${index}`,
          artist: 'someone else',
          duration_ms: 500_000,
          source: 'bilibili',
        })),
        {
          id: 'bilibili:BVranked',
          title: 'song-116 official audio',
          artist: 'artist',
          duration_ms: 180_000,
          source: 'bilibili',
        },
      ]
    }
    if (command === 'get_bili_video_pages') {
      if (args.bvid !== 'BVranked') return []
      return [{ cid: 1_160, title: 'song-116 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      assert.equal(args.bvid, 'BVranked')
      return {
        url: 'https://audio.example/bili-auto-ranked',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolved = await resolvePlaybackSource(track(116), autoSourceSettings)

  assert.equal(resolved?.url, 'https://audio.example/bili-auto-ranked')
  assert.ok(calls.some(call => call.command === 'get_bili_audio_url'))
})

await run('rejects a multipage Bilibili fallback that only matches by duration', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVcollection',
        title: 'song-115 complete collection',
        artist: 'artist',
        duration_ms: 0,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      return [
        { cid: 1_151, title: 'Disc 1', duration_seconds: 240 },
        { cid: 1_152, title: 'Disc 2', duration_seconds: 180 },
      ]
    }
    if (command === 'get_bili_audio_url') {
      throw new Error('a duration-only page must not be resolved')
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolved = await resolvePlaybackSource(track(115), autoSourceSettings)

  assert.equal(resolved, null)
  assert.equal(calls.some(call => call.command === 'get_bili_audio_url'), false)
})

await run('rejects a matching Bilibili collection when none of its pages match the song', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVunrelatedpages',
        title: 'song-112 complete collection',
        artist: 'artist',
        duration_ms: 0,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      return [
        { cid: 1_121, title: 'unrelated opening theme', duration_seconds: 180 },
        { cid: 1_122, title: 'unrelated ending theme', duration_seconds: 180 },
      ]
    }
    if (command === 'get_bili_audio_url') {
      throw new Error('an unrelated page must not be resolved')
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolved = await resolvePlaybackSource(track(112), autoSourceSettings)

  assert.equal(resolved, null)
  assert.equal(calls.some(call => call.command === 'get_bili_audio_url'), false)
})

await run('does not reuse an auto-switched source after auto source switching is disabled', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVcachetoggle',
        title: 'song-110 official audio',
        artist: 'artist',
        duration_ms: 180_000,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      return [{ cid: 1_100, title: 'song-110 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      return {
        url: 'https://audio.example/bili-cache-toggle',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolver = new PlaybackUrlResolver()
  const enabled = await resolver.resolve(track(110), autoSourceSettings)
  const disabled = await resolver.resolve(track(110), settings)

  assert.equal(enabled.type, 'success')
  assert.equal(enabled.source, 'bilibili')
  assert.equal(disabled.type, 'failure')
  assert.equal(
    calls.filter(call => call.command === 'get_netease_song_url').length,
    6,
  )
  assert.equal(calls.filter(call => call.command === 'search').length, 3)
})

await run('does not reuse an auto-switched source after Bilibili quality changes', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVcachequality',
        title: 'song-111 official audio',
        artist: 'artist',
        duration_ms: 180_000,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      return [{ cid: 1_110, title: 'song-111 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      return {
        url: `https://audio.example/bili-cache-${args.quality}`,
        bandwidth: args.quality === 'low' ? 128_000 : 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolver = new PlaybackUrlResolver()
  const high = await resolver.resolve(track(111), autoSourceSettings)
  const low = await resolver.resolve(track(111), {
    ...autoSourceSettings,
    biliQuality: 'low',
  })

  assert.equal(high.type, 'success')
  assert.equal(high.url, 'https://audio.example/bili-cache-high')
  assert.equal(low.type, 'success')
  assert.equal(low.url, 'https://audio.example/bili-cache-low')
  assert.deepEqual(
    calls
      .filter(call => call.command === 'get_bili_audio_url')
      .map(call => call.args.quality),
    ['high', 'low'],
  )
})

await run('does not reuse a synced NetEase audioId as the Bilibili fallback id', async () => {
  globalThis.__playbackInvoke = async (command, args) => {
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVsynced',
        title: 'song-108 official audio',
        artist: 'artist',
        duration_ms: 180_000,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      assert.equal(args.bvid, 'BVsynced')
      return [{ cid: 1_080, title: 'song-108 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      assert.equal(args.bvid, 'BVsynced')
      assert.equal(args.avid, null)
      assert.equal(args.cid, 1_080)
      return {
        url: 'https://audio.example/bili-auto-synced',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const syncedTrack = {
    ...track(108),
    syncPayload: {
      channelId: 'netease',
      audioId: '108',
    },
  }
  const resolved = await resolvePlaybackSource(syncedTrack, autoSourceSettings)

  assert.equal(resolved?.url, 'https://audio.example/bili-auto-synced')
  assert.equal(resolved?.cacheKeyOverride, 'bili-auto-108-high')
})

await run('does not auto switch a no-permission track when the setting is disabled', async () => {
  const calls = []
  globalThis.__playbackInvoke = async (command, args) => {
    calls.push({ command, args })
    return {
      url: null,
      bitrate: 0,
      format: 'mp3',
      is_preview: false,
      unavailable_reason: 'no_permission',
    }
  }

  const resolved = await resolvePlaybackSource(track(107), settings)

  assert.equal(resolved, null)
  assert.equal(calls.some(call => call.command === 'search'), false)
})

await run('keeps the Android auto-source trigger boundary narrow', async () => {
  assert.equal(shouldAutoSwitchNeteaseSource(true, false, 'no_permission'), true)
  assert.equal(shouldAutoSwitchNeteaseSource(true, false, 'no_play_url'), true)
  assert.equal(shouldAutoSwitchNeteaseSource(true, true, null), true)
  assert.equal(shouldAutoSwitchNeteaseSource(false, true, 'no_permission'), false)
  assert.equal(shouldAutoSwitchNeteaseSource(true, false, 'unknown'), false)
  assert.equal(shouldAutoSwitchNeteaseSource(true, false, null), false)
})

await run('uses Android Bilibili duration buckets and retries without a filter', async () => {
  assert.equal(bilibiliAutoSourceDurationFilter(0), 0)
  assert.equal(bilibiliAutoSourceDurationFilter(599_999), 1)
  assert.equal(bilibiliAutoSourceDurationFilter(600_000), 2)
  assert.equal(bilibiliAutoSourceDurationFilter(1_800_000), 3)
  assert.equal(bilibiliAutoSourceDurationFilter(3_600_000), 4)

  const searchDurations = []
  globalThis.__playbackInvoke = async (command, args) => {
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      searchDurations.push(args.bilibiliDuration)
      if (args.bilibiliDuration === 1) return []
      assert.equal(args.bilibiliDuration, 0)
      return [{
        id: 'bilibili:BVdurationretry',
        title: 'song-117 official audio',
        artist: 'artist',
        duration_ms: 180_000,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      return [{ cid: 1_170, title: 'song-117 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      return {
        url: 'https://audio.example/bili-duration-retry',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolved = await resolvePlaybackSource(track(117), autoSourceSettings)

  assert.equal(resolved?.url, 'https://audio.example/bili-duration-retry')
  assert.deepEqual(searchDurations, [1, 0, 1, 0, 1, 0])
})

await run('reports and sanitizes Bilibili fallback search failures and empty results', async () => {
  resetPlaybackLogs()
  let searchCalls = 0
  globalThis.__playbackInvoke = async (command) => {
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      searchCalls++
      if (searchCalls === 1) {
        throw new Error('search failed at https://api.example/search Cookie=session-secret')
      }
      return []
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolution = await new PlaybackUrlResolver().resolve(track(117), autoSourceSettings)
  const messages = playbackLogMessages()
  const serializedLogs = JSON.stringify(globalThis.__playbackLogs)

  assert.equal(resolution.type, 'failure')
  assert.match(
    resolution.message,
    /Bilibili fallback was attempted but failed during search: no usable candidates/i,
  )
  assert.notEqual(resolution.message, 'No playable stream returned')
  assert.ok(messages.includes('NetEase Bilibili auto-source fallback started'))
  assert.ok(messages.includes('NetEase Bilibili auto-source search failed'))
  assert.ok(messages.includes('NetEase Bilibili auto-source search returned no usable results'))
  assert.ok(messages.includes('NetEase Bilibili auto-source fallback failed'))
  assert.doesNotMatch(serializedLogs, /session-secret/)
  assert.doesNotMatch(serializedLogs, /https?:\/\//)
})

await run('reports Bilibili page errors and low matches before the final page failure', async () => {
  resetPlaybackLogs()
  globalThis.__playbackInvoke = async (command, args) => {
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_permission',
      }
    }
    if (command === 'search') {
      return [
        {
          id: 'bilibili:BVpageerror',
          title: 'unrelated upload',
          artist: 'someone else',
          duration_ms: 600_000,
          source: 'bilibili',
        },
        {
          id: 'bilibili:BVlowmatch',
          title: 'another unrelated upload',
          artist: 'someone else',
          duration_ms: 600_000,
          source: 'bilibili',
        },
      ]
    }
    if (command === 'get_bili_video_pages') {
      if (args.bvid === 'BVpageerror') {
        throw new Error('page lookup failed Authorization=Bearer page-secret https://api.example/pages')
      }
      return [{ cid: 1_181, title: 'unrelated long video', duration_seconds: 600 }]
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolution = await new PlaybackUrlResolver().resolve(track(118), autoSourceSettings)
  const messages = playbackLogMessages()
  const serializedLogs = JSON.stringify(globalThis.__playbackLogs)

  assert.equal(resolution.type, 'failure')
  assert.match(resolution.message, /failed during page: no page passed matching/i)
  assert.ok(messages.includes('NetEase Bilibili auto-source page lookup failed'))
  assert.ok(messages.includes('NetEase Bilibili auto-source page match rejected'))
  assert.ok(messages.includes('NetEase Bilibili auto-source fallback failed'))
  assert.doesNotMatch(serializedLogs, /page-secret/)
  assert.doesNotMatch(serializedLogs, /https?:\/\//)
})

await run('reports and sanitizes Bilibili stream lookup failures', async () => {
  resetPlaybackLogs()
  globalThis.__playbackInvoke = async (command) => {
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_play_url',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVstreamerror',
        title: 'song-119 official audio',
        artist: 'artist',
        duration_ms: 180_000,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      return [{ cid: 1_190, title: 'song-119 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      throw new Error('stream failed at https://stream.example/audio Authorization=Bearer stream-secret')
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const resolution = await new PlaybackUrlResolver().resolve(track(119), autoSourceSettings)
  const messages = playbackLogMessages()
  const serializedLogs = JSON.stringify(globalThis.__playbackLogs)

  assert.equal(resolution.type, 'failure')
  assert.match(resolution.message, /failed during stream: no playable stream/i)
  assert.ok(messages.includes('NetEase Bilibili auto-source stream lookup failed'))
  assert.ok(messages.includes('NetEase Bilibili auto-source fallback failed'))
  assert.doesNotMatch(serializedLogs, /stream-secret/)
  assert.doesNotMatch(serializedLogs, /https?:\/\//)
})

await run('switches a no-play-url NetEase track to Bilibili and caches it under a reusable fallback key', async () => {
  globalThis.__playbackInvoke = async (command, args) => {
    if (command === 'get_netease_song_url') {
      return {
        url: null,
        bitrate: 0,
        format: 'mp3',
        is_preview: false,
        unavailable_reason: 'no_play_url',
      }
    }
    if (command === 'search') {
      return [{
        id: 'bilibili:BVnoplay',
        title: 'song-110 official audio',
        artist: 'artist',
        duration_ms: 180_000,
        source: 'bilibili',
      }]
    }
    if (command === 'get_bili_video_pages') {
      assert.equal(args.bvid, 'BVnoplay')
      return [{ cid: 1_100, title: 'song-110 official audio', duration_seconds: 180 }]
    }
    if (command === 'get_bili_audio_url') {
      assert.equal(args.bvid, 'BVnoplay')
      assert.equal(args.cid, 1_100)
      return {
        url: 'https://audio.example/bili-auto-no-play-url',
        bandwidth: 320_000,
        codecs: 'mp4a.40.2',
        candidates: [],
      }
    }
    throw new Error(`Unexpected command: ${command}`)
  }

  const originalTrack = track(110)
  const resolved = await resolvePlaybackSource(originalTrack, autoSourceSettings)

  assert.equal(resolved?.url, 'https://audio.example/bili-auto-no-play-url')
  assert.equal(
    playbackCacheWriteOptions(resolved, 0).cacheKey,
    playbackCacheReadCandidates(originalTrack, autoSourceSettings).at(-1).cacheKey,
  )
})

await run('enables the Android-aligned auto source switch in persisted settings', async () => {
  const [settingsSource, rustSettingsSource, settingsViewSource] = await Promise.all([
    readFile(new URL('../src/stores/settings.ts', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/src/settings/store.rs', import.meta.url), 'utf8'),
    readFile(new URL('../src/views/SettingsView.vue', import.meta.url), 'utf8'),
  ])

  assert.match(settingsSource, /neteaseAutoSourceSwitch:\s*true/)
  assert.match(rustSettingsSource, /netease_auto_source_switch:\s*true/)
  assert.match(settingsViewSource, /v-model="neteaseAutoSourceSwitch"/)
})

await run('the player attempts structured fallback sources instead of flattening cache identity', async () => {
  const playerSource = await readFile(
    new URL('../src/stores/player.ts', import.meta.url),
    'utf8',
  )

  assert.match(playerSource, /playbackSourceCandidates\(resolved\)/)
  assert.match(playerSource, /played\.resolved/)
})

let playbackUiSource

await run('uses the resolved Bilibili source for fallback quality UI', async () => {
  const nowPlayingSource = await readFile(
    new URL('../src/components/NowPlaying.vue', import.meta.url),
    'utf8',
  )

  assert.match(nowPlayingSource, /const currentPlaybackSource = computed/)
  assert.match(
    nowPlayingSource,
    /resolvedPlaybackSourceForUi\(\s*player\.currentTrack,\s*player\.audioInfo\?\.source,?\s*\)/,
  )
  assert.match(nowPlayingSource, /qualityOptionsForSource\(currentPlaybackSource\)/)

  const helperSource = await readFile(
    new URL('../src/modules/playback/playbackUiSource.ts', import.meta.url),
    'utf8',
  )
  const helperCompiled = ts.transpileModule(helperSource, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText
  playbackUiSource = await import(
    `data:text/javascript;base64,${Buffer.from(helperCompiled).toString('base64')}`
  )

  const fallbackTrack = track(113)
  assert.equal(playbackUiSource.logicalPlaybackSource(fallbackTrack), 'netease')
  assert.equal(
    playbackUiSource.resolvedPlaybackSourceForUi(fallbackTrack, 'bilibili'),
    'bilibili',
  )
})

await run('replays a NetEase fallback after Bilibili quality changes', async () => {
  const settingsViewSource = await readFile(
    new URL('../src/views/SettingsView.vue', import.meta.url),
    'utf8',
  )

  assert.match(settingsViewSource, /shouldReplayForQualityChange\(source,\s*\{/)
  assert.match(settingsViewSource, /audioInfoSource:\s*player\.audioInfo\?\.source/)

  const fallbackTrack = track(114)
  const playbackState = {
    track: fallbackTrack,
    audioInfoSource: 'bilibili',
    isLoadingAudio: false,
    isPlayingFromDownload: false,
  }
  assert.equal(
    playbackUiSource.shouldReplayForQualityChange('bilibili', playbackState),
    true,
  )
  assert.equal(
    playbackUiSource.shouldReplayForQualityChange('netease', playbackState),
    false,
  )
})

console.log('playback source tests passed')
