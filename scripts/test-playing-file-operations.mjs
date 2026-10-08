import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import { ref } from 'vue'

const source = await readFile(new URL('../src/stores/player.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('player.ts', source, ts.ScriptTarget.ES2022, true)
function declaration(name) {
  let found
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) found = node
    ts.forEachChild(node, visit)
  }
  visit(parsed)
  assert.ok(found, `missing file release operation ${name}`)
  return found.getText(parsed)
}
const compiled = ts.transpileModule([
  declaration('audioFilePathKey'), declaration('withReleasedAudioFile'), declaration('handleDownloadedFileRemoved'),
  declaration('pause'), declaration('seekTo'), declaration('resume'), declaration('togglePlayPause'),
  declaration('currentAudioFileMutation'),
  (() => {
    let play
    const visit = node => {
      if (ts.isFunctionDeclaration(node) && node.name?.text === 'play') play = node
      ts.forEachChild(node, visit)
    }
    visit(parsed)
    assert.ok(play)
    // 一起听拦截判断 + 等待文件操作的两条语句
    return `async function blockedPlay(track, commandSource = 'local', startPositionMs = 0, forceResolve = false, allowRememberedPosition = true) { ${play.body.statements.slice(0, 3).map(node => node.getText(parsed)).join('\n')} }`
  })(),
].join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no }); return { promise, resolve, reject } }
const path = 'C:/Music/song.flac'
function runtime({ playing = true, releaseError = false, switching = false } = {}) {
  const currentTrack = ref({ id: switching ? 'netease:2' : 'netease:1', audioUrl: switching ? '' : path, title: 'Song' })
  const queue = ref([{ ...currentTrack.value }])
  const isPlaying = ref(playing), isLoadingAudio = ref(switching), isPlayingFromDownload = ref(true)
  const positionMs = ref(43000), audioInfo = ref({ format: 'FLAC' }), hasPlaybackSession = ref(true)
  const release = deferred(), calls = [], mutations = new Map()
  const context = {
    currentTrack, queue, isPlaying, isLoadingAudio, isPlayingFromDownload, positionMs, audioInfo, hasPlaybackSession,
    isPlayingFromCache: ref(false), audioFileMutations: mutations,
    getPlaybackSourceKind: () => 'netease', isRemotePlaybackTrack: track => !track.id.startsWith('local:'),
    playbackStartupWatchdog: { cancel: () => {} }, replacePlaybackDemand: () => {}, freezeRenderedPosition: () => {},
    savePlayerState: () => {}, log: { warn: () => {} }, markCommandSource: () => {},
    blockedByListenTogether: () => false, blocksLocalSongInRoom: () => false,
    persistLongFormProgress: () => {}, persistCurrentLongFormProgress: () => {},
    shouldDeferPlaybackSeek: () => false,
    durationMs: ref(240000), lastSeekCommand: ref({ seq: 0 }),
    markOptimisticSeek: pos => { positionMs.value = pos; return pos },
    useDownloadStore: () => ({ getDownloadedTrack: () => ({ filePath: switching ? 'C:/Music/next.flac' : path }) }),
    invoke: async command => { calls.push(command); if (command === 'release_audio_file') { await release.promise; if (releaseError) throw new Error('release failed'); return true } },
    play: async (track, command, pos) => { calls.push({ resume: track.id, pos, url: track.audioUrl }); isPlaying.value = true },
  }
  const globals = `let playbackRequestToken = 7, loadedPlaybackRequestToken = ${switching ? 6 : 7}, _currentLoadedFromDownloadPath = "C:/Music/song.flac", _interpIsPlaying = true, _needsReload = false, lastUrlResolveTime = 99; let deferredPlaybackSeek = null, pendingSeek = null, seekGuardUntil = 0;`
  const methods = new Function(...Object.keys(context), `${globals}\n${compiled}\nreturn {withReleasedAudioFile, handleDownloadedFileRemoved, pause, seekTo, resume, togglePlayPause, blockedPlay, isInterpolating: () => _interpIsPlaying, interrupt: () => { playbackRequestToken++; currentTrack.value = {id:'netease:2'} }};`)(...Object.values(context))
  return { ...methods, ...context, release, calls }
}

