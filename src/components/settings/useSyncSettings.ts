import { computed, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useSettingsStore } from '@/stores/settings'
import { useSyncStore, type SyncFrequency } from '@/stores/sync'
import { usePlayerStore } from '@/stores/player'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { applyTheme } from '@/utils/theme'
import { applyThemeColor } from '@/utils/themeColor'

const GITHUB_TOKEN_URL = 'https://github.com/settings/tokens/new?scopes=repo&description=NeriPlayer%20Backup'

/** 解析 owner/repo 格式的仓库名，格式不符时返回 null */
export function parseGitHubRepo(value: string): { owner: string; repo: string } | null {
  const parts = value.split('/')
  return parts.length === 2 && parts[0] && parts[1] ? { owner: parts[0], repo: parts[1] } : null
}

export function formatSyncTime(ms: number, locale: string): string {
  if (!ms) return ''
  return new Date(ms).toLocaleString(locale)
}

/** 备份与同步分区：GitHub / WebDAV 配置流程、协议升级确认与配置导入导出 */
export function useSyncSettings(openExternalUrl: (url: string) => Promise<void>) {
  const { t, locale } = useI18n()
  const syncStore = useSyncStore()
  const player = usePlayerStore()
  const lt = useListenTogetherStore()
  const { darkMode, colorMode, themeColor: selectedColor } = storeToRefs(useSettingsStore())

  const syncFrequencyOptions = computed<Array<{ value: SyncFrequency; label: string }>>(() => [
    { value: 'immediate', label: t('settings.sync_immediate') },
    { value: 'every_10_minutes', label: t('settings.sync_every_10_minutes') },
    { value: 'every_15_minutes', label: t('settings.sync_every_15_minutes') },
    { value: 'every_30_minutes', label: t('settings.sync_every_30_minutes') },
  ])

  const configTransferMode = ref<'export' | 'import' | null>(null)

  // 导入只写回了设置值：主题、强调色和播放引擎（均衡器、响度、倍速等）要按新值重新应用
  async function applyImportedConfig() {
    applyTheme(darkMode.value, false)
    // 三态取色：仅「默认取色」需要按预设主题色重刷，system/cover 由动态取色路径接管
    if (colorMode.value === 'default') applyThemeColor(selectedColor.value, undefined, false)
    lt.reloadIdentity()
    await player.applyPersistedSettings()
  }

  // GitHub 同步引导（对齐 Android）
  const showGitHubDialog = ref(false)
  const githubPhase = ref<1 | 2>(1) // 当前步骤：1=token 验证，2=仓库选择
  const githubToken = ref('')
  const githubUsername = ref('')
  const githubIsValidating = ref(false)
  const githubRepoMode = ref<'create' | 'existing'>('create')
  const githubNewRepoName = ref('neriplayer-backup')
  const githubExistingRepo = ref('') // owner/repo 格式
  const githubIsSettingRepo = ref(false)

  function openGitHubSetup() {
    githubPhase.value = 1
    githubToken.value = ''
    githubUsername.value = ''
    githubRepoMode.value = 'create'
    githubNewRepoName.value = 'neriplayer-backup'
    githubExistingRepo.value = ''
    syncStore.dialogError = null
    showGitHubDialog.value = true
  }

  async function githubValidateToken() {
    if (!githubToken.value.trim()) return
    githubIsValidating.value = true
    syncStore.dialogError = null
    const username = await syncStore.validateGitHubToken(githubToken.value)
    githubIsValidating.value = false
    if (username) {
      githubUsername.value = username
      githubPhase.value = 2
    }
  }

  async function githubFinishSetup() {
    githubIsSettingRepo.value = true
    syncStore.dialogError = null
    let ok = false
    if (githubRepoMode.value === 'create') {
      ok = await syncStore.createGitHubRepo(githubNewRepoName.value || 'neriplayer-backup')
    } else {
      const repo = parseGitHubRepo(githubExistingRepo.value)
      if (repo) {
        ok = await syncStore.useExistingGitHubRepo(repo.owner, repo.repo)
      } else {
        syncStore.dialogError = t('settings.github_repo_format_hint')
      }
    }
    githubIsSettingRepo.value = false
    if (ok) showGitHubDialog.value = false
  }

  async function openGitHubTokenPage() {
    await openExternalUrl(GITHUB_TOKEN_URL)
  }

  // WebDAV 同步对话框状态
  const showWebDavDialog = ref(false)
  const webdavUrl = ref('')
  const webdavUsername = ref('')
  const webdavPassword = ref('')
  const webdavBasePath = ref('')
  const webdavConfiguring = ref(false)

  async function configureWebDav() {
    if (!webdavUrl.value.trim() || !webdavUsername.value.trim() || !webdavPassword.value) return
    webdavConfiguring.value = true
    const ok = await syncStore.configureWebDav(
      webdavUrl.value, webdavUsername.value, webdavPassword.value, webdavBasePath.value || undefined,
    )
    webdavConfiguring.value = false
    if (ok) {
      showWebDavDialog.value = false
      webdavUrl.value = ''
      webdavUsername.value = ''
      webdavPassword.value = ''
      webdavBasePath.value = ''
    }
  }

  // 清除 GitHub 配置确认
  const showClearGitHubConfirm = ref(false)
  async function confirmClearGitHub() {
    showClearGitHubConfirm.value = false
    await syncStore.disconnectGitHub()
  }

  const hideProtocolUpgrade = ref(false)
  const upgradeDevicesConfirmed = ref(false)
  watch(() => syncStore.pendingProtocolUpgrade, () => {
    hideProtocolUpgrade.value = false
    upgradeDevicesConfirmed.value = false
  })

  return {
    syncFrequencyOptions,
    configTransferMode,
    applyImportedConfig,
    showGitHubDialog,
    githubPhase,
    githubToken,
    githubUsername,
    githubIsValidating,
    githubRepoMode,
    githubNewRepoName,
    githubExistingRepo,
    githubIsSettingRepo,
    openGitHubSetup,
    githubValidateToken,
    githubFinishSetup,
    openGitHubTokenPage,
    showWebDavDialog,
    webdavUrl,
    webdavUsername,
    webdavPassword,
    webdavBasePath,
    webdavConfiguring,
    configureWebDav,
    formatSyncTime: (ms: number) => formatSyncTime(ms, locale.value),
    showClearGitHubConfirm,
    confirmClearGitHub,
    hideProtocolUpgrade,
    upgradeDevicesConfirmed,
  }
}

export type SyncSettings = ReturnType<typeof useSyncSettings>
