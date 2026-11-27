import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UpdateCheck } from '@/modules/update/updateModel'
import { useUpdateStore } from '@/stores/update'
import { createInvokeRouter, deferred, flushPromises } from './storeTestUtils'

const mocks = vi.hoisted(() => ({
  router: null as ReturnType<typeof import('./storeTestUtils').createInvokeRouter> | null,
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  toasts: [] as Array<{ text: string; action?: { handler: () => void } }>,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => mocks.router!.invoke(command, args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (event: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(event, handler)
    return () => { mocks.listeners.delete(event) }
  },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/i18n', () => ({ default: { global: { t: (key: string) => key } } }))
vi.mock('@/stores/toast', () => ({
  useToastStore: () => ({
    show: (text: string, _type: string, options: { action?: { handler: () => void } }) => {
      mocks.toasts.push({ text, action: options.action })
    },
  }),
}))

function check(overrides: Partial<UpdateCheck> = {}): UpdateCheck {
  return {
    status: 'available',
    currentVersion: '1.0.0',
    latestVersion: '1.1.0',
    releaseUrl: 'https://example.invalid/r',
    inApp: true,
    unsupportedReason: null,
    summary: { downloadBytes: 1000, fullBytes: 9000, changedFiles: 1, totalFiles: 4 },
    ...overrides,
  }
}

describe('update store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    mocks.router = createInvokeRouter()
    mocks.listeners.clear()
    mocks.toasts = []
  })

  it('opens the dialog for a manual check and only toasts for a background one', async () => {
    mocks.router!.on('update_check', () => check())
    const store = useUpdateStore()

    await store.checkForUpdates({ manual: false })
    expect(store.phase).toBe('available')
    expect(store.dialogOpen).toBe(false)
    expect(mocks.toasts.map(toast => toast.text)).toEqual(['update.toast_available'])
    mocks.toasts[0].action!.handler()
    expect(store.dialogOpen).toBe(true)

    store.dialogOpen = false
    await store.checkForUpdates({ manual: true })
    expect(store.dialogOpen).toBe(true)
    expect(store.lastCheckAt).not.toBeNull()
  })

  it('stays silent about a skipped version on background checks', async () => {
    mocks.router!.on('update_check', () => check())
    const store = useUpdateStore()
    await store.checkForUpdates({ manual: true })
    store.skipCurrentVersion()
    expect(store.skippedVersion).toBe('1.1.0')
    expect(store.dialogOpen).toBe(false)

    await store.checkForUpdates({ manual: false })
    expect(mocks.toasts).toHaveLength(0)
  })

  it('maps up-to-date and disabled builds to their own phases', async () => {
    mocks.router!.on('update_check', () => check({ status: 'upToDate', latestVersion: null }), () => check({ status: 'disabled' }))
    const store = useUpdateStore()
    await store.checkForUpdates({ manual: true })
    expect(store.phase).toBe('upToDate')
    expect(store.dialogOpen).toBe(false)
    await store.checkForUpdates({ manual: true })
    expect(store.phase).toBe('disabled')
  })

  it('reports manual check failures but swallows background ones', async () => {
    mocks.router!.on('update_check', () => { throw { code: 'network', message: 'offline' } })
    const store = useUpdateStore()
    await store.checkForUpdates({ manual: false })
    expect(store.phase).toBe('idle')
    expect(store.errorCode).toBeNull()

    await store.checkForUpdates({ manual: true })
    expect(store.phase).toBe('error')
    expect(store.errorCode).toBe('network')
    expect(store.errorStage).toBe('check')
  })

  it('downloads with progress, then installs', async () => {
    const download = deferred<unknown>()
    mocks.router!.on('update_check', () => check())
    mocks.router!.on('update_download', () => download.promise)
    mocks.router!.on('update_install', () => undefined)
    const store = useUpdateStore()
    await store.checkForUpdates({ manual: true })

    const running = store.download()
    await flushPromises()
    expect(store.phase).toBe('downloading')
    expect(store.busy).toBe(true)
    mocks.listeners.get('update:progress')!({ payload: { downloaded: 400, total: 1000 } })
    expect(store.progress).toEqual({ downloaded: 400, total: 1000 })

    download.resolve(undefined)
    await running
    expect(store.phase).toBe('ready')
    expect(mocks.listeners.has('update:progress')).toBe(false)

    await store.install()
    expect(store.phase).toBe('installing')
    expect(mocks.router!.callsOf('update_install')).toHaveLength(1)
  })

  it('returns to available when the user cancels, and retries failed downloads', async () => {
    mocks.router!.on('update_check', () => check())
    mocks.router!.on(
      'update_download',
      () => { throw { code: 'cancelled', message: 'cancelled' } },
      () => { throw { code: 'corrupted', message: 'bad hash' } },
      () => undefined,
    )
    mocks.router!.on('update_cancel', () => undefined)
    const store = useUpdateStore()
    await store.checkForUpdates({ manual: true })

    await store.download()
    expect(store.phase).toBe('available')
    expect(store.errorCode).toBeNull()

    await store.download()
    expect(store.phase).toBe('error')
    expect(store.errorStage).toBe('download')

    store.retry()
    await flushPromises()
    expect(store.phase).toBe('ready')
  })

  it('does not start a download for builds that cannot update in place', async () => {
    mocks.router!.on('update_check', () => check({ inApp: false, unsupportedReason: 'windowsMsi', summary: null }))
    const store = useUpdateStore()
    await store.checkForUpdates({ manual: true })
    await store.download()
    expect(mocks.router!.callsOf('update_download')).toHaveLength(0)
    expect(store.phase).toBe('available')
  })

  it('forwards cancel only while downloading and respects the auto-check switch', async () => {
    mocks.router!.on('update_check', () => check())
    const store = useUpdateStore()
    await store.cancelDownload()
    expect(mocks.router!.callsOf('update_cancel')).toHaveLength(0)

    store.setAutoCheck(false)
    await store.checkOnLaunch()
    expect(mocks.router!.callsOf('update_check')).toHaveLength(0)

    store.setAutoCheck(true)
    await store.checkOnLaunch()
    expect(mocks.router!.callsOf('update_check')).toHaveLength(1)
    // 刚检查过，间隔内再次启动不重复请求
    await store.checkOnLaunch()
    expect(mocks.router!.callsOf('update_check')).toHaveLength(1)
  })
})
