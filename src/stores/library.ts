import { defineStore } from 'pinia'
import { ref } from 'vue'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { usePlayerStore, type TrackInfo } from './player'
import { useSettingsStore } from './settings'
import { createLogger } from '@/utils/logger'

const log = createLogger('library')

interface BackendTrack {
  id: string
  title: string
  artist: string
  album: string
  duration_ms: number
  cover_url?: string | null
  url: string
  source?: string
  added_at?: number
  sync_payload?: Record<string, unknown>
}

interface ScanProgress {
  sessionId: string
  visitedEntries: number
  tracks: number
  skipped: number
  currentPath: string
}

export const useLibraryStore = defineStore('library', () => {
  const tracks = ref<TrackInfo[]>([])
  const isScanning = ref(false)
  const scanError = ref<string | null>(null)
  const scanSkipped = ref<{ path: string; reason: string }[]>([])
  const lastScanDir = ref<string | null>(localStorage.getItem('neri:last_scan_dir'))
  const scanDir = ref<string | null>(null)
  const isCancelling = ref(false)
  const scanCancelled = ref(false)
  const scanProgress = ref<ScanProgress | null>(null)
  const playlistTracks = ref<TrackInfo[]>([])
  const playlistIndexError = ref<string | null>(null)
  const isSavingTags = ref(false)
  let activeSessionId: string | null = null
  let playlistIndexRequest = 0

  function toTrack(track: BackendTrack): TrackInfo {
    return {
      id: track.id,
      title: track.title,
      artist: track.artist,
      album: track.album,
      durationMs: track.duration_ms,
      coverUrl: toDisplayableCoverUrl(track.cover_url),
      audioUrl: track.url,
      source: track.source,
      addedAt: track.added_at,
      syncPayload: track.sync_payload,
    }
  }

  async function scanDirectory(dir: string) {
    if (isScanning.value || isSavingTags.value) return
    const sessionId = crypto.randomUUID()
    activeSessionId = sessionId
    isScanning.value = true
    isCancelling.value = false
    scanCancelled.value = false
    scanDir.value = dir
    scanError.value = null
    scanSkipped.value = []
    scanProgress.value = { sessionId, visitedEntries: 0, tracks: 0, skipped: 0, currentPath: dir }
    tracks.value = []
    let unlisten: UnlistenFn | undefined

    try {
      unlisten = await listen<ScanProgress>('local-scan-progress', ({ payload }) => {
        if (payload.sessionId === activeSessionId) scanProgress.value = payload
      })
      if (isCancelling.value) {
        scanCancelled.value = true
        return
      }
      const settings = useSettingsStore()
      const results = await invoke<{ tracks: BackendTrack[]; skipped: { path: string; reason: string }[] }>('scan_local_files', {
        sessionId,
        dir,
        nameTemplate: settings.downloadNameTemplate || null,
      })
      if (isCancelling.value) {
        scanCancelled.value = true
        return
      }
      scanSkipped.value = results.skipped || []

      tracks.value = (results.tracks || []).map(toTrack)

      lastScanDir.value = dir
      // 持久化扫描路径
      localStorage.setItem('neri:last_scan_dir', dir)
      await refreshPlaylistIndex()
    } catch (error) {
      if (isCancelling.value || String(error).includes('Scan cancelled')) scanCancelled.value = true
      else {
        scanError.value = String(error)
        log.error('Scan failed:', error)
      }
    } finally {
      unlisten?.()
      activeSessionId = null
      isScanning.value = false
      isCancelling.value = false
    }
  }

  async function cancelScan() {
    if (!activeSessionId || isCancelling.value) return
    isCancelling.value = true
    try {
      await invoke<boolean>('cancel_local_scan', { sessionId: activeSessionId })
    } catch (error) {
      isCancelling.value = false
      scanError.value = String(error)
      log.error('Cancel scan failed:', error)
    }
  }

  async function refreshPlaylistIndex() {
    const request = ++playlistIndexRequest
    try {
      const result = await invoke<BackendTrack[]>('get_local_playlist_tracks')
      if (request !== playlistIndexRequest) return
      playlistTracks.value = result.map(toTrack)
      playlistIndexError.value = null
    } catch (error) {
      if (request !== playlistIndexRequest) return
      playlistIndexError.value = String(error)
      log.error('Read playlist file index failed:', error)
    }
  }

  async function saveTrackTags(track: TrackInfo, tags: { title: string; artist: string; album: string }) {
    if (isSavingTags.value || isScanning.value || !scanDir.value || !tracks.value.some(item => item.id === track.id && item.audioUrl === track.audioUrl)) {
      throw new Error('Local file is no longer in the current scan preview')
    }
    isSavingTags.value = true
    try {
      const player = usePlayerStore()
      await player.withReleasedAudioFile(track.audioUrl, async () => {
        await invoke('edit_local_file_tags', { scanRoot: scanDir.value, filePath: track.audioUrl, ...tags })
        tracks.value = tracks.value.map(item => item.id === track.id ? { ...item, ...tags } : item)
        if (player.currentTrack?.audioUrl === track.audioUrl) player.updateCurrentTrackInfo(tags)
      })
    } finally {
      isSavingTags.value = false
    }
  }

  function toDisplayableCoverUrl(value?: string | null) {
    if (!value) return ''
    if (/^(https?:|asset:|data:|blob:)/i.test(value)) return value
    return convertFileSrc(value)
  }

  // 启动时恢复上次扫描路径
  function restoreLastScan() {
    const dir = localStorage.getItem('neri:last_scan_dir')
    if (dir && !isScanning.value && tracks.value.length === 0) void scanDirectory(dir)
  }

  return {
    tracks, isScanning, scanError, scanSkipped, lastScanDir, scanDir,
    isCancelling, scanCancelled, scanProgress, playlistTracks, playlistIndexError,
    isSavingTags, saveTrackTags, scanDirectory, restoreLastScan, cancelScan, refreshPlaylistIndex,
  }
})
