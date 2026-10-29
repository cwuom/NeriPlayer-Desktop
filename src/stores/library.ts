import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { usePlayerStore, type TrackInfo } from './player'
import { useSettingsStore } from './settings'
import { createLogger } from '@/utils/logger'
import { isMacPlatform } from '@/modules/shortcuts/platform'

const log = createLogger('library')
const LEGACY_SCAN_DIR_KEY = 'neri:last_scan_dir'

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
  file_modified_ms?: number
  sync_payload?: Record<string, unknown>
}

export interface LocalFolder {
  path: string
  addedAt: number
  trackCount: number
  available: boolean
}

interface LocalLibrarySnapshot {
  folders: LocalFolder[]
  tracks: BackendTrack[]
  scanning: boolean
  skipped: { path: string; reason: string }[]
}

interface ScanProgress {
  folder: string
  visitedEntries: number
  tracks: number
  skipped: number
  currentPath: string
}

export function isInsideFolder(path: string, folder: string): boolean {
  const windowsPath = /^[a-zA-Z]:[\\/]|^\\\\/.test(folder)
  const caseInsensitive = windowsPath || isMacPlatform
  const normalize = (value: string) => {
    const separators = windowsPath ? value.replace(/\//g, '\\') : value
    return caseInsensitive ? separators.toLowerCase() : separators
  }
  const normalizedPath = normalize(path)
  // 去掉末尾分隔符，盘符根目录 D:\ 与 / 才能和其下的文件对上
  const normalizedFolder = normalize(folder).replace(windowsPath ? /[\\/]+$/ : /\/+$/, '')
  if (normalizedPath === normalizedFolder) return true
  return normalizedPath.startsWith(normalizedFolder)
    && (windowsPath ? ['/', '\\'] : ['/']).includes(normalizedPath.charAt(normalizedFolder.length))
}

/// 本地音乐库：文件夹与曲目索引由后端持久化并监视变化，这里只做镜像。
/// 启动时直接读索引出列表，不再每次整夹重扫
export const useLibraryStore = defineStore('library', () => {
  const tracks = ref<TrackInfo[]>([])
  const folders = ref<LocalFolder[]>([])
  const isScanning = ref(false)
  const isLoaded = ref(false)
  const scanError = ref<string | null>(null)
  const scanSkipped = ref<{ path: string; reason: string }[]>([])
  const scanProgress = ref<ScanProgress | null>(null)
  const playlistTracks = ref<TrackInfo[]>([])
  const playlistIndexError = ref<string | null>(null)
  const isSavingTags = ref(false)
  let playlistIndexRequest = 0
  let snapshotRequest = 0
  let started: Promise<void> | null = null

  const hasFolders = computed(() => folders.value.length > 0)

  function toDisplayableCoverUrl(value?: string | null) {
    if (!value) return ''
    if (/^(https?:|asset:|data:|blob:)/i.test(value)) return value
    return convertFileSrc(value)
  }

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
      fileModifiedMs: track.file_modified_ms,
      syncPayload: track.sync_payload,
    }
  }

  function nameTemplate(): string | null {
    return useSettingsStore().downloadNameTemplate || null
  }

  function applySnapshot(snapshot: LocalLibrarySnapshot) {
    folders.value = snapshot.folders
    tracks.value = snapshot.tracks.map(toTrack)
    isScanning.value = snapshot.scanning
    scanSkipped.value = snapshot.skipped || []
    if (!snapshot.scanning) scanProgress.value = null
    isLoaded.value = true
  }

  async function refresh() {
    const request = ++snapshotRequest
    try {
      const snapshot = await invoke<LocalLibrarySnapshot>('local_library_snapshot', { nameTemplate: nameTemplate() })
      if (request !== snapshotRequest) return
      applySnapshot(snapshot)
      scanError.value = null
    } catch (error) {
      if (request !== snapshotRequest) return
      scanError.value = String(error)
      log.error('Load local library failed:', error)
    }
  }

  /// 旧版只记一个「上次扫描目录」：第一次启动时把它收编成音乐文件夹
  async function migrateLegacyScanDir() {
    const legacy = localStorage.getItem(LEGACY_SCAN_DIR_KEY)
    if (!legacy) return
    if (!folders.value.length) {
      try {
        applySnapshot(await invoke<LocalLibrarySnapshot>('local_library_add_folder', { path: legacy, nameTemplate: nameTemplate() }))
      } catch (error) {
        log.warn('Legacy scan directory could not be added:', error)
      }
    }
    localStorage.removeItem(LEGACY_SCAN_DIR_KEY)
  }

  function ensureStarted(): Promise<void> {
    if (started) return started
    started = (async () => {
      try {
        await listen('local-library-changed', () => { void refresh() })
        await listen<ScanProgress>('local-library-progress', ({ payload }) => {
          isScanning.value = true
          scanProgress.value = payload
        })
        await listen('playlists-changed', () => { void refreshPlaylistIndex() })
      } catch {
        // 浏览器预览没有 Tauri 事件桥
      }
      await refresh()
      await migrateLegacyScanDir()
    })()
    return started
  }

  async function addFolder(path: string) {
    scanError.value = null
    try {
      applySnapshot(await invoke<LocalLibrarySnapshot>('local_library_add_folder', { path, nameTemplate: nameTemplate() }))
    } catch (error) {
      scanError.value = String(error)
      throw error
    }
  }

  async function removeFolder(path: string) {
    applySnapshot(await invoke<LocalLibrarySnapshot>('local_library_remove_folder', { path }))
  }

  async function rescan(full = false) {
    scanError.value = null
    applySnapshot(await invoke<LocalLibrarySnapshot>('local_library_rescan', { full, nameTemplate: nameTemplate() }))
  }

  async function cancelScan() {
    await invoke('local_library_cancel_scan')
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

  function folderOf(track: TrackInfo): string | null {
    return folders.value
      .filter(folder => isInsideFolder(track.audioUrl, folder.path))
      .sort((left, right) => right.path.length - left.path.length)[0]?.path ?? null
  }

  async function saveTrackTags(track: TrackInfo, tags: { title: string; artist: string; album: string }) {
    const root = folderOf(track)
    if (isSavingTags.value || !root || !tracks.value.some(item => item.audioUrl === track.audioUrl)) {
      throw new Error('Local file is no longer in the music library')
    }
    isSavingTags.value = true
    try {
      const player = usePlayerStore()
      await player.withReleasedAudioFile(track.audioUrl, async () => {
        await invoke('edit_local_file_tags', { scanRoot: root, filePath: track.audioUrl, ...tags })
        tracks.value = tracks.value.map(item => item.audioUrl === track.audioUrl ? { ...item, ...tags } : item)
        if (player.currentTrack?.audioUrl === track.audioUrl) player.updateCurrentTrackInfo(tags)
      })
      await refresh()
    } finally {
      isSavingTags.value = false
    }
  }

  return {
    tracks, folders, hasFolders, isScanning, isLoaded, scanError, scanSkipped, scanProgress,
    playlistTracks, playlistIndexError, isSavingTags,
    ensureStarted, refresh, addFolder, removeFolder, rescan, cancelScan, refreshPlaylistIndex, saveTrackTags, folderOf,
  }
})
