import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/localScanPreview.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { localReferenceKey, scanMetadataFingerprint, duplicateScanTrackIds, existingScanTrackIds, filterScanTracks } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

const song = (id, extra = {}) => ({
  id, title: 'Song', artist: 'Artist', album: 'Album', durationMs: 180000,
  audioUrl: `C:\\Music\\${id}.mp3`, source: 'local', ...extra,
})

assert.equal(localReferenceKey('\\\\?\\C:\\Music\\Song.mp3'), 'c:/music/song.mp3')
assert.equal(localReferenceKey('file:///C:/Music/Song%20Name.mp3'), 'c:/music/song name.mp3')
assert.equal(localReferenceKey('/Music/Song.mp3'), '/Music/Song.mp3')
assert.equal(localReferenceKey('https://example.com/audio.mp3'), null)
assert.equal(localReferenceKey(''), null)

const first = song('a')
const copy = song('b', { title: ' SONG ', artist: 'Artist\t', album: '  ALBUM ' })
assert.equal(scanMetadataFingerprint(first), scanMetadataFingerprint(copy))
assert.equal(scanMetadataFingerprint(song('missing', { album: '' })), null)
assert.equal(scanMetadataFingerprint(song('placeholder', { album: '__local_files__' })), null)
assert.deepEqual([...duplicateScanTrackIds([first, copy, song('longer', { durationMs: 180001 })])], ['b'])

assert.deepEqual([...existingScanTrackIds([first, copy], [song('renamed', { audioUrl: 'c:/music/A.mp3' })])], ['a'])
assert.deepEqual([...existingScanTrackIds([first], [song('elsewhere', { audioUrl: 'C:/Other/a.mp3' })])], [])
assert.deepEqual([...existingScanTrackIds([first], [song('remote', { source: 'netease', audioUrl: 'https://example.com/audio.mp3' })])], [])
assert.deepEqual([...existingScanTrackIds([first], [song('downloaded', { source: 'netease', audioUrl: 'C:/Music/a.mp3' })])], ['a'])

const tracks = [first, copy, song('missing', { artist: 'Unknown Artist', album: '' })]
const options = { query: '', metadataOnly: false, hideExisting: false, hideDuplicates: false, existingIds: new Set(['a']), duplicateIds: new Set(['b']) }
assert.deepEqual(filterScanTracks(tracks, { ...options, hideExisting: true }).map(t => t.id), ['b', 'missing'])
assert.deepEqual(filterScanTracks(tracks, { ...options, hideDuplicates: true }).map(t => t.id), ['a', 'missing'])
assert.deepEqual(filterScanTracks(tracks, { ...options, metadataOnly: true }).map(t => t.id), ['a', 'b'])
assert.deepEqual(filterScanTracks(tracks, { ...options, query: 'c:/music/a' }).map(t => t.id), ['a'])
assert.deepEqual(filterScanTracks(tracks, { ...options, query: 'song album' }).map(t => t.id), ['a', 'b'])
console.log('local scan preview policy tests passed')
