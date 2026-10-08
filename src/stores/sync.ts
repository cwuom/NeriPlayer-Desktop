import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToastStore } from './toast'
import { useHistoryStore } from './history'
import { useLyricOffsetStore } from './lyricOffset'
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

export type SyncBackend = 'github' | 'webdav'

/** 同步期间本地又有修改时，等这一轮结束后再补一轮（对齐 Android 的 follow-up 同步） */
export const FOLLOW_UP_SYNC_DELAY_MS = 5_000

export interface SyncProtocolUpgrade {
  backend: SyncBackend
  target: string
  fingerprint: string
  sourceProtocol: 0 | 3
  targetProtocol: 4
}

export function parseSyncProtocolUpgrade(error: unknown): SyncProtocolUpgrade | null {
  const message = String(error)
  const marker = 'SYNC_PROTOCOL_UPGRADE_REQUIRED:'
  const index = message.indexOf(marker)
  if (index < 0) return null
  try {
    const value = JSON.parse(message.slice(index + marker.length))
    if ((value.backend !== 'github' && value.backend !== 'webdav')
      || typeof value.target !== 'string' || !value.target
      || !/^[0-9a-f]{64}$/.test(value.fingerprint)
      || (value.sourceProtocol !== 0 && value.sourceProtocol !== 3)
      || value.targetProtocol !== 4) return null
    return value as SyncProtocolUpgrade
  } catch {
    return null
  }
}

/** 不会自己恢复的 WebDAV 失败（对齐 Android SyncWorkerFailurePolicy 的永久失败） */
const WEBDAV_FAILURE_MESSAGES = {
  WEBDAV_AUTH_FAILED: 'settings.webdav_auth_failed',
  WEBDAV_ACCESS_DENIED: 'settings.webdav_access_denied',
  WEBDAV_DIRECTORY_NOT_FOUND: 'settings.webdav_directory_not_found',
  WEBDAV_NOT_DIRECTORY: 'settings.webdav_not_directory',
  WEBDAV_MISSING_CONDITION: 'settings.webdav_missing_condition',
} as const
type WebDavFailureCode = keyof typeof WEBDAV_FAILURE_MESSAGES

/** Rust 侧 SyncFailure 的稳定代码（src-tauri/src/sync/failure.rs） */
export type SyncFailure =
  | { code: 'GITHUB_TOKEN_EXPIRED' }
  | { code: 'GITHUB_RATE_LIMITED', retryAt: number, automatic: boolean }
  | { code: WebDavFailureCode }

export function parseSyncFailure(error: unknown): SyncFailure | null {
  const message = String(error ?? '')
  if (message.includes('GITHUB_TOKEN_EXPIRED')) return { code: 'GITHUB_TOKEN_EXPIRED' }
  const limited = /GITHUB_RATE_LIMITED:(\d+):([01])/.exec(message)
  if (limited) return { code: 'GITHUB_RATE_LIMITED', retryAt: Number(limited[1]), automatic: limited[2] === '1' }
  const webdav = (Object.keys(WEBDAV_FAILURE_MESSAGES) as WebDavFailureCode[]).find(code => message.includes(code))
  return webdav ? { code: webdav } : null
}

function isWebDavFailure(failure: SyncFailure | null): failure is { code: WebDavFailureCode } {
  return failure !== null && failure.code in WEBDAV_FAILURE_MESSAGES
}

function formatRetryTime(at: number): string {
  const locale = String((i18n.global as any).locale?.value ?? '') || undefined
  return new Date(at).toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' })
}

/** 带稳定代码的失败换成本地化文案，其余保留后端原文 */
export function describeSyncError(error: unknown, fallback = 'Sync failed'): string {
  const failure = parseSyncFailure(error)
  if (isWebDavFailure(failure)) return t(WEBDAV_FAILURE_MESSAGES[failure.code])
  switch (failure?.code) {
    case 'GITHUB_TOKEN_EXPIRED':
      return t('settings.github_token_expired')
    case 'GITHUB_RATE_LIMITED':
      return t(failure.automatic ? 'settings.github_rate_limited' : 'settings.github_rate_limited_stopped', {
        time: formatRetryTime(failure.retryAt),
      })
    default:
      return String(error || fallback)
  }
}

