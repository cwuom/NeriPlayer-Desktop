import { invoke } from '@tauri-apps/api/core'
import type { AudioInfo } from '@/stores/player'

type LocalAudioInfo = Pick<AudioInfo, 'bitrate' | 'codec' | 'format' | 'sampleRateHz' | 'bitDepth' | 'channelCount'>

export async function loadLocalAudioInfo(
  path: string,
  isCurrent: () => boolean,
  update: (info: AudioInfo) => void,
): Promise<void> {
  if (!isCurrent()) return
  const filename = path.split(/[\\/]/).pop() || ''
  const extension = filename.includes('.') ? filename.split('.').pop()?.toLowerCase() : undefined
  const fallback: AudioInfo = { source: 'local', format: extension }
  update(fallback)
  const info = await invoke<LocalAudioInfo>('get_local_audio_info', { path })
  // 文件探测可以晚于切歌完成，旧结果不能覆盖当前音频信息
  if (isCurrent()) update({ ...fallback, ...info, source: 'local' })
}
