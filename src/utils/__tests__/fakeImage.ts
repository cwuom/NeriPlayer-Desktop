import { vi } from 'vitest'

export type FakeImageOutcome = 'load' | 'error' | 'hang'

/** 只模拟 HTMLImageElement 的加载结果（onload/onerror 与尺寸），不涉及像素渲染 */
export class FakeImage {
  static outcome: FakeImageOutcome = 'load'
  static naturalSize = 300
  static instances: FakeImage[] = []

  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  crossOrigin: string | null = null
  referrerPolicy = ''
  decoding = 'auto'
  naturalWidth = 0
  naturalHeight = 0
  private source = ''

  constructor() {
    FakeImage.instances.push(this)
  }

  get src(): string {
    return this.source
  }

  set src(value: string) {
    this.source = value
    const outcome = FakeImage.outcome
    queueMicrotask(() => {
      if (outcome === 'load') {
        this.naturalWidth = FakeImage.naturalSize
        this.naturalHeight = FakeImage.naturalSize
        this.onload?.()
      } else if (outcome === 'error') {
        this.onerror?.()
      }
    })
  }
}

/** 重置状态并替换全局 Image；调用方需在 afterEach 中 vi.unstubAllGlobals() */
export function installFakeImage(): typeof FakeImage {
  FakeImage.outcome = 'load'
  FakeImage.naturalSize = 300
  FakeImage.instances = []
  vi.stubGlobal('Image', FakeImage)
  return FakeImage
}
