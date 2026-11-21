import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useAuthStore } from '@/stores/auth'

export type CookieLoginPlatform = 'netease' | 'bilibili' | 'youtube'

/** 平台账号分区：登录状态、头像兜底、退出确认与 Cookie 登录入口 */
export function useAccountSettings() {
  const { t } = useI18n()
  const auth = useAuthStore()

  // 头像地址可能短暂失效，记录具体失败地址，换地址后仍允许重新加载
  const failedAvatarUrls = ref<Record<string, string | null>>({})

  function isAvatarLoadFailed(platform: string, avatarUrl: string | null) {
    return Boolean(avatarUrl && failedAvatarUrls.value[platform] === avatarUrl)
  }

  function handleAvatarError(platform: string, avatarUrl: string | null) {
    if (!avatarUrl) return
    failedAvatarUrls.value = { ...failedAvatarUrls.value, [platform]: avatarUrl }
  }

  // 平台账号配置
  const platformAccounts = computed(() => [
    { key: 'netease', label: t('settings.netease_account'), iconSvg: '/icons/ic_netease.svg', auth: auth.netease, login: auth.loginNetease },
    { key: 'bilibili', label: t('settings.bilibili_account'), iconSvg: '/icons/ic_bilibili.svg', auth: auth.bilibili, login: auth.loginBilibili },
    { key: 'youtube', label: t('settings.youtube_account'), iconSvg: '/icons/ic_youtube.svg', auth: auth.youtube, login: auth.loginYoutube },
  ] as const)

  // 退出登录确认对话框
  const showLogoutConfirm = ref(false)
  const logoutTargetKey = ref('')
  const logoutTargetLabel = ref('')
  const showCookieLogin = ref(false)
  const cookieLoginPlatform = ref<CookieLoginPlatform>('netease')

  function requestLogout(key: string, label: string) {
    logoutTargetKey.value = key
    logoutTargetLabel.value = label
    showLogoutConfirm.value = true
  }

  async function confirmLogout() {
    showLogoutConfirm.value = false
    await auth.logout(logoutTargetKey.value)
  }

  return {
    isAvatarLoadFailed,
    handleAvatarError,
    platformAccounts,
    showLogoutConfirm,
    logoutTargetLabel,
    showCookieLogin,
    cookieLoginPlatform,
    requestLogout,
    confirmLogout,
  }
}

export type AccountSettings = ReturnType<typeof useAccountSettings>
