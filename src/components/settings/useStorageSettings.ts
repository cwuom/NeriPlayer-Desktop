import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { invalidateBiliSubtitleCache } from '@/modules/lyrics/biliSubtitleLoader'
import {
  clearBrowserCache,
  mergeBrowserCacheUsage,
  type StorageCacheClearOptions,
  type StorageUsageSummary,
} from '@/utils/storage'
import { settingsLog as log, type SettingsIpc } from './settingsShared'

/** 存储与日志分区：缓存占用 / 清理，以及日志级别与日志目录 */
export function useStorageSettings(ipc: SettingsIpc) {
  const { t } = useI18n()
  const toast = useToastStore()
  const { downloadDir } = storeToRefs(useSettingsStore())

  const logLevelOptions = computed<Array<{ value: string; label: string }>>(() => [
    { value: 'off', label: t('settings.log_level_off') },
    { value: 'error', label: t('settings.log_level_error') },
    { value: 'warn', label: t('settings.log_level_warn') },
    { value: 'info', label: t('settings.log_level_info') },
    { value: 'debug', label: t('settings.log_level_debug') },
    { value: 'trace', label: t('settings.log_level_trace') },
  ])

  async function openLogDir() {
    try {
      const dir = await ipc.getLogDir()
      // 走后端 reveal_in_file_manager，避免依赖 opener:allow-reveal-item-in-dir 权限
      // （capability 只授了 opener:allow-open-url，revealItemInDir 会被 ACL 拒绝）
      await ipc.revealInFileManager(dir)
    } catch (error) {
      log.error('failed to open log dir:', error)
      toast.error(t('settings.open_log_dir_failed'))
    }
  }

  // 文件日志开关仅在下次启动生效（受插件限制）：用户切换时提示需重启。
  // 用 @change 而非 watch，避免启动 hydrate 时误触发提示
  function onLogToFileChange() {
    toast.show(t('settings.log_to_file_restart_hint'), 'info')
  }

  const showStorageManagement = ref(false)
  const storageLoading = ref(false)
  const storageClearing = ref(false)
  const storageSummary = ref<StorageUsageSummary | null>(null)

  async function loadStorageUsage() {
    storageLoading.value = true
    try {
      const result = await ipc.getStorageUsage(downloadDir.value || null)
      storageSummary.value = mergeBrowserCacheUsage(result)
    } catch (error) {
      log.error('Failed to read storage usage:', error)
      toast.error(t('settings.storage_load_failed'))
    } finally {
      storageLoading.value = false
    }
  }

  async function openStorageManagement() {
    showStorageManagement.value = true
    await loadStorageUsage()
  }

  async function clearStorageCache(options: StorageCacheClearOptions) {
    storageClearing.value = true
    try {
      const result = await ipc.clearStorageCache(options, downloadDir.value || null)
      clearBrowserCache(options)
      if (options.lyricsCache) invalidateBiliSubtitleCache()
      const mb = ((result.clearedBytes || 0) / 1024 / 1024).toFixed(1)
      if (result.failedCount > 0) {
        toast.show(t('settings.storage_cache_partial', { mb, count: result.failedCount }), 'info')
      } else if (result.clearedBytes === 0) {
        toast.show(t('settings.storage_cache_empty'), 'info')
      } else {
        toast.success(t('settings.storage_cache_cleared', { mb }))
      }
      await loadStorageUsage()
    } catch (error) {
      log.error('Failed to clear storage cache:', error)
      toast.error(t('settings.storage_cache_failed'))
    } finally {
      storageClearing.value = false
    }
  }

  return {
    logLevelOptions,
    openLogDir,
    onLogToFileChange,
    showStorageManagement,
    storageLoading,
    storageClearing,
    storageSummary,
    openStorageManagement,
    clearStorageCache,
  }
}
