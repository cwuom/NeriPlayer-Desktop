// 对齐 Android YouTubeSeekRefreshPolicy：判断 YouTube 直链在跳转 / 继续播放前是否需要先换新地址。
// 缓存或已下载播放没有在线地址（currentUrl 为空），一律不刷新。

const URL_EXPIRY_GRACE_MS = 2 * 60 * 1000
// 网页类客户端的直链缺 PoToken 时只能取开头一段，远跳或续播会被 403
const PO_TOKEN_CLIENTS = new Set(['', 'WEB_REMIX', 'WEB_CREATOR', 'TVHTML5'])

function parseUrl(url: string): URL | null {
  try {
    return new URL(url)
  } catch {
    return null
  }
}

export function isYouTubeGoogleVideoHost(host: string): boolean {
  const normalized = host.toLowerCase()
  return normalized === 'googlevideo.com' || normalized.endsWith('.googlevideo.com')
}

/** 清单流、分段流，或已带解好的 n / sig 的直链，可以直接按范围跳转 */
export function supportsSeekingWithoutUrlRefresh(url: string): boolean {
  const parsed = parseUrl(url)
  if (!parsed || !isYouTubeGoogleVideoHost(parsed.hostname)) return false
  const host = parsed.hostname.toLowerCase()
  const path = parsed.pathname.toLowerCase()
  if (host.startsWith('manifest.') || path.includes('/api/manifest/')) return true
  if (path.includes('/playlist/index.m3u8') || path.includes('/file/seg.ts')) return true
  const params = parsed.searchParams
  return !!params.get('n')?.trim() || !!params.get('sig')?.trim() || !!params.get('signature')?.trim()
}

function shouldRefreshForMissingPoToken(parsed: URL): boolean {
  if (!isYouTubeGoogleVideoHost(parsed.hostname)) return false
  if ((parsed.searchParams.get('source') ?? '').toLowerCase() !== 'youtube') return false
  if (parsed.searchParams.get('pot')?.trim()) return false
  return PO_TOKEN_CLIENTS.has((parsed.searchParams.get('c') ?? '').trim().toUpperCase())
}

function isNearExpiry(parsed: URL, now: number): boolean {
  const seconds = Number(parsed.searchParams.get('expire'))
  if (!parsed.searchParams.has('expire') || !Number.isFinite(seconds)) return false
  return now + URL_EXPIRY_GRACE_MS >= seconds * 1000
}

function onlineUrl(isYouTubeTrack: boolean, currentUrl?: string | null): string | null {
  if (!isYouTubeTrack) return null
  const url = currentUrl?.trim()
  if (!url || url.startsWith('file://')) return null
  return url
}

export function shouldRefreshUrlBeforeSeek(isYouTubeTrack: boolean, currentUrl?: string | null, now = Date.now()): boolean {
  const url = onlineUrl(isYouTubeTrack, currentUrl)
  if (!url) return false
  const parsed = parseUrl(url)
  if (parsed && (shouldRefreshForMissingPoToken(parsed) || isNearExpiry(parsed, now))) return true
  return !supportsSeekingWithoutUrlRefresh(url)
}

export function shouldRefreshUrlBeforeResume(isYouTubeTrack: boolean, currentUrl?: string | null, now = Date.now()): boolean {
  const url = onlineUrl(isYouTubeTrack, currentUrl)
  if (!url) return false
  const parsed = parseUrl(url)
  return !!parsed && (shouldRefreshForMissingPoToken(parsed) || isNearExpiry(parsed, now))
}
