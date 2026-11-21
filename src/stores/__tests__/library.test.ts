import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TrackInfo } from '@/types/track'
import { isInsideFolder, useLibraryStore } from '@/stores/library'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

type EventHandler = (event: { payload: unknown }) => void

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listenError: null as Error | null,
  listenCalls: 0,
  settings: { downloadNameTemplate: '' },
  player: null as null | {
    currentTrack: { audioUrl: string } | null
    released: string[]
    updated: Array<Record<string, string>>
    withReleasedAudioFile: (path: string, action: () => Promise<void>) => Promise<void>
    updateCurrentTrackInfo: (tags: Record<string, string>) => void
  },
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
  convertFileSrc: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (event: string, handler: EventHandler) => {
    mocks.listenCalls++
    if (mocks.listenError) throw mocks.listenError
    mocks.listeners.set(event, handler)
    return () => { mocks.listeners.delete(event) }
  },
}))
vi.mock('@/modules/shortcuts/platform', () => ({ isMacPlatform: false }))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/settings', () => ({ useSettingsStore: () => mocks.settings }))
vi.mock('@/stores/player', () => ({ usePlayerStore: () => mocks.player }))

const LEGACY_SCAN_DIR_KEY = 'neri:last_scan_dir'

function backendTrack(path: string, overrides: Record<string, unknown> = {}) {
  return {
    id: `local:${path}`, title: `title ${path}`, artist: 'artist', album: 'album', duration_ms: 1234,
    cover_url: null, url: path, source: 'local', added_at: 10, file_modified_ms: 20, ...overrides,
  }
}

function snapshot(paths: string[], overrides: Record<string, unknown> = {}) {
  return {
    folders: [{ path: '/music', addedAt: 1, trackCount: paths.length, available: true }],
    tracks: paths.map(path => backendTrack(path)),
    scanning: false,
    skipped: [],
    ...overrides,
  }
}

describe('isInsideFolder', () => {
  it('matches POSIX paths by whole path segments only', () => {
    expect(isInsideFolder('/music/a.flac', '/music')).toBe(true)
    expect(isInsideFolder('/music/a.flac', '/music/')).toBe(true)
    expect(isInsideFolder('/music', '/music')).toBe(true)
    expect(isInsideFolder('/music2/a.flac', '/music')).toBe(false)
    expect(isInsideFolder('/Music/a.flac', '/music')).toBe(false)
    expect(isInsideFolder('/any/file.mp3', '/')).toBe(true)
  })

  it('treats Windows paths case-insensitively with either separator', () => {
    expect(isInsideFolder('d:/Music/Song.mp3', 'D:\\music')).toBe(true)
    expect(isInsideFolder('D:\\song.mp3', 'D:\\')).toBe(true)
    expect(isInsideFolder('D:\\MusicBox\\a.mp3', 'D:\\Music')).toBe(false)
    expect(isInsideFolder('\\\\nas\\share\\a.mp3', '\\\\nas\\share')).toBe(true)
  })
})

