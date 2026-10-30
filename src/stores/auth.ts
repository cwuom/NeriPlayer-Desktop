import { defineStore } from 'pinia'
import { ref, computed, onScopeDispose } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
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
type AuthPlatform = 'netease' | 'bilibili' | 'youtube'

function isAuthPlatform(platform: string): platform is AuthPlatform {
  return platform === 'netease' || platform === 'bilibili' || platform === 'youtube'
}

function sameAuth(a: PlatformAuth, b: PlatformAuth): boolean {
  return a.loggedIn === b.loggedIn && a.nickname === b.nickname && a.avatarUrl === b.avatarUrl
}

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
  // 启动时的登录态检查是否已经返回（无论成败）；返回前各平台 loggedIn 都还是默认的 false
  const statusChecked = ref(false)
  const youtubeProfileRefreshAttempted = ref(false)
  const youtubeProfileRefreshing = ref(false)
  const authRevisions: Record<AuthPlatform, number> = { netease: 0, bilibili: 0, youtube: 0 }
  let loginSequence = 0
  let statusSequence = 0
  let profileListener: Promise<void> | null = null
  let unlistenProfile: UnlistenFn | null = null
  let disposed = false

  function ensureProfileListener() {
    if (!profileListener) {
      profileListener = listen<string>('auth-profile-updated', async event => {
        if (disposed || !isAuthPlatform(event.payload)) return
        await checkStatus()
        // 账号 ID 不在界面摘要中，补齐后仍需让曲库重拉一次
        if (!disposed) notifyAuthChanged(event.payload)
      }).then(unlisten => {
        if (disposed) unlisten()
        else unlistenProfile = unlisten
      }).catch(error => log.warn('Failed to listen for account profile updates:', error))
    }
    return profileListener
  }

  onScopeDispose(() => {
    disposed = true
    unlistenProfile?.()
  })

  const isAnyLoggedIn = computed(() =>
    netease.value.loggedIn || bilibili.value.loggedIn || youtube.value.loggedIn
  )

  /** 启动时检查所有平台登录状态 */
  async function checkStatus(options: { invalidatePending?: readonly string[] } = {}) {
    for (const platform of options.invalidatePending ?? []) {
      if (!isAuthPlatform(platform)) continue
      authRevisions[platform]++
      if (loggingIn.value === platform) {
        loginSequence++
        loggingIn.value = null
      }
    }
    const request = ++statusSequence
    const revisions = { ...authRevisions }
    try {
      await ensureProfileListener()
      const status = await invoke<any>('check_auth_status')
      if (request !== statusSequence || disposed) return
      // 账号没变就保留原对象：页面按对象身份判断换号，设置页、同步后的例行检查不能让首页整页重载
      for (const [platform, target, raw] of [['netease', netease, status.netease], ['bilibili', bilibili, status.bilibili], ['youtube', youtube, status.youtube]] as const) {
        if (revisions[platform] !== authRevisions[platform]) continue
        const next = mapAuth(raw)
        if (!sameAuth(target.value, next)) target.value = next
      }
      if (needsYoutubeProfileRefresh(youtube.value)) {
        void refreshYoutubeProfile()
      }
    } catch (e) {
      log.error('Failed to check auth status:', e)
    } finally {
      statusChecked.value = true
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
    const revision = authRevisions.youtube
    try {
      const info = await invoke<any>('refresh_youtube_profile')
      if (revision !== authRevisions.youtube) return
      const mapped = mapAuth(info)
      if (mapped.loggedIn && !sameAuth(youtube.value, mapped)) {
        youtube.value = mapped
      }
    } catch (e) {
      log.warn('Failed to refresh YouTube profile:', e)
    } finally {
      youtubeProfileRefreshing.value = false
      if (revision !== authRevisions.youtube && needsYoutubeProfileRefresh(youtube.value)) {
        youtubeProfileRefreshAttempted.value = false
        void refreshYoutubeProfile()
      }
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
    if (platform === 'youtube' && needsYoutubeProfileRefresh(youtube.value)) {
      youtubeProfileRefreshAttempted.value = false
      void refreshYoutubeProfile()
    }
  }

  /** 通用登录流程 */
  async function doLogin(
    key: AuthPlatform,
    command: string,
    target: typeof netease,
  ) {
    const toast = useToastStore()
    const revision = ++authRevisions[key]
    const request = ++loginSequence
    loggingIn.value = key
    try {
      const info = await invoke<any>(command)
      if (revision !== authRevisions[key]) return
      const mapped = mapAuth(info)
      authRevisions[key]++
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
      if (revision !== authRevisions[key] || String(e).includes('Authentication superseded')) return
      const msg = String(e)
      if (msg.includes('cancelled') || msg.includes('cancel')) {
        toast.show(t('settings.login_cancelled'), 'info')
      } else {
        toast.error(t('settings.login_failed', { platform: platformLabel(key) }))
      }
      log.error(`${key} login failed:`, e)
    } finally {
      if (request === loginSequence) loggingIn.value = null
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
    if (!isAuthPlatform(platform)) return
    const revision = ++authRevisions[platform]
    if (loggingIn.value === platform) {
      loginSequence++
      loggingIn.value = null
    }
    const toast = useToastStore()
    try {
      await invoke('logout', { platform })
      if (revision !== authRevisions[platform]) return
      authRevisions[platform]++
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
  async function loginWithCookies(platform: string, rawCookies: string): Promise<boolean> {
    if (!isAuthPlatform(platform)) return false
    const toast = useToastStore()
    const target = platform === 'netease' ? netease : platform === 'bilibili' ? bilibili : youtube
    const revision = ++authRevisions[platform]
    const request = ++loginSequence
    loggingIn.value = platform
    try {
      const info = await invoke<any>('login_with_cookies', { platform, rawCookies })
      if (revision !== authRevisions[platform]) return false
      const mapped = mapAuth(info)
      if (!mapped.loggedIn) return false
      authRevisions[platform]++
      target.value = mapped
      if (platform === 'youtube') {
        youtubeProfileRefreshAttempted.value = false
        if (needsYoutubeProfileRefresh(mapped)) void refreshYoutubeProfile()
      }
      if (mapped.loggedIn) {
        notifyAuthChanged(platform)
        toast.success(t('settings.login_success', { platform: platformLabel(platform) }))
      }
      return true
    } catch (e: any) {
      if (revision !== authRevisions[platform] || String(e).includes('Authentication superseded')) return false
      toast.error(String(e))
      log.error(`${platform} cookie login failed:`, e)
      return false
    } finally {
      if (request === loginSequence) loggingIn.value = null
    }
  }

  return {
    netease, bilibili, youtube, loggingIn, statusChecked, isAnyLoggedIn,
    checkStatus, refreshYoutubeProfile, loginNetease, loginBilibili, loginYoutube, loginWithCookies, notifyAuthChanged, logout,
  }
})
