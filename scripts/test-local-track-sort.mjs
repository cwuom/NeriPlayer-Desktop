import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/localTrackSort.ts', import.meta.url), 'utf8').catch(error => {
  if (error.code === 'ENOENT') return ''
  throw error
})
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { sortLocalTracks } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
assert.equal(typeof sortLocalTracks, 'function', 'local tracks expose a sorting policy')

const track = (id, extra = {}) => ({
  id, title: `Track ${id}`, artist: 'Artist', album: 'Album', durationMs: 1000, ...extra,
})
const ids = tracks => tracks.map(item => item.id)
const tracks = Object.freeze([
  track('ten', { title: 'Track 10', artist: 'Artist 2', album: 'Album 10', durationMs: 3000, fileModifiedMs: 200 }),
  track('two', { title: 'Track 2', artist: 'Artist 10', album: 'Album 2', durationMs: 1000, fileModifiedMs: 300 }),
  track('one', { title: 'Track 1', artist: 'Artist 1', album: 'Album 1', durationMs: 2000, fileModifiedMs: 100 }),
])

assert.deepEqual(ids(sortLocalTracks(tracks, 'default')), ['ten', 'two', 'one'])
assert.deepEqual(ids(sortLocalTracks(tracks, 'default', true)), ['one', 'two', 'ten'])
assert.notEqual(sortLocalTracks(tracks, 'default'), tracks, 'default order still returns an independent array')
for (const [sortBy, ascending, descending] of [
  ['title', ['one', 'two', 'ten'], ['ten', 'two', 'one']],
  ['artist', ['one', 'ten', 'two'], ['two', 'ten', 'one']],
  ['album', ['one', 'two', 'ten'], ['ten', 'two', 'one']],
  ['duration', ['two', 'one', 'ten'], ['ten', 'one', 'two']],
  ['modified', ['one', 'ten', 'two'], ['two', 'ten', 'one']],
]) {
  assert.deepEqual(ids(sortLocalTracks(tracks, sortBy)), ascending, `${sortBy} sorts ascending`)
  assert.deepEqual(ids(sortLocalTracks(tracks, sortBy, true)), descending, `${sortBy} sorts descending`)
}
assert.deepEqual(ids(tracks), ['ten', 'two', 'one'], 'sorting never rearranges the library index')
assert.equal(sortLocalTracks(tracks, 'title')[0], tracks[2], 'sorting preserves track identity')

const withMissingTimes = [
  track('missing'), track('new', { fileModifiedMs: 200 }), track('zero', { fileModifiedMs: 0 }),
  track('old', { fileModifiedMs: 100 }), track('negative', { fileModifiedMs: -1 }),
  track('nan', { fileModifiedMs: Number.NaN }), track('infinite', { fileModifiedMs: Number.POSITIVE_INFINITY }),
]
assert.deepEqual(ids(sortLocalTracks(withMissingTimes, 'modified')),
  ['old', 'new', 'missing', 'zero', 'negative', 'nan', 'infinite'], 'unknown modification times always appear last')
assert.deepEqual(ids(sortLocalTracks(withMissingTimes, 'modified', true)),
  ['new', 'old', 'missing', 'zero', 'negative', 'nan', 'infinite'], 'descending order keeps missing times last')

const equalKeys = ['first', 'second', 'third'].map(id => track(id, {
  title: 'Same', artist: 'Same', album: 'Same', durationMs: 1000, fileModifiedMs: 100,
}))
for (const sortBy of ['title', 'artist', 'album', 'duration', 'modified']) {
  for (const descending of [false, true]) {
    assert.deepEqual(ids(sortLocalTracks(equalKeys, sortBy, descending)), ['first', 'second', 'third'],
      `${sortBy} preserves the source order of equal keys`)
  }
}
assert.deepEqual(sortLocalTracks([], 'modified'), [])
console.log('local track sorting policy tests passed')
