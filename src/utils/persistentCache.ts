// 平台详情与歌词缓存：落在用户数据库（对齐 Android Room 平台缓存）
//
// 以前存在 localStorage，5 MB 配额让大歌单反复触发「缩减 → 清桶」，缓存
// 实际经常失效。数据库按「过期时间 + 条数 + 字节」裁剪；浏览器开发模式没有
// Rust 后端时退回进程内缓存
import { invoke } from '@tauri-apps/api/core'
import { hasUserDataBackend, persistUserData } from '@/modules/persistence/userData'

export type CacheBucket = 'platform_detail' | 'lyrics' | 'playback_quality'

export interface CacheOptions {
  maxAgeMs: number
  maxEntries: number
  /// 整桶字节上限；不传则用默认值
  maxBytes?: number
}

const DEFAULT_MAX_BYTES = 64 * 1024 * 1024

/** 迁移到数据库之前的 localStorage 缓存桶，启动后直接丢弃 */
export const LEGACY_CACHE_BUCKET_KEYS = [
  'neri:playlist-detail-cache:v1',
  'neri:lyrics-cache:v3',
  'neri:lyrics-cache:v2',
  'neri:lyrics-cache:v1',
] as const

interface MemoryEntry {
  value: unknown
  updatedAt: number
}

const memory = new Map<string, Map<string, MemoryEntry>>()

function memoryBucket(bucket: CacheBucket) {
  let entries = memory.get(bucket)
  if (!entries) {
    entries = new Map()
    memory.set(bucket, entries)
  }
  return entries
}

export async function getCachedValue<T>(bucket: CacheBucket, key: string, maxAgeMs: number): Promise<T | null> {
  if (!hasUserDataBackend()) {
    const entry = memoryBucket(bucket).get(key)
    if (!entry || Date.now() - entry.updatedAt > maxAgeMs) return null
    return entry.value as T
  }
  try {
    return (await invoke<T | null>('cache_get', { bucket, key, maxAgeMs })) ?? null
  } catch (error) {
    console.warn(`[cache] ${bucket} read failed:`, error)
    return null
  }
}

export async function setCachedValue<T>(bucket: CacheBucket, key: string, value: T, options: CacheOptions): Promise<void> {
  if (!hasUserDataBackend()) {
    const entries = memoryBucket(bucket)
    entries.delete(key)
    entries.set(key, { value, updatedAt: Date.now() })
    while (entries.size > options.maxEntries) entries.delete(entries.keys().next().value!)
    return
  }
  try {
    await persistUserData('cache_put', {
      bucket,
      key,
      value,
      maxAgeMs: options.maxAgeMs,
      maxEntries: options.maxEntries,
      maxBytes: options.maxBytes ?? DEFAULT_MAX_BYTES,
    })
  } catch (error) {
    console.warn(`[cache] ${bucket} write failed:`, error)
  }
}

export async function removeCachedValue(bucket: CacheBucket, key: string): Promise<void> {
  if (!hasUserDataBackend()) {
    memoryBucket(bucket).delete(key)
    return
  }
  try {
    await persistUserData('cache_remove', { bucket, key })
  } catch (error) {
    console.warn(`[cache] ${bucket} remove failed:`, error)
  }
}

export function removeLegacyCacheBuckets(storage: Pick<Storage, 'removeItem'> | undefined = globalThis.localStorage) {
  if (!storage) return
  for (const key of LEGACY_CACHE_BUCKET_KEYS) storage.removeItem(key)
}
