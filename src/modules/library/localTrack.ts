import type { TrackInfo } from '@/types/track'

export function isLocalTrack(track: Pick<TrackInfo, 'id' | 'source'>): boolean {
  return track.source === 'local' || track.id.startsWith('local:')
}

/** 本地文件曲目在磁盘上的路径；不是本地文件（或地址是网络、资源协议）时返回空串 */
export function localTrackFilePath(track: Pick<TrackInfo, 'id' | 'source' | 'audioUrl'>): string {
  if (!isLocalTrack(track)) return ''
  const url = track.audioUrl?.trim() ?? ''
  if (!url) return ''
  if (!/^file:/i.test(url)) {
    const windowsPath = /^[A-Za-z]:[\\/]/.test(url)
    return !windowsPath && /^[A-Za-z][A-Za-z0-9+.-]*:/.test(url) ? '' : url
  }
  try {
    const parsed = new URL(url)
    const path = decodeURIComponent(parsed.pathname)
    if (parsed.hostname) return `//${parsed.hostname}${path}`
    // file:///C:/Music/a.flac 的 pathname 是 /C:/Music/a.flac
    return /^\/[A-Za-z]:\//.test(path) ? path.slice(1) : path
  } catch {
    return ''
  }
}
