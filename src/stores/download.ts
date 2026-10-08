import { defineStore } from 'pinia'
import { ref, computed, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { openPath } from '@tauri-apps/plugin-opener'
import { usePlayerStore, type TrackInfo } from './player'
import { useSettingsStore } from './settings'
import { useToastStore } from './toast'
import i18n from '@/i18n'
import { createLogger } from '@/utils/logger'
import { resolveDownloadSource } from '@/modules/playback/playbackSource'
import { DownloadQueue } from '@/modules/download/downloadQueue'
import {
  consumeResolvingCancellation,
  markResolvingTasksCancelled,
} from '@/modules/download/downloadCancellation'

const log = createLogger('download')
const PENDING_DOWNLOADS_KEY = 'neri:pending-downloads'
const PENDING_DOWNLOAD_STATUSES = new Set<ActiveDownloadTask['status']>(['queued', 'resolving', 'downloading', 'processing'])

export interface DownloadedTrack {
  id: string
  title: string
  artist: string
  album: string
  durationMs: number
  coverUrl: string | null
  source: string
  filePath: string
  fileSize: number
  downloadedAt: number
}

interface DownloadValidationResult {
  tracks: any[]
  removed_count?: number
  removedCount?: number
  integrity_mismatch_count?: number
  integrityMismatchCount?: number
}

export interface ActiveDownloadTask {
  trackId: string
  title: string
  artist: string
  source: string
  coverUrl?: string
  status: 'queued' | 'resolving' | 'downloading' | 'processing' | 'cancelling' | 'cancelled' | 'error' | 'already_exists'
  progress?: number
  downloadedBytes?: number
  totalBytes?: number
  message?: string
  speedBytesPerSecond?: number
}

type DownloadOutcome = 'completed' | 'failed' | 'cancelled' | 'existing'
interface DownloadBatch {
  outcomes: Map<string, DownloadOutcome>
  lastCompletedId?: string
  usedDefaultDir: boolean
}
interface DownloadAttempt {
  batch: DownloadBatch
  queueId: string
  outcome?: DownloadOutcome
}

export const useDownloadStore = defineStore('download', () => {
  const downloads = ref<DownloadedTrack[]>([])
  const downloading = ref<Map<string, ActiveDownloadTask>>(new Map())
  const activeDownloads = computed(() => Array.from(downloading.value.values()))
  const runningDownloadCount = computed(() => activeDownloads.value.filter(task =>
    ['queued', 'resolving', 'downloading', 'processing', 'cancelling'].includes(task.status),
  ).length)
  const settings = useSettingsStore()
  const queue = new DownloadQueue(() => settings.downloadParallelism)
  const requestedTracks = new Map<string, TrackInfo>()
  const speedSamples = new Map<string, { bytes: number; time: number }>()
  const attempts = new Map<string, DownloadAttempt>()
  const launching = new Set<DownloadAttempt>()
  let batch: DownloadBatch | null = null
  let completionCheck: Promise<void> | null = null
  let batchRevision = 0
  let attemptSequence = 0
  let downloadsLoadGeneration = 0
  let latestDownloadsLoad: Promise<void> | null = null
  watch(() => settings.downloadParallelism, () => queue.refresh())
  // 没下完的任务记在本机，重启后接着下（对齐 Android GlobalDownloadManager 恢复下载任务）
  let savedPendingIds: string | null = null
  watch(downloading, savePendingDownloads)

  // resolving 阶段的请求 token 集合，后端尚无任务时先在前端取消（DL-7）
  const resolvingCancelled = new Set<string>()
  let resolvingTokenSequence = 0
  const resolvingRequestTokens = new Map<string, string>()

  let eventsInitialized = false
  let eventsReady: Promise<void> | null = null
  let eventsGeneration = 0
  const terminalCleanupTimers = new Map<string, ReturnType<typeof setTimeout>>()

  let unlistenProgress: (() => void) | null = null
  let unlistenDirFallback: (() => void) | null = null
  let unlistenDownloadsChanged: (() => void) | null = null

  function initEvents() {
    if (eventsInitialized) return eventsReady ?? Promise.resolve()
    eventsInitialized = true
    const generation = ++eventsGeneration

    // 保存 UnlistenFn 并在 HMR dispose 时反注册, 避免 dev 下模块热重载重复挂监听
    // 导致重复 toast / 重复 loadDownloads（DL-13）
    if (import.meta.hot) {
      import.meta.hot.dispose(() => {
        if (eventsGeneration !== generation) return
        eventsInitialized = false
        eventsReady = null
        eventsGeneration += 1
        unlistenProgress?.()
        unlistenDirFallback?.()
        unlistenDownloadsChanged?.()
        unlistenProgress = null
        unlistenDirFallback = null
        unlistenDownloadsChanged = null
      })
    }

    const progressListening = listen<{ trackId: string; status: string; fileSize?: number; message?: string; downloadedBytes?: number; totalBytes?: number }>(
      'download-progress',
      (e) => {
        const { trackId, status, message, downloadedBytes, totalBytes } = e.payload
        const current = downloading.value.get(trackId)
        if (attempts.get(trackId)?.outcome) return

        if (status === 'queued' || status === 'processing') {
          setTaskStatus(trackId, status)
        } else if (status === 'start') {
          clearTerminalCleanup(trackId)
          downloading.value = new Map(downloading.value.set(trackId, {
            trackId,
            title: current?.title || trackId,
            artist: current?.artist || '',
            source: current?.source || '',
            coverUrl: current?.coverUrl,
            status: 'downloading',
            progress: current?.progress,
            downloadedBytes: current?.downloadedBytes,
            totalBytes: current?.totalBytes,
          }))
        } else if (status === 'downloading') {
          clearTerminalCleanup(trackId)
          const progress = totalBytes && totalBytes > 0
            ? Math.max(0, Math.min(100, Math.round((downloadedBytes || 0) / totalBytes * 100)))
            : undefined

          const time = Date.now()
          const sample = speedSamples.get(trackId)
          const bytes = downloadedBytes ?? current?.downloadedBytes ?? 0
          const speed = sample && time > sample.time && bytes >= sample.bytes
            ? (bytes - sample.bytes) * 1000 / (time - sample.time)
            : current?.speedBytesPerSecond
          speedSamples.set(trackId, { bytes, time })
          downloading.value = new Map(downloading.value.set(trackId, {
            trackId,
            title: current?.title || trackId,
            artist: current?.artist || '',
            source: current?.source || '',
            coverUrl: current?.coverUrl,
            status: 'downloading',
            progress,
            downloadedBytes: downloadedBytes ?? current?.downloadedBytes,
            totalBytes: totalBytes ?? current?.totalBytes,
            speedBytesPerSecond: speed,
          }))
        } else if (status === 'complete') {
          clearTerminalCleanup(trackId)
          downloading.value.delete(trackId)
          downloading.value = new Map(downloading.value)
          requestedTracks.delete(trackId)
          speedSamples.delete(trackId)
          finishAttempt(trackId, 'completed')
          void loadDownloads({ silent: true })
        } else if (status === 'error') {
          setTaskTerminalStatus(trackId, 'error', message)
          finishAttempt(trackId, 'failed')
        } else if (status === 'cancelled') {
          setTaskTerminalStatus(trackId, 'cancelled')
          finishAttempt(trackId, 'cancelled')
        } else if (status === 'already_exists') {
          setTaskTerminalStatus(trackId, 'already_exists')
          requestedTracks.delete(trackId)
          finishAttempt(trackId, 'existing')
        }
      },
    )
    const progressReady = progressListening
      .then((un) => {
        if (eventsGeneration === generation && eventsInitialized) {
          unlistenProgress = un
        } else {
          un()
        }
      })
      .catch((error) => {
        if (eventsGeneration === generation) {
          eventsGeneration++
          eventsInitialized = false
          eventsReady = null
          unlistenDirFallback?.()
          unlistenDownloadsChanged?.()
          unlistenDirFallback = null
          unlistenDownloadsChanged = null
        }
        log.error('Register download progress listener failed:', error)
        throw error
      })

    // 下载目录回退信息合并到批次汇总，下载过程中保持安静
    const fallbackListening = listen<{ requestedDir: string }>('download-dir-fallback', () => {
      if (batch) batch.usedDefaultDir = true
    })
    void fallbackListening
      .then((un) => {
        if (eventsGeneration === generation && eventsInitialized) {
          unlistenDirFallback = un
        } else {
          un()
        }
      })
      .catch((error) => log.error('Register download fallback listener failed:', error))
    void listen('downloads-changed', () => { void loadDownloads({ silent: true }) })
      .then((unlisten) => {
        if (eventsGeneration === generation && eventsInitialized) unlistenDownloadsChanged = unlisten
        else unlisten()
      })
      .catch(error => log.error('Register downloads changed listener failed:', error))
    eventsReady = progressReady
    // 旧调用方只注册事件，下载入口仍通过返回的 Promise 等待并处理失败
    void eventsReady.catch(() => {})
    return eventsReady
  }

  function clearTerminalCleanup(trackId: string) {
    const timer = terminalCleanupTimers.get(trackId)
    if (timer) {
      clearTimeout(timer)
      terminalCleanupTimers.delete(trackId)
    }
  }

  function setTaskStatus(trackId: string, status: ActiveDownloadTask['status'], message?: string) {
    const current = downloading.value.get(trackId)
    downloading.value = new Map(downloading.value.set(trackId, {
      trackId,
      title: current?.title || trackId,
      artist: current?.artist || '',
      source: current?.source || '',
      coverUrl: current?.coverUrl ?? requestedTracks.get(trackId)?.coverUrl,
      status,
      progress: current?.progress,
      downloadedBytes: current?.downloadedBytes,
      totalBytes: current?.totalBytes,
      message,
      speedBytesPerSecond: current?.speedBytesPerSecond,
    }))
  }

  function setTaskTerminalStatus(trackId: string, status: Extract<ActiveDownloadTask['status'], 'cancelled' | 'error' | 'already_exists'>, message?: string) {
    clearTerminalCleanup(trackId)
    if (!downloading.value.has(trackId) && !requestedTracks.has(trackId)) return
    setTaskStatus(trackId, status, message)
    speedSamples.delete(trackId)
    if (status === 'error' || status === 'cancelled') return
    const timer = setTimeout(() => {
      terminalCleanupTimers.delete(trackId)
      const current = downloading.value.get(trackId)
      if (current?.status === status) {
        downloading.value.delete(trackId)
        downloading.value = new Map(downloading.value)
      }
    }, 1800)
    terminalCleanupTimers.set(trackId, timer)
  }

  function finishAttempt(trackId: string, outcome: DownloadOutcome, attempt = attempts.get(trackId)) {
    if (!attempt || attempt.outcome) return
    attempt.outcome = outcome
    if (attempts.get(trackId) === attempt) {
      attempt.batch.outcomes.set(trackId, outcome)
      if (outcome === 'completed') attempt.batch.lastCompletedId = trackId
    }
    batchRevision++
    queue.finish(attempt.queueId)
    scheduleBatchCompletion()
  }

  function scheduleBatchCompletion() {
    if (!batch || !queue.isIdle || launching.size > 0 || completionCheck) return
    const finished = batch
    const revision = batchRevision
    completionCheck = (async () => {
      let refresh = loadDownloads({ silent: true })
      // 后续刷新可能持有更晚的清单，汇总需等它提交后再选择最后下载文件
      while (true) {
        await refresh
        const latest = latestDownloadsLoad
        if (!latest || latest === refresh) break
        refresh = latest
      }
      // 刷新期间追加或重试的任务仍属于当前批次，等待它们全部结束后再汇总
      if (batch !== finished || batchRevision !== revision || !queue.isIdle || launching.size > 0) return
      batch = null
      attempts.clear()
      const counts = { completed: 0, failed: 0, cancelled: 0, existing: 0 }
      for (const outcome of finished.outcomes.values()) counts[outcome]++
      if (counts.completed === 0) return
      const toast = useToastStore()
      const t = (key: string, params?: Record<string, unknown>) => (i18n.global as any).t(key, params)
      const completedIds = Array.from(finished.outcomes).filter(([, outcome]) => outcome === 'completed').map(([id]) => id)
      const lastCompletedId = finished.lastCompletedId && finished.outcomes.get(finished.lastCompletedId) === 'completed'
        ? finished.lastCompletedId : completedIds[completedIds.length - 1]
      const downloaded = downloads.value.find(track => track.id === lastCompletedId)
      const configuredDir = finished.usedDefaultDir ? '' : settings.downloadDir
      const summary = t('download.batch_finished', counts)
      toast.show(
        finished.usedDefaultDir ? `${summary} · ${t('download.dir_fallback')}` : summary,
        counts.failed > 0 || counts.cancelled > 0 ? 'info' : 'success',
        {
          duration: 6000,
          action: {
            label: t('download.open_folder'),
            handler: async () => {
              try {
                if (downloaded?.filePath) await invoke('reveal_file', { path: downloaded.filePath })
                else await openPath(configuredDir || await invoke<string>('get_default_download_dir'))
              } catch (error) {
                log.error('Open completed download folder failed:', error)
                toast.error(t('download.reveal_failed'))
              }
            },
          },
        },
      )
    })().finally(() => {
      completionCheck = null
      scheduleBatchCompletion()
    })
  }

  function loadDownloads(options: { silent?: boolean } = {}): Promise<void> {
    const generation = ++downloadsLoadGeneration
    const startedDuringDownload = batch !== null || !queue.isIdle || launching.size > 0
    const loading = refreshDownloads(generation, options, startedDuringDownload)
    latestDownloadsLoad = loading
    return loading
  }

  async function refreshDownloads(generation: number, options: { silent?: boolean }, startedDuringDownload: boolean) {
    try {
      const result = await invoke<DownloadValidationResult>('validate_downloads')
      if (generation !== downloadsLoadGeneration) return
      const raw = result.tracks || []
      downloads.value = (raw || []).map((t: any) => ({
        id: t.id,
        title: t.title,
        artist: t.artist,
        album: t.album,
        durationMs: t.duration_ms,
        coverUrl: t.cover_url || null,
        source: t.source,
        filePath: t.file_path,
        fileSize: t.file_size,
        downloadedAt: t.downloaded_at,
      }))
      const removedCount = result.removed_count ?? result.removedCount ?? 0
      const silent = options.silent || startedDuringDownload || batch !== null || !queue.isIdle || launching.size > 0
      if (removedCount > 0 && !silent) {
        const toast = useToastStore()
        toast.show((i18n.global as any).t('download.missing_cleaned', { count: removedCount }), 'info')
      }
      const mismatchCount = result.integrity_mismatch_count
        ?? result.integrityMismatchCount
        ?? 0
      if (mismatchCount > 0 && !silent) {
        const toast = useToastStore()
        toast.show(
          (i18n.global as any).t('download.integrity_mismatch', { count: mismatchCount }),
          'info',
        )
      }
    } catch (e) {
      log.error('Load downloads failed:', e)
    }
  }

  /** 进度事件很频繁，只在未完成的任务集合变化时写盘 */
  function savePendingDownloads() {
    const pending = activeDownloads.value
      .filter(task => PENDING_DOWNLOAD_STATUSES.has(task.status))
      .map(task => requestedTracks.get(task.trackId))
      .filter((track): track is TrackInfo => !!track)
    const ids = pending.map(track => track.id).join('\n')
    if (ids === savedPendingIds) return
    savedPendingIds = ids
    try {
      if (pending.length > 0) localStorage.setItem(PENDING_DOWNLOADS_KEY, JSON.stringify(pending))
      else localStorage.removeItem(PENDING_DOWNLOADS_KEY)
    } catch (error) {
      log.warn('Saving pending downloads failed:', error)
    }
  }

  /** 启动时把上次没下完的任务重新排队（地址早已过期，按当前设置重新解析）；返回重新排队的数量 */
  async function resumePendingDownloads(): Promise<number> {
    let pending: unknown
    try {
      pending = JSON.parse(localStorage.getItem(PENDING_DOWNLOADS_KEY) || '[]')
    } catch {
      pending = []
    }
    if (!Array.isArray(pending) || pending.length === 0) return 0
    await initEvents()
    await loadDownloads()
    let resumed = 0
    for (const track of pending as TrackInfo[]) {
      if (!track || typeof track.id !== 'string' || !track.id) continue
      if (isDownloaded(track.id) || isDownloading(track.id)) continue
      void downloadTrack(track)
      resumed++
    }
    savePendingDownloads()
    if (resumed > 0) {
      useToastStore().show((i18n.global as any).t('download.resumed_pending', { count: resumed }), 'info')
    }
    return resumed
  }

  /**
   * 下载曲目：先解析音频 URL（按来源分支），再调用后端下载
   */
  async function downloadTrack(track: TrackInfo) {
    if (isDownloaded(track.id)) {
      downloading.value = new Map(downloading.value.set(track.id, {
        trackId: track.id, title: track.title, artist: track.artist,
        source: getDownloadedTrack(track.id)?.source || '', coverUrl: track.coverUrl,
        status: 'already_exists',
      }))
      setTaskTerminalStatus(track.id, 'already_exists')
      return
    }

    if (isDownloading(track.id)) {
      return // 正在下载中
    }

    const source = track.id.startsWith('netease:')
      ? 'netease'
      : track.id.startsWith('qq:')
        ? 'qq'
      : track.id.startsWith('bilibili:')
        ? 'bilibili'
        : track.id.startsWith('youtube:')
          ? 'youtube'
          : 'local'

    if (source === 'local') {
      requestedTracks.set(track.id, { ...track })
      downloading.value = new Map(downloading.value.set(track.id, {
        trackId: track.id, title: track.title, artist: track.artist, source,
        coverUrl: track.coverUrl, status: 'error', message: (i18n.global as any).t('player.not_available'),
      }))
      return
    }
    batch ??= { outcomes: new Map(), usedDefaultDir: false }
    batch.outcomes.delete(track.id)
    const attempt: DownloadAttempt = { batch, queueId: `${track.id}:${++attemptSequence}` }
    attempts.set(track.id, attempt)
    batchRevision++
    clearTerminalCleanup(track.id)
    requestedTracks.set(track.id, { ...track })
    downloading.value = new Map(downloading.value.set(track.id, {
      trackId: track.id, title: track.title, artist: track.artist, source,
      coverUrl: track.coverUrl,
      status: 'queued', progress: 0, downloadedBytes: 0,
    }))
    queue.enqueue(attempt.queueId, () => { void startDownload(track, source, attempt) })
  }

  async function startDownload(track: TrackInfo, source: string, attempt: DownloadAttempt) {
    launching.add(attempt)
    const requestToken = `${track.id}:${++resolvingTokenSequence}`
    resolvingRequestTokens.set(track.id, requestToken)
    downloading.value = new Map(downloading.value.set(track.id, {
      trackId: track.id,
      title: track.title,
      artist: track.artist,
      source,
      coverUrl: track.coverUrl,
      status: 'resolving',
      progress: 0,
      downloadedBytes: 0,
    }))

    try {
      await initEvents()
      if (consumeResolvingCancellation(resolvingCancelled, requestToken)) {
        if (resolvingRequestTokens.get(track.id) === requestToken) resolvingRequestTokens.delete(track.id)
        if (attempts.get(track.id) === attempt) setTaskTerminalStatus(track.id, 'cancelled')
        finishAttempt(track.id, 'cancelled', attempt)
        return
      }
      const follow = settings.downloadFollowPlaybackQuality
      const youtubeQuality = follow ? settings.youtubeQuality : settings.downloadYoutubeQuality
      const resolved = await resolveDownloadSource(track, {
        neteaseQuality: follow ? settings.neteaseQuality : settings.downloadNeteaseQuality,
        qqMusicQuality: follow ? settings.qqMusicQuality : settings.downloadQqMusicQuality,
        biliQuality: follow ? settings.biliQuality : settings.downloadBiliQuality,
        youtubeQuality,
        youtubePlaybackSource: settings.youtubePlaybackSource,
      })

      // 解析期间被取消则不再启动后端下载（DL-7）
      if (consumeResolvingCancellation(resolvingCancelled, requestToken)) {
        if (resolvingRequestTokens.get(track.id) === requestToken) {
          resolvingRequestTokens.delete(track.id)
        }
        if (attempts.get(track.id) === attempt) setTaskTerminalStatus(track.id, 'cancelled')
        finishAttempt(track.id, 'cancelled', attempt)
        return
      }

      // 确定来源
      await invoke('download_track', {
        url: resolved.url,
        streamType: resolved.streamType ?? 'direct',
        expectedContentLength: resolved.expectedContentLength,
        expectedContentMd5: resolved.expectedContentMd5,
        youtubeVideoId: source === 'youtube' ? track.id.slice('youtube:'.length) : null,
        youtubeQuality: source === 'youtube' ? youtubeQuality : null,
        trackId: track.id,
        title: track.title,
        artist: track.artist,
        album: track.album || '',
        durationMs: resolved.durationMs || track.durationMs,
        coverUrl: track.coverUrl || null,
        source,
        downloadDir: useSettingsStore().downloadDir || null,
        nameTemplate: useSettingsStore().downloadNameTemplate || null,
      })

      // 取消可能与启动命令并发发生，命令返回后再消费一次 token，
      // 确保后端任务已注册后仍能收到取消请求
      if (consumeResolvingCancellation(resolvingCancelled, requestToken)) {
        if (resolvingRequestTokens.get(track.id) === requestToken) {
          try {
            await invoke('cancel_download', { trackId: track.id })
          } catch (cancelError) {
            log.error('Cancel download after launch failed:', cancelError)
          }
        }
      }
      if (resolvingRequestTokens.get(track.id) === requestToken) {
        resolvingRequestTokens.delete(track.id)
      }
    } catch (e: any) {
      log.error('Download failed:', e)
      const wasCancelled = resolvingCancelled.has(requestToken)
      if (resolvingRequestTokens.get(track.id) === requestToken) {
        resolvingRequestTokens.delete(track.id)
      }
      resolvingCancelled.delete(requestToken)
      const msg = typeof e === 'string' ? e : e?.message || String(e)
      const lowerMsg = msg.toLowerCase()
      if (attempts.get(track.id) !== attempt) {
        finishAttempt(track.id, 'cancelled', attempt)
        return
      }
      if (attempt.outcome) return
      if (wasCancelled || lowerMsg.includes('cancelled') || lowerMsg.includes('canceled')) {
        setTaskTerminalStatus(track.id, 'cancelled')
        finishAttempt(track.id, 'cancelled', attempt)
      } else if (lowerMsg.includes('already downloaded')) {
        setTaskTerminalStatus(track.id, 'already_exists')
        finishAttempt(track.id, 'existing', attempt)
      } else {
        setTaskTerminalStatus(track.id, 'error', msg)
        finishAttempt(track.id, 'failed', attempt)
      }
    } finally {
      launching.delete(attempt)
      scheduleBatchCompletion()
    }
  }

  async function deleteDownload(trackId: string, options: { silent?: boolean } = {}) {
    try {
      const target = getDownloadedTrack(trackId)
      const player = usePlayerStore()
      const remove = async () => {
        await invoke('delete_download', { trackId })
        downloads.value = downloads.value.filter(t => t.id !== trackId)
        player.handleDownloadedFileRemoved(trackId, target?.filePath)
      }
      if (target?.filePath) await player.withReleasedAudioFile(target.filePath, remove)
      else await remove()
      if (!options.silent) {
        const toast = useToastStore()
        toast.success((i18n.global as any).t('download.deleted'))
      }
    } catch (e) {
      log.error('Delete download failed:', e)
      await loadDownloads()
      throw e
    }
  }

  async function redownloadTrack(track: TrackInfo) {
    if (isDownloading(track.id)) return
    if (isDownloaded(track.id)) {
      await deleteDownload(track.id, { silent: true })
    }
    await downloadTrack(track)
  }

  function isDownloaded(trackId: string): boolean {
    return downloads.value.some(t => t.id === trackId)
  }

  function isDownloading(trackId: string): boolean {
    const status = downloading.value.get(trackId)?.status
    return !!status && ['queued', 'resolving', 'downloading', 'processing', 'cancelling'].includes(status)
  }

  function getDownloadedTrack(trackId: string): DownloadedTrack | undefined {
    return downloads.value.find(t => t.id === trackId)
  }

  async function cancelDownload(trackId: string) {
    const current = downloading.value.get(trackId)
    const resolvingToken = resolvingRequestTokens.get(trackId)
    const attempt = attempts.get(trackId)
    if (attempt && queue.cancelPending(attempt.queueId)) {
      setTaskTerminalStatus(trackId, 'cancelled')
      finishAttempt(trackId, 'cancelled')
      return true
    }
    if (current?.status === 'cancelling') return true
    if (!current || !isDownloading(trackId)) return false
    if (current.status === 'resolving' && resolvingToken) {
      // 在等待 IPC 前取消解析代际，避免解析先完成后漏掉后台任务
      resolvingCancelled.add(resolvingToken)
    }

    try {
      if (current) {
        setTaskStatus(trackId, 'cancelling')
      }
      const cancelled = await invoke<boolean>('cancel_download', { trackId })
      if (cancelled) {
        return true
      }
      // 后端查无任务：任务仍处于 resolving 阶段（URL 解析中）。标记取消，
      // 待解析完成时跳过后端下载，而不是误报取消失败（DL-7）
      if (current.status === 'resolving' && resolvingToken) {
        if (resolvingRequestTokens.get(trackId) === resolvingToken) {
          setTaskTerminalStatus(trackId, 'cancelled')
        }
        return true
      }
      if (downloading.value.get(trackId)?.status === 'cancelling') {
        setTaskStatus(trackId, current.status, (i18n.global as any).t('download.cancel_failed'))
      }
    } catch (e) {
      log.error('Cancel download failed:', e)
      if (current.status === 'resolving' && resolvingToken) {
        if (resolvingRequestTokens.get(trackId) === resolvingToken) setTaskTerminalStatus(trackId, 'cancelled')
        return true
      }
      if (downloading.value.get(trackId)?.status === 'cancelling') {
        setTaskStatus(trackId, current.status, (i18n.global as any).t('download.cancel_failed'))
      }
    }
    return false
  }

  async function cancelAllDownloads() {
    const cancelledPending: string[] = []
    for (const task of activeDownloads.value) {
      const attempt = attempts.get(task.trackId)
      if (attempt && queue.cancelPending(attempt.queueId)) cancelledPending.push(task.trackId)
    }
    for (const trackId of cancelledPending) {
      setTaskTerminalStatus(trackId, 'cancelled')
      finishAttempt(trackId, 'cancelled')
    }
    const resolvingIds = markResolvingTasksCancelled(
      downloading.value.values(),
      resolvingCancelled,
      trackId => resolvingRequestTokens.get(trackId),
    )
    try {
      const cancelled = await invoke<number>('cancel_all_downloads')
      for (const { trackId, token } of resolvingIds) {
        if (
          resolvingRequestTokens.get(trackId) === token
          && downloading.value.get(trackId)?.status === 'resolving'
        ) {
          setTaskTerminalStatus(trackId, 'cancelled')
        }
      }
      if (cancelled > 0) {
        const next = new Map(downloading.value)
        for (const [trackId, task] of next.entries()) {
          if (task.status === 'downloading' || task.status === 'processing') {
            next.set(trackId, { ...task, status: 'cancelling' })
          }
        }
        downloading.value = next
      }
    } catch (e) {
      log.error('Cancel downloads failed:', e)
      for (const { trackId, token } of resolvingIds) {
        if (
          resolvingRequestTokens.get(trackId) === token
          && downloading.value.get(trackId)?.status === 'resolving'
        ) {
          setTaskTerminalStatus(trackId, 'cancelled')
        }
      }
      for (const task of activeDownloads.value) {
        if (isDownloading(task.trackId)) setTaskStatus(task.trackId, task.status, (i18n.global as any).t('download.cancel_failed'))
      }
    }
  }

  function retryDownload(trackId: string) {
    const track = requestedTracks.get(trackId)
    if (track && !isDownloading(trackId)) void downloadTrack(track)
  }

  function clearFinishedTasks() {
    for (const task of activeDownloads.value) {
      if (isDownloading(task.trackId)) continue
      clearTerminalCleanup(task.trackId)
      requestedTracks.delete(task.trackId)
      downloading.value.delete(task.trackId)
    }
    downloading.value = new Map(downloading.value)
  }

  return {
    downloads,
    downloading,
    activeDownloads,
    runningDownloadCount,
    retryDownload,
    clearFinishedTasks,
    loadDownloads,
    downloadTrack,
    resumePendingDownloads,
    redownloadTrack,
    deleteDownload,
    isDownloaded,
    isDownloading,
    getDownloadedTrack,
    cancelDownload,
    cancelAllDownloads,
    initEvents,
  }
})
