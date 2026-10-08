// 音频缓存按首选音质建键（对齐 Android），实际播放的可能是降级或候选流；
// 这里按同一个键记下实际音质，缓存命中时用它展示，而不是把首选音质当成实际音质
import { getCachedValue, setCachedValue } from '@/utils/persistentCache'

export interface PlayedQuality {
  qualityKey?: string
  codecLabel?: string
  bitrateKbps?: number
  mimeType?: string
}

const BUCKET = 'playback_quality'
const OPTIONS = {
  maxAgeMs: 180 * 24 * 60 * 60 * 1000,
  maxEntries: 5000,
  maxBytes: 2 * 1024 * 1024,
}

export function rememberPlayedQuality(cacheKey: string, quality: PlayedQuality): void {
  if (!cacheKey || !quality.qualityKey) return
  void setCachedValue(BUCKET, cacheKey, quality, OPTIONS)
}

export function recallPlayedQuality(cacheKey: string): Promise<PlayedQuality | null> {
  if (!cacheKey) return Promise.resolve(null)
  return getCachedValue<PlayedQuality>(BUCKET, cacheKey, OPTIONS.maxAgeMs)
}
