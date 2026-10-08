import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/playback/youtubeSeekRefreshPolicy.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { shouldRefreshUrlBeforeSeek: seek, shouldRefreshUrlBeforeResume: resume } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

// 用例移植自 Android YouTubeSeekRefreshPolicyTest
const direct = 'https://rr1---sn-aigzrn7k.googlevideo.com/videoplayback?source=youtube&mime=audio%2Fwebm&clen=3965665'
assert.equal(seek(true, direct), true, 'direct stream without n/sig must refresh before seeking')

const manifest = 'https://manifest.googlevideo.com/api/manifest/hls_playlist/expire/1773862162/id/demo/itag/234/source/youtube/playlist/index.m3u8'
assert.equal(seek(true, manifest), false)

const segment = 'https://rr3---sn-aigl6ney.googlevideo.com/videoplayback/playlist/index.m3u8/begin/0/len/3750/file/seg.ts'
assert.equal(seek(true, segment), false)

const resolved = 'https://rr1---sn-aigl6ney.googlevideo.com/videoplayback?source=youtube&mime=audio%2Fwebm&clen=3586688&n=resolved-n&sig=resolved-signature&pot=po-token-123'
assert.equal(seek(true, resolved), false)

for (const client of ['WEB_REMIX', 'WEB_CREATOR', 'TVHTML5']) {
  const missingPot = `https://rr1---sn-aigl6ney.googlevideo.com/videoplayback?source=youtube&c=${client}&mime=audio%2Fwebm&clen=3586688&n=resolved-n&sig=resolved-signature`
  assert.equal(seek(true, missingPot), true, `${client} without pot`)
  assert.equal(resume(true, missingPot), true, `${client} without pot`)
}

const now = 1_800_000_000_000
const nearlyExpired = `${resolved}&expire=${Math.floor((now + 60_000) / 1000)}`
assert.equal(resume(true, nearlyExpired, now), true)
assert.equal(seek(true, nearlyExpired, now), true)
const fresh = `${resolved}&expire=${Math.floor((now + 30 * 60_000) / 1000)}`
assert.equal(resume(true, fresh, now), false)

assert.equal(seek(false, direct), false, 'non-YouTube tracks never refresh')

const lookalike = 'https://rr1---sn-aigl6ney.fakegooglevideo.com/videoplayback?source=youtube&c=WEB_REMIX&mime=audio%2Fwebm&clen=3586688&n=resolved-n&sig=resolved-signature&pot=po-token-123'
assert.equal(seek(true, lookalike), true)
assert.equal(resume(true, lookalike), false)

// 缓存 / 已下载播放没有在线地址
assert.equal(seek(true, null), false)
assert.equal(seek(true, ''), false)
assert.equal(seek(true, 'file:///storage/emulated/0/Music/demo.m4a'), false)

assert.equal(seek(true, 'https://music.youtube.com/api/stats?foo=bar'), true)

console.log('youtube seek refresh policy tests passed')
