import { ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { isValidLtNickname } from '@/stores/listenTogether/protocol'
import { settingsLog as log, type SettingsIpc } from './settingsShared'

/** 网络相关：代理绕过、YouTube 连通性探测，以及一起听的身份与昵称 */
export function useNetworkSettings(ipc: SettingsIpc) {
  const { t } = useI18n()
  const toast = useToastStore()
  const lt = useListenTogetherStore()
  const { bypassProxy, internationalizationEnabled, ltNickname } = storeToRefs(useSettingsStore())

  // 网络：绕过代理
  async function handleBypassProxyChange(val: boolean) {
    const previous = bypassProxy.value
    bypassProxy.value = val
    try {
      await ipc.setBypassProxy(val)
    } catch (e) {
      log.error('Failed to set bypass proxy:', e)
      bypassProxy.value = previous
      toast.error(t('settings.bypass_proxy_failed'))
    }
  }

  // YouTube 国际化
  const intlChecking = ref(false)

  // 对齐 Android：开关直接生效。开启时只探测传输层连通性并提示，不回退用户的选择
  async function handleIntlToggle(val: boolean) {
    if (intlChecking.value) return
    internationalizationEnabled.value = val
    if (!val) return
    intlChecking.value = true
    try {
      const reachable = await ipc.probePlatformConnectivity('youtube')
      if (!reachable) toast.show(t('settings.intl_check_failed'), 'info')
    } catch (error) {
      log.warn('YouTube connectivity probe failed:', error)
    } finally {
      intlChecking.value = false
    }
  }

  // 一起听
  const showResetLtIdentityConfirm = ref(false)

  function confirmResetLtIdentity() {
    showResetLtIdentityConfirm.value = false
    if (lt.resetIdentity()) toast.success(t('listen_together.identity_reset'))
    else toast.error(t('listen_together.reset_identity_in_room'))
  }

  // 与协议校验同一规则：去首尾空白，空值回到默认昵称，非法值不保存
  function handleLtNicknameChange(event: Event) {
    const input = event.target as HTMLInputElement
    const value = input.value.trim()
    if (value && !isValidLtNickname(value)) {
      toast.error(t('listen_together.invalid_nickname'))
      input.value = ltNickname.value
      return
    }
    ltNickname.value = value
    input.value = value
  }

  return {
    handleBypassProxyChange,
    intlChecking,
    handleIntlToggle,
    showResetLtIdentityConfirm,
    confirmResetLtIdentity,
    handleLtNicknameChange,
  }
}

export type NetworkSettings = ReturnType<typeof useNetworkSettings>
