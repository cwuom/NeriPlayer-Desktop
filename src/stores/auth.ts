import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useToastStore } from './toast'
import { useRecommendStore } from './recommend'
import { useLikedSongsStore } from './likedSongs'
import i18n from '@/i18n'
import { createLogger } from '@/utils/logger'
import { AuthMutationRequestCoordinator } from '@/modules/auth/authMutationRequest'
import { AuthStatusRequestCoordinator } from '@/modules/auth/authStatusRequest'

const log = createLogger('auth')

export interface PlatformAuth {
  loggedIn: boolean
  nickname: string | null
  avatarUrl: string | null
  accountId: string | null
}

export interface AuthStatusResponse {
  netease: PlatformAuth & { platform: string }
  bilibili: PlatformAuth & { platform: string }
  youtube: PlatformAuth & { platform: string }
}

const emptyAuth = (): PlatformAuth => ({
  loggedIn: false,
  nickname: null,
  avatarUrl: null,
  accountId: null,
})

/** 后端 snake_case -> 前端 camelCase */
function mapAuth(raw: any): PlatformAuth {
  return {
    loggedIn: raw?.logged_in ?? false,
    nickname: raw?.nickname ?? null,
    avatarUrl: raw?.avatar_url ?? null,
    accountId: raw?.account_id ?? null,
  }
}

function hasNeteaseSessionBoundary(previous: PlatformAuth, next: PlatformAuth) {
  return previous.loggedIn !== next.loggedIn || (
    next.loggedIn && previous.accountId !== next.accountId
  )
}

function hasPlatformSessionBoundary(previous: PlatformAuth, next: PlatformAuth) {
  return previous.loggedIn !== next.loggedIn || (
    next.loggedIn
    && previous.accountId !== null
    && next.accountId !== null
    && previous.accountId !== next.accountId
  )
}

