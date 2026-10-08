import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

let sequence = 0
async function load(source, dependencies = {}) {
  const key = `__downloadTest${++sequence}`
  globalThis[key] = dependencies
  let compiled = ts.transpileModule(source.replaceAll('import.meta.hot', 'undefined'), {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `missing dependency ${name}`)
    const clause = node.importClause
    const assignments = []
    if (clause?.name) assignments.push(`const ${clause.name.text} = deps[${JSON.stringify(name)}].default`)
    if (clause?.namedBindings && ts.isNamedImports(clause.namedBindings)) {
      const names = clause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text)
      assignments.push(`const { ${names.join(', ')} } = deps[${JSON.stringify(name)}]`)
    }
    compiled = compiled.slice(0, node.getStart(parsed)) + assignments.join(';\n') + compiled.slice(node.end)
  }
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`)
  } finally { delete globalThis[key] }
}
const storage = new Map()
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, String(value)),
  removeItem: key => storage.delete(key),
}
const root = new URL('../src/', import.meta.url)
const source = await readFile(new URL('stores/download.ts', root), 'utf8')
const queue = await load(await readFile(new URL('modules/download/downloadQueue.ts', root), 'utf8'))
const cancellation = await load(await readFile(new URL('modules/download/downloadCancellation.ts', root), 'utf8'))
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
async function flush() { for (let i = 0; i < 15; i++) { await Promise.resolve(); await vue.nextTick() } }
const track = id => ({ id: `netease:${id}`, title: id, artist: 'Artist', album: 'Album', durationMs: 10000, coverUrl: `https://fixture.test/${id}.jpg` })
const manifestTrack = (id, filePath) => ({
  id: `netease:${id}`, title: id, artist: 'Artist', album: 'Album', duration_ms: 10000,
  source: 'netease', file_path: filePath, file_size: 1024, downloaded_at: 1,
})
const stream = { url: 'https://fixture.test/audio.mp3', durationMs: 10000 }

async function runtime(options = {}) {
  const events = new Map()
  const invoked = []
  const resolved = []
  const messages = []
  const settings = vue.reactive({
    downloadParallelism: 1, downloadFollowPlaybackQuality: false,
    downloadNeteaseQuality: 'exhigh', downloadQqMusicQuality: 'high',
    downloadBiliQuality: 'high', downloadYoutubeQuality: 'high',
    downloadDir: '', downloadNameTemplate: '',
  })
  const loaded = await load(source, {
    pinia, vue,
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      invoked.push({ command, args })
      if (command === 'validate_downloads') return options.validate ? options.validate() : { tracks: options.downloads || [] }
      if (command === 'delete_download' && options.deleteError) throw new Error('fixture delete failed')
      if (command === 'cancel_download') return options.cancel ? options.cancel.promise : false
      if (command === 'cancel_all_downloads') return 0
      if (command === 'get_default_download_dir') return 'E:/Music/NeriPlayer'
      if (command === 'download_track') {
        events.get('download-progress')?.({ payload: { trackId: args.trackId, status: 'start' } })
        if (options.start) return options.start(args)
      }
    } },
    '@tauri-apps/api/event': { listen: async (name, callback) => {
      if (name === 'download-progress' && options.listening) await options.listening.promise
      events.set(name, callback)
      return () => events.delete(name)
    } },
    './settings': { useSettingsStore: () => settings },
    './player': { usePlayerStore: () => ({
      withReleasedAudioFile: async (path, operation) => {
        invoked.push({ command: 'releaseAudioFile', args: { path } })
        if (options.release) await options.release.promise
        return operation()
      },
      handleDownloadedFileRemoved: (trackId, path) => invoked.push({ command: 'fileRemoved', args: { trackId, path } }),
    }) },
    './toast': { useToastStore: () => Object.fromEntries(['show', 'error', 'success'].map(method => [method, (...args) => messages.push({ method, args })])) },
    '@tauri-apps/plugin-opener': { openPath: async path => invoked.push({ command: 'openPath', args: { path } }) },
    '@/i18n': { default: { global: { t: (key, params) => params ? `${key}:${JSON.stringify(params)}` : key } } },
    '@/utils/logger': { createLogger: () => ({ error() {}, warn() {} }) },
    '@/modules/playback/playbackSource': { resolveDownloadSource: async (item, quality) => {
      resolved.push({ item, quality })
      return options.resolve ? options.resolve(item) : stream
    } },
    '@/modules/download/downloadCancellation': cancellation,
    '@/modules/download/downloadQueue': queue,
  })
  pinia.setActivePinia(pinia.createPinia())
  const store = loaded.useDownloadStore()
  return {
    store, settings, invoked, resolved, messages,
    emit: payload => events.get('download-progress')({ payload }),
    emitEvent: (name, payload) => events.get(name)?.({ payload }),
  }
}

