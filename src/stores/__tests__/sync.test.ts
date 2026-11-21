import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  FOLLOW_UP_SYNC_DELAY_MS,
  describeSyncError,
  normalizeSyncFrequency,
  parseSyncFailure,
  useSyncStore,
} from '@/stores/sync'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  toasts: [] as string[],
  appliedHistory: [] as Array<{ payload: unknown; epoch: number | undefined; current: boolean }>,
  historyOutcome: 'applied' as 'applied' | 'deferred' | 'skipped',
  lyricOffsets: [] as unknown[],
  skipRules: [] as unknown[],
  appliedSettings: [] as unknown[],
  locales: [] as string[],
  authChecks: [] as unknown[],
  authNotified: [] as string[],
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@/i18n', () => ({
  default: {
    global: {
      t: (key: string, params?: Record<string, unknown>) =>
        (params && Object.keys(params).length ? `${key}|${JSON.stringify(params)}` : key),
      locale: { value: 'en' },
    },
  },
  setLocale: (locale: string) => { mocks.locales.push(locale) },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({
    show: (text: string, type = 'info') => { mocks.toasts.push(`${type}:${text}`) },
    success: (text: string) => { mocks.toasts.push(`success:${text}`) },
    error: (text: string) => { mocks.toasts.push(`error:${text}`) },
  }),
}))
vi.mock('@/stores/history', () => ({
  useHistoryStore: () => ({
    syncSnapshotEpoch: () => 7,
    getSyncSnapshot: () => ({ entries: [{ playedAt: 1 }], deletions: [] }),
    applySyncPayload: async (payload: unknown, isCurrent: () => boolean, epoch?: number) => {
      mocks.appliedHistory.push({ payload, epoch, current: isCurrent() })
      return mocks.historyOutcome
    },
  }),
}))
vi.mock('@/stores/lyricOffset', () => ({
  useLyricOffsetStore: () => ({ replaceFromSync: (value: unknown) => { mocks.lyricOffsets.push(value) } }),
}))
vi.mock('@/stores/biliVideoSkip', () => ({
  useBiliVideoSkipStore: () => ({ replaceFromSync: (value: unknown) => { mocks.skipRules.push(value) } }),
}))
vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => ({
    snapshot: () => ({ locale: 'en' }),
    applySnapshot: (value: unknown) => { mocks.appliedSettings.push(value) },
  }),
}))
vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({
    checkStatus: async (options: unknown) => { mocks.authChecks.push(options) },
    notifyAuthChanged: (platform: string) => { mocks.authNotified.push(platform) },
  }),
}))

const githubConfig = {
  configured: true, owner: 'me', repo: 'backup', autoSync: true, lastSyncTime: 10,
  dataSaver: false, silentFailures: false, historyUpdateMode: 'immediate',
}
const webdavConfig = { configured: false, serverUrl: '', basePath: '', autoSync: false, lastSyncTime: 0 }

describe('sync error helpers', () => {
  it('parses GitHub rate limits and WebDAV failure codes from backend messages', () => {
    expect(parseSyncFailure('Error: GITHUB_RATE_LIMITED:1700000000000:1')).toEqual({
      code: 'GITHUB_RATE_LIMITED', retryAt: 1700000000000, automatic: true,
    })
    expect(parseSyncFailure('GITHUB_RATE_LIMITED:5:0')).toMatchObject({ automatic: false })
    expect(parseSyncFailure('GITHUB_TOKEN_EXPIRED')).toEqual({ code: 'GITHUB_TOKEN_EXPIRED' })
    expect(parseSyncFailure('plain failure')).toBeNull()
    expect(parseSyncFailure(undefined)).toBeNull()
  })

  it('localizes coded failures and keeps other messages', () => {
    expect(describeSyncError('GITHUB_TOKEN_EXPIRED')).toBe('settings.github_token_expired')
    expect(describeSyncError('GITHUB_RATE_LIMITED:0:0')).toMatch(/^settings\.github_rate_limited_stopped\|/)
    expect(describeSyncError('GITHUB_RATE_LIMITED:0:1')).toMatch(/^settings\.github_rate_limited\|/)
    expect(describeSyncError('remote rejected push')).toBe('remote rejected push')
    expect(describeSyncError('', 'fallback')).toBe('fallback')
  })

  it('maps legacy frequency names onto the current options', () => {
    expect(normalizeSyncFrequency('batched')).toBe('every_10_minutes')
    expect(normalizeSyncFrequency(' BATCHED_15 ')).toBe('every_15_minutes')
    expect(normalizeSyncFrequency('batched_30')).toBe('every_30_minutes')
    expect(normalizeSyncFrequency(null)).toBe('immediate')
    expect(normalizeSyncFrequency('weekly')).toBe('immediate')
  })
})

