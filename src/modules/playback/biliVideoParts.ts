import type { TrackInfo } from '@/stores/player'

export interface BiliVideoPage {
  cid: number
  page: number
  part: string
  duration: number
}

export interface BiliVideoInfo {
  bvid: string
  aid: number
  title: string
  owner: string
  cover: string
  cid: number
  duration: number
  pages: BiliVideoPage[]
}

export function buildBiliPartTracks(info: BiliVideoInfo, parent: TrackInfo): TrackInfo[] {
  return info.pages.map(page => {
    const rawTitle = page.part.trim()
    const normalized = rawTitle.replace(/^\d+\.\s*/, '').trim() || rawTitle || info.title
    const separators = [...normalized.matchAll(/\s[-\u2013\u2014]\s/g)]
    const separator = separators.length === 1 ? separators[0] : undefined
    const title = separator ? normalized.slice(0, separator.index).trim() : ''
    const artist = separator ? normalized.slice(separator.index! + separator[0].length).trim() : ''
    return {
      id: `bilibili:${info.aid}:${page.cid}`,
      title: title && artist ? title : normalized,
      artist: title && artist ? artist : info.owner,
      album: `Bilibili|${page.cid}|${info.bvid}`,
      durationMs: page.duration * 1000,
      coverUrl: parent.coverUrl || info.cover,
      audioUrl: '',
      source: 'bilibili',
      syncPayload: {
        channelId: 'bilibili', audioId: String(info.aid), subAudioId: String(page.cid), bvid: info.bvid,
      },
    }
  })
}
