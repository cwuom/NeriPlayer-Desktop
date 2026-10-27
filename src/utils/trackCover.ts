/// 网易云等接口常返回 http:// 封面，CSP 只放行 https 图片，不升级就整片加载失败。
/// Windows 上本地文件经 convertFileSrc 变成 http://asset.localhost，不能动
export function toHttpsImageUrl(value: string): string {
  if (value.startsWith('//')) return `https:${value}`
  if (/^http:\/\/asset\.localhost\//i.test(value)) return value
  return value.replace(/^http:\/\//i, 'https://')
}

export interface TrackCoverSource {
  coverUrl?: unknown
  cover_url?: unknown
  syncPayload?: Record<string, unknown> | null
}

/** YouTube Music thumbnail size suffix: =w60-h60-l90-rj -> =w226-h226 */
const YOUTUBE_SIZE_PARAM_RE = /=w\d+(-h\d+)?(-[a-zA-Z0-9-]+)*$/
const YOUTUBE_S_PARAM_RE = /=s\d+(-[a-zA-Z0-9-]+)*$/
/** List rows render at roughly 40-120 CSS px; a 1200x1200 decode is about 6 MB per image */
const YOUTUBE_LIST_SIZE = '=w226-h226'
const YOUTUBE_DETAIL_SIZE = '=w1200-h1200'

/**
 * Cover size class: list thumbnails for rows, detail for the player page.
 * `original` keeps the URL untouched (the tray has its own dataURL cache and shows one image).
 */
export type CoverSize = 'list' | 'detail' | 'original'

/** Target edge for the NetEase `?param` suffix: 300 for lists, 640 for the player (2x enough) */
export const COVER_SIZE_LIST = 300
export const COVER_SIZE_DETAIL = 640

const NET_EASE_COVER_HOST = /(^|\.)music\.126\.net$/i

/**
 * Append `?param=WxH` to NetEase covers so the server returns display-sized art; other
 * hosts are returned unchanged.
 *
 * Without the parameter the API hands out originals: a sampled 3000x3000 / 1.4 MB cover
 * decodes to about 36 MB of RGBA, so a few dozen rows push the WebProcess into hundreds of
 * megabytes. An existing `param` is only upgraded when it is smaller, which keeps the call
 * idempotent and never downgrades the player page.
 */
export function withCoverSize(raw: string, px: number, exact = false): string {
  if (typeof raw !== 'string') return ''
  const url = raw.trim()
  if (!url) return ''
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return url
  }
  if (!NET_EASE_COVER_HOST.test(parsed.hostname)) return url
  const match = /^(\d+)y(\d+)$/i.exec(parsed.searchParams.get('param') || '')
  if (!exact && match && Math.min(Number(match[1]), Number(match[2])) >= px) return url
  parsed.searchParams.set('param', `${px}y${px}`)
  return parsed.toString()
}

function coverSizeParam(size: CoverSize): string {
  return size === 'detail' ? YOUTUBE_DETAIL_SIZE : YOUTUBE_LIST_SIZE
}

/**
 * Set the YouTube Music thumbnail size by usage. Same shape as Android's
 * upgradeYouTubeThumbnailUrl, except lists no longer force 1200x1200.
 */
export function upgradeYouTubeThumbnailUrl(url: string, size: CoverSize = 'list'): string {
  if (typeof url !== 'string') return ''
  const trimmed = url.trim()
  if (!trimmed) return url
  const target = coverSizeParam(size)

  if (YOUTUBE_SIZE_PARAM_RE.test(trimmed)) {
    return trimmed.replace(YOUTUBE_SIZE_PARAM_RE, target)
  }

  if (YOUTUBE_S_PARAM_RE.test(trimmed) && isGoogleImageHost(trimmed)) {
    return trimmed.replace(YOUTUBE_S_PARAM_RE, target)
  }

  if (isGoogleImageHost(trimmed) && !trimmed.includes('=')) {
    return `${trimmed}${target}`
  }

  return trimmed
}

/** Remove only the size suffix/param that this app added, and only on known hosts. */
function stripOriginalCoverSize(url: string): string {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return url
  }
  if (NET_EASE_COVER_HOST.test(parsed.hostname)) {
    const param = parsed.searchParams.get('param')
    if (param && /^\d+y\d+$/i.test(param)) parsed.searchParams.delete('param')
    return parsed.toString()
  }
  if (isGoogleImageHost(url)) {
    parsed.pathname = parsed.pathname.replace(YOUTUBE_SIZE_PARAM_RE, '').replace(YOUTUBE_S_PARAM_RE, '')
    return parsed.toString()
  }
  return url
}

function firstCoverValue(track: TrackCoverSource | null | undefined): string {
  const payload = track?.syncPayload
  const candidates = [
    track?.coverUrl,
    track?.cover_url,
    payload?.customCoverUrl,
    payload?.custom_cover_url,
    payload?.coverUrl,
    payload?.cover_url,
    payload?.originalCoverUrl,
    payload?.original_cover_url,
  ]
  const value = candidates.find(candidate => typeof candidate === 'string' && candidate.trim())
  return typeof value === 'string' ? value.trim() : ''
}

/** Resolvable cover URL: thumbnails by default, pass `detail` on the player page */
export function getTrackCoverUrl(
  track: TrackCoverSource | null | undefined,
  size: CoverSize = 'list',
): string {
  const value = firstCoverValue(track)
  if (!value) return ''
  // Synced NetEase tracks often carry http:// covers while the CSP allows https only
  const https = toHttpsImageUrl(value)
  if (size === 'original') return stripOriginalCoverSize(https)
  return withCoverSize(
    upgradeYouTubeThumbnailUrl(https, size),
    size === 'detail' ? COVER_SIZE_DETAIL : COVER_SIZE_LIST,
    size === 'list',
  )
}

/**
 * Cover written into downloaded file metadata: keep custom/original artwork untouched
 * (no NetEase `param`), but upgrade YouTube thumbnails to the full 1200px image.
 */
export function getDownloadCoverUrl(track: TrackCoverSource | null | undefined): string {
  const value = firstCoverValue(track)
  if (!value) return ''
  return upgradeYouTubeThumbnailUrl(toHttpsImageUrl(value), 'detail')
}

function isGoogleImageHost(url: string): boolean {
  try {
    const host = new URL(url.startsWith('//') ? `https:${url}` : url).hostname.toLowerCase()
    return (
      host === 'lh3.googleusercontent.com'
      || host.endsWith('.googleusercontent.com')
      || host === 'yt3.ggpht.com'
      || host.endsWith('.ggpht.com')
    )
  } catch {
    return (
      url.includes('lh3.googleusercontent.com')
      || url.includes('googleusercontent.com')
      || url.includes('yt3.ggpht.com')
      || url.includes('ggpht.com')
    )
  }
}
