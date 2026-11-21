import { invoke } from '@tauri-apps/api/core'
import {
  BilibiliCoverCache,
  type ResolveBilibiliCoverOptions,
} from './bilibiliCoverCache'

export {
  normalizeBilibiliCoverUrl,
  normalizeCoverUrlForDisplay,
  normalizeProxiedCoverUrl,
} from './bilibiliCoverCache'

const COVER_DECODE_TIMEOUT_MS = 10_000

const coverCache = new BilibiliCoverCache(
  (url, forceRefresh) => invoke<string>('fetch_bilibili_cover', { url, forceRefresh }),
  decodeBilibiliCover,
)

export function resolveCoverImage(
  rawUrl: string,
  options?: ResolveBilibiliCoverOptions,
): Promise<string> {
  return coverCache.resolve(rawUrl, options)
}

export function peekCoverImage(rawUrl: string): string {
  return coverCache.peek(rawUrl)
}

export const resolveBilibiliCover = resolveCoverImage

function decodeBilibiliCover(dataUrl: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    let settled = false
    // 显式写 number：测试配置同时带 Node 类型时，ReturnType 会取到 NodeJS.Timeout 重载
    let timer: number | null = null

    const finish = (complete: () => void) => {
      if (settled) return
      settled = true
      if (timer !== null) window.clearTimeout(timer)
      image.onload = null
      image.onerror = null
      complete()
    }

    timer = window.setTimeout(
      () => finish(() => reject(new Error('Bilibili cover decode timed out'))),
      COVER_DECODE_TIMEOUT_MS,
    )
    image.decoding = 'async'
    image.onload = () => finish(() => {
      if (image.naturalWidth > 0 && image.naturalHeight > 0) resolve()
      else reject(new Error('Bilibili cover decoded without image dimensions'))
    })
    image.onerror = () => finish(() => reject(new Error('Bilibili cover decode failed')))
    image.src = dataUrl
  })
}
