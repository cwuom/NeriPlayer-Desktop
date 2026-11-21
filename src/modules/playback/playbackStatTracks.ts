import { invoke } from '@tauri-apps/api/core'

/** 按同步身份键还原统计记录对应的曲目；后端还原不了的位置为 null */
export function fetchPlaybackStatTracks(identityKeys: string[]) {
  return invoke<Array<Record<string, unknown> | null>>('get_playback_stat_tracks', { identityKeys })
}