describe('library store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    mocks.listeners = new Map()
    mocks.listenError = null
    mocks.listenCalls = 0
    mocks.settings = { downloadNameTemplate: '' }
    mocks.player = {
      currentTrack: null,
      released: [],
      updated: [],
      async withReleasedAudioFile(path, action) {
        this.released.push(path)
        await action()
      },
      updateCurrentTrackInfo(tags) { this.updated.push(tags) },
    }
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  describe('refresh', () => {
    it('maps the backend snapshot and makes local cover paths displayable', async () => {
      mocks.settings.downloadNameTemplate = '%title%'
      router.on('local_library_snapshot', () => ({
        ...snapshot([]),
        tracks: [
          backendTrack('/music/a.flac', { cover_url: '/covers/a.jpg', sync_payload: { k: 1 } }),
          backendTrack('/music/b.flac', { cover_url: 'https://img/b.jpg' }),
          backendTrack('/music/c.flac', { cover_url: '' }),
        ],
        skipped: [{ path: '/music/bad.mp3', reason: 'corrupt' }],
      }))
      const library = useLibraryStore()

      await library.refresh()

      expect(router.callsOf('local_library_snapshot')[0].args).toEqual({ nameTemplate: '%title%' })
      expect(library.tracks[0]).toEqual<TrackInfo>({
        id: 'local:/music/a.flac', title: 'title /music/a.flac', artist: 'artist', album: 'album', durationMs: 1234,
        coverUrl: `asset://localhost/${encodeURIComponent('/covers/a.jpg')}`, audioUrl: '/music/a.flac', source: 'local',
        addedAt: 10, fileModifiedMs: 20, syncPayload: { k: 1 },
      })
      expect(library.tracks.map(track => track.coverUrl).slice(1)).toEqual(['https://img/b.jpg', ''])
      expect(library.scanSkipped).toEqual([{ path: '/music/bad.mp3', reason: 'corrupt' }])
      expect(library.hasFolders).toBe(true)
      expect(library.isLoaded).toBe(true)
    })

    it('sends a null name template when none is configured', async () => {
      router.on('local_library_snapshot', () => snapshot([]))
      await useLibraryStore().refresh()
      expect(router.callsOf('local_library_snapshot')[0].args).toEqual({ nameTemplate: null })
    })

    it('keeps the newest snapshot when an older request returns last', async () => {
      const older = deferred<unknown>()
      router.on('local_library_snapshot', () => older.promise, () => snapshot(['/music/new.flac']))
      const library = useLibraryStore()

      const first = library.refresh()
      await library.refresh()
      older.resolve(snapshot(['/music/old.flac']))
      await first

      expect(library.tracks.map(track => track.audioUrl)).toEqual(['/music/new.flac'])
    })

    it('ignores a late failure from a superseded request', async () => {
      const older = deferred<unknown>()
      router.on('local_library_snapshot', () => older.promise, () => snapshot(['/music/new.flac']))
      const library = useLibraryStore()

      const first = library.refresh()
      await library.refresh()
      older.reject('stale failure')
      await first

      expect(library.scanError).toBeNull()
    })

    it('keeps the previous list and reports the error when loading fails', async () => {
      router.on('local_library_snapshot', () => snapshot(['/music/a.flac']), () => { throw 'index locked' })
      const library = useLibraryStore()
      await library.refresh()

      await library.refresh()

      expect(library.scanError).toBe('index locked')
      expect(library.tracks).toHaveLength(1)
    })

    it('keeps scan progress only while the backend is still scanning', async () => {
      router.on('local_library_snapshot', () => snapshot([], { scanning: true }), () => snapshot([]))
      const library = useLibraryStore()
      library.scanProgress = { folder: '/music', visitedEntries: 1, tracks: 0, skipped: 0, currentPath: '/music/x' }

      await library.refresh()
      expect(library.isScanning).toBe(true)
      expect(library.scanProgress).not.toBeNull()

      await library.refresh()
      expect(library.isScanning).toBe(false)
      expect(library.scanProgress).toBeNull()
    })
  })

  describe('ensureStarted', () => {
    it('subscribes once, follows backend events and loads the index', async () => {
      router.on('local_library_snapshot', () => snapshot(['/music/a.flac']))
      router.on('get_local_playlist_tracks', () => [backendTrack('/playlists/x.mp3')])
      const library = useLibraryStore()

      await Promise.all([library.ensureStarted(), library.ensureStarted()])
      expect(mocks.listenCalls).toBe(3)
      expect([...mocks.listeners.keys()].sort()).toEqual(['local-library-changed', 'local-library-progress', 'playlists-changed'])
      expect(router.callsOf('local_library_snapshot')).toHaveLength(1)

      const progress = { folder: '/music', visitedEntries: 5, tracks: 2, skipped: 0, currentPath: '/music/b.flac' }
      mocks.listeners.get('local-library-progress')!({ payload: progress })
      expect(library.isScanning).toBe(true)
      expect(library.scanProgress).toEqual(progress)

      mocks.listeners.get('local-library-changed')!({ payload: null })
      mocks.listeners.get('playlists-changed')!({ payload: null })
      await flushPromises()
      expect(router.callsOf('local_library_snapshot')).toHaveLength(2)
      expect(library.playlistTracks.map(track => track.audioUrl)).toEqual(['/playlists/x.mp3'])
    })

    it('still loads the library when the event bridge is unavailable', async () => {
      mocks.listenError = new Error('no tauri')
      router.on('local_library_snapshot', () => snapshot(['/music/a.flac']))
      const library = useLibraryStore()

      await library.ensureStarted()

      expect(library.tracks).toHaveLength(1)
    })

    it('adopts the legacy scan directory as a folder when the library is empty', async () => {
      storage.setItem(LEGACY_SCAN_DIR_KEY, '/old/music')
      router.on('local_library_snapshot', () => ({ ...snapshot([]), folders: [] }))
      router.on('local_library_add_folder', () => ({ ...snapshot(['/old/music/a.mp3']), folders: [{ path: '/old/music', addedAt: 1, trackCount: 1, available: true }] }))
      const library = useLibraryStore()

      await library.ensureStarted()

      expect(router.callsOf('local_library_add_folder')[0].args).toEqual({ path: '/old/music', nameTemplate: null })
      expect(library.folders.map(folder => folder.path)).toEqual(['/old/music'])
      expect(storage.getItem(LEGACY_SCAN_DIR_KEY)).toBeNull()
    })

    it('drops the legacy key without re-adding when folders already exist or adding fails', async () => {
      storage.setItem(LEGACY_SCAN_DIR_KEY, '/old/music')
      router.on('local_library_snapshot', () => snapshot([]))
      await useLibraryStore().ensureStarted()
      expect(router.callsOf('local_library_add_folder')).toHaveLength(0)
      expect(storage.getItem(LEGACY_SCAN_DIR_KEY)).toBeNull()

      setActivePinia(createPinia())
      storage.setItem(LEGACY_SCAN_DIR_KEY, '/gone')
      router.set('local_library_snapshot', () => ({ ...snapshot([]), folders: [] }))
      router.on('local_library_add_folder', () => { throw 'missing directory' })
      await useLibraryStore().ensureStarted()
      expect(storage.getItem(LEGACY_SCAN_DIR_KEY)).toBeNull()
    })
  })

  describe('folder operations', () => {
    it('reports and rethrows a failure to add a folder', async () => {
      router.on('local_library_add_folder', () => { throw 'permission denied' })
      const library = useLibraryStore()

      await expect(library.addFolder('/root')).rejects.toBe('permission denied')
      expect(library.scanError).toBe('permission denied')
    })

    it('applies the snapshot returned by remove and rescan', async () => {
      router.on('local_library_remove_folder', () => ({ ...snapshot([]), folders: [] }))
      router.on('local_library_rescan', () => snapshot(['/music/z.flac']))
      const library = useLibraryStore()

      await library.removeFolder('/music')
      expect(library.hasFolders).toBe(false)

      await library.rescan(true)
      expect(router.callsOf('local_library_rescan')[0].args).toEqual({ full: true, nameTemplate: null })
      expect(library.tracks.map(track => track.audioUrl)).toEqual(['/music/z.flac'])
    })

    it('resolves a track to the most specific containing folder', async () => {
      router.on('local_library_snapshot', () => ({
        ...snapshot([]),
        folders: [
          { path: '/music', addedAt: 1, trackCount: 0, available: true },
          { path: '/music/jazz', addedAt: 2, trackCount: 0, available: true },
        ],
      }))
      const library = useLibraryStore()
      await library.refresh()

      const at = (audioUrl: string) => ({ id: 'x', title: '', artist: '', album: '', durationMs: 0, coverUrl: '', audioUrl })
      expect(library.folderOf(at('/music/jazz/a.flac'))).toBe('/music/jazz')
      expect(library.folderOf(at('/music/rock/a.flac'))).toBe('/music')
      expect(library.folderOf(at('/elsewhere/a.flac'))).toBeNull()
    })
  })

  describe('playlist index', () => {
    it('keeps only the newest result and reports failures', async () => {
      const older = deferred<unknown>()
      router.on('get_local_playlist_tracks', () => older.promise, () => [backendTrack('/p/new.mp3')], () => { throw 'unreadable' })
      const library = useLibraryStore()

      const first = library.refreshPlaylistIndex()
      await library.refreshPlaylistIndex()
      older.resolve([backendTrack('/p/old.mp3')])
      await first
      expect(library.playlistTracks.map(track => track.audioUrl)).toEqual(['/p/new.mp3'])

      await library.refreshPlaylistIndex()
      expect(library.playlistIndexError).toBe('unreadable')
      expect(library.playlistTracks).toHaveLength(1)
    })
  })

  describe('saveTrackTags', () => {
    async function loaded() {
      router.on('local_library_snapshot', () => snapshot(['/music/a.flac', '/music/b.flac']))
      const library = useLibraryStore()
      await library.refresh()
      return library
    }

    it('writes tags while the file is released, updates the list and the playing track, then reloads', async () => {
      const library = await loaded()
      const tags = { title: 'New title', artist: 'New artist', album: 'New album' }
      router.on('edit_local_file_tags', () => undefined)
      mocks.player!.currentTrack = { audioUrl: '/music/a.flac' }
      router.set('local_library_snapshot', () => snapshot(['/music/a.flac', '/music/b.flac']))

      await library.saveTrackTags(library.tracks[0], tags)

      expect(mocks.player!.released).toEqual(['/music/a.flac'])
      expect(router.callsOf('edit_local_file_tags')[0].args).toEqual({ scanRoot: '/music', filePath: '/music/a.flac', ...tags })
      expect(mocks.player!.updated).toEqual([tags])
      expect(router.callsOf('local_library_snapshot')).toHaveLength(2)
      expect(library.isSavingTags).toBe(false)
    })

    it('updates the in-memory track before the reload completes', async () => {
      const library = await loaded()
      const reload = deferred<unknown>()
      router.on('edit_local_file_tags', () => undefined)
      router.set('local_library_snapshot', () => reload.promise)
      const tags = { title: 'T', artist: 'A', album: 'B' }

      const saving = library.saveTrackTags(library.tracks[1], tags)
      await flushPromises()

      expect(library.tracks[1]).toMatchObject(tags)
      expect(library.tracks[0].title).toBe('title /music/a.flac')
      expect(mocks.player!.updated).toEqual([])
      reload.resolve(snapshot(['/music/a.flac', '/music/b.flac']))
      await saving
    })

    it('refuses files that left the library and concurrent saves', async () => {
      const library = await loaded()
      const stranger = { ...library.tracks[0], audioUrl: '/elsewhere/a.flac' }
      await expect(library.saveTrackTags(stranger, { title: '', artist: '', album: '' })).rejects.toThrow(/no longer in the music library/)

      const edit = deferred<unknown>()
      router.on('edit_local_file_tags', () => edit.promise)
      const first = library.saveTrackTags(library.tracks[0], { title: 'x', artist: 'y', album: 'z' })
      await flushPromises()
      await expect(library.saveTrackTags(library.tracks[1], { title: 'x', artist: 'y', album: 'z' })).rejects.toThrow()
      edit.resolve(undefined)
      await first
    })

    it('resets the saving flag and keeps the old tags when writing fails', async () => {
      const library = await loaded()
      router.on('edit_local_file_tags', () => { throw 'read-only file' })

      await expect(library.saveTrackTags(library.tracks[0], { title: 'x', artist: 'y', album: 'z' })).rejects.toBe('read-only file')

      expect(library.isSavingTags).toBe(false)
      expect(library.tracks[0].title).toBe('title /music/a.flac')
    })
  })
})
