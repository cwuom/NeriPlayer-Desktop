import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToastStore } from './toast'
import { useRecommendStore } from './recommend'
import i18n from '@/i18n'
import { createLogger } from '@/utils/logger'
import { playbackUrlResolver } from '@/modules/playback/playbackSource'
import { playbackPrefetchManager } from '@/modules/playback/playbackPrefetch'

/// 登录态变化事件：库页等已挂载页面据此重新拉取平台数据
export const AUTH_CHANGED_EVENT = 'neri:auth-changed'

const log = createLogger('auth')

export interface PlatformAuth {
  loggedIn: boolean
  nickname: string | null
  avatarUrl: string | null
}

export interface AuthStatusResponse {
  netease: PlatformAuth & { platform: string }
  bilibili: PlatformAuth & { platform: string }
  youtube: PlatformAuth & { platform: string }
}

const emptyAuth = (): PlatformAuth => ({ loggedIn: false, nickname: null, avatarUrl: null })

/** 后端 snake_case -> 前端 camelCase */
function mapAuth(raw: any): PlatformAuth {
  return {
    loggedIn: raw?.logged_in ?? false,
    nickname: raw?.nickname ?? null,
    avatarUrl: raw?.avatar_url ?? null,
  }
}

export const useAuthStore = defineStore('auth', () => {
  const netease = ref<PlatformAuth>(emptyAuth())
  const bilibili = ref<PlatformAuth>(emptyAuth())
  const youtube = ref<PlatformAuth>(emptyAuth())

  // 正在登录的平台（用于 loading 状态）
  const loggingIn = ref<string | null>(null)
  const youtubeProfileRefreshAttempted = ref(false)
  const youtubeProfileRefreshing = ref(false)

  const isAnyLoggedIn = computed(() =>
    netease.value.loggedIn || bilibili.value.loggedIn || youtube.value.loggedIn
  )

  /** 启动时检查所有平台登录状态 */
  async function checkStatus() {
    try {
      const status = await invoke<any>('check_auth_status')
      netease.value = mapAuth(status.netease)
      bilibili.value = mapAuth(status.bilibili)
      youtube.value = mapAuth(status.youtube)
      if (needsYoutubeProfileRefresh(youtube.value)) {
        void refreshYoutubeProfile()
      }
    } catch (e) {
      log.error('Failed to check auth status:', e)
    }
  }

  function needsYoutubeProfileRefresh(value: PlatformAuth) {
    return value.loggedIn && (!value.nickname || !value.avatarUrl)
  }

  async function refreshYoutubeProfile() {
    if (
      youtubeProfileRefreshing.value ||
      youtubeProfileRefreshAttempted.value ||
      !needsYoutubeProfileRefresh(youtube.value)
    ) return

    youtubeProfileRefreshing.value = true
    youtubeProfileRefreshAttempted.value = true
    try {
      const info = await invoke<any>('refresh_youtube_profile')
      const mapped = mapAuth(info)
      if (mapped.loggedIn) {
        youtube.value = mapped
      }
    } catch (e) {
      log.warn('Failed to refresh YouTube profile:', e)
    } finally {
      youtubeProfileRefreshing.value = false
    }
  }

  const { t } = i18n.global

  // 平台 key -> 显示名映射
  const platformLabel = (key: string) => {
    const map: Record<string, string> = {
      netease: t('settings.netease_account'),
      bilibili: t('settings.bilibili_account'),
      youtube: t('settings.youtube_account'),
    }
    return map[key] ?? key
  }

  /// 登录态变化：作废该平台缓存并广播，让已挂载的页面立刻重拉
  ///
  /// 不广播的话，库页在 onMounted 时才判断是否需要拉数据，
  /// 用户在设置页登录完切回去看到的仍然是空列表。
  function notifyAuthChanged(platform: string) {
    playbackUrlResolver.clear()
    playbackPrefetchManager.clear()
    useRecommendStore().invalidatePlatform(platform)
    window.dispatchEvent(new CustomEvent(AUTH_CHANGED_EVENT, { detail: { platform } }))
  }

  /** 通用登录流程 */
  async function doLogin(
    key: string,
    command: string,
    target: typeof netease,
  ) {
    const toast = useToastStore()
    loggingIn.value = key
    try {
      const info = await invoke<any>(command)
      const mapped = mapAuth(info)
      target.value = mapped
      if (key === 'youtube') {
        youtubeProfileRefreshAttempted.value = false
        if (needsYoutubeProfileRefresh(mapped)) void refreshYoutubeProfile()
      }
      if (mapped.loggedIn) {
        notifyAuthChanged(key)
        toast.success(t('settings.login_success', { platform: platformLabel(key) }))
      }
    } catch (e: any) {
      const msg = String(e)
      if (msg.includes('cancelled') || msg.includes('cancel')) {
        toast.show(t('settings.login_cancelled'), 'info')
      } else {
        toast.error(t('settings.login_failed', { platform: platformLabel(key) }))
      }
      log.error(`${key} login failed:`, e)
    } finally {
      loggingIn.value = null
    }
  }

  /** 网易云登录 */
  async function loginNetease() {
    await doLogin('netease', 'login_netease', netease)
  }

  /** B站登录 */
  async function loginBilibili() {
    await doLogin('bilibili', 'login_bilibili', bilibili)
  }

  /** YouTube Music 登录 */
  async function loginYoutube() {
    await doLogin('youtube', 'login_youtube', youtube)
  }

  /** 登出指定平台 */
  async function logout(platform: string) {
    const toast = useToastStore()
    try {
      await invoke('logout', { platform })
      switch (platform) {
        case 'netease': netease.value = emptyAuth(); break
        case 'bilibili': bilibili.value = emptyAuth(); break
        case 'youtube': youtube.value = emptyAuth(); break
      }
      notifyAuthChanged(platform)
      toast.success(t('settings.logout_success', { platform: platformLabel(platform) }))
    } catch (e) {
      log.error(`Logout ${platform} failed:`, e)
    }
  }

  /** Cookie 粘贴登录（对齐 Android 端） */
  async function loginWithCookies(platform: string, rawCookies: string) {
    const toast = useToastStore()
    const target = platform === 'netease' ? netease : platform === 'bilibili' ? bilibili : youtube
    loggingIn.value = platform
    try {
      const info = await invoke<any>('login_with_cookies', { platform, rawCookies })
      const mapped = mapAuth(info)
      target.value = mapped
      if (platform === 'youtube') {
        youtubeProfileRefreshAttempted.value = false
        if (needsYoutubeProfileRefresh(mapped)) void refreshYoutubeProfile()
      }
      if (mapped.loggedIn) {
        notifyAuthChanged(platform)
        toast.success(t('settings.login_success', { platform: platformLabel(platform) }))
      }
    } catch (e: any) {
      toast.error(String(e))
      log.error(`${platform} cookie login failed:`, e)
    } finally {
      loggingIn.value = null
    }
  }

  return {
    netease, bilibili, youtube, loggingIn, isAnyLoggedIn,
    checkStatus, refreshYoutubeProfile, loginNetease, loginBilibili, loginYoutube, loginWithCookies, logout,
  }
})