{
  const r = await runtime()
  await r.store.downloadTrack(track('silent-a'))
  await r.store.downloadTrack(track('silent-b'))
  await flush()
  assert.equal(r.messages.length, 0, 'enqueue and start stay silent')
  assert.equal(r.store.downloading.get('netease:silent-a').coverUrl, track('silent-a').coverUrl)
  assert.equal(r.store.downloading.get('netease:silent-b').coverUrl, track('silent-b').coverUrl)
  r.emit({ trackId: 'netease:silent-a', status: 'downloading', downloadedBytes: 100, totalBytes: 100 })
  r.emit({ trackId: 'netease:silent-a', status: 'processing' })
  assert.equal(r.store.downloading.get('netease:silent-a').coverUrl, track('silent-a').coverUrl)
  await flush()
  assert.equal(r.resolved.length, 1, 'processing retains its queue slot')
  assert.equal(r.messages.length, 0)
  r.emit({ trackId: 'netease:silent-a', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 0, 'per-track completion stays silent')
  await r.store.downloadTrack(track('silent-c'))
  r.emit({ trackId: 'netease:silent-b', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 0, 'appended tasks stay in the same batch')
  r.emit({ trackId: 'netease:silent-c', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /"completed":3/)
  r.emit({ trackId: 'netease:silent-c', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1, 'duplicate terminal events do not notify again')
  await r.store.downloadTrack(track('next-batch'))
  await flush()
  r.emit({ trackId: 'netease:next-batch', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 2)
  assert.match(r.messages[1].args[0], /"completed":1/)
}
{
  const r = await runtime()
  for (const id of ['success', 'failure', 'cancelled']) await r.store.downloadTrack(track(id))
  await flush()
  r.emit({ trackId: 'netease:success', status: 'complete' })
  await flush()
  r.emit({ trackId: 'netease:failure', status: 'error', message: 'fixture inline failure' })
  await flush()
  assert.equal(r.messages.length, 0)
  assert.equal(r.store.downloading.get('netease:failure').message, 'fixture inline failure')
  assert.equal(r.store.downloading.get('netease:failure').coverUrl, track('failure').coverUrl)
  r.emit({ trackId: 'netease:cancelled', status: 'cancelled' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.equal(r.messages[0].method, 'show')
  assert.equal(r.messages[0].args[1], 'info', 'mixed outcomes are not presented as total success')
  assert.match(r.messages[0].args[0], /"completed":1/)
  assert.match(r.messages[0].args[0], /"failed":1/)
  assert.match(r.messages[0].args[0], /"cancelled":1/)
  r.store.retryDownload('netease:failure')
  await flush()
  r.emit({ trackId: 'netease:failure', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 2)
  assert.match(r.messages[1].args[0], /"failed":0/)
}
{
  const resolution = deferred()
  const r = await runtime({ resolve: item => item.id.endsWith(':late-cancel') ? resolution.promise : stream })
  r.settings.downloadParallelism = 2
  await r.store.downloadTrack(track('success'))
  await r.store.downloadTrack(track('late-cancel'))
  await r.store.downloadTrack(track('pending-cancel'))
  await flush()
  r.emit({ trackId: 'netease:success', status: 'complete' })
  await flush()
  await r.store.cancelAllDownloads()
  assert.equal(r.messages.length, 0, 'cancel all is silent and unresolved cancellation blocks summary')
  resolution.resolve(stream)
  await flush()
  r.emit({ trackId: 'netease:pending-cancel', status: 'cancelled' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /"cancelled":2/)
}
{
  const validation = deferred()
  let calls = 0
  const r = await runtime({ validate: () => ++calls === 1 ? validation.promise : { tracks: [] } })
  await r.store.downloadTrack(track('async-first'))
  await flush()
  r.emit({ trackId: 'netease:async-first', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 0)
  await r.store.downloadTrack(track('async-appended'))
  await flush()
  validation.resolve({ tracks: [] })
  await flush()
  assert.equal(r.messages.length, 0, 'appending during refresh postpones summary')
  r.emit({ trackId: 'netease:async-appended', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /"completed":2/)
}
{
  const r = await runtime()
  for (const id of ['retry-a', 'retry-b']) await r.store.downloadTrack(track(id))
  await flush()
  r.emit({ trackId: 'netease:retry-a', status: 'error', message: 'retry me' })
  r.store.retryDownload('netease:retry-a')
  await flush()
  r.emit({ trackId: 'netease:retry-b', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 0)
  r.emit({ trackId: 'netease:retry-a', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /"completed":2/)
  assert.match(r.messages[0].args[0], /"failed":0/, 'retry replaces the previous result for this track')
}
for (const oldFails of [false, true]) {
  const oldResolution = deferred()
  let attempts = 0
  const r = await runtime({ resolve: () => ++attempts === 1 ? oldResolution.promise : stream })
  await r.store.downloadTrack(track('generation'))
  await flush()
  await r.store.cancelDownload('netease:generation')
  r.store.retryDownload('netease:generation')
  await flush()
  assert.equal(r.store.downloading.get('netease:generation').status, 'queued', 'immediate retry waits for cancelled resolver to settle')
  assert.equal(r.resolved.length, 1)
  if (oldFails) oldResolution.reject(new Error('old resolver failed'))
  else oldResolution.resolve(stream)
  await flush()
  assert.equal(r.store.downloading.get('netease:generation').status, 'downloading')
  assert.equal(r.resolved.length, 2)
  assert.equal(r.invoked.filter(item => item.command === 'download_track').length, 1)
  assert.equal(r.messages.length, 0)
  r.emit({ trackId: 'netease:generation', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /"completed":1/)
  assert.match(r.messages[0].args[0], /"cancelled":0/)
}
{
  const startup = deferred()
  const r = await runtime({ start: () => startup.promise })
  await r.store.downloadTrack(track('startup'))
  await flush()
  r.emit({ trackId: 'netease:startup', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 0, 'complete event waits for startup IPC to settle')
  startup.resolve()
  await flush()
  assert.equal(r.messages.length, 1)
}
{
  const oldResolution = deferred()
  let resolutions = 0
  const r = await runtime({ resolve: item => item.id.endsWith(':parallel-retry') && ++resolutions === 1 ? oldResolution.promise : stream })
  r.settings.downloadParallelism = 2
  await r.store.downloadTrack(track('parallel-retry'))
  await flush()
  await r.store.cancelDownload('netease:parallel-retry')
  r.store.retryDownload('netease:parallel-retry')
  await flush()
  assert.equal(r.store.downloading.get('netease:parallel-retry').status, 'downloading')
  r.settings.downloadParallelism = 1
  await vue.nextTick()
  await r.store.downloadTrack(track('slot-check'))
  oldResolution.resolve(stream)
  await flush()
  assert.equal(r.store.downloading.get('netease:parallel-retry').status, 'downloading', 'old resolver cannot modify the live retry')
  assert.equal(r.store.downloading.get('netease:slot-check').status, 'queued', 'old resolver cannot release the retry slot')
  r.emit({ trackId: 'netease:parallel-retry', status: 'complete' })
  await flush()
  r.emit({ trackId: 'netease:slot-check', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /"completed":2/)
  assert.match(r.messages[0].args[0], /"cancelled":0/)
}
{
  const saved = { id: 'netease:folder', title: 'Folder', artist: 'Artist', album: 'Album', duration_ms: 10000, source: 'netease', file_path: 'E:/Music/folder.mp3', file_size: 1024, downloaded_at: 1 }
  const r = await runtime({ downloads: [saved] })
  await r.store.downloadTrack(track('folder'))
  await flush()
  r.emit({ trackId: saved.id, status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  const options = r.messages[0].args.at(-1)
  await options.action.handler()
  assert.deepEqual(r.invoked.at(-1), { command: 'reveal_file', args: { path: saved.file_path } })
  await r.store.downloadTrack(track('folder'))
  assert.equal(r.messages.length, 1, 'already downloaded track stays silent')
  assert.equal(r.store.downloading.get(saved.id).status, 'already_exists')
  await r.store.deleteDownload(saved.id)
  assert.equal(r.messages.length, 2, 'explicit delete retains its result toast')
}
{
  const r = await runtime({ validate: () => ({ tracks: [], removed_count: 1, integrity_mismatch_count: 1 }) })
  for (const id of ['failed-only', 'existing-only', 'cancelled-only']) await r.store.downloadTrack(track(id))
  await flush()
  r.emitEvent('download-dir-fallback', { requestedDir: 'unavailable' })
  r.emitEvent('downloads-changed')
  r.emit({ trackId: 'netease:failed-only', status: 'error', message: 'fixture failure' })
  await flush()
  r.emit({ trackId: 'netease:existing-only', status: 'already_exists' })
  await flush()
  r.emit({ trackId: 'netease:cancelled-only', status: 'cancelled' })
  await flush()
  assert.equal(r.messages.length, 0, 'failure, cancellation, existing file, fallback and automatic validation stay silent')
  assert.equal(r.store.downloading.get('netease:failed-only').status, 'error')
  assert.equal(r.store.downloading.get('netease:cancelled-only').status, 'cancelled')
}
{
  const r = await runtime()
  r.settings.downloadDir = 'E:/CustomDownload'
  await r.store.downloadTrack(track('default-folder'))
  await flush()
  r.emitEvent('download-dir-fallback', { requestedDir: r.settings.downloadDir })
  r.emit({ trackId: 'netease:default-folder', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  assert.match(r.messages[0].args[0], /download.dir_fallback/)
  await r.messages[0].args.at(-1).action.handler()
  assert.deepEqual(r.invoked.at(-1), { command: 'openPath', args: { path: 'E:/Music/NeriPlayer' } })
}
{
  const lateValidation = deferred()
  let validations = 0
  const r = await runtime({ validate: () => ++validations === 1 ? lateValidation.promise : { tracks: [] } })
  await r.store.downloadTrack(track('late-validation'))
  await flush()
  const loading = r.store.loadDownloads()
  r.emit({ trackId: 'netease:late-validation', status: 'complete' })
  await flush()
  assert.equal(r.messages.length, 1)
  lateValidation.resolve({ tracks: [], removed_count: 1, integrity_mismatch_count: 1 })
  await loading
  await flush()
  assert.equal(r.messages.length, 1, 'validation started during a download stays silent after the batch drains')
}

{
  const older = deferred(), newer = deferred()
  let validations = 0
  const r = await runtime({ validate: () => ++validations === 1 ? older.promise : newer.promise })
  const first = r.store.loadDownloads()
  const second = r.store.loadDownloads()
  const latest = manifestTrack('latest', 'E:/Music/latest.flac')
  newer.resolve({ tracks: [latest] })
  await second
  assert.deepEqual(r.store.downloads.map(item => [item.id, item.filePath]), [[latest.id, latest.file_path]])
  older.resolve({ tracks: [manifestTrack('stale', 'E:/Music/stale.mp3')], removed_count: 1, integrity_mismatch_count: 1 })
  await first
  await flush()
  assert.deepEqual(r.store.downloads.map(item => [item.id, item.filePath]), [[latest.id, latest.file_path]], 'older validation cannot overwrite the latest manifest')
  assert.equal(r.messages.length, 0, 'obsolete validation warnings stay silent')
}
for (const newerFirst of [false, true]) {
  const summaryRefresh = deferred(), eventRefresh = deferred()
  let validations = 0
  const r = await runtime({ validate: () => ++validations === 1 ? summaryRefresh.promise : eventRefresh.promise })
  await r.store.downloadTrack(track('last-file'))
  await flush()
  r.emit({ trackId: 'netease:last-file', status: 'complete' })
  await flush()
  assert.equal(validations, 2, 'completion event and summary refresh can overlap')
  const latest = manifestTrack('last-file', 'E:/Music/latest-last-file.flac')
  if (newerFirst) {
    eventRefresh.resolve({ tracks: [latest] })
    await flush()
    assert.deepEqual(r.store.downloads.map(item => item.filePath), [latest.file_path])
  } else {
    summaryRefresh.resolve({ tracks: [manifestTrack('last-file', 'E:/Music/obsolete-last-file.mp3')], removed_count: 1 })
    await flush()
    assert.equal(r.store.downloads.length, 0, 'superseded summary snapshot is never committed')
  }
  assert.equal(r.messages.length, 0, 'summary waits for the latest refresh to settle')
  if (newerFirst) summaryRefresh.resolve({ tracks: [manifestTrack('stale', 'E:/Music/stale.mp3')], removed_count: 1 })
  else eventRefresh.resolve({ tracks: [latest], removed_count: 1, integrity_mismatch_count: 1 })
  await flush()
  assert.deepEqual(r.store.downloads.map(item => [item.id, item.filePath]), [[latest.id, latest.file_path]])
  assert.equal(r.messages.length, 1, 'automatic refreshes produce only the final batch summary')
  await r.messages[0].args.at(-1).action.handler()
  assert.deepEqual(r.invoked.at(-1), { command: 'reveal_file', args: { path: latest.file_path } })
}

{
  const gate = deferred()
  const r = await runtime({ listening: gate })
  await r.store.downloadTrack(track('a'))
  await r.store.downloadTrack(track('b'))
  assert.equal(r.store.downloading.get('netease:b').status, 'queued')
  assert.equal(r.resolved.length, 0)
  await r.store.cancelDownload('netease:a')
  gate.resolve()
  await flush()
  assert.deepEqual(r.resolved.map(item => item.item.id), ['netease:b'])
  assert.equal(r.store.downloading.get('netease:a').status, 'cancelled')
  assert.equal(r.invoked.filter(item => item.command === 'download_track').length, 1)
}
{
  const r = await runtime()
  for (const id of ['a', 'b', 'c']) await r.store.downloadTrack(track(id))
  await flush()
  assert.equal(r.resolved.length, 1)
  assert.equal(r.resolved[0].quality.neteaseQuality, 'exhigh')
  r.emit({ trackId: 'netease:a', status: 'processing' })
  assert.equal(r.store.downloading.get('netease:a').status, 'processing')
  r.settings.downloadParallelism = 2
  await flush()
  assert.equal(r.resolved.length, 2)
  r.settings.downloadParallelism = 1
  await flush()
  r.emit({ trackId: 'netease:a', status: 'complete' })
  await flush()
  assert.equal(r.resolved.length, 2)
  r.emit({ trackId: 'netease:b', status: 'complete' })
  await flush()
  assert.equal(r.resolved.length, 3)
  r.emit({ trackId: 'netease:c', status: 'error', message: 'fixture failure' })
  assert.equal(r.store.isDownloading('netease:c'), false)
  assert.equal(r.store.downloading.get('netease:c').message, 'fixture failure')
  r.store.retryDownload('netease:c')
  await flush()
  assert.equal(r.resolved.length, 4)
  assert.equal(r.store.isDownloading('netease:c'), true)
  r.emit({ trackId: 'netease:c', status: 'cancelled' })
  r.store.clearFinishedTasks()
  assert.equal(r.store.downloading.size, 0)
}
{
  const resolution = deferred()
  const r = await runtime({ resolve: () => resolution.promise })
  await r.store.downloadTrack(track('a'))
  await r.store.downloadTrack(track('b'))
  await flush()
  await r.store.cancelAllDownloads()
  resolution.reject(new Error('fixture resolver failed after cancellation'))
  await flush()
  assert.equal(r.invoked.filter(item => item.command === 'download_track').length, 0)
  assert.equal(r.resolved.length, 1)
  assert.equal(r.store.downloading.get('netease:a').status, 'cancelled')
  assert.equal(r.store.downloading.get('netease:b').status, 'cancelled')
  assert.equal(r.messages.filter(item => item.method === 'error').length, 0)
}
{
  const resolution = deferred(), cancel = deferred()
  const r = await runtime({ resolve: () => resolution.promise, cancel })
  await r.store.downloadTrack(track('a'))
  await flush()
  const cancelling = r.store.cancelDownload('netease:a')
  resolution.resolve(stream)
  await flush()
  cancel.resolve(false)
  assert.equal(await cancelling, true)
  assert.equal(r.invoked.filter(item => item.command === 'download_track').length, 0)
  assert.equal(r.store.downloading.get('netease:a').status, 'cancelled')
}
for (const failed of [false, true]) {
  const resolution = deferred()
  const r = await runtime({ resolve: () => resolution.promise })
  await r.store.downloadTrack(track('a'))
  await flush()
  await r.store.cancelDownload('netease:a')
  r.store.clearFinishedTasks()
  assert.equal(r.store.downloading.size, 0)
  if (failed) resolution.reject(new Error('fixture failure'))
  else resolution.resolve(stream)
  await flush()
  assert.equal(r.store.downloading.size, 0)
}
for (const retained of [false, true]) {
  const saved = { id: 'netease:a', title: 'A', artist: 'Artist', album: 'Album', duration_ms: 10000, source: 'netease', file_path: 'E:/Music/a.mp3', file_size: 1024, downloaded_at: 1 }
  const r = await runtime({ deleteError: true, downloads: retained ? [saved] : [] })
  r.store.downloads = [{ id: saved.id, title: saved.title, artist: saved.artist, album: saved.album, durationMs: saved.duration_ms, source: saved.source, filePath: saved.file_path, fileSize: saved.file_size, downloadedAt: saved.downloaded_at }]
  await assert.rejects(r.store.deleteDownload(saved.id), /fixture delete failed/)
  assert.equal(r.store.downloads.length, retained ? 1 : 0, 'failed deletion refreshes the actual manifest state')
}
{
  const release = deferred()
  const saved = manifestTrack('playing', 'E:/Music/playing.flac')
  const r = await runtime({ release, downloads: [saved] })
  await r.store.loadDownloads()
  const removing = r.store.deleteDownload(saved.id, { silent: true })
  await flush()
  assert.equal(r.invoked.some(call => call.command === 'delete_download'), false, 'delete must wait for file release')
  assert.equal(r.store.downloads.length, 1)
  release.resolve(); await removing
  assert.deepEqual(r.invoked.filter(call => ['releaseAudioFile', 'delete_download', 'fileRemoved'].includes(call.command)).map(call => call.command), ['releaseAudioFile', 'delete_download', 'fileRemoved'])
  assert.equal(r.store.downloads.length, 0)
}
{
  storage.clear()
  const r = await runtime()
  await r.store.downloadTrack(track('pending-a'))
  await r.store.downloadTrack(track('pending-b'))
  await r.store.downloadTrack(track('pending-c'))
  await flush()
  const pendingIds = () => JSON.parse(storage.get('neri:pending-downloads') ?? '[]').map(item => item.id)
  assert.deepEqual(pendingIds(), ['netease:pending-a', 'netease:pending-b', 'netease:pending-c'])
  r.emit({ trackId: 'netease:pending-a', status: 'complete' })
  await flush()
  await r.store.cancelDownload('netease:pending-c')
  await flush()
  assert.deepEqual(pendingIds(), ['netease:pending-b'], 'finished and cancelled downloads leave the list')

  const restarted = await runtime({ downloads: [manifestTrack('pending-a', 'E:/Music/pending-a.mp3')] })
  assert.equal(await restarted.store.resumePendingDownloads(), 1)
  await flush()
  assert.ok(restarted.store.isDownloading('netease:pending-b'), 'the unfinished download is queued again')
  assert.equal(restarted.resolved.length, 1, 'its address is resolved again with the current settings')
  assert.match(restarted.messages.at(-1).args[0], /download\.resumed_pending:\{"count":1\}/)

  storage.clear()
  const clean = await runtime()
  assert.equal(await clean.store.resumePendingDownloads(), 0)
  assert.equal(clean.messages.length, 0)
}
console.log('download store lifecycle tests passed')