{
  const r = runtime(); let changed = false
  const operation = r.withReleasedAudioFile(path, async () => { changed = true; r.handleDownloadedFileRemoved('netease:1', path) })
  for (let i = 0; i < 6; i++) await Promise.resolve()
  assert.equal(changed, false, 'delete must wait for decoder release acknowledgment')
  assert.equal(r.audioFileMutations.size, 1)
  r.release.resolve(); await operation
  assert.equal(changed, true)
  assert.equal(r.currentTrack.value.audioUrl, '')
  assert.equal(r.queue.value[0].audioUrl, '')
  assert.deepEqual(r.calls.at(-1), { resume: 'netease:1', pos: 43000, url: '' })
  assert.equal(r.audioFileMutations.size, 0)
}
{
  const r = runtime({ playing: false }); r.release.resolve()
  await r.withReleasedAudioFile(path, async () => {})
  assert.equal(r.isPlaying.value, false, 'editing a paused track must stay paused')
  assert.ok(r.calls.every(call => typeof call === 'string'))
}
{
  const r = runtime(); let changed = false
  const operation = r.withReleasedAudioFile(path, async () => { changed = true })
  r.interrupt(); r.release.resolve(); await operation
  assert.equal(changed, true)
  assert.ok(r.calls.every(call => typeof call === 'string'), 'finishing an old file operation must not replay a superseded song')
}
{
  const r = runtime({ releaseError: true }); let changed = false; r.release.resolve()
  await assert.rejects(r.withReleasedAudioFile(path, async () => { changed = true }), /release failed/)
  assert.equal(changed, false, 'failed release must leave files intact')
  assert.equal(r.audioFileMutations.size, 0)
}
{
  const r = runtime()
  const operation = r.withReleasedAudioFile(path, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  await r.pause(); r.release.resolve(); await operation
  assert.equal(r.isPlaying.value, false, 'user pause during release must cancel automatic restoration')
  assert.ok(r.calls.every(call => typeof call === 'string'))
}
{
  const r = runtime()
  const operation = r.withReleasedAudioFile(path, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  await r.seekTo(90000); r.release.resolve(); await operation
  assert.equal(r.calls.at(-1).pos, 90000, 'seek during editing must replace the old saved position')
}
{
  const r = runtime({ playing: false, switching: true }); r.release.resolve()
  await r.withReleasedAudioFile(path, async () => {})
  assert.deepEqual(r.calls, ['release_audio_file'], 'deleting the previous file must not invalidate a different pending song')
  assert.equal(r.isLoadingAudio.value, true)
  assert.equal(r.currentTrack.value.id, 'netease:2')
}
{
  const r = runtime()
  const operation = r.withReleasedAudioFile(path, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  const request = r.blockedPlay({ id: 'local:1', audioUrl: path }, 'local', 60000)
  assert.ok(r.calls.every(call => typeof call === 'string'), 'a modified file must not be reopened before release/write ends')
  r.release.resolve(); await operation; await request
  assert.deepEqual(r.calls.at(-1), { resume: 'local:1', pos: 60000, url: path })
  assert.equal(r.calls.filter(call => typeof call !== 'string').length, 1, 'new same-file play replaces old restoration')
}
{
  const r = runtime({ playing: false })
  const operation = r.withReleasedAudioFile(path, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  await r.resume(); await r.seekTo(72000); r.release.resolve(); await operation
  assert.equal(r.calls.at(-1).pos, 72000, 'resume during editing uses the latest requested position')
}
{
  const r = runtime()
  const operation = r.withReleasedAudioFile(path, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  const request = r.blockedPlay({ id: 'local:1', audioUrl: path }, 'local', 60000)
  await r.pause(); r.release.resolve(); await operation; await request
  assert.ok(r.calls.every(call => typeof call === 'string'), 'pause must also cancel a same-file play waiting for editing')
}
{
  const r = runtime(), otherFile = 'C:/Music/other.flac'
  const operation = r.withReleasedAudioFile(otherFile, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  const request = r.blockedPlay({ id: 'local:other', audioUrl: otherFile }, 'local', 0)
  await r.pause()
  assert.ok(r.calls.includes('pause'), 'pausing a deferred other-file play must also pause the still audible old song')
  r.release.resolve(); await operation; await request
  assert.ok(r.calls.every(call => typeof call === 'string'))
}
{
  const r = runtime(), otherFile = 'C:/Music/other.flac'
  const operation = r.withReleasedAudioFile(otherFile, async () => {})
  for (let i = 0; i < 6; i++) await Promise.resolve()
  const request = r.blockedPlay({ id: 'local:other', audioUrl: otherFile }, 'local', 0)
  await r.togglePlayPause()
  assert.equal(r.isInterpolating(), false, 'pause button must freeze the old song progress while another file is edited')
  r.release.resolve(); await operation; await request
}
console.log('playing file operation regressions passed')
