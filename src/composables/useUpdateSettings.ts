import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useUpdateStore } from '@/stores/update'
import { progressPercent } from '@/modules/update/updateModel'

/** 关于分区的「软件更新」卡片：一句话状态 + 一个随状态变化的主按钮 */
export function useUpdateSettings() {
  const { t } = useI18n()
  const update = useUpdateStore()

  const currentVersion = computed(() => update.check?.currentVersion ?? '')

  const statusText = computed(() => {
    const latest = update.check?.latestVersion ?? ''
    switch (update.phase) {
      case 'checking': return t('update.status_checking')
      case 'upToDate': return t('update.status_up_to_date', { version: currentVersion.value })
      case 'available': return t('update.status_available', { version: latest })
      case 'downloading': return t('update.status_downloading', { percent: progressPercent(update.progress) })
      case 'ready': return t('update.status_ready')
      case 'installing': return t('update.status_installing')
      case 'disabled': return t('update.status_disabled')
      case 'error': return t('update.status_error')
      default: return t('settings.software_update_desc')
    }
  })

  const icon = computed(() => {
    switch (update.phase) {
      case 'checking': return 'progress_activity'
      case 'available':
      case 'ready': return 'system_update'
      case 'downloading':
      case 'installing': return 'downloading'
      case 'error': return 'error'
      case 'upToDate': return 'verified'
      default: return 'update'
    }
  })

  const actionText = computed(() => {
    switch (update.phase) {
      case 'available': return t('update.update_now')
      case 'ready': return t('update.restart_now')
      case 'downloading':
      case 'error': return t('update.view')
      default: return t('update.check_now')
    }
  })

  /** 有结果要看（新版本、进度、错误）时打开对话框，否则发起一次手动检查 */
  function runAction() {
    if (['available', 'downloading', 'ready', 'error'].includes(update.phase)) update.openDialog()
    else void update.checkForUpdates({ manual: true })
  }

  return { update, statusText, icon, actionText, runAction }
}
