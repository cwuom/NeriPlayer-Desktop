import { createLogger } from '@/utils/logger'
import type { StorageCacheClearOptions, StorageUsageSummary } from '@/utils/storage'

// 设置页拆出的各 composable 沿用页面原有日志作用域，日志过滤方式保持不变
export const settingsLog = createLogger('settings-view')

/** 纯函数需要的翻译函数子集，便于脱离 vue-i18n 单测 */
export type Translate = (key: string, named?: Record<string, unknown>) => string

export interface BuildInfo {
  app_version: string
  build_uuid: string
  build_timestamp: string
  version: string
}

export interface StorageClearResult {
  clearedBytes: number
  deletedFiles: number
  failedCount: number
}

export interface AudioOutputDeviceOption {
  name: string
  /** Fallback display name from the backend: alsa-lib description or PCM name */
  label: string
  /** `sink` for a sound server output, `device` for a cpal/ALSA device */
  kind?: string | null
  /** Card display name, matching KDE */
  card?: string | null
  /** Role key composed into `<card> <role>`; absent for sound server sinks */
  role?: string | null
  detail?: string | null
  isDefault: boolean
}

/**
 * 设置页依赖的后端命令。
 *
 * 组件层不允许新增直接 invoke()（ui_invoke_baseline 只减不增），
 * 因此由 SettingsView 实现后注入，composable 只负责状态与流程。
 */
export interface SettingsIpc {
  listAudioOutputDevices(): Promise<AudioOutputDeviceOption[]>
  setAudioOutputDevice(name: string | null): Promise<unknown>
  getLogDir(): Promise<string>
  revealInFileManager(path: string): Promise<unknown>
  setBypassProxy(bypass: boolean): Promise<unknown>
  getDefaultDownloadDir(): Promise<string>
  setDownloadDir(path: string): Promise<string>
  getStorageUsage(downloadDir: string | null): Promise<StorageUsageSummary>
  clearStorageCache(options: StorageCacheClearOptions, downloadDir: string | null): Promise<StorageClearResult>
  probePlatformConnectivity(platform: string): Promise<boolean>
  importBackgroundImage(source: string): Promise<string>
  clearBackgroundImages(): Promise<unknown>
  getBuildInfo(): Promise<BuildInfo>
}

// 兼容返回 { path } 对象的旧对话框实现，非字符串且无 path 时退化为字符串化结果
export function dialogSelectionPath(result: unknown): string {
  if (typeof result === 'string') return result
  const path = typeof result === 'object' && result !== null && 'path' in result ? result.path : undefined
  return (typeof path === 'string' && path) || String(result)
}

// 折叠区 <Transition> 的 JS hooks：高度在 0 与 scrollHeight 之间过渡。
// 必须保持单参数，Vue 依据形参个数判断是否需要手动调用 done()
export function onExpandEnter(el: Element) {
  const e = el as HTMLElement
  e.style.overflow = 'hidden'
  e.style.height = '0'
  // 强制 reflow
  void e.offsetHeight
  e.style.transition = 'height 300ms cubic-bezier(0.2, 0, 0, 1), opacity 250ms ease'
  e.style.height = e.scrollHeight + 'px'
  e.style.opacity = '1'
}

export function onExpandAfterEnter(el: Element) {
  const e = el as HTMLElement
  e.style.height = ''
  e.style.overflow = ''
  e.style.transition = ''
}

export function onExpandLeave(el: Element) {
  const e = el as HTMLElement
  e.style.overflow = 'hidden'
  e.style.height = e.scrollHeight + 'px'
  void e.offsetHeight
  e.style.transition = 'height 250ms cubic-bezier(0.3, 0, 0.8, 0.15), opacity 200ms ease'
  e.style.height = '0'
  e.style.opacity = '0'
}

export function onExpandAfterLeave(el: Element) {
  const e = el as HTMLElement
  e.style.height = ''
  e.style.overflow = ''
  e.style.transition = ''
  e.style.opacity = ''
}
