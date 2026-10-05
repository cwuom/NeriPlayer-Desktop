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

  const protocolCompiled = ts.transpileModule(protocolSource, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText
  const protocolUrl = `data:text/javascript;base64,${Buffer.from(protocolCompiled).toString('base64')}`
  const queueSource = await readFile(path.join(root, 'src/stores/listenTogether/queue.ts'), 'utf8')
  const queueCompiled = ts.transpileModule(queueSource, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const queueUrl = `data:text/javascript;base64,${Buffer.from(queueCompiled).toString('base64')}`
  const compiled = ts.transpileModule(mapperSource.replace(
    "from './protocol'", `from '${protocolUrl}'`,
  ).replace("from './queue'", `from '${queueUrl}'`), {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
  return { ...await import(moduleUrl), ...await import(queueUrl) }
}

async function loadProtocolModule() {
  const protocolPath = path.join(root, 'src/stores/listenTogether/protocol.ts')
  const protocolSource = await readFile(protocolPath, 'utf8')
  const compiled = ts.transpileModule(protocolSource, {
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
  isTrustedInboundStreamUrl,
  trustedInboundStreamUrls,
  hasSameLtTrackSequence,
  getLtQueueReference,
  buildListenTogetherQueueMutationPlan,
} = await loadMapperModule()

const {
  normalizeLtHttpBaseUrl,
  normalizeLtInviteBaseUrl,
  normalizeLtJoinSecret,
  resolveLtJoinSecret,
} = await loadProtocolModule()

assert.equal(
  normalizeLtHttpBaseUrl(' https://listen.example/room/ '),
  'https://listen.example/room',
)
assert.equal(normalizeLtInviteBaseUrl('https://listen.example/room/'), 'https://listen.example/room')
assert.equal(normalizeLtInviteBaseUrl('http://listen.example'), null)
assert.equal(normalizeLtInviteBaseUrl('https://listen.example?token=secret'), null)
assert.equal(normalizeLtInviteBaseUrl('javascript:alert(1)'), null)
assert.equal(normalizeLtJoinSecret(' invite-secret '), 'invite-secret')
assert.equal(normalizeLtJoinSecret('x'.repeat(257)), undefined)
assert.equal(resolveLtJoinSecret(undefined, 'invite-secret'), 'invite-secret')
assert.equal(resolveLtJoinSecret('response-secret', 'invite-secret'), 'response-secret')

// buildStableKey: 对齐 Android buildStableTrackKey
assert.equal(buildStableKey('netease', '123'), 'netease:123')
assert.equal(buildStableKey('bilibili', 'BV1', '999'), 'bilibili:BV1:999')
assert.equal(buildStableKey('bilibili', 'BV1'), 'bilibili:BV1')
assert.equal(buildStableKey('youtubeMusic', 'vid'), 'youtubeMusic:vid')
assert.equal(
  buildStableKey('youtubeMusic', 'vid', undefined, 'PL123'),
  'youtubeMusic:vid:PL123',
)
assert.equal(hasSameLtTrackSequence([{ stableKey: 'youtubeMusic:vid:A' }],
  [{ stableKey: 'youtubeMusic:vid:B' }]), false)
assert.equal(hasSameLtTrackSequence([{ stableKey: 'a' }, { stableKey: 'b' }],
  [{ stableKey: 'b' }, { stableKey: 'a' }]), false)
assert.equal(hasSameLtTrackSequence([{ stableKey: 'a' }, { stableKey: 'a' }],
  [{ stableKey: 'a' }, { stableKey: 'a' }]), true)

// 入站直链必须同时满足协议和来源 CDN 白名单
assert.equal(isTrustedInboundStreamUrl('https://m801.music.126.net/song.mp3', 'netease'), true)
assert.equal(isTrustedInboundStreamUrl('https://example.test/song.mp3', 'netease'), false)
assert.equal(isTrustedInboundStreamUrl('file:///tmp/song.mp3', 'netease'), false)
assert.equal(isTrustedInboundStreamUrl('https://rr1.googlevideo.com/videoplayback?id=1', 'youtubeMusic'), true)
assert.equal(isTrustedInboundStreamUrl('https://rr1.googlevideo.com/videoplayback?id=1', 'qqMusic'), false)
assert.equal(isTrustedInboundStreamUrl('https://audio.mountaintoys.cn/a', 'bilibili'), true)
assert.equal(isTrustedInboundStreamUrl('https://evil-music.126.net/a', 'netease'), false)
assert.equal(isTrustedInboundStreamUrl('https://music.126.net.evil.test/a', 'netease'), false)

// 多候选数组优先，旧单链接补尾，按平台限制去重并复用白名单
{
  const primary = 'https://m801.music.126.net/a'
  const backup = 'https://m802.music.126.net/b'
  const legacy = 'https://m803.music.126.net/c'
  assert.deepEqual(trustedInboundStreamUrls('netease', [
    'file:///private.mp3', ` ${primary} `, primary, backup,
  ], legacy), [primary, backup, legacy])
  assert.equal(trustedInboundStreamUrls('netease', [primary, backup, legacy,
    'https://m804.music.126.net/d']).length, 3)
  assert.equal(trustedInboundStreamUrls('bilibili', [
    'https://a.bilivideo.com/a', 'https://b.bilivideo.cn/b', 'https://c.mountaintoys.cn/c',
  ]).length, 2)
  assert.equal(trustedInboundStreamUrls('youtubeMusic', [
    'https://a.googlevideo.com/a', 'https://b.googlevideo.com/b',
  ]).length, 1)
  const received = ltTrackToTrackInfo({
    stableKey: 'netease:42', channelId: 'netease', audioId: '42', name: 'Song',
    artist: 'Artist', durationMs: 1000, streamUrl: 'https://evil.test/song',
    streamUrls: [primary, backup],
  })
  assert.equal(received.audioUrl, primary)
  assert.deepEqual(received.syncPayload.streamUrls, [primary, backup])
}

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

// QQ Music 没有 Android 频道，不能进入跨端共享队列；当前曲被过滤时不回退首项
{
  const { queue, resolvedIndex } = toShareableQueueSnapshot([
    {
      id: 'qq:mid', title: 'QQ', artist: 'Artist', album: '', durationMs: 1,
      coverUrl: '', audioUrl: '',
    },
    {
      id: 'netease:ok', title: 'Netease', artist: 'Artist', album: '', durationMs: 1,
      coverUrl: '', audioUrl: '',
    },
  ], 0)
  assert.deepEqual(queue.map(t => t.stableKey), ['netease:ok'])
  assert.equal(resolvedIndex, -1)
}

// 本地当前曲被排除时同样没有有效共享索引
{
  const { queue, resolvedIndex } = toShareableQueueSnapshot([
    {
      id: 'netease:ok', title: 'Netease', artist: 'Artist', album: '', durationMs: 1,
      coverUrl: '', audioUrl: '',
    },
    {
      id: 'local:private', title: 'Local', artist: 'Artist', album: '', durationMs: 1,
      coverUrl: '', audioUrl: '/tmp/private.mp3',
    },
  ], 1)
  assert.deepEqual(queue.map(t => t.stableKey), ['netease:ok'])
  assert.equal(resolvedIndex, -1)
}

// 重复歌曲必须保留当前 occurrence，过滤不共享歌曲后仍按原队列顺序定位
{
  const song = id => ({ id, title: id, artist: 'Artist', album: '', durationMs: 1,
    coverUrl: '', audioUrl: '' })
  const queue = [song('netease:dup'), song('local:x'), song('qq:y'), song('netease:dup')]
  const primary = 'https://m801.music.126.net/a'
  const backup = 'https://m802.music.126.net/b'
  const snapshot = toShareableQueueSnapshot(queue, 3, true, primary, false, [primary, backup])
  assert.equal(snapshot.resolvedIndex, 1)
  assert.equal(snapshot.queue[0].streamUrl, undefined)
  assert.deepEqual(snapshot.queue[0].streamUrls, [])
  assert.deepEqual(snapshot.queue[1].streamUrls, [primary, backup])
  const hidden = toShareableQueueSnapshot(queue, 3, false, primary, false, [primary, backup])
  assert.ok(hidden.queue.every(track => !track.streamUrl && track.streamUrls.length === 0))

  const large = Array.from({ length: 2200 }, () => song('netease:dup'))
  const bounded = toShareableQueueSnapshot(large, 2100)
  assert.equal(bounded.queue.length, 2000)
  assert.equal(bounded.resolvedIndex, 1900)
  assert.equal(toShareableQueueSnapshot([song('local:x'), ...large], 0).resolvedIndex, -1)
}

// 已导入房间的同名重复项重建映射后仍保留各自基准引用
{
  const local = occurrence => ({
    id: 'netease:dup', title: 'same', artist: 'same', album: '', durationMs: 1,
    coverUrl: '', audioUrl: '', playlistKey: JSON.stringify({ stableKey: 'netease:dup', occurrence }),
  })
  const base = [trackInfoToLtTrack(local(0)), trackInfoToLtTrack(local(1))]
  for (const retained of [0, 1]) {
    const target = [trackInfoToLtTrack(local(retained))]
    const plan = buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, target, 0)
    assert.deepEqual(plan.mutation.operations, [{ type: 'remove', target: {
      stableKey: 'netease:dup', occurrence: 1 - retained,
    } }])
    assert.deepEqual(getLtQueueReference(target[0]), { stableKey: 'netease:dup', occurrence: retained })
    assert.equal(JSON.stringify(target[0]).includes('occurrence'), false)
    assert.equal(JSON.stringify(target[0]).includes('playlistKey'), false)
  }
  for (const playlistKey of ['legacy-key', '{}', '{"stableKey":"wrong","occurrence":0}',
    '{"stableKey":"netease:dup","occurrence":-1}', '{"stableKey":"netease:dup","occurrence":0.5}']) {
    assert.equal(getLtQueueReference(trackInfoToLtTrack({ ...local(0), playlistKey })), undefined)
  }
}

console.log('test-listen-together-mapper: ok')
