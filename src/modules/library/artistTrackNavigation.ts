import { invoke } from '@tauri-apps/api/core'
import type { TrackInfo } from '@/stores/player'
import { isBiliVideoSkipTrack, resolveBiliVideoSkipBvid } from '@/modules/playback/biliVideoSkip'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'
import { toHttpsImageUrl } from '@/utils/trackCover'

interface ArtistSearchRoute {
  name: 'explore'
  query: { q: string; platform: 'netease'; kind: 'artists' }
}

function payloadText(payload: Record<string, unknown> | undefined, ...keys: string[]): string {
  for (const key of keys) {
    const value = payload?.[key]
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return ''
}

export function artistSearchRoute(track: TrackInfo): ArtistSearchRoute | null {
  if (!getPlaybackSourceKind(track)) return null
  const artist = track.artist.trim()
  const customArtist = payloadText(track.syncPayload, 'customArtist', 'custom_artist')
  if (!artist || customArtist !== artist) return null
  const originalArtist = payloadText(track.syncPayload, 'originalArtist', 'original_artist', 'artist')
  if (customArtist === originalArtist) return null
  // 自定义歌手和播放身份可以来自不同平台，歌词匹配 ID 也不能当作歌手身份
  return { name: 'explore', query: { q: artist, platform: 'netease', kind: 'artists' } }
}

function biliArtistVideoIdentity(track: TrackInfo): { bvid: string | null; avid: number | null } | null {
  if (!isBiliVideoSkipTrack(track)) return null
  const bvid = resolveBiliVideoSkipBvid(track)
  if (bvid) return { bvid, avid: null }
  const values = [track.syncPayload?.audioId, track.syncPayload?.audio_id,
    track.id.replace(/^bilibili:/i, '').split(':')[0]]
  for (const value of values) {
    const text = String(value ?? '').trim()
    if (!/^\d+$/.test(text)) continue
    const avid = Number(text)
    if (Number.isSafeInteger(avid) && avid > 0) return { bvid: null, avid }
  }
  return null
}

export function artistNavigationTargetKey(track: TrackInfo | null): string {
  if (!track) return ''
  const source = getPlaybackSourceKind(track)
  const neteaseSongId = source === 'netease'
    ? Number(track.syncPayload?.audioId ?? track.syncPayload?.audio_id ?? track.id.replace(/^netease:/i, ''))
    : null
  // 歌词、偏移和封面写回会替换曲目对象，只比较真正决定歌手目标的字段
  return JSON.stringify({
    id: track.id,
    source,
    artist: track.artist.trim(),
    search: artistSearchRoute(track),
    biliVideo: source === 'bilibili' ? biliArtistVideoIdentity(track) : null,
    neteaseSongId: neteaseSongId !== null && Number.isSafeInteger(neteaseSongId) && neteaseSongId > 0 ? neteaseSongId : null,
  })
}

export async function resolveBiliArtistUploader(track: TrackInfo): Promise<{
  id: string; name: string; cover_url: string | null
} | null> {
  const identity = biliArtistVideoIdentity(track)
  if (!identity) return null
  const info = await invoke<{ owner_mid: number; owner: string; owner_face: string } | null>('get_bili_video_info', identity)
  if (!info || !Number.isSafeInteger(info.owner_mid) || info.owner_mid <= 0) return null
  return {
    id: String(info.owner_mid), name: info.owner,
    cover_url: info.owner_face?.trim() ? toHttpsImageUrl(info.owner_face.trim()) : null,
  }
}
