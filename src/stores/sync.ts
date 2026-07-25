import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToastStore } from './toast'
import { useHistoryStore } from './history'
import { useSettingsStore } from './settings'
import { useAuthStore } from './auth'
import i18n from '@/i18n'
import { setLocale } from '@/i18n'
import { createLogger } from '@/utils/logger'

const log = createLogger('sync')

// 全局 i18n 翻译（非组件上下文）
const t = (key: string, params?: Record<string, any>) =>
  (i18n.global as any).t(key, params)

export interface SyncConfig {
  configured: boolean
  autoSync: boolean
  lastSyncTime: number
}

export type SyncFrequency =
  | 'immediate'
  | 'every_10_minutes'
  | 'every_15_minutes'
  | 'every_30_minutes'

const SYNC_FREQUENCY_DELAYS: Record<SyncFrequency, number> = {
  immediate: 0,
  every_10_minutes: 10 * 60 * 1000,
  every_15_minutes: 15 * 60 * 1000,
  every_30_minutes: 30 * 60 * 1000,
}

export function normalizeSyncFrequency(value: unknown): SyncFrequency {
  switch (String(value ?? '').trim().toLowerCase()) {
    case 'every_10_minutes':
    case 'batched':
    case 'batched_10':
      return 'every_10_minutes'
    case 'every_15_minutes':
    case 'batched_15':
      return 'every_15_minutes'
    case 'every_30_minutes':
    case 'batched_30':
      return 'every_30_minutes'
    default:
      return 'immediate'
  }
}

export function syncFrequencyDelayMs(frequency: SyncFrequency): number {
  return SYNC_FREQUENCY_DELAYS[frequency]
}

export interface GitHubSyncConfig extends SyncConfig {
  owner: string
  repo: string
  dataSaver: boolean
  silentFailures: boolean
  historyUpdateMode: SyncFrequency
}

export interface WebDavSyncConfig extends SyncConfig {
  serverUrl: string
  basePath: string
}

export interface SyncResult {
  success: boolean
  message: string
  playlistsAdded: number
  playlistsUpdated: number
  playlistsDeleted: number
  songsAdded: number
  songsRemoved: number
}

