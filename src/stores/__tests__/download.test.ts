import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import type { TrackInfo } from '@/types/track'
import type { ResolvedPlaybackSource } from '@/modules/playback/playbackSource'
import type { ToastOptions } from '@/stores/toast'
import { useDownloadStore } from '@/stores/download'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises, type Deferred } from './storeTestUtils'

type Resolved = Partial<ResolvedPlaybackSource>
type EventHandler = (event: { payload: unknown }) => void

interface SettingsStub {
  downloadParallelism: number
  downloadFollowPlaybackQuality: boolean
  neteaseQuality: string
  downloadNeteaseQuality: string
  qqMusicQuality: string
  downloadQqMusicQuality: string
  biliQuality: string
  downloadBiliQuality: string
  youtubeQuality: string
  downloadYoutubeQuality: string
  youtubePlaybackSource: string
  downloadDir: string
  downloadNameTemplate: string
}

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  toasts: [] as Array<{ text: string; type: string; options?: unknown }>,
  resolve: null as null | ((track: TrackInfo, settings: Record<string, unknown>) => Promise<Partial<ResolvedPlaybackSource>>),
  settings: null as SettingsStub | null,
  removedFiles: [] as Array<[string, string | undefined]>,
  releasedFiles: [] as string[],
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (event: string, handler: EventHandler) => {
    mocks.listeners.set(event, handler)
    return () => { mocks.listeners.delete(event) }
  },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/i18n', () => ({
  default: {
    global: {
      t: (key: string, params?: Record<string, unknown>) => (params && Object.keys(params).length ? `${key}:${JSON.stringify(params)}` : key),
    },
  },
}))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({
    show: (text: string, type = 'info', options?: unknown) => { mocks.toasts.push({ text, type, options }) },
    success: (text: string) => { mocks.toasts.push({ text, type: 'success' }) },
    error: (text: string) => { mocks.toasts.push({ text, type: 'error' }) },
  }),
}))
vi.mock('@/stores/settings', () => ({ useSettingsStore: () => mocks.settings }))
vi.mock('@/stores/player', () => ({
  usePlayerStore: () => ({
    async withReleasedAudioFile(path: string, action: () => Promise<void>) {
      mocks.releasedFiles.push(path)
      await action()
    },
    handleDownloadedFileRemoved(trackId: string, path?: string) {
      mocks.removedFiles.push([trackId, path])
    },
  }),
}))
vi.mock('@/modules/playback/playbackSource', () => ({
  resolveDownloadSource: (track: TrackInfo, settings: Record<string, unknown>) => mocks.resolve!(track, settings),
}))

const PENDING_KEY = 'neri:pending-downloads'

function track(id: string, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return { id, title: `title ${id}`, artist: 'artist', album: 'album', durationMs: 180_000, coverUrl: '', audioUrl: '', ...overrides }
}

function manifestRow(id: string, filePath = `/dl/${id.replace(':', '_')}.flac`) {
  return {
    id, title: `title ${id}`, artist: 'artist', album: 'album', duration_ms: 180_000, cover_url: null,
    source: id.split(':')[0], file_path: filePath, file_size: 1024, downloaded_at: 1,
  }
}