/** 升级失败只给出本地化原因或 HTTP 状态码，不暴露服务器地址和响应正文 */
export function describeUpgradeFailure(error: unknown): string {
  if (parseSyncFailure(error)) return describeSyncError(error)
  const status = /\((\d{3})\)/.exec(String(error ?? ''))?.[1]
  return status
    ? t('settings.sync_upgrade_attempt_failed_status', { status })
    : t('settings.sync_upgrade_attempt_failed')
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
  const protocolUpgrades = ref<SyncProtocolUpgrade[]>([])
  const pendingProtocolUpgrade = computed(() => protocolUpgrades.value[0] ?? null)
  // 升级失败留在确认对话框里显示；换成另一份确认时清掉
  const upgradeError = ref<string | null>(null)
  watch(pendingProtocolUpgrade, () => { upgradeError.value = null })

  function rememberProtocolUpgrade(challenge: SyncProtocolUpgrade) {
    protocolUpgrades.value = protocolUpgrades.value.filter(item => item.backend !== challenge.backend)
    protocolUpgrades.value.push(challenge)
  }

  function clearProtocolUpgrade(backend: 'github' | 'webdav') {
    protocolUpgrades.value = protocolUpgrades.value.filter(item => item.backend !== backend)
  }

  const configurationGeneration = { github: 0, webdav: 0 }
  const configurationReads = { github: 0, webdav: 0 }
  const githubPreferenceRevision = { autoSync: 0, dataSaver: 0, silentFailures: 0 }
  let webdavPreferenceRevision = 0
  let dialogOwner: SyncProtocolUpgrade['backend'] | null = null
  let preferenceGeneration = 0
  let preferenceRead = 0
  // 认证、目录这类 WebDAV 失败不会自己恢复：提示一次后暂停自动同步，直到手动同步成功或配置变更
  let webdavBlockedBy: WebDavFailureCode | null = null

  function invalidateConfiguration(backend: SyncProtocolUpgrade['backend']) {
    clearProtocolUpgrade(backend)
    if (backend === 'webdav') webdavBlockedBy = null
    if (dialogOwner === backend) {
      dialogOwner = null
      dialogError.value = null
    }
    return ++configurationGeneration[backend]
  }

  function beginConfiguration(backend: SyncProtocolUpgrade['backend']) {
    const generation = invalidateConfiguration(backend)
    dialogOwner = backend
    dialogError.value = null
    return generation
  }

  function isCurrentConfiguration(backend: SyncProtocolUpgrade['backend'], generation: number) {
    return configurationGeneration[backend] === generation
  }

  function configurationFailed(backend: SyncProtocolUpgrade['backend'], generation: number, error: unknown, fallback: string) {
    if (isCurrentConfiguration(backend, generation) && dialogOwner === backend) {
      dialogError.value = describeSyncError(error, fallback)
    }
  }

  function githubPreferencesAfterRequest(
    started: typeof githubPreferenceRevision,
    proposed = { autoSync: true, dataSaver: true, silentFailures: false },
  ) {
    const current = github.value
    return {
      autoSync: githubPreferenceRevision.autoSync !== started.autoSync ? current.autoSync : proposed.autoSync,
      dataSaver: githubPreferenceRevision.dataSaver !== started.dataSaver ? current.dataSaver : proposed.dataSaver,
      silentFailures: githubPreferenceRevision.silentFailures !== started.silentFailures ? current.silentFailures : proposed.silentFailures,
    }
  }

  // 只在读取结果赋值时抑制保存，网络等待不应吞掉用户修改
  let _loading = false
  function applyLoadedConfig(apply: () => void) {
    _loading = true
    try {
      apply()
    } finally {
      _loading = false
    }
  }

  /** 加载同步配置 */
  async function loadConfigs(generations = { ...configurationGeneration }) {
    const startedGithub = { ...githubPreferenceRevision }
    const startedWebdav = webdavPreferenceRevision
    const reads = {
      github: isCurrentConfiguration('github', generations.github) ? ++configurationReads.github : null,
      webdav: isCurrentConfiguration('webdav', generations.webdav) ? ++configurationReads.webdav : null,
    }
    const currentPreferenceRead = ++preferenceRead
    const currentPreferenceGeneration = preferenceGeneration
    let legacyFrequency: SyncFrequency | undefined
    try {
      const gh = await invoke<any>('get_github_sync_config')
      if (isCurrentConfiguration('github', generations.github) && reads.github === configurationReads.github) {
        legacyFrequency = normalizeSyncFrequency(gh.historyUpdateMode)
        applyLoadedConfig(() => {
          github.value = {
            configured: gh.configured ?? false,
            owner: gh.owner ?? '',
            repo: gh.repo ?? '',
            lastSyncTime: gh.lastSyncTime ?? 0,
            ...githubPreferencesAfterRequest(startedGithub, {
              autoSync: gh.autoSync ?? false,
              dataSaver: gh.dataSaver ?? true,
              silentFailures: gh.silentFailures ?? false,
            }),
            historyUpdateMode: syncFrequency.value,
          }
        })
      }
    } catch (e) {
      log.error('loadGitHubConfig:', e)
    }

    try {
      const wd = await invoke<any>('get_webdav_sync_config')
      if (isCurrentConfiguration('webdav', generations.webdav) && reads.webdav === configurationReads.webdav) {
        applyLoadedConfig(() => {
          webdav.value = {
            configured: wd.configured ?? false,
            serverUrl: wd.serverUrl ?? '',
            basePath: wd.basePath ?? '',
            autoSync: webdavPreferenceRevision !== startedWebdav ? webdav.value.autoSync : wd.autoSync ?? false,
            lastSyncTime: wd.lastSyncTime ?? 0,
          }
        })
      }
    } catch (e) {
      log.error('loadWebDavConfig:', e)
    }

    try {
      const preferences = await invoke<any>('get_sync_preferences')
      if (currentPreferenceRead === preferenceRead && currentPreferenceGeneration === preferenceGeneration) {
        applyLoadedConfig(() => {
          syncFrequency.value = normalizeSyncFrequency(preferences.historyUpdateMode)
          github.value.historyUpdateMode = syncFrequency.value
        })
      }
    } catch (e) {
      // 兼容浏览器开发模式和未升级的后端，回退到旧 GitHub 字段
      if (legacyFrequency !== undefined && currentPreferenceRead === preferenceRead
        && currentPreferenceGeneration === preferenceGeneration) {
        const frequency = legacyFrequency
        applyLoadedConfig(() => {
          syncFrequency.value = frequency
          github.value.historyUpdateMode = syncFrequency.value
        })
      }
    }
  }

  // 播放历史频率是跨 GitHub/WebDAV 的全局偏好
  watch(
    syncFrequency,
    async (value) => {
      github.value.historyUpdateMode = value
      if (_loading) return
      preferenceGeneration++
      try {
        await invoke('update_sync_preferences', { historyUpdateMode: value })
      } catch (e) {
        log.error('Failed to save sync frequency:', e)
      }
    },
    { flush: 'sync' },
  )

  // 监听 GitHub 子设置变化，自动保存到后端
  watch(
    () => ({
      autoSync: github.value.autoSync,
      dataSaver: github.value.dataSaver,
      silentFailures: github.value.silentFailures,
    }),
    async (val, previous) => {
      if (_loading) return
      if (val.autoSync !== previous.autoSync) githubPreferenceRevision.autoSync++
      if (val.dataSaver !== previous.dataSaver) githubPreferenceRevision.dataSaver++
      if (val.silentFailures !== previous.silentFailures) githubPreferenceRevision.silentFailures++
      if (!github.value.configured) return
      try {
        await invoke('update_github_sync_settings', {
          ...(val.autoSync !== previous.autoSync ? { autoSync: val.autoSync } : {}),
          ...(val.dataSaver !== previous.dataSaver ? { dataSaver: val.dataSaver } : {}),
          ...(val.silentFailures !== previous.silentFailures ? { silentFailures: val.silentFailures } : {}),
        })
      } catch (e) {
        log.error('Failed to save GitHub sync settings:', e)
      }
    },
    { deep: true, flush: 'sync' },
  )

  // 监听 WebDAV autoSync 变化
  watch(
    () => webdav.value.autoSync,
    async (val, previous) => {
      if (_loading) return
      if (val !== previous) webdavPreferenceRevision++
      if (!webdav.value.configured) return
      try {
        await invoke('update_webdav_sync_settings', { autoSync: val })
      } catch (e) {
        log.error('Failed to save WebDAV sync settings:', e)
      }
    },
    { flush: 'sync' },
  )

  /** 验证 GitHub token */
  async function validateGitHubToken(token: string): Promise<string | null> {
    const generation = beginConfiguration('github')
    try {
      const result = await invoke<any>('validate_github_token', { token })
      if (!isCurrentConfiguration('github', generation)) return null
      await loadConfigs({ github: generation, webdav: configurationGeneration.webdav })
      if (!isCurrentConfiguration('github', generation)) return null
      return result.username as string
    } catch (e: any) {
      configurationFailed('github', generation, e, 'Token validation failed')
      return null
    }
  }

  /** 创建新仓库 */
  async function createGitHubRepo(repoName: string): Promise<boolean> {
    const started = { ...githubPreferenceRevision }
    const generation = beginConfiguration('github')
    try {
      const result = await invoke<any>('create_github_repo', { repoName })
      if (!isCurrentConfiguration('github', generation)) return false
      clearProtocolUpgrade('github')
      applyLoadedConfig(() => {
        github.value = {
          configured: true, owner: result.owner, repo: result.repo,
          ...githubPreferencesAfterRequest(started), lastSyncTime: 0,
          historyUpdateMode: syncFrequency.value,
        }
      })
      return true
    } catch (e: any) {
      configurationFailed('github', generation, e, 'Failed to create repository')
      return false
    }
  }

  /** 使用已有仓库 */
  async function useExistingGitHubRepo(owner: string, repo: string): Promise<boolean> {
    const started = { ...githubPreferenceRevision }
    const generation = beginConfiguration('github')
    try {
      const result = await invoke<any>('use_existing_github_repo', { owner, repo })
      if (!isCurrentConfiguration('github', generation)) return false
      clearProtocolUpgrade('github')
      applyLoadedConfig(() => {
        github.value = {
          configured: true, owner: result.owner, repo: result.repo,
          ...githubPreferencesAfterRequest(started), lastSyncTime: 0,
          historyUpdateMode: syncFrequency.value,
        }
      })
      return true
    } catch (e: any) {
      configurationFailed('github', generation, e, 'Repository not found or inaccessible')
      return false
    }
  }

  /** 配置 GitHub 同步（一步到位，保留兼容） */
  async function configureGitHub(token: string, repo: string) {
    const started = { ...githubPreferenceRevision }
    const generation = beginConfiguration('github')
    try {
      const result = await invoke<any>('configure_github_sync', { token, repo })
      if (!isCurrentConfiguration('github', generation)) return false
      clearProtocolUpgrade('github')
      applyLoadedConfig(() => {
        github.value = {
          configured: true, owner: result.owner, repo: result.repo,
          ...githubPreferencesAfterRequest(started), lastSyncTime: 0,
          historyUpdateMode: syncFrequency.value,
        }
      })
      return true
    } catch (e: any) {
      configurationFailed('github', generation, e, 'Failed to configure GitHub sync')
      return false
    }
  }

  /** 执行 GitHub 同步。silent=true 时成功不弹 toast（自动同步场景） */
  async function syncGitHub(silent = false) {
    if (isSyncing.value) return
    if (silent && protocolUpgrades.value.some(item => item.backend === 'github')) return
    const generation = configurationGeneration.github
    const toast = useToastStore()
    const history = useHistoryStore()
    isSyncing.value = true
    try {
      const historyEpoch = history.syncSnapshotEpoch()
      const historySnapshot = history.getSyncSnapshot()
      const result = await invoke<any>('sync_github', {
        historyEntries: historySnapshot.entries,
        historyDeletions: historySnapshot.deletions,
      })
      applySyncedLyricOffsets(result)
      if (!isCurrentConfiguration('github', generation)) return
      clearProtocolUpgrade('github')
      if (result.deferred) {
        requestFollowUpSync(['github'])
        if (!silent) toast.show(t('settings.sync_deferred'))
        return
      }
      if (result.history) {
        const applied = await history.applySyncPayload(
          result.history,
          () => isCurrentConfiguration('github', generation),
          historyEpoch,
        )
        if (applied === 'deferred') requestFollowUpSync(['github'])
      }
      if (!isCurrentConfiguration('github', generation)) return
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
      if (!isCurrentConfiguration('github', generation)) return
      const upgrade = parseSyncProtocolUpgrade(e)
      if (upgrade) {
        rememberProtocolUpgrade(upgrade)
        return
      }
      const failure = parseSyncFailure(e)
      if (failure?.code === 'GITHUB_TOKEN_EXPIRED') {
        // 后端已清掉失效的 token：这类失败不会自己恢复，静默同步也要提示，再刷新成未连接状态
        toast.error(describeSyncError(e))
        await loadConfigs()
        return
      }
      if (failure?.code === 'GITHUB_RATE_LIMITED') {
        if (failure.automatic) scheduleRateLimitRetry(failure.retryAt)
        // 自动同步撞上限流不打扰用户，自动重试用完时只提示一次
        if (!silent || (!failure.automatic && rateLimitNoticeAt !== failure.retryAt)) {
          rateLimitNoticeAt = failure.retryAt
          toast.error(describeSyncError(e))
        }
        return
      }
      // 错误始终显示（除非 silentFailures 开启）
      if (!silent || !github.value.silentFailures) {
        toast.error(describeSyncError(e))
      }
    } finally {
      isSyncing.value = false
      scheduleFollowUpSync()
    }
  }

  /** 断开 GitHub 同步 */
  async function disconnectGitHub() {
    const generation = invalidateConfiguration('github')
    const toast = useToastStore()
    try {
      await invoke('disconnect_github_sync')
      if (!isCurrentConfiguration('github', generation)) return
      clearProtocolUpgrade('github')
      applyLoadedConfig(() => {
        github.value = {
          configured: false, owner: '', repo: '',
          autoSync: false, lastSyncTime: 0,
          dataSaver: true, silentFailures: false, historyUpdateMode: syncFrequency.value,
        }
      })
      toast.success(t('settings.github_disconnected'))
    } catch (e) {
      if (!isCurrentConfiguration('github', generation)) return
      log.error('disconnectGitHub:', e)
    }
  }

  /** 配置 WebDAV 同步 */
  async function configureWebDav(serverUrl: string, username: string, password: string, basePath?: string) {
    const started = webdavPreferenceRevision
    const generation = beginConfiguration('webdav')
    try {
      await invoke('configure_webdav_sync', { serverUrl, username, password, basePath })
      if (!isCurrentConfiguration('webdav', generation)) return false
      clearProtocolUpgrade('webdav')
      applyLoadedConfig(() => {
        webdav.value = {
          configured: true, serverUrl, basePath: basePath || '', lastSyncTime: 0,
          autoSync: webdavPreferenceRevision !== started ? webdav.value.autoSync : true,
        }
      })
      return true
    } catch (e: any) {
      configurationFailed('webdav', generation, e, 'Failed to configure WebDAV sync')
      return false
    }
  }

  /** 执行 WebDAV 同步。silent=true 时成功不弹 toast（自动同步场景） */
  async function syncWebDav(silent = false) {
    if (isSyncing.value) return
    if (silent && protocolUpgrades.value.some(item => item.backend === 'webdav')) return
    if (silent && webdavBlockedBy) return
    const generation = configurationGeneration.webdav
    const toast = useToastStore()
    const history = useHistoryStore()
    isSyncing.value = true
    try {
      const historyEpoch = history.syncSnapshotEpoch()
      const historySnapshot = history.getSyncSnapshot()
      const result = await invoke<any>('sync_webdav', {
        historyEntries: historySnapshot.entries,
        historyDeletions: historySnapshot.deletions,
      })
      applySyncedLyricOffsets(result)
      if (!isCurrentConfiguration('webdav', generation)) return
      clearProtocolUpgrade('webdav')
      webdavBlockedBy = null
      if (result.deferred) {
        requestFollowUpSync(['webdav'])
        if (!silent) toast.show(t('settings.sync_deferred'))
        return
      }
      if (result.history) {
        const applied = await history.applySyncPayload(
          result.history,
          () => isCurrentConfiguration('webdav', generation),
          historyEpoch,
        )
        if (applied === 'deferred') requestFollowUpSync(['webdav'])
      }
      if (!isCurrentConfiguration('webdav', generation)) return
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
      if (!isCurrentConfiguration('webdav', generation)) return
      const upgrade = parseSyncProtocolUpgrade(e)
      if (upgrade) {
        rememberProtocolUpgrade(upgrade)
        return
      }
      const failure = parseSyncFailure(e)
      if (isWebDavFailure(failure)) {
        webdavBlockedBy = failure.code
        const reason = describeSyncError(e)
        toast.error(webdav.value.autoSync ? t('settings.webdav_auto_sync_paused', { reason }) : reason)
        return
      }
      if (!webdav.value.autoSync || !silent) {
        toast.error(describeSyncError(e))
      }
    } finally {
      isSyncing.value = false
      scheduleFollowUpSync()
    }
  }

  /** 断开 WebDAV 同步 */
  async function disconnectWebDav() {
    const generation = invalidateConfiguration('webdav')
    const toast = useToastStore()
    try {
      await invoke('disconnect_webdav_sync')
      if (!isCurrentConfiguration('webdav', generation)) return
      clearProtocolUpgrade('webdav')
      applyLoadedConfig(() => {
        webdav.value = { configured: false, serverUrl: '', basePath: '', autoSync: false, lastSyncTime: 0 }
      })
      toast.success(t('settings.webdav_disconnected'))
    } catch (e) {
      if (!isCurrentConfiguration('webdav', generation)) return
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

  let rateLimitTimer: ReturnType<typeof setTimeout> | null = null
  let rateLimitRetryAt = 0
  let rateLimitNoticeAt = 0

  /** 限流冷却结束时自动重试 GitHub 同步（对齐 Android GitHubRateLimitedWorkerHost） */
  function scheduleRateLimitRetry(retryAt: number) {
    if (rateLimitTimer && rateLimitRetryAt >= retryAt) return
    if (rateLimitTimer) clearTimeout(rateLimitTimer)
    rateLimitRetryAt = retryAt
    rateLimitTimer = setTimeout(() => {
      rateLimitTimer = null
      if (!github.value.configured || !github.value.autoSync) return
      if (isSyncing.value) requestFollowUpSync(['github'])
      else void syncGitHub(true)
    }, Math.max(1_000, retryAt - Date.now()))
  }

  const followUpBackends = new Set<SyncBackend>()
  let followUpTimer: ReturnType<typeof setTimeout> | null = null

  /** 合并结果校正过的逐曲歌词偏移已经写进数据库，内存里的映射跟上（不论账号配置是否已变化） */
  function applySyncedLyricOffsets(result: { lyricOffsets?: Record<string, number> } | null | undefined) {
    if (result?.lyricOffsets) useLyricOffsetStore().replaceFromSync(result.lyricOffsets)
  }

  /** 记下需要补同步的提供商，不传时取所有已开启自动同步的提供商 */
  function requestFollowUpSync(backends?: SyncBackend[]) {
    const requested = backends ?? [
      ...(github.value.configured && github.value.autoSync ? ['github' as const] : []),
      ...(webdav.value.configured && webdav.value.autoSync ? ['webdav' as const] : []),
    ]
    for (const backend of requested) followUpBackends.add(backend)
    scheduleFollowUpSync()
  }

  function scheduleFollowUpSync() {
    if (followUpBackends.size === 0 || followUpTimer || isSyncing.value) return
    followUpTimer = setTimeout(() => {
      followUpTimer = null
      // 计时期间又开始了同步，由它结束时重新安排
      if (isSyncing.value) return
      const backends = [...followUpBackends]
      followUpBackends.clear()
      void (async () => {
        if (backends.includes('github') && github.value.configured) await syncGitHub(true)
        if (backends.includes('webdav') && webdav.value.configured) await syncWebDav(true)
      })()
    }, FOLLOW_UP_SYNC_DELAY_MS)
  }

  /** 旧客户端读不懂升级后的归档，必须先确认所有设备都已更新（对齐 Android SyncProtocolUpgradeDialog） */
  async function approveProtocolUpgrade(allDevicesUpdated: boolean) {
    const challenge = pendingProtocolUpgrade.value
    if (!challenge || isSyncing.value || !allDevicesUpdated) return
    const generation = configurationGeneration[challenge.backend]
    upgradeError.value = null
    isSyncing.value = true
    try {
      await invoke('approve_sync_protocol_upgrade', { challenge, allDevicesUpdated })
      if (!isCurrentConfiguration(challenge.backend, generation)) return
      clearProtocolUpgrade(challenge.backend)
      isSyncing.value = false
      if (challenge.backend === 'github') await syncGitHub()
      else await syncWebDav()
    } catch (error) {
      if (!isCurrentConfiguration(challenge.backend, generation)) return
      upgradeError.value = describeUpgradeFailure(error)
    } finally {
      isSyncing.value = false
      scheduleFollowUpSync()
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
        toast.success(t('settings.import_success', { count: result.imported }))
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
    const generations = {
      github: invalidateConfiguration('github'),
      webdav: invalidateConfiguration('webdav'),
    }
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
      await auth.checkStatus()
      await loadConfigs(generations)
      toast.success(t('settings.import_config_success'))
      return result
    } catch (e: any) {
      toast.error(e?.toString() || t('settings.import_config_failed'))
      return { success: false }
    }
  }

  return {
    github, webdav, syncFrequency, isSyncing, lastResult, dialogError,
    pendingProtocolUpgrade, upgradeError, approveProtocolUpgrade,
    loadConfigs,
    validateGitHubToken, createGitHubRepo, useExistingGitHubRepo,
    configureGitHub, syncGitHub, syncAuto, requestFollowUpSync, disconnectGitHub,
    configureWebDav, syncWebDav, disconnectWebDav,
    exportPlaylists, importPlaylists, exportConfig, importConfig,
  }
})
