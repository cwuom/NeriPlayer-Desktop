import { vi } from 'vitest'

// Node 22+ 自带 globalThis.localStorage（未传 --localstorage-file 时为 undefined），
// vitest 的 happy-dom 环境因此不会覆盖它，测试需要显式注入可控的内存实现
export class MemoryStorage {
  private readonly items = new Map<string, string>()
  /** 返回 true 时 setItem 抛 QuotaExceededError，用于模拟配额超限 */
  rejectWrite: ((key: string, value: string) => boolean) | null = null
  rejectRead = false
  rejectRemove = false

  get length(): number {
    return this.items.size
  }

  key(index: number): string | null {
    return [...this.items.keys()][index] ?? null
  }

  getItem(key: string): string | null {
    if (this.rejectRead) throw new DOMException('storage blocked', 'SecurityError')
    return this.items.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    const text = String(value)
    if (this.rejectWrite?.(key, text)) throw new DOMException('quota exceeded', 'QuotaExceededError')
    this.items.set(key, text)
  }

  removeItem(key: string): void {
    if (this.rejectRemove) throw new DOMException('storage blocked', 'SecurityError')
    this.items.delete(key)
  }

  clear(): void {
    this.items.clear()
  }
}

/** 注入全新的内存 localStorage；调用方需在 afterEach 中 vi.unstubAllGlobals() */
export function installMemoryStorage(): MemoryStorage {
  const storage = new MemoryStorage()
  vi.stubGlobal('localStorage', storage)
  return storage
}
