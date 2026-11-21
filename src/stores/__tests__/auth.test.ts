// @vitest-environment happy-dom
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AUTH_CHANGED_EVENT, useAuthStore, type AuthStatusResponse, type BackendAuthInfo } from '@/stores/auth'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  profileHandler: null as ((event: { payload: string }) => Promise<void>) | null,
  toasts: [] as string[],
  invalidated: [] as string[],
  resolverCleared: 0,
  prefetchCleared: 0,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (_event: string, handler: (event: { payload: string }) => Promise<void>) => {
    mocks.profileHandler = handler
    return () => { mocks.profileHandler = null }
  },
}))
vi.mock('@/i18n', () => ({
  default: {
    global: {
      t: (key: string, params?: Record<string, unknown>) => (params ? `${key}|${JSON.stringify(params)}` : key),
    },
  },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({
    show: (text: string, type = 'info') => { mocks.toasts.push(`${type}:${text}`) },
    success: (text: string) => { mocks.toasts.push(`success:${text}`) },
    error: (text: string) => { mocks.toasts.push(`error:${text}`) },
  }),
}))
vi.mock('@/stores/recommend', () => ({
  useRecommendStore: () => ({ invalidatePlatform: (platform: string) => { mocks.invalidated.push(platform) } }),
}))
vi.mock('@/modules/playback/playbackSource', () => ({
  playbackUrlResolver: { clear: () => { mocks.resolverCleared++ } },
}))
vi.mock('@/modules/playback/playbackPrefetch', () => ({
  playbackPrefetchManager: { clear: () => { mocks.prefetchCleared++ } },
}))

function info(platform: string, nickname: string | null, avatar: string | null = `https://img/${platform}.png`): BackendAuthInfo {
  return { platform, logged_in: true, nickname, avatar_url: avatar }
}

function loggedOutInfo(platform: string): BackendAuthInfo {
  return { platform, logged_in: false, nickname: null, avatar_url: null }
}

function status(overrides: Partial<AuthStatusResponse> = {}): AuthStatusResponse {
  return {
    netease: loggedOutInfo('netease'),
    bilibili: loggedOutInfo('bilibili'),
    youtube: loggedOutInfo('youtube'),
    ...overrides,
  }
}

const loggedOut = { loggedIn: false, nickname: null, avatarUrl: null }

