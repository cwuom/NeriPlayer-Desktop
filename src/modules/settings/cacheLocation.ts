import { invoke } from '@tauri-apps/api/core'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'

/** 音频与封面缓存所在目录（后端 MediaCacheLocation） */
export interface MediaCacheLocation {
  /** 当前生效的目录；自定义目录不可用时为默认目录 */
  path: string
  defaultPath: string
  /** 已保存的自定义目录，空串为默认 */
  customPath: string
  isDefault: boolean
}

/** 旧缓存的处理方式：搬到新目录，或丢弃后从头缓存 */
export type ExistingCacheAction = 'move' | 'discard'

export interface MediaCacheRelocation {
  location: MediaCacheLocation
  movedFiles: number
  movedBytes: number
  removedFiles: number
  /** 搬不动或删不掉、留在旧目录的文件数 */
  failedCount: number
}

export function getMediaCacheLocation(): Promise<MediaCacheLocation> {
  return invoke<MediaCacheLocation>('get_media_cache_dir')
}

export function setMediaCacheDir(path: string, existing: ExistingCacheAction): Promise<MediaCacheRelocation> {
  return invoke<MediaCacheRelocation>('set_media_cache_dir', { path, existing })
}

export function resetMediaCacheDir(existing: ExistingCacheAction): Promise<MediaCacheRelocation> {
  return invoke<MediaCacheRelocation>('reset_media_cache_dir', { existing })
}

export function openMediaCacheDir(): Promise<void> {
  return invoke<void>('open_media_cache_dir')
}

/** 选择新的缓存目录；取消时返回 null */
export async function pickMediaCacheDir(defaultPath?: string): Promise<string | null> {
  const result = await dialogOpen({ directory: true, multiple: false, defaultPath: defaultPath || undefined })
  return selectedDirectory(result)
}

export function selectedDirectory(result: unknown): string | null {
  const value = Array.isArray(result) ? result[0] : result
  if (typeof value === 'string') return value.trim() || null
  // 兼容返回 { path } 对象的旧对话框实现
  if (value && typeof value === 'object' && 'path' in value && typeof value.path === 'string') {
    return value.path.trim() || null
  }
  return null
}

const ERROR_KEYS: Record<string, string> = {
  CACHE_DIR_NOT_ABSOLUTE: 'settings.cache_location_error_invalid',
  CACHE_DIR_NOT_DIRECTORY: 'settings.cache_location_error_invalid',
  CACHE_DIR_NOT_WRITABLE: 'settings.cache_location_error_not_writable',
  CACHE_DIR_OVERLAPS: 'settings.cache_location_error_overlap',
  CACHE_DIR_BUSY: 'settings.cache_location_error_busy',
}

/** 后端错误码转成文案键；未知错误归为通用失败 */
export function cacheLocationErrorKey(error: unknown): string {
  const message = typeof error === 'string' ? error : error instanceof Error ? error.message : ''
  return ERROR_KEYS[message.trim()] ?? 'settings.cache_location_failed'
}
