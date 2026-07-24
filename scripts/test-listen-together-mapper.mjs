/**
 * Listen Together mapper 对齐回归
 * node scripts/test-listen-together-mapper.mjs
 */
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

async function loadMapperModule() {
  const protocolPath = path.join(root, 'src/stores/listenTogether/protocol.ts')
  const mapperPath = path.join(root, 'src/stores/listenTogether/mapper.ts')
  const protocolSource = await readFile(protocolPath, 'utf8')
  const mapperSource = await readFile(mapperPath, 'utf8')

  // 去掉对 protocol 的 import, 把 channel 常量内联后一起 transpile
  const protocolConsts = protocolSource
    .split('export interface')[0]
    .replace(/^[\s\S]*?export const LtChannels/, 'const LtChannels')
  const mapperBody = mapperSource
    .replace(/import type \{ TrackInfo \} from ['"]@\/stores\/player['"]\s*/g, '')
    .replace(/import \{ LtChannels, type ListenTogetherTrack \} from ['"]\.\/protocol['"]\s*/g, '')

  const combined = `${protocolConsts}\n${mapperBody}\nexport { LtChannels }\n`
  const compiled = ts.transpileModule(combined, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
  return import(moduleUrl)
}

const {
  buildStableKey,
  trackInfoToLtTrack,
  ltTrackToTrackInfo,
  toShareableQueueSnapshot,
} = await loadMapperModule()

// buildStableKey: 对齐 Android buildStableTrackKey
assert.equal(buildStableKey('netease', '123'), 'netease:123')
assert.equal(buildStableKey('bilibili', 'BV1', '999'), 'bilibili:BV1:999')
assert.equal(buildStableKey('bilibili', 'BV1'), 'bilibili:BV1')
assert.equal(buildStableKey('youtubeMusic', 'vid'), 'youtubeMusic:vid')
assert.equal(
  buildStableKey('youtubeMusic', 'vid', undefined, 'PL123'),
  'youtubeMusic:vid:PL123',
)

// netease 往返
{
  const track = {
    id: 'netease:42',
    title: 'Song',
    artist: 'Artist',
    album: 'Album',
    durationMs: 1000,
    coverUrl: 'https://cover',
    audioUrl: '',
    source: 'netease',
  }
  const lt = trackInfoToLtTrack(track)
  assert.equal(lt.channelId, 'netease')
  assert.equal(lt.audioId, '42')
  assert.equal(lt.stableKey, 'netease:42')
  const back = ltTrackToTrackInfo(lt)
  assert.equal(back.id, 'netease:42')
  assert.equal(back.source, 'netease')
}

// bilibili subAudioId 从 album 提取
{
  const track = {
    id: 'bilibili:BV1xx',
    title: 'Bili',
    artist: 'UP',
    album: 'Bilibili|888',
    durationMs: 2000,
    coverUrl: '',
    audioUrl: '',
    source: 'bilibili',
  }
  const lt = trackInfoToLtTrack(track)
  assert.equal(lt.channelId, 'bilibili')
  assert.equal(lt.audioId, 'BV1xx')
  assert.equal(lt.subAudioId, '888')
  assert.equal(lt.stableKey, 'bilibili:BV1xx:888')
  const back = ltTrackToTrackInfo(lt)
  assert.equal(back.id, 'bilibili:BV1xx')
  assert.equal(back.album, 'Bilibili|888')
}

// YouTube: playlistContext 进入 stableKey, mediaUri 回填
{
  const track = {
    id: 'youtube:abc',
    title: 'YT',
    artist: 'Chan',
    album: '',
    durationMs: 3000,
    coverUrl: '',
    audioUrl: '',
    source: 'youtube',
    syncPayload: {
      channelId: 'youtube_music',
      audioId: 'abc',
      mediaUri: 'ytmusic://video/abc?playlistId=RDEM',
      playlistContextId: 'RDEM',
    },
  }
  const lt = trackInfoToLtTrack(track)
  assert.equal(lt.channelId, 'youtubeMusic')
  assert.equal(lt.audioId, 'abc')
  assert.equal(lt.playlistContextId, 'RDEM')
  assert.equal(lt.stableKey, 'youtubeMusic:abc:RDEM')
  assert.equal(lt.mediaUri, 'ytmusic://video/abc?playlistId=RDEM')

  const back = ltTrackToTrackInfo(lt)
  assert.equal(back.id, 'youtube:abc')
  assert.equal(back.source, 'youtube')
  assert.equal(back.syncPayload?.playlistContextId, 'RDEM')
  assert.equal(back.syncPayload?.mediaUri, 'ytmusic://video/abc?playlistId=RDEM')
}

// 默认 shareable 快照排除 local
{
  const { queue, resolvedIndex } = toShareableQueueSnapshot(
    [
      {
        id: 'netease:1',
        title: 'A',
        artist: 'a',
        album: '',
        durationMs: 1,
        coverUrl: '',
        audioUrl: '',
      },
      {
        id: 'local:x',
        title: 'Local',
        artist: 'l',
        album: '',
        durationMs: 1,
        coverUrl: '',
        audioUrl: '/tmp/a.mp3',
      },
      {
        id: 'netease:2',
        title: 'B',
        artist: 'b',
        album: '',
        durationMs: 1,
        coverUrl: '',
        audioUrl: '',
      },
    ],
    2,
  )
  assert.deepEqual(
    queue.map((t) => t.stableKey),
    ['netease:1', 'netease:2'],
  )
  assert.equal(resolvedIndex, 1)
}

{
  const tracks = [
    {
      id: 'netease:1',
      title: 'A',
      artist: 'a',
      album: '',
      durationMs: 1,
      coverUrl: '',
      audioUrl: '',
    },
    {
      id: 'netease:2',
      title: 'B',
      artist: 'b',
      album: '',
      durationMs: 1,
      coverUrl: '',
      audioUrl: '',
    },
  ]
  const streamUrl = 'https://music.126.net/current.mp3'
  const shared = toShareableQueueSnapshot(tracks, 1, true, streamUrl)

  assert.equal(shared.queue[0].streamUrl, undefined)
  assert.equal(shared.queue[1].streamUrl, streamUrl)

  const privateSnapshot = toShareableQueueSnapshot(tracks, 1, false, streamUrl)
  assert.equal(privateSnapshot.queue[0].streamUrl, undefined)
  assert.equal(privateSnapshot.queue[1].streamUrl, undefined)
}

{
  const duplicateTracks = [
    {
      id: 'netease:duplicate',
      title: 'First copy',
      artist: 'a',
      album: '',
      durationMs: 1,
      coverUrl: '',
      audioUrl: '',
    },
    {
      id: 'netease:duplicate',
      title: 'Second copy',
      artist: 'b',
      album: '',
      durationMs: 2,
      coverUrl: '',
      audioUrl: '',
    },
  ]
  const streamUrl = 'https://music.126.net/duplicate-current.mp3'
  const shared = toShareableQueueSnapshot(duplicateTracks, 1, true, streamUrl)

  assert.equal(shared.resolvedIndex, 1)
  assert.equal(shared.queue[0].streamUrl, undefined)
  assert.equal(shared.queue[1].streamUrl, streamUrl)
}

console.log('test-listen-together-mapper: ok')
