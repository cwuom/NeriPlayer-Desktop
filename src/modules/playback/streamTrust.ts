// channelId 取值与一起听协议 LtChannels 一致；放在 modules 层，播放取流与一起听 store 共用

// 入站 streamUrl 白名单：除 http(s) 外还要匹配曲目来源的 CDN 域名，
// 否则任意房间成员都能把第三方 URL 伪装成音频直链投递到播放器
export function isTrustedInboundStreamUrl(url?: string, channelId?: string): boolean {
  if (!url) return false
  const v = url.trim()
  if (!v) return false
  let parsed: URL
  try {
    parsed = new URL(v)
  } catch {
    return false
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return false
  const host = parsed.hostname.toLowerCase()
  if (!host) return false
  switch (channelId) {
    case 'netease':
      return host === 'music.126.net' || host.endsWith('.music.126.net')
    case 'bilibili':
      return host === 'bilivideo.com'
        || host.endsWith('.bilivideo.com')
        || host === 'bilivideo.cn'
        || host.endsWith('.bilivideo.cn')
        || host === 'hdslb.com'
        || host.endsWith('.hdslb.com')
        || host === 'mountaintoys.cn'
        || host.endsWith('.mountaintoys.cn')
    case 'youtubeMusic':
      return host === 'googlevideo.com'
        || host.endsWith('.googlevideo.com')
        || host === 'youtube.com'
        || host.endsWith('.youtube.com')
        || host === 'youtube-nocookie.com'
        || host.endsWith('.youtube-nocookie.com')
    default:
      return false
  }
}

export function trustedInboundStreamUrls(
  channelId: string,
  streamUrls?: readonly string[] | null,
  legacyStreamUrl?: string | null,
): string[] {
  const limit = channelId === 'bilibili' ? 2
    : channelId === 'youtubeMusic' ? 1 : 3
  const candidates = [...(streamUrls || []), legacyStreamUrl]
  const trusted = candidates
    .filter((url): url is string => typeof url === 'string')
    .map(url => url.trim())
    .filter(url => isTrustedInboundStreamUrl(url, channelId))
  return [...new Set(trusted)].slice(0, limit)
}
