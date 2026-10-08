import { computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useDownloadStore } from '@/stores/download'
import type { TrackInfo } from '@/stores/player'
import { createContextMenuItem } from '@/utils/contextMenu'
import { createLogger } from '@/utils/logger'

const log = createLogger('track-download-menu')

export function useTrackDownloadMenu(readTrack: () => TrackInfo | null | undefined, closeMenu: () => void) {
  const downloads = useDownloadStore()
  const { t } = useI18n()

  function disabled(track: TrackInfo | null | undefined) {
    return !track || track.id.startsWith('local:') || downloads.isDownloading(track.id)
  }

  const downloadMenuItem = computed(() => {
    const track = readTrack()
    const task = track ? downloads.downloading.get(track.id) : undefined
    const statusKey = task?.status === 'resolving' ? 'resolving'
      : task?.status === 'cancelling' ? 'cancelling'
      : track && task && downloads.isDownloading(track.id) ? 'downloading' : null
    const label = statusKey ? t(`download.${statusKey}`)
      : track && downloads.isDownloaded(track.id) ? t('download.redownload') : t('download.download')
    return createContextMenuItem(label, { id: 'download', icon: 'download', disabled: disabled(track) })
  })

  async function downloadFromMenu() {
    // 队列菜单关闭时会清除索引，先保存操作目标
    const track = readTrack()
    closeMenu()
    if (!track || disabled(track)) return
    try {
      if (downloads.isDownloaded(track.id)) await downloads.redownloadTrack(track)
      else await downloads.downloadTrack(track)
    } catch (error) {
      log.error('Download from context menu failed:', error)
    }
  }

  onMounted(() => {
    void downloads.initEvents().catch(error => log.error('Initialize download events failed:', error))
    void downloads.loadDownloads().catch(error => log.error('Load downloads failed:', error))
  })

  return { downloadMenuItem, downloadFromMenu }
}
