import { ref } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { toHttpsImageUrl } from '@/utils/trackCover'
import { normalizeProxiedCoverUrl } from '@/utils/bilibiliCoverCache'

/**
 * 封面能否在直连失败后改走后端代理（B 站、网易云、QQ、YouTube 等图床）。
 * WebView 直连 googleusercontent / ytimg 常被网络环境拦下，只用 <img> 会直接落成占位图
 */
export function hasProxiedCoverFallback(url?: string | null): boolean {
  return !!url && !!normalizeProxiedCoverUrl(url)
}

/** CSP 只放行 https 图片，网络封面要先升级；本地路径须转成 asset 协议 WebView 才能加载 */
export function toDisplayableLibraryCoverUrl(value?: string | null): string {
  if (!value) return ''
  if (/^(https?:)?\/\//i.test(value)) return toHttpsImageUrl(value)
  if (/^(asset:|data:|blob:)/i.test(value)) return value
  return convertFileSrc(value)
}

/** 封面地址参与 key：地址变化后会重新尝试加载，而不是沿用旧的失败记录 */
export function libraryCoverKey(scope: string, id: string | number, url?: string | null): string {
  return `${scope}:${id}:${url || ''}`
}

/** 资料库各列表共用的封面失败记录，失败后回退占位图标 */
export function useLibraryCovers() {
  const failedLibraryCoverKeys = ref<Set<string>>(new Set())

  function isLibraryCoverFailed(scope: string, id: string | number, url?: string | null): boolean {
    if (!url) return false
    return failedLibraryCoverKeys.value.has(libraryCoverKey(scope, id, url))
  }

  function markLibraryCoverFailed(scope: string, id: string | number, url?: string | null) {
    if (!url) return
    failedLibraryCoverKeys.value = new Set(failedLibraryCoverKeys.value).add(libraryCoverKey(scope, id, url))
  }

  return { isLibraryCoverFailed, markLibraryCoverFailed }
}
