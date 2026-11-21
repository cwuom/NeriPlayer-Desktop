import { invoke } from '@tauri-apps/api/core'
import type { PlaybackAudioSource } from './playbackSource'

// 当前音频质量信息
export interface AudioInfo {
  bitrate?: number  // kbps
  codec?: string    // e.g. "MP3", "FLAC", "AAC", "Opus"
  format?: string   // 原始格式标识
  source?: PlaybackAudioSource
  qualityKey?: string
  qualityLabel?: string
  qualityOptions?: Array<{ key: string; label: string }>
  mimeType?: string
  sampleRateHz?: number
  bitDepth?: number
  channelCount?: number
  specLabel?: string
}

export type PlaybackAudioProperties = Pick<AudioInfo, 'bitrate' | 'codec' | 'sampleRateHz' | 'bitDepth' | 'channelCount'>

export async function loadPlaybackAudioInfo(
  requestGeneration: number,
  isCurrent: () => boolean,
  update: (info: PlaybackAudioProperties) => void,
): Promise<void> {
  if (!isCurrent()) return
  const raw = await invoke<PlaybackAudioProperties | null>('get_playback_audio_info', { requestGeneration })
  if (!isCurrent() || !raw || typeof raw !== 'object') return

  const info: PlaybackAudioProperties = {}
  for (const key of ['bitrate', 'sampleRateHz', 'bitDepth', 'channelCount'] as const) {
    const value = raw[key]
    if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) continue
    if (key !== 'bitrate' && !Number.isInteger(value)) continue
    info[key] = value
  }
  if (typeof raw.codec === 'string' && raw.codec.trim() && raw.codec.trim().toLowerCase() !== 'unknown') {
    info.codec = raw.codec.trim()
  }
  if (Object.keys(info).length > 0) update(info)
}