describe('download store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage
  let manifest: Array<ReturnType<typeof manifestRow>>
  let resolutions: Map<string, Deferred<Resolved>>

  const status = (store: ReturnType<typeof useDownloadStore>, id: string) => store.downloading.get(id)?.status
  const downloadCalls = () => router.callsOf('download_track').map(call => call.args?.trackId)

  function emit(trackId: string, progressStatus: string, extra: Record<string, unknown> = {}) {
    mocks.listeners.get('download-progress')!({ payload: { trackId, status: progressStatus, ...extra } })
  }

  /** 让下一次取流停在解析阶段，由用例决定何时返回 */
  function holdResolution(id: string): Deferred<Resolved> {
    const pending = deferred<Resolved>()
    resolutions.set(id, pending)
    return pending
  }

  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(1_000_000)
    router = createInvokeRouter()
    mocks.router = router
    manifest = []
    router.on('validate_downloads', () => ({ tracks: manifest.map(row => ({ ...row })) }))
    router.on('download_track', () => undefined)
    resolutions = new Map()
    mocks.resolve = (target) => {
      const held = resolutions.get(target.id)
      if (held) {
        resolutions.delete(target.id)
        return held.promise
      }
      return Promise.resolve({ url: `https://cdn/${target.id}`, streamType: 'direct', expectedContentLength: 100 })
    }
    mocks.listeners = new Map()
    mocks.toasts = []
    mocks.removedFiles = []
    mocks.releasedFiles = []
    mocks.settings = reactive<SettingsStub>({
      downloadParallelism: 6,
      downloadFollowPlaybackQuality: true,
      neteaseQuality: 'lossless',
      downloadNeteaseQuality: 'standard',
      qqMusicQuality: 'high',
      downloadQqMusicQuality: 'standard',
      biliQuality: 'hires',
      downloadBiliQuality: 'low',
      youtubeQuality: 'very_high',
      downloadYoutubeQuality: 'low',
      youtubePlaybackSource: 'automatic',
      downloadDir: '/dl',
      downloadNameTemplate: '%title%',
    })
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  describe('starting downloads', () => {
    it('rejects tracks that only exist locally without touching the backend', async () => {
      const store = useDownloadStore()

      await store.downloadTrack(track('local:/a.mp3'))

      expect(store.downloading.get('local:/a.mp3')).toMatchObject({ status: 'error', message: 'player.not_available' })
      expect(store.isDownloading('local:/a.mp3')).toBe(false)
      expect(router.calls).toEqual([])
    })

    it('resolves with the playback quality and hands the stream to the backend', async () => {
      const store = useDownloadStore()

      await store.downloadTrack(track('netease:1', { coverUrl: 'http://p1.music.126.net/c.jpg' }))
      await flushPromises()

      expect(router.callsOf('download_track')[0].args).toEqual({
        url: 'https://cdn/netease:1', streamType: 'direct', expectedContentLength: 100, expectedContentMd5: undefined,
        youtubeVideoId: null, youtubeQuality: null, trackId: 'netease:1', title: 'title netease:1', artist: 'artist',
        album: 'album', durationMs: 180_000, coverUrl: 'https://p1.music.126.net/c.jpg', source: 'netease',
        downloadDir: '/dl', nameTemplate: '%title%',
      })
      expect(status(store, 'netease:1')).toBe('resolving')
    })

    it('uses the dedicated download quality when not following playback', async () => {
      mocks.settings!.downloadFollowPlaybackQuality = false
      const seen: Array<Record<string, unknown>> = []
      mocks.resolve = async (_target, settings) => {
        seen.push(settings)
        return { url: 'https://cdn/x', biliVideoSkipTarget: { bvid: 'BV1xx', cid: 42 }, durationMs: 5000 }
      }
      const store = useDownloadStore()

      await store.downloadTrack(track('youtube:abc'))
      await store.downloadTrack(track('bilibili:BV1xx'))
      await flushPromises()

      expect(seen[0]).toEqual({
        neteaseQuality: 'standard', qqMusicQuality: 'standard', biliQuality: 'low', youtubeQuality: 'low', youtubePlaybackSource: 'automatic',
      })
      const [youtube, bilibili] = router.callsOf('download_track').map(call => call.args)
      expect(youtube).toMatchObject({ youtubeVideoId: 'abc', youtubeQuality: 'low', streamType: 'direct', durationMs: 5000 })
      expect(bilibili).toMatchObject({ album: 'Bilibili|42|BV1xx', youtubeVideoId: null, source: 'bilibili' })
    })

    it('never queues the same track twice while it is in flight', async () => {
      holdResolution('netease:1')
      const store = useDownloadStore()

      await store.downloadTrack(track('netease:1'))
      await store.downloadTrack(track('netease:1'))
      store.retryDownload('netease:1')
      await flushPromises()

      expect(store.activeDownloads).toHaveLength(1)
      expect(store.runningDownloadCount).toBe(1)
    })

    it('respects the parallelism setting and picks up a raised limit immediately', async () => {
      mocks.settings!.downloadParallelism = 1
      for (const id of ['qq:1', 'qq:2', 'qq:3']) holdResolution(id)
      const store = useDownloadStore()

      for (const id of ['qq:1', 'qq:2', 'qq:3']) await store.downloadTrack(track(id))
      await flushPromises()
      expect(['qq:1', 'qq:2', 'qq:3'].map(id => status(store, id))).toEqual(['resolving', 'queued', 'queued'])

      mocks.settings!.downloadParallelism = 3
      await flushPromises()
      expect(['qq:1', 'qq:2', 'qq:3'].map(id => status(store, id))).toEqual(['resolving', 'resolving', 'resolving'])
    })
  })

  describe('progress events', () => {
    it('tracks progress and speed, then refreshes the manifest and announces the batch', async () => {
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()

      emit('netease:1', 'start')
      expect(status(store, 'netease:1')).toBe('downloading')
      emit('netease:1', 'downloading', { downloadedBytes: 50, totalBytes: 200 })
      expect(store.downloading.get('netease:1')).toMatchObject({ progress: 25, downloadedBytes: 50, totalBytes: 200 })
      vi.advanceTimersByTime(1000)
      emit('netease:1', 'downloading', { downloadedBytes: 250, totalBytes: 200 })
      expect(store.downloading.get('netease:1')).toMatchObject({ progress: 100, speedBytesPerSecond: 200 })

      manifest = [manifestRow('netease:1')]
      emit('netease:1', 'complete')
      await flushPromises()

      expect(store.downloading.has('netease:1')).toBe(false)
      expect(store.isDownloaded('netease:1')).toBe(true)
      expect(store.getDownloadedTrack('netease:1')).toMatchObject({ filePath: '/dl/netease_1.flac', durationMs: 180_000, coverUrl: null })
      expect(mocks.toasts).toHaveLength(1)
      expect(mocks.toasts[0]).toMatchObject({
        text: 'download.batch_finished:{"completed":1,"failed":0,"cancelled":0,"existing":0}',
        type: 'success',
      })

      router.on('reveal_file', () => undefined)
      await (mocks.toasts[0].options as ToastOptions).action!.handler()
      expect(router.callsOf('reveal_file')[0].args).toEqual({ path: '/dl/netease_1.flac' })
    })

    it('reports a mixed batch as info and mentions the fallback directory', async () => {
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await store.downloadTrack(track('netease:2'))
      await flushPromises()

      mocks.listeners.get('download-dir-fallback')!({ payload: { requestedDir: '/missing' } })
      emit('netease:2', 'error', { message: 'HTTP 403' })
      manifest = [manifestRow('netease:1')]
      emit('netease:1', 'complete')
      await flushPromises()

      expect(store.downloading.get('netease:2')).toMatchObject({ status: 'error', message: 'HTTP 403' })
      expect(mocks.toasts.map(toast => [toast.text, toast.type])).toEqual([[
        'download.batch_finished:{"completed":1,"failed":1,"cancelled":0,"existing":0} · download.dir_fallback',
        'info',
      ]])
    })

    it('stays quiet when nothing in the batch completed', async () => {
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()

      emit('netease:1', 'error', { message: 'nope' })
      await flushPromises()

      expect(mocks.toasts).toEqual([])
    })

    it('ignores progress for unknown or cancelled tasks', async () => {
      const store = useDownloadStore()
      await store.initEvents()

      emit('netease:404', 'downloading', { downloadedBytes: 1, totalBytes: 2 })
      emit('netease:404', 'start')

      expect(store.downloading.size).toBe(0)
    })

    it('shows an existing file briefly and then forgets it', async () => {
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()

      emit('netease:1', 'already_exists')
      expect(status(store, 'netease:1')).toBe('already_exists')
      vi.advanceTimersByTime(1799)
      expect(store.downloading.has('netease:1')).toBe(true)
      vi.advanceTimersByTime(1)
      expect(store.downloading.has('netease:1')).toBe(false)
    })
  })

  describe('backend failures', () => {
    it.each([
      ['File already downloaded', 'already_exists'],
      ['Download was cancelled by user', 'cancelled'],
    ])('maps "%s" to %s', async (message, expected) => {
      router.set('download_track', () => { throw message })
      const store = useDownloadStore()

      await store.downloadTrack(track('netease:1'))
      await flushPromises()

      expect(status(store, 'netease:1')).toBe(expected)
    })

    it('keeps other errors visible and retries them on request', async () => {
      router.set('download_track', () => { throw new Error('HTTP 403') }, () => undefined)
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()
      expect(store.downloading.get('netease:1')).toMatchObject({ status: 'error', message: 'HTTP 403' })

      store.retryDownload('netease:1')
      await flushPromises()

      expect(downloadCalls()).toEqual(['netease:1', 'netease:1'])
      expect(status(store, 'netease:1')).toBe('resolving')
    })

    it('reports a resolver failure as an error without calling the backend', async () => {
      mocks.resolve = async () => { throw new Error('No downloadable stream') }
      const store = useDownloadStore()

      await store.downloadTrack(track('netease:1'))
      await flushPromises()

      expect(store.downloading.get('netease:1')).toMatchObject({ status: 'error', message: 'No downloadable stream' })
      expect(downloadCalls()).toEqual([])
    })
  })

  describe('cancellation', () => {
    it('cancels a queued task without ever starting it', async () => {
      mocks.settings!.downloadParallelism = 1
      const first = holdResolution('netease:1')
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await store.downloadTrack(track('netease:2'))
      await flushPromises()

      expect(await store.cancelDownload('netease:2')).toBe(true)
      expect(status(store, 'netease:2')).toBe('cancelled')

      first.resolve({ url: 'https://cdn/1' })
      await flushPromises()
      emit('netease:1', 'complete')
      await flushPromises()
      expect(downloadCalls()).toEqual(['netease:1'])
    })

    it('stops a task that is still resolving before it reaches the backend', async () => {
      const resolving = holdResolution('netease:1')
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()

      expect(await store.cancelDownload('netease:1')).toBe(true)
      resolving.resolve({ url: 'https://cdn/1' })
      await flushPromises()

      expect(status(store, 'netease:1')).toBe('cancelled')
      expect(downloadCalls()).toEqual([])
      expect(router.callsOf('cancel_download')).toEqual([])
    })

    it('restores the task with a message when the backend refuses to cancel', async () => {
      router.on('cancel_download', () => false)
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()
      emit('netease:1', 'start')

      expect(await store.cancelDownload('netease:1')).toBe(false)

      expect(store.downloading.get('netease:1')).toMatchObject({ status: 'downloading', message: 'download.cancel_failed' })
    })

    it('waits for the backend event after a successful cancel request', async () => {
      router.on('cancel_download', () => true)
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()
      emit('netease:1', 'start')

      expect(await store.cancelDownload('netease:1')).toBe(true)
      expect(status(store, 'netease:1')).toBe('cancelling')
      emit('netease:1', 'downloading', { downloadedBytes: 10, totalBytes: 20 })
      expect(status(store, 'netease:1')).toBe('cancelling')

      emit('netease:1', 'cancelled')
      expect(status(store, 'netease:1')).toBe('cancelled')
      expect(store.isDownloading('netease:1')).toBe(false)
    })

    it('cancels everything: queued ones locally, running ones through the backend', async () => {
      mocks.settings!.downloadParallelism = 1
      router.on('cancel_all_downloads', () => 1)
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await store.downloadTrack(track('netease:2'))
      await flushPromises()
      emit('netease:1', 'start')

      await store.cancelAllDownloads()

      expect(status(store, 'netease:1')).toBe('cancelling')
      expect(status(store, 'netease:2')).toBe('cancelled')
      emit('netease:1', 'cancelled')
      await flushPromises()
      expect(downloadCalls()).toEqual(['netease:1'])
      expect(store.runningDownloadCount).toBe(0)
    })

    it('restores running tasks when the backend cancelled nothing', async () => {
      router.on('cancel_all_downloads', () => 0)
      const store = useDownloadStore()
      await store.downloadTrack(track('netease:1'))
      await flushPromises()
      emit('netease:1', 'start')

      await store.cancelAllDownloads()

      expect(store.downloading.get('netease:1')).toMatchObject({ status: 'downloading', message: 'download.cancel_failed' })
    })
  })

  describe('manifest', () => {
    it('keeps the newest validation result when an older one returns late', async () => {
      const older = deferred<unknown>()
      router.set('validate_downloads', () => older.promise, () => ({ tracks: [manifestRow('netease:new')] }))
      const store = useDownloadStore()

      const first = store.loadDownloads()
      await store.loadDownloads()
      older.resolve({ tracks: [manifestRow('netease:old')] })
      await first

      expect(store.downloads.map(item => item.id)).toEqual(['netease:new'])
    })

    it('announces cleaned or mismatched files only for a user-initiated load', async () => {
      router.set('validate_downloads', () => ({ tracks: [], removed_count: 2, integrityMismatchCount: 1 }))
      const store = useDownloadStore()

      await store.loadDownloads({ silent: true })
      expect(mocks.toasts).toEqual([])

      await store.loadDownloads()
      expect(mocks.toasts.map(toast => toast.text)).toEqual([
        'download.missing_cleaned:{"count":2}',
        'download.integrity_mismatch:{"count":1}',
      ])
    })

    it('validates the manifest only once for repeated first-use calls', async () => {
      const store = useDownloadStore()

      await Promise.all([store.ensureDownloadsLoaded(), store.ensureDownloadsLoaded()])
      await store.ensureDownloadsLoaded()

      expect(router.callsOf('validate_downloads')).toHaveLength(1)
    })

    it('retries the first-use validation after a failure', async () => {
      router.set('validate_downloads', () => { throw new Error('io') }, () => ({ tracks: [] }))
      const store = useDownloadStore()

      await store.ensureDownloadsLoaded()
      await store.ensureDownloadsLoaded()
      await store.ensureDownloadsLoaded()

      expect(router.callsOf('validate_downloads')).toHaveLength(2)
    })

    it('marks an already downloaded track and redownloads it after deleting the file', async () => {
      manifest = [manifestRow('netease:1')]
      router.on('delete_download', () => { manifest = [] })
      const store = useDownloadStore()
      await store.loadDownloads()

      await store.downloadTrack(track('netease:1'))
      expect(status(store, 'netease:1')).toBe('already_exists')
      expect(downloadCalls()).toEqual([])

      await store.redownloadTrack(track('netease:1'))
      await flushPromises()
      expect(mocks.releasedFiles).toEqual(['/dl/netease_1.flac'])
      expect(mocks.removedFiles).toEqual([['netease:1', '/dl/netease_1.flac']])
      expect(downloadCalls()).toEqual(['netease:1'])
      expect(mocks.toasts).toEqual([])
    })

    it('reloads and rethrows when deleting fails', async () => {
      manifest = [manifestRow('netease:1')]
      router.on('delete_download', () => { throw new Error('in use') })
      const store = useDownloadStore()
      await store.loadDownloads()

      await expect(store.deleteDownload('netease:1')).rejects.toThrow('in use')

      expect(store.isDownloaded('netease:1')).toBe(true)
      expect(router.callsOf('validate_downloads')).toHaveLength(2)
    })

    it('confirms a successful delete', async () => {
      manifest = [manifestRow('netease:1')]
      router.on('delete_download', () => undefined)
      const store = useDownloadStore()
      await store.loadDownloads()

      await store.deleteDownload('netease:1')

      expect(store.isDownloaded('netease:1')).toBe(false)
      expect(mocks.toasts.map(toast => [toast.text, toast.type])).toEqual([['download.deleted', 'success']])
    })
  })

  describe('pending downloads across restarts', () => {
    it('persists unfinished tasks and forgets them once they finish', async () => {
      holdResolution('netease:1')
      const store = useDownloadStore()

      await store.downloadTrack(track('netease:1'))
      await flushPromises()
      expect((JSON.parse(storage.getItem(PENDING_KEY) ?? '[]') as TrackInfo[]).map(item => item.id)).toEqual(['netease:1'])

      await store.cancelDownload('netease:1')
      await flushPromises()
      expect(storage.getItem(PENDING_KEY)).toBeNull()
    })

    it('requeues only tracks that are not downloaded yet', async () => {
      storage.setItem(PENDING_KEY, JSON.stringify([track('netease:1'), track('netease:2'), { id: '' }, null]))
      manifest = [manifestRow('netease:1')]
      const store = useDownloadStore()

      expect(await store.resumePendingDownloads()).toBe(1)
      await flushPromises()

      expect(downloadCalls()).toEqual(['netease:2'])
      expect(mocks.toasts.map(toast => toast.text)).toEqual(['download.resumed_pending:{"count":1}'])
    })

    it('ignores a corrupt pending list', async () => {
      storage.setItem(PENDING_KEY, '[{')
      const store = useDownloadStore()

      expect(await store.resumePendingDownloads()).toBe(0)
      expect(router.calls).toEqual([])
    })
  })

  it('clears finished tasks but keeps running ones', async () => {
    holdResolution('netease:2')
    router.set('download_track', () => { throw new Error('boom') })
    const store = useDownloadStore()
    await store.downloadTrack(track('netease:1'))
    await store.downloadTrack(track('netease:2'))
    await flushPromises()
    expect(status(store, 'netease:1')).toBe('error')

    store.clearFinishedTasks()

    expect([...store.downloading.keys()]).toEqual(['netease:2'])
    store.retryDownload('netease:1')
    await flushPromises()
    expect(downloadCalls()).toEqual(['netease:1'])
  })
})