export const useAuthStore = defineStore('auth', () => {
  const netease = ref<PlatformAuth>(emptyAuth())
  const bilibili = ref<PlatformAuth>(emptyAuth())
  const youtube = ref<PlatformAuth>(emptyAuth())
  // Changes even when the account keeps the same display name.
  const neteaseSessionVersion = ref(0)
  const bilibiliSessionVersion = ref(0)
  const youtubeSessionVersion = ref(0)
  const authMutationRequestCoordinator = new AuthMutationRequestCoordinator()
  const authStatusRequestCoordinator = new AuthStatusRequestCoordinator<any>()
  const hasVerifiedNeteaseAuth = ref(false)

  // 正在登录的平台（用于 loading 状态）
  const loggingIn = ref<string | null>(null)
  let nextLoginOperationId = 0
  const activeLoginOperations = new Map<number, string>()
  const youtubeProfileRefreshAttempted = ref(false)
  const youtubeProfileRefreshing = ref(false)

  const isAnyLoggedIn = computed(() =>
    netease.value.loggedIn || bilibili.value.loggedIn || youtube.value.loggedIn
  )
  const canMutateNetease = computed(() =>
    hasVerifiedNeteaseAuth.value && netease.value.loggedIn
  )

  /** 启动时检查所有平台登录状态 */
  async function checkStatus(invalidatePlatformSessions = false) {
    try {
      const statusRequest = authStatusRequestCoordinator.run(
        () => invoke<any>('check_auth_status'),
      )
      if (!statusRequest.started) {
        await statusRequest.promise
        return
      }

      const needsNeteaseVerification = !hasVerifiedNeteaseAuth.value
      if (needsNeteaseVerification) {
        useRecommendStore().clearPlatformCache('netease')
        useLikedSongsStore().clearCloudLikes()
      }

      const statusResult = await statusRequest.promise
      if (!statusResult.current) return

      const status = statusResult.value
      const nextNetease = mapAuth(status.netease)
      const nextBilibili = mapAuth(status.bilibili)
      const nextYoutube = mapAuth(status.youtube)
      const neteaseSessionChanged = hasNeteaseSessionBoundary(netease.value, nextNetease)
      const bilibiliSessionChanged = hasPlatformSessionBoundary(bilibili.value, nextBilibili)
      const youtubeSessionChanged = hasPlatformSessionBoundary(youtube.value, nextYoutube)
      const advanceNeteaseSession = (
        (invalidatePlatformSessions && !neteaseSessionChanged)
        || (nextNetease.loggedIn && (neteaseSessionChanged || needsNeteaseVerification))
      )
      if (neteaseSessionChanged && !needsNeteaseVerification) {
        useRecommendStore().clearPlatformCache('netease')
        useLikedSongsStore().clearCloudLikes()
      }
      if (bilibiliSessionChanged || invalidatePlatformSessions) {
        useRecommendStore().clearPlatformCache('bilibili')
        bilibiliSessionVersion.value++
      }
      if (youtubeSessionChanged || invalidatePlatformSessions) {
        useRecommendStore().clearPlatformCache('youtube')
        youtubeSessionVersion.value++
      }
      if (invalidatePlatformSessions && !neteaseSessionChanged) {
        useRecommendStore().clearPlatformCache('netease')
        useLikedSongsStore().clearCloudLikes()
      }
      netease.value = nextNetease
      bilibili.value = nextBilibili
      youtube.value = nextYoutube
      if (advanceNeteaseSession) {
        neteaseSessionVersion.value++
        if (nextNetease.loggedIn) void useLikedSongsStore().refreshCloudLikes()
      }
      if (needsYoutubeProfileRefresh(youtube.value)) {
        void refreshYoutubeProfile()
      }
      hasVerifiedNeteaseAuth.value = true
    } catch (e) {
      log.error('Failed to check auth status:', e)
    }
  }

  function needsYoutubeProfileRefresh(value: PlatformAuth) {
    return value.loggedIn && (!value.nickname || !value.avatarUrl)
  }

  function clearNeteaseCacheForAccountChange(platform: string, value: PlatformAuth) {
    if (platform === 'netease' && value.loggedIn) {
      useRecommendStore().clearPlatformCache('netease')
      useLikedSongsStore().clearCloudLikes()
    }
  }

  function invalidateAuthStatusRequests() {
    authStatusRequestCoordinator.invalidate()
  }

  async function reconcileStatus(invalidatePlatformSessions = false) {
    invalidateAuthStatusRequests()
    await checkStatus(invalidatePlatformSessions)
  }

  function beginLoginOperation(platform: string): number {
    const operationId = ++nextLoginOperationId
    activeLoginOperations.set(operationId, platform)
    loggingIn.value = platform
    return operationId
  }

  function finishLoginOperation(operationId: number) {
    activeLoginOperations.delete(operationId)
    let activePlatform: string | null = null
    for (const platform of activeLoginOperations.values()) {
      activePlatform = platform
    }
    loggingIn.value = activePlatform
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

  /** 通用登录流程 */
  async function doLogin(
    key: string,
    command: string,
    target: typeof netease,
  ) {
    const toast = useToastStore()
    const loginOperationId = beginLoginOperation(key)
    if (key === 'netease') hasVerifiedNeteaseAuth.value = false
    invalidateAuthStatusRequests()
    const mutation = authMutationRequestCoordinator.run(key, () => invoke<any>(command))
    try {
      const info = await mutation.promise
      if (!mutation.isCurrent()) return

      const mapped = mapAuth(info)
      invalidateAuthStatusRequests()
      if (key === 'netease') hasVerifiedNeteaseAuth.value = true
      clearNeteaseCacheForAccountChange(key, mapped)
      target.value = mapped
      if (key === 'netease' && mapped.loggedIn) {
        neteaseSessionVersion.value++
        void useLikedSongsStore().refreshCloudLikes()
      }
      if (key === 'bilibili' && mapped.loggedIn) {
        useRecommendStore().clearPlatformCache('bilibili')
        bilibiliSessionVersion.value++
      }
      if (key === 'youtube') {
        useRecommendStore().clearPlatformCache('youtube')
        youtubeSessionVersion.value++
        youtubeProfileRefreshAttempted.value = false
        if (needsYoutubeProfileRefresh(mapped)) void refreshYoutubeProfile()
      }
      if (mapped.loggedIn) {
        toast.success(t('settings.login_success', { platform: platformLabel(key) }))
      }
    } catch (e: any) {
      if (!mutation.isCurrent()) return

      const msg = String(e)
      if (msg.includes('cancelled') || msg.includes('cancel')) {
        toast.show(t('settings.login_cancelled'), 'info')
      } else {
        toast.error(t('settings.login_failed', { platform: platformLabel(key) }))
      }
      log.error(`${key} login failed:`, e)
      await checkStatus()
    } finally {
      finishLoginOperation(loginOperationId)
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
    if (platform === 'netease') hasVerifiedNeteaseAuth.value = false
    invalidateAuthStatusRequests()
    const mutation = authMutationRequestCoordinator.run(
      platform,
      () => invoke('logout', { platform }),
    )
    try {
      await mutation.promise
      if (!mutation.isCurrent()) return

      invalidateAuthStatusRequests()
      switch (platform) {
        case 'netease':
          hasVerifiedNeteaseAuth.value = true
          netease.value = emptyAuth()
          useLikedSongsStore().clearCloudLikes()
          break
        case 'bilibili':
          bilibili.value = emptyAuth()
          bilibiliSessionVersion.value++
          break
        case 'youtube':
          youtube.value = emptyAuth()
          youtubeSessionVersion.value++
          break
      }
      useRecommendStore().clearPlatformCache(platform)
      toast.success(t('settings.logout_success', { platform: platformLabel(platform) }))
    } catch (e) {
      if (!mutation.isCurrent()) return
      log.error(`Logout ${platform} failed:`, e)
      await checkStatus()
    }
  }

  /** Cookie 粘贴登录（对齐 Android 端） */
  async function loginWithCookies(platform: string, rawCookies: string) {
    const toast = useToastStore()
    const target = platform === 'netease' ? netease : platform === 'bilibili' ? bilibili : youtube
    const loginOperationId = beginLoginOperation(platform)
    if (platform === 'netease') hasVerifiedNeteaseAuth.value = false
    invalidateAuthStatusRequests()
    const mutation = authMutationRequestCoordinator.run(
      platform,
      () => invoke<any>('login_with_cookies', { platform, rawCookies }),
    )
    try {
      const info = await mutation.promise
      if (!mutation.isCurrent()) return

      const mapped = mapAuth(info)
      invalidateAuthStatusRequests()
      if (platform === 'netease') hasVerifiedNeteaseAuth.value = true
      clearNeteaseCacheForAccountChange(platform, mapped)
      target.value = mapped
      if (platform === 'netease' && mapped.loggedIn) {
        neteaseSessionVersion.value++
        void useLikedSongsStore().refreshCloudLikes()
      }
      if (platform === 'bilibili' && mapped.loggedIn) {
        useRecommendStore().clearPlatformCache('bilibili')
        bilibiliSessionVersion.value++
      }
      if (platform === 'youtube') {
        useRecommendStore().clearPlatformCache('youtube')
        youtubeSessionVersion.value++
        youtubeProfileRefreshAttempted.value = false
        if (needsYoutubeProfileRefresh(mapped)) void refreshYoutubeProfile()
      }
      if (mapped.loggedIn) {
        toast.success(t('settings.login_success', { platform: platformLabel(platform) }))
      }
    } catch (e: any) {
      if (!mutation.isCurrent()) return
      toast.error(String(e))
      log.error(`${platform} cookie login failed:`, e)
      await checkStatus()
    } finally {
      finishLoginOperation(loginOperationId)
    }
  }

  return {
    netease, bilibili, youtube,
    neteaseSessionVersion, bilibiliSessionVersion, youtubeSessionVersion,
    loggingIn, isAnyLoggedIn, canMutateNetease,
    checkStatus, reconcileStatus, refreshYoutubeProfile, loginNetease, loginBilibili, loginYoutube, loginWithCookies, logout,
  }
})
