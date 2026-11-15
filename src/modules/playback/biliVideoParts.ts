import type { TrackInfo } from '@/stores/player'
import { createLogger } from '@/utils/logger'
import { isBiliVideoSkipTrack, resolveBiliVideoSkipBvid } from './biliVideoSkip'
export { resolveBiliVideoSkipCid as resolveBiliPartCid } from './biliVideoSkip'

const log = createLogger('bili-video-parts')
const NAME_ORDER_STORAGE_PREFIX = 'neri:bili-part-name-order:v1:'

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

export function readBiliPartNameOrder(bvid: string): BiliPartNameOrder {
  try {
    return localStorage.getItem(`${NAME_ORDER_STORAGE_PREFIX}${bvid}`) === 'artist-first'
      ? 'artist-first'
      : 'title-first'
  } catch (error) {
    log.warn('Failed to read part name order:', error)
    return 'title-first'
  }
}

export function saveBiliPartNameOrder(bvid: string, order: BiliPartNameOrder) {
  try {
    const key = `${NAME_ORDER_STORAGE_PREFIX}${bvid}`
    if (order === 'artist-first') localStorage.setItem(key, order)
    else localStorage.removeItem(key)
  } catch (error) {
    log.warn('Failed to save part name order:', error)
  }
}

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

export function resolveBiliVideoReference(track: Pick<TrackInfo, 'id' | 'source' | 'album' | 'syncPayload'>) {
  if (!isBiliVideoSkipTrack(track)) return null
  const bvid = resolveBiliVideoSkipBvid(track)
  const id = track.id.replace(/^bilibili:/i, '').split(':')[0]
  const ids = [track.syncPayload?.audioId, track.syncPayload?.audio_id, id]
  const avid = ids.map(value => Number(/^(?:av)?(\d+)$/i.exec(String(value).trim())?.[1]))
    .find(id => id > 0 && Number.isSafeInteger(id)) ?? null
  return bvid || avid ? { bvid, avid } : null
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
    const [title, artist] = fields ?? [partName, info.owner || parent.artist]
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
