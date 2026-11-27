import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  shouldAutoCheck,
  shouldPrompt,
  updateErrorCode,
  updateErrorMessage,
  type UpdateCheck,
  type UpdateErrorCode,
  type UpdateProgress,
} from '@/modules/update/updateModel'
import { createLogger } from '@/utils/logger'
import i18n from '@/i18n'
import { useToastStore } from '@/stores/toast'

const log = createLogger('update')

export type UpdatePhase =
  | 'idle' | 'checking' | 'upToDate' | 'available' | 'downloading' | 'ready' | 'installing' | 'error' | 'disabled'

const PROGRESS_EVENT = 'update:progress'
// 只是本机偏好，不进设置备份、不跨设备同步，所以放 localStorage
const AUTO_CHECK_KEY = 'neri.update.autoCheck'
const SKIPPED_KEY = 'neri.update.skippedVersion'
const LAST_CHECK_KEY = 'neri.update.lastCheckAt'

function readItem(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeItem(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key)
    else localStorage.setItem(key, value)
  } catch {
    // 隐私模式或存储被禁用时偏好只在本次运行内生效
  }
}

export const useUpdateStore = defineStore('update', () => {
  const phase = ref<UpdatePhase>('idle')
  const check = ref<UpdateCheck | null>(null)
  const progress = ref<UpdateProgress | null>(null)
  const errorCode = ref<UpdateErrorCode | null>(null)
  /** 出错的环节，决定界面上「重试」做什么 */
  const errorStage = ref<'check' | 'download' | 'install' | null>(null)
  const dialogOpen = ref(false)
  // 默认关闭：只有用户在设置里打开过才在启动时联网检查
  const autoCheck = ref(readItem(AUTO_CHECK_KEY) === '1')
  const skippedVersion = ref(readItem(SKIPPED_KEY))
  const lastCheckAt = ref<number | null>(Number(readItem(LAST_CHECK_KEY)) || null)

  const busy = computed(() => ['checking', 'downloading', 'installing'].includes(phase.value))
  const hasUpdate = computed(() => check.value?.status === 'available')

  function fail(stage: 'check' | 'download' | 'install', error: unknown) {
    errorCode.value = updateErrorCode(error)
    errorStage.value = stage
    phase.value = 'error'
    log.warn(`update ${stage} failed: ${updateErrorMessage(error)}`)
  }

  // 启动时发现新版本只弹一条可点的通知，不直接弹窗打断正在做的事
  function notifyAvailable(version: string) {
    const t = i18n.global.t
    useToastStore().show(t('update.toast_available', { version }), 'info', {
      duration: 10_000,
      action: { label: t('update.view'), handler: () => { dialogOpen.value = true } },
    })
  }

  async function checkForUpdates(options: { manual: boolean }) {
    if (busy.value) return
    phase.value = 'checking'
    errorCode.value = null
    try {
      const result = await invoke<UpdateCheck>('update_check')
      check.value = result
      lastCheckAt.value = Date.now()
      writeItem(LAST_CHECK_KEY, String(lastCheckAt.value))
      phase.value = result.status === 'available' ? 'available' : result.status
      if (shouldPrompt(result, skippedVersion.value, options.manual)) {
        if (options.manual) dialogOpen.value = true
        else notifyAvailable(result.latestVersion ?? '')
      }
    } catch (error) {
      if (options.manual) {
        fail('check', error)
      } else {
        // 启动时的静默检查失败（离线、被墙）不打扰用户，回到初始状态
        phase.value = 'idle'
        log.info(`background update check failed: ${updateErrorMessage(error)}`)
      }
    }
  }

  async function download() {
    if (busy.value || !check.value?.inApp) return
    phase.value = 'downloading'
    errorCode.value = null
    progress.value = { downloaded: 0, total: check.value.summary?.downloadBytes ?? 0 }
    let unlisten: UnlistenFn | null = null
    try {
      unlisten = await listen<UpdateProgress>(PROGRESS_EVENT, event => { progress.value = event.payload })
      await invoke('update_download')
      phase.value = 'ready'
    } catch (error) {
      if (updateErrorCode(error) === 'cancelled') phase.value = 'available'
      else fail('download', error)
    } finally {
      unlisten?.()
    }
  }

  async function cancelDownload() {
    if (phase.value !== 'downloading') return
    try {
      await invoke('update_cancel')
    } catch (error) {
      log.warn(`cancel update failed: ${updateErrorMessage(error)}`)
    }
  }

  /** 成功时后端会重启应用，这里不会再收到结果 */
  async function install() {
    if (phase.value !== 'ready') return
    phase.value = 'installing'
    try {
      await invoke('update_install')
    } catch (error) {
      fail('install', error)
    }
  }

  function retry() {
    // 后端没有可用的更新会话（例如应用已重启）时，下载一定会失败，先重新检查
    if (errorCode.value !== 'noUpdate' && (errorStage.value === 'download' || errorStage.value === 'install')) {
      // 暂存文件已校验的部分会被复用，重新下载只补缺的
      phase.value = 'available'
      void download()
    } else {
      void checkForUpdates({ manual: true })
    }
  }

  function skipCurrentVersion() {
    skippedVersion.value = check.value?.latestVersion ?? null
    writeItem(SKIPPED_KEY, skippedVersion.value)
    dialogOpen.value = false
  }

  function setAutoCheck(enabled: boolean) {
    autoCheck.value = enabled
    writeItem(AUTO_CHECK_KEY, enabled ? '1' : '0')
  }

  async function checkOnLaunch() {
    if (!autoCheck.value || !shouldAutoCheck(lastCheckAt.value, Date.now())) return
    await checkForUpdates({ manual: false })
  }

  function openDialog() {
    dialogOpen.value = true
  }

  return {
    phase, check, progress, errorCode, errorStage, dialogOpen, autoCheck, skippedVersion, lastCheckAt,
    busy, hasUpdate,
    checkForUpdates, download, cancelDownload, install, retry, skipCurrentVersion, setAutoCheck, checkOnLaunch, openDialog,
  }
})
