import { computed, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { useDownloadStore } from '@/stores/download'

/** 当前曲目的下载状态与「下载 / 重新下载 / 取消」入口 */
export function useCurrentTrackDownload(showMoreSheet: Ref<boolean>) {
  const player = usePlayerStore()
  const downloadStore = useDownloadStore()
  const { t } = useI18n()

  const currentTrackId = computed(() => player.currentTrack?.id || '')
  const currentDownloadTask = computed(() => currentTrackId.value ? downloadStore.downloading.get(currentTrackId.value) : undefined)
  const currentDownloadedTrack = computed(() => currentTrackId.value ? downloadStore.getDownloadedTrack(currentTrackId.value) : undefined)
  const isCurrentDownloaded = computed(() => currentTrackId.value ? downloadStore.isDownloaded(currentTrackId.value) : false)
  const isCurrentDownloading = computed(() => !!currentDownloadTask.value)
  const isCurrentDownloadCancellable = computed(() => {
    const status = currentDownloadTask.value?.status
    return status === 'resolving' || status === 'downloading'
  })

  function downloadTaskStatusText(status?: string) {
    switch (status) {
      case 'queued': return t('download.queued')
      case 'processing': return t('download.processing')
      case 'resolving': return t('download.resolving')
      case 'downloading': return t('download.downloading')
      case 'cancelling': return t('download.cancelling')
      case 'cancelled': return t('download.cancelled')
      case 'error': return t('download.download_failed')
      case 'already_exists': return t('download.already_exists')
      default: return ''
    }
  }

  const downloadActionIcon = computed(() => {
    if (isCurrentDownloading.value) {
      const status = currentDownloadTask.value?.status
      if (status === 'error') return 'error'
      if (status === 'cancelled') return 'cancel'
      if (isCurrentDownloadCancellable.value) return 'cancel'
      return 'downloading'
    }
    if (isCurrentDownloaded.value) return 'refresh'
    return 'download'
  })

  const downloadActionLabel = computed(() => {
    if (isCurrentDownloadCancellable.value) return t('download.cancel_task')
    if (isCurrentDownloading.value) return downloadTaskStatusText(currentDownloadTask.value?.status) || t('download.downloading')
    if (isCurrentDownloaded.value) return t('download.redownload')
    return t('library.tab_downloads')
  })

  const downloadActionDesc = computed(() => {
    if (isCurrentDownloading.value) {
      const task = currentDownloadTask.value
      if (!task) return ''
      if (task.message) return task.message
      if (typeof task.progress === 'number') return `${task.progress}%`
      return ''
    }
    if (isCurrentDownloaded.value) return t('download.redownload_desc')
    return t('download.download_desc')
  })

  const downloadActionDisabled = computed(() =>
    !player.currentTrack
    || (isCurrentDownloading.value && !isCurrentDownloadCancellable.value && currentDownloadTask.value?.status !== 'error' && currentDownloadTask.value?.status !== 'cancelled')
  )

  // 下载/重新下载歌曲
  async function handleDownloadAction() {
    const track = player.currentTrack
    if (!track) return
    showMoreSheet.value = false
    if (isCurrentDownloadCancellable.value) {
      await downloadStore.cancelDownload(track.id)
      return
    }
    if (isCurrentDownloaded.value) {
      await downloadStore.redownloadTrack(track)
    } else {
      await downloadStore.downloadTrack(track)
    }
  }

  return {
    currentDownloadTask,
    currentDownloadedTrack,
    isCurrentDownloaded,
    downloadTaskStatusText,
    downloadActionIcon,
    downloadActionLabel,
    downloadActionDesc,
    downloadActionDisabled,
    handleDownloadAction,
  }
}