describe('auth store', () => {
  let router: ReturnType<typeof createInvokeRouter>
  let authEvents: string[]
  const recordAuthEvent = (event: Event) => {
    authEvents.push(String((event as CustomEvent<{ platform: string }>).detail.platform))
  }

  beforeEach(() => {
    router = createInvokeRouter()
    mocks.router = router
    mocks.profileHandler = null
    mocks.toasts = []
    mocks.invalidated = []
    mocks.resolverCleared = 0
    mocks.prefetchCleared = 0
    authEvents = []
    window.addEventListener(AUTH_CHANGED_EVENT, recordAuthEvent)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    window.removeEventListener(AUTH_CHANGED_EVENT, recordAuthEvent)
  })

  describe('checkStatus', () => {
    it('maps the snake_case backend summary for every platform', async () => {
      router.on('check_auth_status', () => status({
        netease: info('netease', 'neo'),
        youtube: info('youtube', 'yt'),
      }))
      const auth = useAuthStore()
      expect(auth.statusChecked).toBe(false)

      await auth.checkStatus()

      expect(auth.netease).toEqual({ loggedIn: true, nickname: 'neo', avatarUrl: 'https://img/netease.png' })
      expect(auth.bilibili).toEqual(loggedOut)
      expect(auth.youtube).toEqual({ loggedIn: true, nickname: 'yt', avatarUrl: 'https://img/youtube.png' })
      expect(auth.isAnyLoggedIn).toBe(true)
      expect(auth.statusChecked).toBe(true)
      // 资料完整时不额外刷新 YouTube 资料
      expect(router.callsOf('refresh_youtube_profile')).toHaveLength(0)
    })

    it('drops a status response that arrives after a newer check', async () => {
      const stale = deferred<AuthStatusResponse>()
      router.on('check_auth_status', () => stale.promise, () => status({ netease: info('netease', 'new account') }))
      const auth = useAuthStore()

      const first = auth.checkStatus()
      const second = auth.checkStatus()
      await second
      stale.resolve(status({ netease: info('netease', 'old account') }))
      await first

      expect(auth.netease.nickname).toBe('new account')
    })

    it('keeps the same account object when nothing changed so pages do not reload', async () => {
      router.on('check_auth_status', () => status({ netease: info('netease', 'neo') }))
      const auth = useAuthStore()
      await auth.checkStatus()
      const before = auth.netease

      await auth.checkStatus()

      expect(auth.netease).toBe(before)
    })

    it('marks the check as finished even when the backend fails', async () => {
      router.on('check_auth_status', () => { throw new Error('bridge down') })
      const auth = useAuthStore()

      await expect(auth.checkStatus()).resolves.toBeUndefined()

      expect(auth.statusChecked).toBe(true)
      expect(auth.netease).toEqual(loggedOut)
    })

    it('supersedes a pending login of an invalidated platform', async () => {
      const login = deferred<BackendAuthInfo>()
      router.on('login_bilibili', () => login.promise)
      router.on('check_auth_status', () => status())
      const auth = useAuthStore()

      const pendingLogin = auth.loginBilibili()
      expect(auth.loggingIn).toBe('bilibili')
      await auth.checkStatus({ invalidatePending: ['bilibili', 'not-a-platform'] })
      expect(auth.loggingIn).toBeNull()

      login.resolve(info('bilibili', 'late'))
      await pendingLogin

      expect(auth.bilibili).toEqual(loggedOut)
      expect(mocks.toasts).toEqual([])
    })

    it('reloads status and notifies pages when the backend reports a profile update', async () => {
      router.on('check_auth_status', () => status({ netease: info('netease', 'neo') }))
      const auth = useAuthStore()
      await auth.checkStatus()
      expect(mocks.profileHandler).not.toBeNull()

      await mocks.profileHandler!({ payload: 'unknown' })
      expect(router.callsOf('check_auth_status')).toHaveLength(1)

      await mocks.profileHandler!({ payload: 'netease' })
      expect(router.callsOf('check_auth_status')).toHaveLength(2)
      expect(authEvents).toEqual(['netease'])
      expect(mocks.invalidated).toEqual(['netease'])
    })
  })

  describe('YouTube profile completion', () => {
    it('asks the backend for missing profile fields only once per session', async () => {
      router.on('check_auth_status', () => status({ youtube: info('youtube', null, null) }))
      router.on('refresh_youtube_profile', () => info('youtube', null, null))
      const auth = useAuthStore()

      await auth.checkStatus()
      await flushPromises()
      await auth.checkStatus()
      await flushPromises()

      expect(router.callsOf('refresh_youtube_profile')).toHaveLength(1)
      expect(auth.youtube).toEqual({ loggedIn: true, nickname: null, avatarUrl: null })
    })

    it('applies a completed profile but ignores a logged-out refresh result', async () => {
      router.on('check_auth_status', () => status({ youtube: info('youtube', null) }))
      router.on('refresh_youtube_profile', () => loggedOutInfo('youtube'))
      const auth = useAuthStore()
      await auth.checkStatus()
      await flushPromises()
      expect(auth.youtube.loggedIn).toBe(true)

      // 重新登录后允许再补一次资料
      router.on('login_youtube', () => info('youtube', null))
      router.set('refresh_youtube_profile', () => info('youtube', 'Filled', 'https://img/filled.png'))
      await auth.loginYoutube()
      await flushPromises()

      expect(auth.youtube).toEqual({ loggedIn: true, nickname: 'Filled', avatarUrl: 'https://img/filled.png' })
      expect(router.callsOf('refresh_youtube_profile')).toHaveLength(2)
    })
  })

  describe('login and logout', () => {
    it('stores the account, clears caches and broadcasts the change on success', async () => {
      router.on('login_netease', () => info('netease', 'neo'))
      const auth = useAuthStore()

      await auth.loginNetease()

      expect(auth.netease).toEqual({ loggedIn: true, nickname: 'neo', avatarUrl: 'https://img/netease.png' })
      expect(auth.loggingIn).toBeNull()
      expect(authEvents).toEqual(['netease'])
      expect(mocks.invalidated).toEqual(['netease'])
      expect(mocks.resolverCleared).toBe(1)
      expect(mocks.prefetchCleared).toBe(1)
      expect(mocks.toasts).toEqual(['success:settings.login_success|{"platform":"settings.netease_account"}'])
    })

    it('discards a login that finishes after the user logged out', async () => {
      const login = deferred<BackendAuthInfo>()
      router.on('login_netease', () => login.promise)
      router.on('logout', () => undefined)
      const auth = useAuthStore()

      const pendingLogin = auth.loginNetease()
      await auth.logout('netease')
      expect(auth.loggingIn).toBeNull()
      login.resolve(info('netease', 'too late'))
      await pendingLogin

      expect(auth.netease).toEqual(loggedOut)
      expect(mocks.toasts).toEqual(['success:settings.logout_success|{"platform":"settings.netease_account"}'])
    })

    it('reports a cancelled login as info and other failures as errors', async () => {
      router.on('login_bilibili',
        () => { throw 'user cancelled the window' },
        () => { throw 'network error' },
        () => { throw 'Authentication superseded' })
      const auth = useAuthStore()

      await auth.loginBilibili()
      await auth.loginBilibili()
      await auth.loginBilibili()

      expect(mocks.toasts).toEqual([
        'info:settings.login_cancelled',
        'error:settings.login_failed|{"platform":"settings.bilibili_account"}',
      ])
      expect(auth.loggingIn).toBeNull()
      expect(auth.bilibili).toEqual(loggedOut)
    })

    it('keeps the account when logout fails', async () => {
      router.on('login_youtube', () => info('youtube', 'yt'))
      router.on('logout', () => { throw new Error('keychain locked') })
      const auth = useAuthStore()
      await auth.loginYoutube()
      mocks.toasts = []

      await auth.logout('youtube')
      await auth.logout('not-a-platform')

      expect(auth.youtube.loggedIn).toBe(true)
      expect(mocks.toasts).toEqual([])
      expect(router.callsOf('logout')).toHaveLength(1)
    })
  })

  describe('loginWithCookies', () => {
    it('rejects unknown platforms without calling the backend', async () => {
      const auth = useAuthStore()

      await expect(auth.loginWithCookies('spotify', 'a=b')).resolves.toBe(false)

      expect(router.calls).toEqual([])
    })

    it('returns false and keeps state when the cookies do not log in', async () => {
      router.on('login_with_cookies', () => loggedOutInfo('netease'))
      const auth = useAuthStore()

      await expect(auth.loginWithCookies('netease', 'MUSIC_U=expired')).resolves.toBe(false)

      expect(auth.netease).toEqual(loggedOut)
      expect(authEvents).toEqual([])
      expect(router.callsOf('login_with_cookies')[0].args).toEqual({ platform: 'netease', rawCookies: 'MUSIC_U=expired' })
    })

    it('shows the backend error and returns false on failure', async () => {
      router.on('login_with_cookies', () => { throw 'COOKIE_INVALID' })
      const auth = useAuthStore()

      await expect(auth.loginWithCookies('bilibili', 'SESSDATA=x')).resolves.toBe(false)

      expect(mocks.toasts).toEqual(['error:COOKIE_INVALID'])
      expect(auth.loggingIn).toBeNull()
    })

    it('logs in and broadcasts on success', async () => {
      router.on('login_with_cookies', () => info('bilibili', 'up'))
      const auth = useAuthStore()

      await expect(auth.loginWithCookies('bilibili', 'SESSDATA=ok')).resolves.toBe(true)

      expect(auth.bilibili).toEqual({ loggedIn: true, nickname: 'up', avatarUrl: 'https://img/bilibili.png' })
      expect(authEvents).toEqual(['bilibili'])
    })
  })
})