describe('sync store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let storage: MemoryStorage

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    mocks.toasts = []
    mocks.appliedHistory = []
    mocks.historyOutcome = 'applied'
    mocks.lyricOffsets = []
    mocks.skipRules = []
    mocks.appliedSettings = []
    mocks.locales = []
    mocks.authChecks = []
    mocks.authNotified = []
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    router.on('get_github_sync_config', () => githubConfig)
    router.on('get_webdav_sync_config', () => webdavConfig)
    router.on('get_sync_preferences', () => ({ historyUpdateMode: 'immediate' }))
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  describe('loadConfigs', () => {
    it('maps backend configuration without echoing it back as a user change', async () => {
      router.set('get_sync_preferences', () => ({ historyUpdateMode: 'batched_15' }))
      const sync = useSyncStore()

      await sync.loadConfigs()

      expect(sync.github).toEqual({
        configured: true, owner: 'me', repo: 'backup', autoSync: true, lastSyncTime: 10,
        dataSaver: false, silentFailures: false, historyUpdateMode: 'every_15_minutes',
      })
      expect(sync.syncFrequency).toBe('every_15_minutes')
      // 加载期间的赋值不能触发保存，否则会把刚读到的值又写回后端
      expect(router.callsOf('update_sync_preferences')).toEqual([])
      expect(router.callsOf('update_github_sync_settings')).toEqual([])
    })

    it('falls back to the legacy GitHub frequency when preferences are unavailable', async () => {
      router.set('get_github_sync_config', () => ({ ...githubConfig, historyUpdateMode: 'batched_30' }))
      router.set('get_sync_preferences', () => { throw new Error('unknown command') })
      const sync = useSyncStore()

      await sync.loadConfigs()

      expect(sync.syncFrequency).toBe('every_30_minutes')
    })

    it('ignores a configuration read that finishes after a newer one', async () => {
      const stale = deferred<typeof githubConfig>()
      router.set('get_github_sync_config', () => stale.promise, () => ({ ...githubConfig, repo: 'fresh' }))
      const sync = useSyncStore()

      const first = sync.loadConfigs()
      await sync.loadConfigs()
      stale.resolve({ ...githubConfig, repo: 'stale' })
      await first

      expect(sync.github.repo).toBe('fresh')
    })
  })

  describe('syncGitHub', () => {
    const remoteHistory = { entries: [{ track: { id: 'netease:1' }, playedAt: 5 }], deletions: [] }

    it('applies the merged result and reports success', async () => {
      router.on('sync_github', () => ({
        success: true, message: 'ok',
        playlists_added: 2, playlistsUpdated: 3, songsAdded: 4,
        history: remoteHistory,
        lyricOffsets: { 'netease:1': 300 },
      }))
      const sync = useSyncStore()

      await sync.syncGitHub()

      expect(router.callsOf('sync_github')[0].args).toEqual({ historyEntries: [{ playedAt: 1 }], historyDeletions: [] })
      expect(mocks.appliedHistory).toEqual([{ payload: remoteHistory, epoch: 7, current: true }])
      expect(mocks.lyricOffsets).toEqual([{ 'netease:1': 300 }])
      expect(sync.lastResult).toEqual({
        success: true, message: 'ok',
        playlistsAdded: 2, playlistsUpdated: 3, playlistsDeleted: 0, songsAdded: 4, songsRemoved: 0,
      })
      expect(mocks.toasts).toEqual(['success:settings.github_sync_success'])
      expect(sync.isSyncing).toBe(false)
      // 成功后刷新一次配置（拿到新的 lastSyncTime）
      expect(router.callsOf('get_github_sync_config')).toHaveLength(1)
    })

    it('drops the result when GitHub is disconnected mid-sync but still keeps lyric offsets', async () => {
      const pending = deferred<unknown>()
      router.on('sync_github', () => pending.promise)
      router.on('disconnect_github_sync', () => undefined)
      const sync = useSyncStore()

      const running = sync.syncGitHub()
      await flushPromises()
      await sync.disconnectGitHub()
      pending.resolve({ success: true, message: 'late', history: remoteHistory, lyricOffsets: { 'netease:1': 100 } })
      await running

      expect(mocks.appliedHistory).toEqual([])
      expect(sync.lastResult).toBeNull()
      expect(mocks.toasts).toEqual(['success:settings.github_disconnected'])
      // 偏移已由后端落库，内存映射必须跟上，与账号配置是否变化无关
      expect(mocks.lyricOffsets).toEqual([{ 'netease:1': 100 }])
      expect(sync.github.configured).toBe(false)
    })

    it('defers to a silent follow-up sync when the backend saw local changes', async () => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
      router.on('sync_github',
        () => ({ success: true, message: '', deferred: true, history: remoteHistory }),
        () => ({ success: true, message: 'second' }))
      const sync = useSyncStore()
      await sync.loadConfigs()

      await sync.syncGitHub()
      expect(mocks.appliedHistory).toEqual([])
      expect(mocks.toasts).toEqual(['info:settings.sync_deferred'])

      await vi.advanceTimersByTimeAsync(FOLLOW_UP_SYNC_DELAY_MS)
      expect(router.callsOf('sync_github')).toHaveLength(2)
      // 补同步是静默的，成功不再弹提示
      expect(mocks.toasts).toEqual(['info:settings.sync_deferred'])
      expect(sync.lastResult?.message).toBe('second')
    })

    it('always reports an expired token, even for silent syncs', async () => {
      router.on('sync_github', () => { throw 'GITHUB_TOKEN_EXPIRED' })
      const sync = useSyncStore()

      await sync.syncGitHub(true)

      expect(mocks.toasts).toEqual(['error:settings.github_token_expired'])
      expect(router.callsOf('get_github_sync_config')).toHaveLength(1)
    })

    it('stays quiet for silent failures when silentFailures is enabled', async () => {
      router.set('get_github_sync_config', () => ({ ...githubConfig, silentFailures: true }))
      router.on('sync_github', () => { throw 'network unreachable' })
      const sync = useSyncStore()
      await sync.loadConfigs()

      await sync.syncGitHub(true)
      expect(mocks.toasts).toEqual([])

      await sync.syncGitHub(false)
      expect(mocks.toasts).toEqual(['error:network unreachable'])
    })

    it('holds a protocol upgrade for confirmation and skips silent syncs until approved', async () => {
      const challenge = { backend: 'github', target: 'me/backup', fingerprint: 'a'.repeat(64), sourceProtocol: 3, targetProtocol: 4 }
      router.on('sync_github', () => { throw `SYNC_PROTOCOL_UPGRADE_REQUIRED:${JSON.stringify(challenge)}` })
      const sync = useSyncStore()

      await sync.syncGitHub(true)
      expect(sync.pendingProtocolUpgrade).toMatchObject({ backend: 'github', target: 'me/backup' })
      expect(mocks.toasts).toEqual([])

      await sync.syncGitHub(true)
      expect(router.callsOf('sync_github')).toHaveLength(1)
    })
  })

  describe('GitHub configuration dialog', () => {
    it('returns the validated username', async () => {
      router.on('validate_github_token', () => ({ success: true, username: 'octo' }))
      const sync = useSyncStore()

      await expect(sync.validateGitHubToken('token')).resolves.toBe('octo')
    })

    it('discards a validation that was replaced by a newer configuration step', async () => {
      const validation = deferred<unknown>()
      router.on('validate_github_token', () => validation.promise)
      router.on('create_github_repo', () => ({ success: true, owner: 'octo', repo: 'neri' }))
      const sync = useSyncStore()

      const validating = sync.validateGitHubToken('old-token')
      await flushPromises()
      await expect(sync.createGitHubRepo('neri')).resolves.toBe(true)
      validation.resolve({ success: true, username: 'stale' })

      await expect(validating).resolves.toBeNull()
      expect(sync.github).toMatchObject({ configured: true, owner: 'octo', repo: 'neri' })
    })

    it('shows a localized dialog error when the token is rejected', async () => {
      router.on('validate_github_token', () => { throw 'GITHUB_TOKEN_EXPIRED' })
      const sync = useSyncStore()

      await expect(sync.validateGitHubToken('bad')).resolves.toBeNull()

      expect(sync.dialogError).toBe('settings.github_token_expired')
    })
  })

  describe('config import and export', () => {
    it('returns password and Android confirmation prompts without applying anything', async () => {
      router.on('import_config',
        () => ({ success: false, requiresPassword: true, filePath: '/tmp/a.json' }),
        () => ({ success: false, requiresConfirmation: true, filePath: '/tmp/a.json', platform: 'android' }))
      const sync = useSyncStore()

      await expect(sync.importConfig()).resolves.toEqual({ success: false, requiresPassword: true, filePath: '/tmp/a.json' })
      await expect(sync.importConfig({ filePath: '/tmp/a.json', password: 'pw' })).resolves.toMatchObject({ requiresConfirmation: true })

      expect(router.callsOf('import_config')[1].args).toEqual({ filePath: '/tmp/a.json', password: 'pw' })
      expect(mocks.appliedSettings).toEqual([])
      expect(mocks.authChecks).toEqual([])
    })

    it('applies imported settings, locale, listen-together identity and account changes', async () => {
      router.on('import_config', () => ({
        success: true, settings: { locale: 'ja', volume: 0.5 }, listenTogetherUserUuid: 'lt-1',
        platform: 'pc', changedAuthPlatforms: ['netease'],
      }))
      const sync = useSyncStore()

      const result = await sync.importConfig({ confirmedAndroid: true })

      expect(result.success).toBe(true)
      expect(mocks.appliedSettings).toEqual([{ locale: 'ja', volume: 0.5 }])
      expect(mocks.locales).toEqual(['ja'])
      expect(storage.getItem('neri:lt-uuid')).toBe('lt-1')
      expect(mocks.authChecks).toEqual([{ invalidatePending: ['netease'] }])
      expect(mocks.authNotified).toEqual(['netease', 'bilibili', 'youtube'])
      expect(mocks.toasts).toEqual(['success:settings.import_config_success'])
    })

    it('clears a stale listen-together identity when the import has none', async () => {
      storage.setItem('neri:lt-uuid', 'old')
      router.on('import_config', () => ({ success: true, settings: {}, listenTogetherUserUuid: '', platform: 'pc' }))
      const sync = useSyncStore()

      await sync.importConfig()

      expect(storage.getItem('neri:lt-uuid')).toBeNull()
      expect(mocks.locales).toEqual([])
    })

    it('maps backend error codes to localized messages', async () => {
      router.on('import_config', () => { throw 'Error: CONFIG_PASSWORD_INVALID' })
      router.on('export_config', () => { throw 'disk full' })
      const sync = useSyncStore()

      await expect(sync.importConfig()).resolves.toEqual({ success: false, error: 'settings.config_password_invalid' })
      await expect(sync.exportConfig()).resolves.toEqual({ success: false, error: 'settings.export_config_failed' })
      expect(mocks.toasts).toEqual(['error:settings.config_password_invalid', 'error:settings.export_config_failed'])
    })

    it('sends the settings snapshot and listen-together identity on export', async () => {
      storage.setItem('neri:lt-uuid', 'lt-9')
      router.on('export_config', () => ({ success: true, platform: 'pc', platformName: 'NeriPlayer Desktop' }))
      const sync = useSyncStore()

      await expect(sync.exportConfig({ password: 'pw' })).resolves.toMatchObject({ success: true })

      expect(router.callsOf('export_config')[0].args).toEqual({
        settings: { locale: 'en' }, listenTogetherUserUuid: 'lt-9', password: 'pw', includeLoginState: false,
      })
    })
  })
})
