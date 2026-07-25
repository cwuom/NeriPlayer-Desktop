import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/neteaseLibraryRequest.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { NeteaseLibraryRequestCoordinator } = await import(moduleUrl)

const deferred = () => {
  let resolve
  let reject
  const promise = new Promise((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

{
  const coordinator = new NeteaseLibraryRequestCoordinator()
  let playlistCalls = 0
  let albumCalls = 0
  const playlists = deferred()
  const albums = deferred()
  const loadPlaylists = () => {
    playlistCalls += 1
    return playlists.promise
  }
  const loadAlbums = () => {
    albumCalls += 1
    return albums.promise
  }

  const first = coordinator.run(loadPlaylists, loadAlbums)
  const second = coordinator.run(loadPlaylists, loadAlbums)
  assert.equal(first.started, true)
  assert.equal(second.started, false)
  assert.equal(first.promise, second.promise)
  await Promise.resolve()
  assert.equal(playlistCalls, 1)
  assert.equal(albumCalls, 1)

  playlists.resolve(true)
  albums.reject(new Error('albums unavailable'))
  assert.deepEqual(await first.promise, { current: true, playlistsOk: true, albumsOk: false })
}

{
  const coordinator = new NeteaseLibraryRequestCoordinator()
  const playlists = deferred()
  const albums = deferred()
  const request = coordinator.run(() => playlists.promise, () => albums.promise)

  playlists.resolve(false)
  albums.resolve(true)
  assert.deepEqual(await request.promise, { current: true, playlistsOk: false, albumsOk: true })
}

{
  const coordinator = new NeteaseLibraryRequestCoordinator()
  const request = coordinator.run(
    () => { throw new Error('playlist loader failed synchronously') },
    () => Promise.resolve(true),
  )

  assert.deepEqual(await request.promise, { current: true, playlistsOk: false, albumsOk: true })
}

{
  const coordinator = new NeteaseLibraryRequestCoordinator()
  let nested
  let nestedPlaylistCalls = 0
  let nestedAlbumCalls = 0
  const outer = coordinator.run(
    () => {
      nested = coordinator.run(
        () => {
          nestedPlaylistCalls += 1
          return Promise.resolve(true)
        },
        () => {
          nestedAlbumCalls += 1
          return Promise.resolve(true)
        },
      )
      return Promise.resolve(true)
    },
    () => Promise.resolve(true),
  )

  await Promise.resolve()
  assert.ok(nested)
  assert.equal(nested.started, false)
  assert.equal(nested.promise, outer.promise)
  assert.equal(nestedPlaylistCalls, 0)
  assert.equal(nestedAlbumCalls, 0)
  assert.deepEqual(await outer.promise, { current: true, playlistsOk: true, albumsOk: true })
}

{
  const coordinator = new NeteaseLibraryRequestCoordinator()
  const oldPlaylists = deferred()
  const oldAlbums = deferred()
  const oldRequest = coordinator.run(() => oldPlaylists.promise, () => oldAlbums.promise)

  coordinator.invalidate()

  const freshPlaylists = deferred()
  const freshAlbums = deferred()
  const freshRequest = coordinator.run(() => freshPlaylists.promise, () => freshAlbums.promise)
  assert.equal(freshRequest.started, true)

  oldPlaylists.resolve(true)
  oldAlbums.resolve(true)
  assert.deepEqual(await oldRequest.promise, { current: false, playlistsOk: true, albumsOk: true })

  const coalescedFreshRequest = coordinator.run(() => Promise.resolve(true), () => Promise.resolve(true))
  assert.equal(coalescedFreshRequest.started, false)
  assert.equal(coalescedFreshRequest.promise, freshRequest.promise)

  freshPlaylists.resolve(true)
  freshAlbums.resolve(true)
  assert.deepEqual(await freshRequest.promise, { current: true, playlistsOk: true, albumsOk: true })
}

console.log('library netease request tests passed')
