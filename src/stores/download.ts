import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { TrackInfo } from './player'
import { useSettingsStore } from './settings'
import { useToastStore } from './toast'
import i18n from '@/i18n'
import { createLogger } from '@/utils/logger'

const log = createLogger('download')

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
}

export interface ActiveDownloadTask {
  trackId: string
  title: string
  artist: string
  source: string
  status: 'resolving' | 'downloading' | 'cancelling' | 'cancelled' | 'error' | 'already_exists'
  progress?: number
  downloadedBytes?: number
  totalBytes?: number
  message?: string
}

export const useDownloadStore = defineStore('download', () => {
  const downloads = ref<DownloadedTrack[]>([])
  const downloading = ref<Map<string, ActiveDownloadTask>>(new Map())
  const activeDownloads = computed(() => Array.from(downloading.value.values()))

  let eventsInitialized = false
  const terminalCleanupTimers = new Map<string, ReturnType<typeof setTimeout>>()

  function initEvents() {
    if (eventsInitialized) return
    eventsInitialized = true

    listen<{ trackId: string; status: string; fileSize?: number; message?: string; downloadedBytes?: number; totalBytes?: number }>(
      'download-progress',
      (e) => {
        const { trackId, status, message, downloadedBytes, totalBytes } = e.payload
        const toast = useToastStore()
        const current = downloading.value.get(trackId)

        if (status === 'start') {
          clearTerminalCleanup(trackId)
          downloading.value = new Map(downloading.value.set(trackId, {
            trackId,
            title: current?.title || trackId,
            artist: current?.artist || '',
            source: current?.source || '',
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

          downloading.value = new Map(downloading.value.set(trackId, {
            trackId,
            title: current?.title || trackId,
            artist: current?.artist || '',
            source: current?.source || '',
            status: 'downloading',
            progress,
            downloadedBytes: downloadedBytes ?? current?.downloadedBytes,
            totalBytes: totalBytes ?? current?.totalBytes,
          }))
        } else if (status === 'complete') {
          clearTerminalCleanup(trackId)
          downloading.value.delete(trackId)
          downloading.value = new Map(downloading.value)
          void handleCompletedDownload(trackId, toast)
        } else if (status === 'error') {
          setTaskTerminalStatus(trackId, 'error', message)
          toast.error((i18n.global as any).t('download.download_failed') + (message ? `: ${message}` : ''))
        } else if (status === 'cancelled') {
          setTaskTerminalStatus(trackId, 'cancelled')
          toast.show((i18n.global as any).t('download.cancelled'), 'info')
        } else if (status === 'already_exists') {
          setTaskTerminalStatus(trackId, 'already_exists')
          toast.show((i18n.global as any).t('download.already_exists'), 'info')
        }
      },
    )
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
      status,
      progress: current?.progress,
      downloadedBytes: current?.downloadedBytes,
      totalBytes: current?.totalBytes,
      message,
    }))
  }

  function setTaskTerminalStatus(trackId: string, status: Extract<ActiveDownloadTask['status'], 'cancelled' | 'error' | 'already_exists'>, message?: string) {
    clearTerminalCleanup(trackId)
    setTaskStatus(trackId, status, message)
    const timer = setTimeout(() => {
      terminalCleanupTimers.delete(trackId)
      const current = downloading.value.get(trackId)
      if (current?.status === status) {
        downloading.value.delete(trackId)
        downloading.value = new Map(downloading.value)
      }
    }, status === 'error' ? 3500 : 1800)
    terminalCleanupTimers.set(trackId, timer)
  }

  async function handleCompletedDownload(trackId: string, toast: ReturnType<typeof useToastStore>) {
    await loadDownloads()
    const downloaded = downloads.value.find(t => t.id === trackId)
    const t = (key: string, params?: Record<string, unknown>) => (i18n.global as any).t(key, params)
    if (downloaded?.filePath) {
      toast.success(t('download.downloaded'), {
        duration: 6000,
        action: {
          label: t('download.open_folder'),
          handler: async () => {
            await invoke('reveal_file', { path: downloaded.filePath })
          },
        },
      })
      return
    }
    toast.success(t('download.downloaded'))
  }

  async function loadDownloads() {
    try {
      const result = await invoke<DownloadValidationResult>('validate_downloads')
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
      if (removedCount > 0) {
        const toast = useToastStore()
        toast.show((i18n.global as any).t('download.missing_cleaned', { count: removedCount }), 'info')
      }
    } catch (e) {
      log.error('Load downloads failed:', e)
    }
  }

  /**
   * 下载曲目：先解析音频 URL（按来源分支），再调用后端下载
   */
  async function downloadTrack(track: TrackInfo) {
    initEvents()
    const toast = useToastStore()

    if (isDownloaded(track.id)) {
      toast.success((i18n.global as any).t('download.downloaded'))
      return
    }

    if (downloading.value.has(track.id)) {
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

    downloading.value = new Map(downloading.value.set(track.id, {
      trackId: track.id,
      title: track.title,
      artist: track.artist,
      source,
      status: 'resolving',
      progress: 0,
      downloadedBytes: 0,
    }))
    toast.success((i18n.global as any).t('download.downloading'))

    try {
      let audioUrl = ''

      if (track.id.startsWith('netease:')) {
        const settings = useSettingsStore()
        const songId = parseInt(track.id.replace('netease:', ''))
        const result = await invoke<{
          url: string | null
          is_preview?: boolean
        }>('get_netease_song_download_url', {
          songId,
          quality: settings.neteaseQuality,
        })
        if (result.is_preview) throw new Error('Netease download URL is preview-only')
        if (!result.url) throw new Error('No URL')
        audioUrl = result.url
      } else if (track.id.startsWith('qq:')) {
        const settings = useSettingsStore()
        const songMid = track.id.replace('qq:', '')
        const result = await invoke<{ url: string | null }>('get_qq_song_url', {
          songMid,
          quality: settings.qqMusicQuality,
        })
        if (!result.url) throw new Error('No QQ Music URL')
        audioUrl = result.url
      } else if (track.id.startsWith('bilibili:')) {
        const biliId = track.id.replace('bilibili:', '')
        const isAvid = /^\d+$/.test(biliId)
        const cidMatch = track.album?.match(/^Bilibili\|(\d+)/)
        const cid = cidMatch ? parseInt(cidMatch[1]) : undefined
        const result = await invoke<{ url: string }>('get_bili_audio_url', {
          bvid: isAvid ? '' : biliId,
          avid: isAvid ? parseInt(biliId) : null,
          cid: cid || null,
        })
        audioUrl = result.url
      } else if (track.id.startsWith('youtube:')) {
        const videoId = track.id.replace('youtube:', '')
        const streams = await invoke<{ url: string }[]>('get_youtube_audio_url', { videoId })
        if (!streams?.[0]?.url) throw new Error('No YouTube stream')
        audioUrl = streams[0].url
      } else {
        // 本地文件无需下载
        toast.error((i18n.global as any).t('player.not_available'))
        downloading.value.delete(track.id)
        downloading.value = new Map(downloading.value)
        return
      }

      // 确定来源
      await invoke('download_track', {
        url: audioUrl,
        trackId: track.id,
        title: track.title,
        artist: track.artist,
        album: track.album || '',
        durationMs: track.durationMs,
        coverUrl: track.coverUrl || null,
        source,
        downloadDir: useSettingsStore().downloadDir || null,
        nameTemplate: useSettingsStore().downloadNameTemplate || null,
      })
    } catch (e: any) {
      log.error('Download failed:', e)
      downloading.value.delete(track.id)
      downloading.value = new Map(downloading.value)
      const msg = typeof e === 'string' ? e : e?.message || String(e)
      const lowerMsg = msg.toLowerCase()
      if (!msg.includes('already downloaded') && !lowerMsg.includes('cancelled') && !lowerMsg.includes('canceled')) {
        toast.error((i18n.global as any).t('download.download_failed') + `: ${msg}`)
      }
    }
  }

  async function deleteDownload(trackId: string, options: { silent?: boolean } = {}) {
    try {
      await invoke('delete_download', { trackId })
      downloads.value = downloads.value.filter(t => t.id !== trackId)
      if (!options.silent) {
        const toast = useToastStore()
        toast.success((i18n.global as any).t('download.deleted'))
      }
    } catch (e) {
      log.error('Delete download failed:', e)
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
    return downloading.value.has(trackId)
  }

  function getDownloadedTrack(trackId: string): DownloadedTrack | undefined {
    return downloads.value.find(t => t.id === trackId)
  }

  async function cancelDownload(trackId: string) {
    const current = downloading.value.get(trackId)
    if (current?.status === 'cancelling') return true

    try {
      if (current) {
        setTaskStatus(trackId, 'cancelling')
      }
      const cancelled = await invoke<boolean>('cancel_download', { trackId })
      if (cancelled) {
        return true
      }
      if (current) {
        setTaskTerminalStatus(trackId, 'error', (i18n.global as any).t('download.cancel_failed'))
      }
    } catch (e) {
      log.error('Cancel download failed:', e)
      if (current) {
        setTaskTerminalStatus(trackId, 'error', (i18n.global as any).t('download.cancel_failed'))
      }
    }
    return false
  }

  async function cancelAllDownloads() {
    const toast = useToastStore()
    try {
      const cancelled = await invoke<number>('cancel_all_downloads')
      if (cancelled > 0) {
        const next = new Map(downloading.value)
        for (const [trackId, task] of next.entries()) {
          if (task.status === 'resolving' || task.status === 'downloading') {
            next.set(trackId, { ...task, status: 'cancelling' })
          }
        }
        downloading.value = next
      }
      toast.show(
        cancelled > 0
          ? (i18n.global as any).t('download.cancelled_count', { count: cancelled })
          : (i18n.global as any).t('settings.no_active_downloads'),
        'info',
      )
    } catch (e) {
      log.error('Cancel downloads failed:', e)
      toast.error((i18n.global as any).t('download.cancel_failed'))
    }
  }

  return {
    downloads,
    downloading,
    activeDownloads,
    loadDownloads,
    downloadTrack,
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
