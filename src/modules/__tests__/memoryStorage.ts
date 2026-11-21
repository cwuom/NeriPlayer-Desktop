/**
 * 测试用内存版 localStorage
 *
 * Node 22+ 自带实验性的全局 localStorage，未传 --localstorage-file 时为 undefined，
 * 且会遮住 happy-dom 提供的实现，所以缓存类测试统一用 vi.stubGlobal 注入这个类
 */
export class MemoryStorage {
  private readonly items = new Map<string, string>()

  get length(): number {
    return this.items.size
  }

  key(index: number): string | null {
    return [...this.items.keys()][index] ?? null
  }

  getItem(key: string): string | null {
    return this.items.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    this.items.set(key, String(value))
  }

  removeItem(key: string): void {
    this.items.delete(key)
  }

  clear(): void {
    this.items.clear()
  }
}

/** 找到唯一被写入的缓存桶，用于注入损坏数据，避免在测试里硬编码存储键 */
export function onlyBucketKey(storage: MemoryStorage): string {
  if (storage.length !== 1) throw new Error(`expected exactly one cache bucket, got ${storage.length}`)
  const key = storage.key(0)
  if (!key) throw new Error('expected a cache bucket')
  return key
}

/** 直接改写桶内某条缓存的 value，模拟旧版本或被篡改的数据 */
export function overwriteCachedValue(
  storage: MemoryStorage,
  bucketKey: string,
  entryKey: string,
  value: unknown,
): void {
  const bucket = JSON.parse(storage.getItem(bucketKey) ?? '{}') as Record<string, { value: unknown }>
  const entry = bucket[entryKey]
  if (!entry) throw new Error(`missing cache entry ${entryKey}`)
  entry.value = value
  storage.setItem(bucketKey, JSON.stringify(bucket))
}
