import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import { ref } from 'vue'

const root = new URL('../', import.meta.url)

function evaluate(source, context, names) {
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  return new Function(...Object.keys(context), `${compiled}\nreturn { ${names.join(',')} }`)(...Object.values(context))
}

function declarations(source, names, nested = false) {
  const parsed = ts.createSourceFile('playback.ts', source, ts.ScriptTarget.ES2022, true)
  const found = new Map()
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text)) {
      found.set(node.name.text, node.getText(parsed))
    }
    if (nested) ts.forEachChild(node, visit)
  }
  parsed.statements.forEach(visit)
  return names.map(name => {
    assert.ok(found.has(name), `missing actual playback handler: ${name}`)
    return found.get(name)
  }).join('\n')
}

const queueSource = await readFile(new URL('src/modules/playback/playbackQueue.ts', root), 'utf8')
const { resolvePlaybackQueueStartIndex } = evaluate(queueSource, { exports: {} }, ['resolvePlaybackQueueStartIndex'])
const playerSource = await readFile(new URL('src/stores/player.ts', root), 'utf8')
const exploreSource = await readFile(new URL('src/views/ExploreView.vue', root), 'utf8')
const exploreScript = exploreSource.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]

const song = (source, suffix, title) => ({
  kind: 'song', id: `${source}:${suffix}`, title, artist: `artist ${suffix}`,
  album: 'Search album', duration_ms: 180000, source, cover_url: `${suffix}.jpg`,
})
const oldTrack = {
  id: 'netease:old', title: 'Previous song', artist: 'Previous artist',
  album: '', durationMs: 120000, coverUrl: '', audioUrl: '',
}

function fixture(results, platform = 'netease') {
  const queue = ref([{ ...oldTrack }, { ...oldTrack, id: 'netease:next' }])
  const queueIndex = ref(0)
  const currentTrack = ref(queue.value[0])
  const routes = []
  const recorded = []
  // 保留真实队列替换逻辑，只替代取流和原生音频边界
  function play(track) {
    if (!queue.value.find(item => item.id === track.id)) queue.value.push(track)
    currentTrack.value = track
    queueIndex.value = resolvePlaybackQueueStartIndex(queue.value, track.id, track.playlistKey)
  }
  const player = evaluate(declarations(playerSource, ['playAll'], true), {
    queue, queueIndex, play, resolvePlaybackQueueStartIndex,
    shuffleEnabled: ref(false), shuffleBag: ref([]), shuffleHistory: ref([]), shuffleFuture: ref([]),
    rebuildShuffleBag() {}, setLocalPlaylistSource() {}, tracePlaybackUi() {},
  }, ['playAll'])
  player.play = play
  const handlers = evaluate(declarations(exploreScript, [
    'songToTrack', 'openResult', 'rememberSearch', 'playFromMenu',
  ]), {
    player, songResults: ref(results), activeTab: ref(platform), searchQuery: ref('Song'),
    searchHistory: { record: query => recorded.push(query) },
    openCollection: item => routes.push(item),
  }, ['openResult', 'playFromMenu', 'songToTrack'])
  return { ...handlers, player, queue, queueIndex, currentTrack, routes, recorded }
}

let failures = 0
function test(name, run) {
  try { run(); console.log(`PASS ${name}`) }
  catch (error) { failures++; console.error(`FAIL ${name}: ${error.message}`) }
}

for (const platform of ['netease', 'bilibili', 'youtube']) {
  const results = [song(platform, 'original', 'Song'), song(platform, 'cover', 'Song cover')]
  for (const entry of ['click', 'context menu']) {
    test(`${platform} search ${entry} replaces playback with the selected song only`, () => {
      const state = fixture(results, platform)
      if (entry === 'click') state.openResult(results[1])
      else state.playFromMenu(state.songToTrack(results[1]))
      assert.deepEqual(state.queue.value.map(track => track.id), [`${platform}:cover`])
      assert.equal(state.queueIndex.value, 0)
      assert.equal(state.currentTrack.value.id, `${platform}:cover`)
      assert.equal(state.currentTrack.value.title, 'Song cover')
      assert.deepEqual(state.recorded, ['Song'])
    })
  }
}

test('a linked song replaces previous playback without recording a URL as search history', () => {
  const linked = song('youtube', 'linked', 'Linked song')
  const state = fixture([linked], 'link')
  state.openResult(linked)
  assert.deepEqual(state.queue.value.map(track => track.id), ['youtube:linked'])
  assert.deepEqual(state.recorded, [])
})

test('search collections navigate without changing the current playback queue', () => {
  const state = fixture([])
  const collection = { kind: 'playlist', platform: 'netease', id: 'playlist', name: 'Playlist' }
  state.openResult(collection)
  assert.deepEqual(state.queue.value.map(track => track.id), ['netease:old', 'netease:next'])
  assert.deepEqual(state.routes, [collection])
  assert.deepEqual(state.recorded, ['Song'])
  state.openResult({ kind: 'notice', reason: 'unsupported' })
  assert.deepEqual(state.recorded, ['Song'])
})

test('discovery context playback retains its single-song enqueue behavior', () => {
  const state = fixture([])
  const track = { ...oldTrack, id: 'youtube:discovery' }
  state.playFromMenu(track)
  assert.deepEqual(state.queue.value.map(item => item.id), ['netease:old', 'netease:next', 'youtube:discovery'])
  assert.equal(state.currentTrack.value.id, 'youtube:discovery')
  assert.deepEqual(state.recorded, [])
})

test('explicit play all retains every selected playlist track', () => {
  const state = fixture([])
  const tracks = [oldTrack, { ...oldTrack, id: 'netease:next' }]
  state.player.playAll(tracks, 'netease:next')
  assert.deepEqual(state.queue.value.map(track => track.id), ['netease:old', 'netease:next'])
  assert.equal(state.currentTrack.value.id, 'netease:next')
  assert.equal(state.queueIndex.value, 1)
})

assert.equal(failures, 0, `${failures} search playback regressions failed`)
