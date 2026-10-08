import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/playback/localAudioInfo.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText.replace(/import \{ invoke \} from ['"]@tauri-apps\/api\/core['"];?/, 'const invoke = globalThis.__localAudioInfoInvoke;')
const calls = []
let respond
globalThis.__localAudioInfoInvoke = (command, args) => {
  calls.push({ command, args })
  return respond()
}
const { loadLocalAudioInfo } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
delete globalThis.__localAudioInfoInvoke

const updates = []
const properties = { codec: 'FLAC', format: 'flac', bitrate: 932, sampleRateHz: 48000, bitDepth: 24, channelCount: 2 }
respond = async () => properties
await loadLocalAudioInfo('E:\\Music\\song.FLAC', () => true, info => updates.push(info))
assert.deepEqual(calls[0], { command: 'get_local_audio_info', args: { path: 'E:\\Music\\song.FLAC' } })
assert.deepEqual(updates, [{ source: 'local', format: 'flac' }, { source: 'local', ...properties }])
assert.equal(updates[1].qualityKey, undefined, 'online quality preferences are not file properties')

const count = calls.length
await loadLocalAudioInfo('/Music/old.mp3', () => false, () => assert.fail('stale request updated the state'))
assert.equal(calls.length, count)

let resolve
let current = true
respond = () => new Promise(yes => { resolve = yes })
updates.length = 0
const pending = loadLocalAudioInfo('/Music/old.mp3', () => current, info => updates.push(info))
current = false
resolve(properties)
await pending
assert.deepEqual(updates, [{ source: 'local', format: 'mp3' }], 'late file info cannot overwrite a newly selected track')

updates.length = 0
respond = async () => ({})
await loadLocalAudioInfo('/Music/no-extension', () => true, info => updates.push(info))
assert.deepEqual(updates.at(-1), { source: 'local', format: undefined })
await loadLocalAudioInfo('/Music/fallback.ogg', () => true, info => updates.push(info))
assert.deepEqual(updates.at(-1), { source: 'local', format: 'ogg' })

updates.length = 0
respond = async () => { throw new Error('unreadable file') }
await assert.rejects(loadLocalAudioInfo('/Music/file.mp3', () => true, info => updates.push(info)), /unreadable file/)
assert.deepEqual(updates, [{ source: 'local', format: 'mp3' }], 'failed probe keeps format without inventing bitrate')
console.log('local audio info loading tests passed')
