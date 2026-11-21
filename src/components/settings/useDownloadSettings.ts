import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import { DEFAULT_DOWNLOAD_NAME_TEMPLATE, useSettingsStore } from '@/stores/settings'
import { useDownloadStore } from '@/stores/download'
import { useToastStore } from '@/stores/toast'
import { dialogSelectionPath, settingsLog as log, type SettingsIpc, type Translate } from './settingsShared'

export interface DownloadTaskProgress {
  status: string
  progress?: number
  downloadedBytes?: number
  totalBytes?: number
  message?: string
}

export function formatDownloadSize(bytes?: number): string {
  if (typeof bytes !== 'number') return ''
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

export function activeDownloadStatusText(status: string, t: Translate) {
  switch (status) {
    case 'queued': return t('download.queued')
    case 'resolving': return t('download.resolving')
    case 'processing': return t('download.processing')
    case 'cancelling': return t('download.cancelling')
    case 'cancelled': return t('download.cancelled')
    case 'error': return t('download.download_failed')
    case 'already_exists': return t('download.already_exists')
    default: return t('download.downloading')
  }
}

export function activeDownloadProgressText(task: DownloadTaskProgress, t: Translate) {
  if (['queued', 'resolving', 'processing', 'cancelling', 'cancelled', 'already_exists'].includes(task.status)) {
    return activeDownloadStatusText(task.status, t)
  }
  if (task.status === 'error') {
    return task.message
      ? `${activeDownloadStatusText(task.status, t)} · ${task.message}`
      : activeDownloadStatusText(task.status, t)
  }

  const downloaded = typeof task.downloadedBytes === 'number' ? formatDownloadSize(task.downloadedBytes) : ''
  const total = typeof task.totalBytes === 'number' && task.totalBytes > 0 ? formatDownloadSize(task.totalBytes) : ''
  const percent = typeof task.progress === 'number' ? `${task.progress}%` : ''

  if (downloaded && total && percent) return `${downloaded} / ${total} · ${percent}`
  if (downloaded && total) return `${downloaded} / ${total}`
  if (downloaded && percent) return `${downloaded} · ${percent}`
  return activeDownloadStatusText(task.status, t)
}

const TEMPLATE_PREVIEW_VALUES = [
  ['{title}', '晴天'], ['%title%', '晴天'],
  ['{artist}', '周杰伦'], ['%artist%', '周杰伦'],
  ['{album}', '叶惠美'], ['%album%', '叶惠美'],
  ['{source}', 'netease'], ['%source%', 'netease'],
  ['{id}', 'netease:123456'], ['%id%', 'netease:123456'],
  ['{audioId}', '123456'], ['%audioId%', '123456'],
  ['{subAudioId}', ''], ['%subAudioId%', ''],
  ['{hash}', '4c853a1f'], ['%hash%', '4c853a1f'],
] as const

/** 用固定示例曲目展开文件名模板，空模板按默认模板预览 */
export function downloadTemplatePreview(template: string): string {
  let preview = template || DEFAULT_DOWNLOAD_NAME_TEMPLATE
  for (const [token, value] of TEMPLATE_PREVIEW_VALUES) {
    preview = preview.split(token).join(value)
  }
  return preview
}

/** 下载管理分区：进行中的任务、下载目录与文件名模板 */
export function useDownloadSettings(ipc: SettingsIpc) {
  const { t } = useI18n()
  const router = useRouter()
  const toast = useToastStore()
  const downloadStore = useDownloadStore()
  const { downloadDir, downloadNameTemplate } = storeToRefs(useSettingsStore())

  const activeDownloadCount = computed(() => downloadStore.runningDownloadCount)
  const completedDownloadCount = computed(() => downloadStore.downloads.length)
  const activeDownloadTasks = computed(() => downloadStore.activeDownloads)

  // 下载目录
  const defaultDownloadDir = ref('')

  async function loadDefaultDownloadDir() {
    try {
      defaultDownloadDir.value = await ipc.getDefaultDownloadDir()
    } catch (e) {
      log.error('Failed to get default download dir:', e)
    }
  }

  const displayDownloadDir = computed(() => downloadDir.value || defaultDownloadDir.value || '...')

  async function selectDownloadDir() {
    try {
      const result = await dialogOpen({ directory: true })
      if (result) {
        const path = dialogSelectionPath(result)
        const validated = await ipc.setDownloadDir(path)
        downloadDir.value = validated
        toast.success(t('settings.download_dir_changed'))
      }
    } catch (e) {
      log.error('Failed to set download dir:', e)
      toast.error(t('settings.download_dir_invalid'))
    }
  }

  function resetDownloadDir() {
    downloadDir.value = ''
  }

  // 下载文件名格式
  const showDownloadTemplateDialog = ref(false)
  const pendingTemplate = ref('')

  function openDownloadTemplateDialog() {
    pendingTemplate.value = downloadNameTemplate.value || DEFAULT_DOWNLOAD_NAME_TEMPLATE
    showDownloadTemplateDialog.value = true
  }

  const templatePreview = computed(() => downloadTemplatePreview(pendingTemplate.value))

  function applyDownloadTemplate() {
    downloadNameTemplate.value = pendingTemplate.value.trim() || DEFAULT_DOWNLOAD_NAME_TEMPLATE
    showDownloadTemplateDialog.value = false
  }

  function resetDownloadTemplate() {
    pendingTemplate.value = DEFAULT_DOWNLOAD_NAME_TEMPLATE
    downloadNameTemplate.value = DEFAULT_DOWNLOAD_NAME_TEMPLATE
    showDownloadTemplateDialog.value = false
  }

  function cancelAllDownloads() {
    downloadStore.cancelAllDownloads()
  }

  function cancelDownload(trackId: string) {
    downloadStore.cancelDownload(trackId)
  }

  function goToDownloads() {
    router.push('/downloads')
  }

  return {
    activeDownloadCount,
    completedDownloadCount,
    activeDownloadTasks,
    activeDownloadProgressText: (task: DownloadTaskProgress) => activeDownloadProgressText(task, t),
    loadDefaultDownloadDir,
    displayDownloadDir,
    selectDownloadDir,
    resetDownloadDir,
    showDownloadTemplateDialog,
    pendingTemplate,
    openDownloadTemplateDialog,
    templatePreview,
    applyDownloadTemplate,
    resetDownloadTemplate,
    cancelAllDownloads,
    cancelDownload,
    goToDownloads,
  }
}

export type DownloadSettings = ReturnType<typeof useDownloadSettings>