export const useSyncStore = defineStore('sync', () => {
  const github = ref<GitHubSyncConfig>({
    configured: false, owner: '', repo: '',
    autoSync: false, lastSyncTime: 0,
    dataSaver: true, silentFailures: false,
    historyUpdateMode: 'immediate',
  })
  const webdav = ref<WebDavSyncConfig>({ configured: false, serverUrl: '', basePath: '', autoSync: false, lastSyncTime: 0 })
  const syncFrequency = ref<SyncFrequency>('immediate')

  const isSyncing = ref(false)
  const lastResult = ref<SyncResult | null>(null)
  // 仅弹窗内部配置流程的错误（token 验证、仓库创建等）
  const dialogError = ref<string | null>(null)

  // 防止 loadConfigs 触发 watch 保存
  let _loading = false

  /** 加载同步配置 */
  async function loadConfigs() {
    _loading = true
    let legacyFrequency: SyncFrequency = 'immediate'
    try {
      const gh = await invoke<any>('get_github_sync_config')
      legacyFrequency = normalizeSyncFrequency(gh.historyUpdateMode)
      github.value = {
        configured: gh.configured ?? false,
        owner: gh.owner ?? '',
        repo: gh.repo ?? '',
        autoSync: gh.autoSync ?? false,
        lastSyncTime: gh.lastSyncTime ?? 0,
        dataSaver: gh.dataSaver ?? true,
        silentFailures: gh.silentFailures ?? false,
        historyUpdateMode: legacyFrequency,
      }
    } catch (e) {
      log.error('loadGitHubConfig:', e)
    }

    try {
      const wd = await invoke<any>('get_webdav_sync_config')
      webdav.value = {
        configured: wd.configured ?? false,
        serverUrl: wd.serverUrl ?? '',
        basePath: wd.basePath ?? '',
        autoSync: wd.autoSync ?? false,
        lastSyncTime: wd.lastSyncTime ?? 0,
      }
    } catch (e) {
      log.error('loadWebDavConfig:', e)
    }

    try {
      const preferences = await invoke<any>('get_sync_preferences')
      syncFrequency.value = normalizeSyncFrequency(preferences.historyUpdateMode)
    } catch (e) {
      // 兼容浏览器开发模式和未升级的后端，回退到旧 GitHub 字段
      syncFrequency.value = legacyFrequency
    }
    github.value.historyUpdateMode = syncFrequency.value
    _loading = false
  }

  // 播放历史频率是跨 GitHub/WebDAV 的全局偏好
  watch(
    syncFrequency,
    async (value) => {
      github.value.historyUpdateMode = value
      if (_loading) return
      try {
        await invoke('update_sync_preferences', { historyUpdateMode: value })
      } catch (e) {
        log.error('Failed to save sync frequency:', e)
      }
    },
  )

  // 监听 GitHub 子设置变化，自动保存到后端
  watch(
    () => ({
      autoSync: github.value.autoSync,
      dataSaver: github.value.dataSaver,
      silentFailures: github.value.silentFailures,
    }),
    async (val) => {
      if (_loading || !github.value.configured) return
      try {
        await invoke('update_github_sync_settings', {
          autoSync: val.autoSync,
          dataSaver: val.dataSaver,
          silentFailures: val.silentFailures,
        })
      } catch (e) {
        log.error('Failed to save GitHub sync settings:', e)
      }
    },
    { deep: true },
  )

  // 监听 WebDAV autoSync 变化
  watch(
    () => webdav.value.autoSync,
    async (val) => {
      if (_loading || !webdav.value.configured) return
      try {
        await invoke('update_webdav_sync_settings', { autoSync: val })
      } catch (e) {
        log.error('Failed to save WebDAV sync settings:', e)
      }
    },
  )

  /** 验证 GitHub token */
  async function validateGitHubToken(token: string): Promise<string | null> {
    dialogError.value = null
    try {
      const result = await invoke<any>('validate_github_token', { token })
      return result.username as string
    } catch (e: any) {
      dialogError.value = e?.toString() || 'Token validation failed'
      return null
    }
  }

  /** 创建新仓库 */
  async function createGitHubRepo(repoName: string): Promise<boolean> {
    dialogError.value = null
    try {
      const result = await invoke<any>('create_github_repo', { repoName })
      github.value = {
        configured: true, owner: result.owner, repo: result.repo,
        autoSync: true, lastSyncTime: 0,
        dataSaver: true, silentFailures: false, historyUpdateMode: syncFrequency.value,
      }
      return true
    } catch (e: any) {
      dialogError.value = e?.toString() || 'Failed to create repository'
      return false
    }
  }

  /** 使用已有仓库 */
  async function useExistingGitHubRepo(owner: string, repo: string): Promise<boolean> {
    dialogError.value = null
    try {
      const result = await invoke<any>('use_existing_github_repo', { owner, repo })
      github.value = {
        configured: true, owner: result.owner, repo: result.repo,
        autoSync: true, lastSyncTime: 0,
        dataSaver: true, silentFailures: false, historyUpdateMode: syncFrequency.value,
      }
      return true
    } catch (e: any) {
      dialogError.value = e?.toString() || 'Repository not found or inaccessible'
      return false
    }
  }

  /** 配置 GitHub 同步（一步到位，保留兼容） */
  async function configureGitHub(token: string, repo: string) {
    dialogError.value = null
    try {
      const result = await invoke<any>('configure_github_sync', { token, repo })
      github.value = {
        configured: true, owner: result.owner, repo: result.repo,
        autoSync: true, lastSyncTime: 0,
        dataSaver: true, silentFailures: false, historyUpdateMode: syncFrequency.value,
      }
      return true
    } catch (e: any) {
      dialogError.value = e?.toString() || 'Failed to configure GitHub sync'
      return false
    }
  }

  /** 执行 GitHub 同步。silent=true 时成功不弹 toast（自动同步场景） */
  async function syncGitHub(silent = false) {
    if (isSyncing.value) return
    const toast = useToastStore()
    const history = useHistoryStore()
    isSyncing.value = true
    try {
      const historySnapshot = history.getSyncSnapshot()
      const result = await invoke<any>('sync_github', {
        historyEntries: historySnapshot.entries,
        historyDeletions: historySnapshot.deletions,
      })
      if (result.history) await history.applySyncPayload(result.history)
      lastResult.value = {
        success: result.success, message: result.message,
        playlistsAdded: result.playlists_added ?? result.playlistsAdded ?? 0,
        playlistsUpdated: result.playlists_updated ?? result.playlistsUpdated ?? 0,
        playlistsDeleted: result.playlists_deleted ?? result.playlistsDeleted ?? 0,
        songsAdded: result.songs_added ?? result.songsAdded ?? 0,
        songsRemoved: result.songs_removed ?? result.songsRemoved ?? 0,
      }
      if (!silent) {
        toast.success(t('settings.github_sync_success'))
      }
      await loadConfigs()
    } catch (e: any) {
      // 错误始终显示（除非 silentFailures 开启）
      const message = e?.toString() || 'Sync failed'
      const tokenExpired = /token|unauthorized|401|expired/i.test(message)
      if (!silent || !github.value.silentFailures || tokenExpired) {
        toast.error(message)
      }
    } finally {
      isSyncing.value = false
    }
  }

  /** 断开 GitHub 同步 */
  async function disconnectGitHub() {
    const toast = useToastStore()
    try {
      await invoke('disconnect_github_sync')
      github.value = {
        configured: false, owner: '', repo: '',
        autoSync: false, lastSyncTime: 0,
        dataSaver: true, silentFailures: false, historyUpdateMode: syncFrequency.value,
      }
      toast.success(t('settings.github_disconnected'))
    } catch (e) {
      log.error('disconnectGitHub:', e)
    }
  }

  /** 配置 WebDAV 同步 */
  async function configureWebDav(serverUrl: string, username: string, password: string, basePath?: string) {
    dialogError.value = null
    try {
      await invoke('configure_webdav_sync', { serverUrl, username, password, basePath })
      webdav.value = {
        configured: true, serverUrl, basePath: basePath || '', autoSync: true, lastSyncTime: 0,
      }
      return true
    } catch (e: any) {
      dialogError.value = e?.toString() || 'Failed to configure WebDAV sync'
      return false
    }
  }

  /** 执行 WebDAV 同步。silent=true 时成功不弹 toast（自动同步场景） */
  async function syncWebDav(silent = false) {
    if (isSyncing.value) return
    const toast = useToastStore()
    const history = useHistoryStore()
    isSyncing.value = true
    try {
      const historySnapshot = history.getSyncSnapshot()
      const result = await invoke<any>('sync_webdav', {
        historyEntries: historySnapshot.entries,
        historyDeletions: historySnapshot.deletions,
      })
      if (result.history) await history.applySyncPayload(result.history)
      lastResult.value = {
        success: result.success, message: result.message,
        playlistsAdded: result.playlists_added ?? result.playlistsAdded ?? 0,
        playlistsUpdated: result.playlists_updated ?? result.playlistsUpdated ?? 0,
        playlistsDeleted: result.playlists_deleted ?? result.playlistsDeleted ?? 0,
        songsAdded: result.songs_added ?? result.songsAdded ?? 0,
        songsRemoved: result.songs_removed ?? result.songsRemoved ?? 0,
      }
      if (!silent) {
        toast.success(t('settings.webdav_sync_success'))
      }
      await loadConfigs()
    } catch (e: any) {
      if (!webdav.value.autoSync || !silent) {
        toast.error(e?.toString() || 'Sync failed')
      }
    } finally {
      isSyncing.value = false
    }
  }

  /** 断开 WebDAV 同步 */
  async function disconnectWebDav() {
    const toast = useToastStore()
    try {
      await invoke('disconnect_webdav_sync')
      webdav.value = { configured: false, serverUrl: '', basePath: '', autoSync: false, lastSyncTime: 0 }
      toast.success(t('settings.webdav_disconnected'))
    } catch (e) {
      log.error('disconnectWebDav:', e)
    }
  }

  /** 按 Android 的策略顺序执行所有已启用自动同步的提供商 */
  async function syncAuto(silent = true) {
    if (github.value.configured && github.value.autoSync) {
      await syncGitHub(silent)
    }
    if (webdav.value.configured && webdav.value.autoSync) {
      await syncWebDav(silent)
    }
  }

  /** 清除缓存 */
  async function clearCache() {
    const toast = useToastStore()
    try {
      const result = await invoke<any>('clear_app_cache')
      const bytes = result.clearedBytes ?? 0
      const failedCount = result.failedCount ?? 0
      const mb = (bytes / 1024 / 1024).toFixed(1)
      if (bytes === 0 && failedCount === 0) {
        toast.success(t('settings.cache_empty'))
      } else if (failedCount > 0) {
        toast.success(t('settings.cache_clear_partial', { mb, count: failedCount }))
      } else {
        toast.success(t('settings.cache_cleared', { mb }))
      }
    } catch (e: any) {
      toast.error(e?.toString() || t('settings.cache_clear_failed'))
    }
  }

  /** 导出播放列表 */
  async function exportPlaylists() {
    const toast = useToastStore()
    try {
      const result = await invoke<any>('export_playlists')
      if (result.success) {
        toast.success(t('settings.export_success', { count: result.count }))
      }
    } catch (e: any) {
      toast.error(e?.toString() || t('settings.export_failed'))
    }
  }

  /** 导入播放列表 */
  async function importPlaylists() {
    const toast = useToastStore()
    try {
      const result = await invoke<any>('import_playlists')
      if (result.success) {
        if (result.importedFavorites > 0) {
          toast.success(t('settings.import_success_with_favorites', {
            count: result.imported,
            favorites: result.importedFavorites,
          }))
        } else {
          toast.success(t('settings.import_success', { count: result.imported }))
        }
        if (result.onlineFavoritesAvailable === false) {
          toast.show(t('settings.import_online_favorites_unavailable'), 'info', 6000)
        }
      }
    } catch (e: any) {
      toast.error(e?.toString() || t('settings.import_failed'))
    }
  }

  async function exportConfig() {
    const toast = useToastStore()
    const settings = useSettingsStore()
    try {
      const result = await invoke<any>('export_config', {
        settings: settings.snapshot(),
        listenTogetherUserUuid: localStorage.getItem('neri:lt-uuid') || '',
      })
      if (result.success) {
        toast.success(t('settings.export_config_success'))
      }
      return result
    } catch (e: any) {
      toast.error(e?.toString() || t('settings.export_config_failed'))
      return { success: false }
    }
  }

  async function importConfig() {
    const toast = useToastStore()
    const settings = useSettingsStore()
    const auth = useAuthStore()
    try {
      const result = await invoke<any>('import_config')
      if (!result.success) return result
      settings.applySnapshot(result.settings)
      if (result.listenTogetherUserUuid) {
        localStorage.setItem('neri:lt-uuid', result.listenTogetherUserUuid)
      } else {
        localStorage.removeItem('neri:lt-uuid')
      }
      if (result.settings?.locale) setLocale(result.settings.locale, false)
      await auth.reconcileStatus(true)
      await loadConfigs()
      toast.success(t('settings.import_config_success'))
      const warningKeys: Record<string, string> = {
        listen_together_url_invalid: 'settings.import_config_warning_listen_together_url',
        youtube_authorization_unsupported: 'settings.import_config_warning_youtube_authorization',
        youtube_guest_cookies_ignored: 'settings.import_config_warning_youtube_guest_cookies',
        youtube_multi_account_unsupported: 'settings.import_config_warning_youtube_account',
        netease_auth_verification_failed: 'settings.import_config_warning_netease_auth',
      }
      for (const warning of result.warnings || []) {
        toast.show(t(warningKeys[warning] || 'settings.import_config_warning_unknown'), 'info', 6000)
      }
      return result
    } catch (e: any) {
      toast.error(e?.toString() || t('settings.import_config_failed'))
      await auth.reconcileStatus()
      return { success: false }
    }
  }

  return {
    github, webdav, syncFrequency, isSyncing, lastResult, dialogError,
    loadConfigs,
    validateGitHubToken, createGitHubRepo, useExistingGitHubRepo,
    configureGitHub, syncGitHub, syncAuto, disconnectGitHub,
    configureWebDav, syncWebDav, disconnectWebDav,
    clearCache, exportPlaylists, importPlaylists, exportConfig, importConfig,
  }
})
