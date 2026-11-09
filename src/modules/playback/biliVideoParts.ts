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

export type BiliPartNameOrder = 'title-first' | 'artist-first'

function stripPartNumber(part: string): string {
  const trimmed = part.trim()
  return trimmed.replace(/^\d+\.\s*/, '').trim() || trimmed
}

function splitPartName(name: string): [string, string] | null {
  const separators = [...name.matchAll(/\s[-\u2013\u2014]\s/g)]
  if (separators.length !== 1) return null
  const separator = separators[0]
  const left = name.slice(0, separator.index).trim()
  const right = name.slice(separator.index! + separator[0].length).trim()
  return left && right ? [left, right] : null
}

export function hasBiliPartNameSeparator(part: string): boolean {
  return splitPartName(stripPartNumber(part)) !== null
}

export function buildBiliPartTracks(
  info: BiliVideoInfo,
  parent: TrackInfo,
  nameOrder: BiliPartNameOrder = 'title-first',
): TrackInfo[] {
  return info.pages.map(page => {
    const partName = stripPartNumber(page.part) || info.title
    const fields = splitPartName(partName)
    if (fields && nameOrder === 'artist-first') fields.reverse()
    const [title, artist] = fields ?? [partName || info.title, info.owner]
    return {
      id: `bilibili:${info.aid}:${page.cid}`,
      title,
      artist,
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
