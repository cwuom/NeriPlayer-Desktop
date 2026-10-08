import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/library.ts', import.meta.url), 'utf8')
let sequence = 0
async function load(dependencies) {
  const key = `__localScanTest${++sequence}`
  globalThis[key] = dependencies
  let compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `missing dependency ${name}`)
    const names = node.importClause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text)
    const assignment = `const { ${names.join(', ')} } = deps[${JSON.stringify(name)}]`
    compiled = compiled.slice(0, node.getStart(parsed)) + assignment + compiled.slice(node.end)
  }
  try { return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`) }
  finally { delete globalThis[key] }
}
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
async function flush() { for (let i = 0; i < 6; i++) await Promise.resolve() }
const track = { id: 'local:C:/Music/a.wav', title: 'A', artist: 'Artist', album: 'Album', duration_ms: 1000, url: 'C:/Music/a.wav', cover_url: 'C:/Music/a.png', source: 'local', sync_payload: { sourceStableKey: 'online:1' } }
async function runtime(options = {}) {
  const storage = new Map()
  globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) }
  const events = new Map(), calls = []
  const module = await load({
    pinia, vue,
    '@tauri-apps/api/core': { convertFileSrc: path => `asset:${path}`, invoke: async (command, args) => {
      calls.push({ command, args })
      if (command === 'scan_local_files') return options.scan ? options.scan.promise : { tracks: [track], skipped: [] }
      if (command === 'get_local_playlist_tracks') return [track]
      if (command === 'cancel_local_scan') return true
      if (command === 'edit_local_file_tags' && options.editFailure) throw new Error('fixture write failure')
    } },
    '@tauri-apps/api/event': { listen: async (name, callback) => {
      if (options.listening) await options.listening.promise
      events.set(name, callback)
      return () => events.delete(name)
    } },
    './settings': { useSettingsStore: () => ({ downloadNameTemplate: '{title}' }) },
    './player': { usePlayerStore: () => ({ withReleasedAudioFile: async (path, operation) => {
      calls.push({ command: 'releaseAudioFile', args: { path } })
      if (options.release) await options.release.promise
      return operation()
    } }) },
    '@/utils/logger': { createLogger: () => ({ error() {} }) },
  })
  pinia.setActivePinia(pinia.createPinia())
  return { store: module.useLibraryStore(), calls, events, storage }
}

{
  const r = await runtime()
  await r.store.scanDirectory('C:/Music')
  assert.equal(r.store.tracks[0].coverUrl, 'asset:C:/Music/a.png')
  assert.deepEqual(r.store.tracks[0].syncPayload, { sourceStableKey: 'online:1' })
  assert.equal(r.store.lastScanDir, 'C:/Music')
  assert.equal(r.storage.get('neri:last_scan_dir'), 'C:/Music')
  assert.equal(r.store.isScanning, false)
  assert.equal(r.events.size, 0)
  assert.ok(r.calls.every(item => !/create_playlist|add.*playlist|remove.*playlist/.test(item.command)), 'scanning must never modify playlists')
}
{
  const scan = deferred(), r = await runtime({ scan })
  const running = r.store.scanDirectory('C:/Music')
  await flush()
  const sessionId = r.calls.find(item => item.command === 'scan_local_files').args.sessionId
  const emit = payload => r.events.get('local-scan-progress')({ payload })
  emit({ sessionId: 'stale', visitedEntries: 999, tracks: 999 })
  assert.equal(r.store.scanProgress.tracks, 0)
  emit({ sessionId, visitedEntries: 100, tracks: 12, skipped: 1, currentPath: 'C:/Music/a.wav' })
  assert.equal(r.store.scanProgress.tracks, 12)
  await r.store.cancelScan()
  scan.reject(new Error('Scan cancelled'))
  await running
  assert.equal(r.store.scanCancelled, true)
  assert.equal(r.store.scanError, null)
  assert.equal(r.store.tracks.length, 0)
  assert.equal(r.events.size, 0)
}
{
  const listening = deferred(), r = await runtime({ listening })
  const running = r.store.scanDirectory('C:/Music')
  await r.store.cancelScan()
  listening.resolve()
  await running
  assert.equal(r.store.scanCancelled, true)
  assert.equal(r.calls.some(item => item.command === 'scan_local_files'), false)
  assert.equal(r.events.size, 0)
}
{
  const r = await runtime({ editFailure: true })
  await r.store.scanDirectory('C:/Music')
  await assert.rejects(r.store.saveTrackTags(r.store.tracks[0], { title: 'New', artist: 'Artist', album: 'Album' }), /fixture write failure/)
  assert.equal(r.store.tracks[0].title, 'A')
  assert.equal(r.store.isSavingTags, false)
}
{
  const r = await runtime()
  await r.store.scanDirectory('C:/Music')
  await r.store.saveTrackTags(r.store.tracks[0], { title: 'New', artist: 'Artist', album: 'Album' })
  assert.equal(r.store.tracks[0].title, 'New')
  assert.equal(r.calls.find(item => item.command === 'edit_local_file_tags').args.scanRoot, 'C:/Music')
  await assert.rejects(r.store.saveTrackTags({ ...r.store.tracks[0], id: 'stale' }, { title: 'New', artist: 'Artist', album: 'Album' }), /no longer/)
}
{
  const release = deferred(), r = await runtime({ release })
  await r.store.scanDirectory('C:/Music')
  const writing = r.store.saveTrackTags(r.store.tracks[0], { title: 'New', artist: 'Artist', album: 'Album' })
  await flush()
  assert.equal(r.calls.some(call => call.command === 'edit_local_file_tags'), false, 'tag writing must wait for decoder release')
  release.resolve(); await writing
  assert.deepEqual(r.calls.filter(call => ['releaseAudioFile', 'edit_local_file_tags'].includes(call.command)).map(call => call.command), ['releaseAudioFile', 'edit_local_file_tags'])
}
console.log('local scan store lifecycle tests passed')
